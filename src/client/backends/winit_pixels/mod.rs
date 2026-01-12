// Copyright 2024 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::HashMap;
use std::collections::HashSet;
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::thread;

use pixels::{Pixels, SurfaceTexture};
use raw_window_handle::DisplayHandle;
use raw_window_handle::HandleError;
use raw_window_handle::HasDisplayHandle;
use raw_window_handle::HasWindowHandle;
use raw_window_handle::RawDisplayHandle;
use raw_window_handle::RawWindowHandle;
use raw_window_handle::WindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::dpi::PhysicalPosition;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::CursorIcon;
use winit::window::Window;
use winit::window::WindowLevel;

use calloop::EventLoop as CalloopEventLoop;
use calloop::channel::Event as CalloopChannelEvent;
use tracing::{debug, warn};

use crate::client::config::KeyboardMode;
use crate::filtering;
use crate::prelude::*;
use crate::protocols::wprs as proto;
use crate::protocols::wprs::DisplayConfig;
use crate::protocols::wprs::RecvType;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::SendType;
use crate::protocols::wprs::Serializer;
use crate::protocols::wprs::geometry::{Point, Size};
use crate::protocols::wprs::wayland::{
    AxisScroll, AxisSource, BufferAssignment, BufferData, BufferFormat, Mode, OutputEvent,
    OutputInfo, Subpixel, SurfaceRequest, SurfaceRequestPayload, Transform, UncompressedBufferData,
    WlSurfaceId,
};
use crate::protocols::wprs::xdg_shell::XdgPopupState;
use crate::protocols::wprs::xdg_shell::{
    DecorationMode, ToplevelClose, ToplevelConfigure, ToplevelEvent, WindowState,
};

#[derive(Clone, Debug)]
pub struct WinitPixelsOptions {
    pub keyboard_mode: KeyboardMode,
    pub xkb_keymap_file: Option<std::path::PathBuf>,
    pub ui_scale_factor: f64,
}

#[derive(Debug)]
pub enum UserEvent {
    ServerMessage(RecvType<Request>),
    DecodedFrame(DecodedFrame),
}

#[derive(Debug)]
pub struct DecodedFrame {
    pub surface_id: WlSurfaceId,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
struct DecodeJob {
    surface_id: WlSurfaceId,
    metadata: crate::protocols::wprs::wayland::BufferMetadata,
    filtered: crate::vec4u8::Vec4u8s,
}

struct WindowRenderer {
    window: Arc<Window>,
    surface_handle: *mut OwnedSurface,
    pixels: ManuallyDrop<Pixels<'static>>,
    surface_size: (u32, u32),
    buffer_size: (u32, u32),
}

impl WindowRenderer {
    fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let surface_w = size.width.max(1);
        let surface_h = size.height.max(1);

        let surface_handle = Box::new(OwnedSurface::new(window.as_ref()).location(loc!())?);
        let surface_handle = Box::into_raw(surface_handle);
        let surface_ref: &'static OwnedSurface = unsafe { &*surface_handle };
        let surface_texture = SurfaceTexture::new(surface_w, surface_h, surface_ref);
        let pixels = Pixels::new(surface_w, surface_h, surface_texture)
            .map_err(|err| anyhow!("create pixels failed: {err:?}"))?;

        Ok(Self {
            window,
            surface_handle,
            pixels: ManuallyDrop::new(pixels),
            surface_size: (surface_w, surface_h),
            buffer_size: (1, 1),
        })
    }

    fn resize_surface(&mut self, size: PhysicalSize<u32>) -> Result<()> {
        let width = size.width.max(1);
        let height = size.height.max(1);
        if self.surface_size == (width, height) {
            return Ok(());
        }
        self.surface_size = (width, height);
        self.pixels
            .resize_surface(width, height)
            .map_err(|err| anyhow!("pixels resize_surface failed: {err:?}"))?;
        Ok(())
    }

    fn resize_buffer(&mut self, width: u32, height: u32) -> Result<()> {
        let width = width.max(1);
        let height = height.max(1);
        if self.buffer_size == (width, height) {
            return Ok(());
        }
        self.buffer_size = (width, height);
        self.pixels
            .resize_buffer(width, height)
            .map_err(|err| anyhow!("pixels resize_buffer failed: {err:?}"))?;
        Ok(())
    }

    fn update_frame(&mut self, frame: &DecodedFrame) -> Result<()> {
        self.resize_buffer(frame.width, frame.height).location(loc!())?;
        let dst = self.pixels.frame_mut();
        if dst.len() != frame.pixels.len() {
            return Ok(());
        }
        dst.copy_from_slice(&frame.pixels);
        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        self.pixels
            .render()
            .map_err(|err| anyhow!("pixels render failed: {err:?}"))?;
        Ok(())
    }
}

impl Drop for WindowRenderer {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.pixels) };
        if !self.surface_handle.is_null() {
            unsafe { drop(Box::from_raw(self.surface_handle)) };
            self.surface_handle = std::ptr::null_mut();
        }
    }
}

fn decode_filtered_to_rgba(
    metadata: &crate::protocols::wprs::wayland::BufferMetadata,
    filtered: &crate::vec4u8::Vec4u8s,
) -> Vec<u8> {
    let width = metadata.width as usize;
    let height = metadata.height as usize;
    let src_stride = metadata.stride as usize;
    let row_bytes = width * 4;

    let mut unfiltered = vec![0u8; metadata.len()];
    filtering::unfilter(filtered, &mut unfiltered);

    let mut rgba = vec![0u8; row_bytes * height];
    for y in 0..height {
        let src = &unfiltered[y * src_stride..y * src_stride + row_bytes];
        let dst = &mut rgba[y * row_bytes..y * row_bytes + row_bytes];
        for x in 0..width {
            let i = x * 4;
            let b = src[i];
            let g = src[i + 1];
            let r = src[i + 2];
            let a = match metadata.format {
                BufferFormat::Argb8888 => src[i + 3],
                BufferFormat::Xrgb8888 => 0xFF,
            };
            dst[i] = r;
            dst[i + 1] = g;
            dst[i + 2] = b;
            dst[i + 3] = a;
        }
    }

    rgba
}

fn output_info_from_monitor(id: u32, monitor: &winit::monitor::MonitorHandle) -> OutputInfo {
    let name = monitor.name();
    let scale_factor = monitor.scale_factor().round() as i32;
    let position = monitor.position();
    let size = monitor.size();
    OutputInfo {
        id,
        model: name.clone().unwrap_or_default(),
        make: String::new(),
        location: Point {
            x: position.x,
            y: position.y,
        },
        physical_size: Size { w: 0, h: 0 },
        subpixel: Subpixel::Unknown,
        transform: Transform::Normal,
        scale_factor: scale_factor.max(1),
        mode: Mode {
            dimensions: Size {
                w: size.width as i32,
                h: size.height as i32,
            },
            refresh_rate: 60_000,
            current: true,
            preferred: true,
        },
        name,
        description: None,
    }
}

struct App {
    serializer: Serializer<proto::Event, Request>,
    decode_tx: std::sync::mpsc::Sender<DecodeJob>,
    buffer_cache: Option<UncompressedBufferData>,
    windows: HashMap<WlSurfaceId, WindowRenderer>,
    surface_by_window: HashMap<winit::window::WindowId, WlSurfaceId>,
    outputs_sent: bool,

    server_display_config: Option<DisplayConfig>,
    surface_scale_factor: HashMap<WlSurfaceId, i32>,

    keyboard_mode: KeyboardMode,
    xkb_keymap_sent: bool,
    xkb_keymap_file: Option<std::path::PathBuf>,
    ui_scale_factor: f64,

    serial_counter: u32,
    focused_surface: Option<WlSurfaceId>,
    surfaces_with_frame: HashSet<WlSurfaceId>,
    pressed_keycodes: HashSet<u32>,
    last_cursor_pos: HashMap<winit::window::WindowId, crate::protocols::wprs::geometry::Point<f64>>,
    pointer_inside: HashSet<winit::window::WindowId>,
    pointer_surface: Option<WlSurfaceId>,

    popup_state_by_surface: HashMap<WlSurfaceId, XdgPopupState>,
}

impl App {
    fn ui_scale(&self) -> f64 {
        self.ui_scale_factor.max(0.1)
    }

    fn to_remote_surface_coords(
        &self,
        window_logical: crate::protocols::wprs::geometry::Point<f64>,
    ) -> crate::protocols::wprs::geometry::Point<f64> {
        let scale = self.ui_scale();
        crate::protocols::wprs::geometry::Point {
            x: window_logical.x / scale,
            y: window_logical.y / scale,
        }
    }

    fn schedule_decode(
        &self,
        surface_id: WlSurfaceId,
        metadata: crate::protocols::wprs::wayland::BufferMetadata,
        filtered: crate::vec4u8::Vec4u8s,
    ) {
        // If the receiver is gone, we are shutting down.
        let _ = self.decode_tx.send(DecodeJob {
            surface_id,
            metadata,
            filtered,
        });
    }

    fn compute_popup_position(&self, popup: &XdgPopupState) -> Option<PhysicalPosition<i32>> {
        let parent_renderer = self.windows.get(&popup.parent_surface_id)?;
        let parent_pos = parent_renderer
            .window
            .inner_position()
            .or_else(|_| parent_renderer.window.outer_position())
            .ok()?;

        let anchor = popup.positioner.anchor_rect;
        let offset = popup.positioner.offset;

        let client_scale = parent_renderer.window.scale_factor();
        let total_scale = client_scale * self.ui_scale();

        let dx = (anchor.loc.x + offset.x) as f64 * total_scale;
        let dy = (anchor.loc.y + offset.y) as f64 * total_scale;

        Some(PhysicalPosition::new(
            parent_pos.x.saturating_add(dx.round() as i32),
            parent_pos.y.saturating_add(dy.round() as i32),
        ))
    }

    fn update_popup_position(&self, popup_surface_id: WlSurfaceId, popup: &XdgPopupState) {
        let Some(renderer) = self.windows.get(&popup_surface_id) else {
            return;
        };
        let Some(pos) = self.compute_popup_position(popup) else {
            return;
        };
        renderer.window.set_outer_position(pos);
    }

    fn update_popups_for_parent(&self, parent_surface_id: WlSurfaceId) {
        for (popup_surface_id, popup) in &self.popup_state_by_surface {
            if popup.parent_surface_id == parent_surface_id {
                self.update_popup_position(*popup_surface_id, popup);
            }
        }
    }

    fn cursor_icon_from_wayland_name(name: &str) -> CursorIcon {
        match name {
            "default" | "left_ptr" | "arrow" => CursorIcon::Default,
            "pointer" | "hand" | "hand1" | "hand2" => CursorIcon::Pointer,
            "text" | "xterm" | "ibeam" => CursorIcon::Text,
            "crosshair" => CursorIcon::Crosshair,
            "move" | "all-scroll" => CursorIcon::Move,
            "not-allowed" | "forbidden" => CursorIcon::NotAllowed,
            "wait" | "watch" => CursorIcon::Wait,
            "progress" | "left_ptr_watch" => CursorIcon::Progress,
            "help" | "question_arrow" => CursorIcon::Help,
            "context-menu" => CursorIcon::ContextMenu,

            "e-resize" => CursorIcon::EResize,
            "w-resize" => CursorIcon::WResize,
            "n-resize" => CursorIcon::NResize,
            "s-resize" => CursorIcon::SResize,
            "ne-resize" => CursorIcon::NeResize,
            "nw-resize" => CursorIcon::NwResize,
            "se-resize" => CursorIcon::SeResize,
            "sw-resize" => CursorIcon::SwResize,
            "col-resize" | "ew-resize" | "sb_h_double_arrow" => CursorIcon::EwResize,
            "row-resize" | "ns-resize" | "sb_v_double_arrow" => CursorIcon::NsResize,

            "grab" => CursorIcon::Grab,
            "grabbing" => CursorIcon::Grabbing,
            "zoom-in" => CursorIcon::ZoomIn,
            "zoom-out" => CursorIcon::ZoomOut,

            _ => CursorIcon::Default,
        }
    }

    fn handle_cursor_image(&mut self, cursor: crate::protocols::wprs::wayland::CursorImage) {
        let serial = cursor.serial;
        let status = cursor.status;

        let target_surface = self.pointer_surface.or(self.focused_surface);
        let Some(surface_id) = target_surface else {
            debug!(
                "cursor image update without an active window: serial={serial} status={status:?}"
            );
            return;
        };
        let Some(renderer) = self.windows.get(&surface_id) else {
            return;
        };

        match status {
            crate::protocols::wprs::wayland::CursorImageStatus::Hidden => {
                debug!("cursor hidden: surface={surface_id:?} serial={serial}");
                renderer.window.set_cursor_visible(false);
            },
            crate::protocols::wprs::wayland::CursorImageStatus::Named(name) => {
                let icon = Self::cursor_icon_from_wayland_name(&name);
                debug!(
                    "cursor named: surface={surface_id:?} serial={} name={name:?} icon={icon:?}",
                    serial
                );
                renderer.window.set_cursor_visible(true);
                renderer.window.set_cursor(icon);
            },
            crate::protocols::wprs::wayland::CursorImageStatus::Surface { .. } => {
                debug!(
                    "cursor surface: surface={surface_id:?} serial={} (custom cursor unsupported in winit backend)",
                    serial
                );
                renderer.window.set_cursor_visible(true);
                renderer.window.set_cursor(CursorIcon::Default);
            },
        }
    }

    fn generate_keymap_from_tools() -> Result<String> {
        let setxkbmap_output = std::process::Command::new("setxkbmap")
            .args(["-print"])
            .output()
            .location(loc!())?;
        if !setxkbmap_output.status.success() {
            bail!("setxkbmap -print failed: {:?}", setxkbmap_output.status);
        }

        let mut xkbcomp = std::process::Command::new("xkbcomp")
            .args(["-xkb", "-", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .location(loc!())?;

        {
            let stdin = xkbcomp.stdin.as_mut().location(loc!())?;
            use std::io::Write as _;
            stdin.write_all(&setxkbmap_output.stdout).location(loc!())?;
        }

        let output = xkbcomp.wait_with_output().location(loc!())?;
        if !output.status.success() {
            bail!(
                "xkbcomp -xkb - - failed: {:?}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8(output.stdout).location(loc!())?)
    }

    fn maybe_send_keymap(&mut self) {
        if self.xkb_keymap_sent {
            return;
        }
        self.xkb_keymap_sent = true;

        if self.keyboard_mode != KeyboardMode::Keymap {
            return;
        }

        let keymap = if let Some(path) = self.xkb_keymap_file.as_ref() {
            match std::fs::read_to_string(path).with_context(loc!(), || {
                format!("failed to read xkb keymap file {path:?}")
            }) {
                Ok(keymap) => keymap,
                Err(err) => {
                    warn!("{err:?}; continuing with evdev mapping");
                    return;
                },
            }
        } else {
            match Self::generate_keymap_from_tools() {
                Ok(keymap) => keymap,
                Err(err) => {
                    warn!(
                        "failed to generate xkb keymap via tools: {err:?}; continuing with evdev mapping"
                    );
                    return;
                },
            }
        };

        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::KeyboardEvent(
                crate::protocols::wprs::wayland::KeyboardEvent::Keymap(keymap),
            )));
    }

    fn next_serial(&mut self) -> u32 {
        self.serial_counter = self.serial_counter.wrapping_add(1);
        if self.serial_counter == 0 {
            self.serial_counter = 1;
        }
        self.serial_counter
    }

    fn cursor_pos_for(
        &self,
        window_id: winit::window::WindowId,
    ) -> crate::protocols::wprs::geometry::Point<f64> {
        self.last_cursor_pos
            .get(&window_id)
            .copied()
            .unwrap_or(crate::protocols::wprs::geometry::Point { x: 0.0, y: 0.0 })
    }

    fn send_pointer_event(
        &mut self,
        surface_id: WlSurfaceId,
        position: crate::protocols::wprs::geometry::Point<f64>,
        kind: crate::protocols::wprs::wayland::PointerEventKind,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::PointerFrame(vec![
                crate::protocols::wprs::wayland::PointerEvent {
                    surface_id,
                    position,
                    kind,
                },
            ])));
    }

    fn set_keyboard_focus(&mut self, surface_id: Option<WlSurfaceId>) {
        self.maybe_send_keymap();
        if self.focused_surface == surface_id {
            return;
        }
        if self.focused_surface.is_some() {
            let serial = self.next_serial();
            self.serializer
                .writer()
                .send(SendType::Object(proto::Event::KeyboardEvent(
                    crate::protocols::wprs::wayland::KeyboardEvent::Leave { serial },
                )));
        }

        self.focused_surface = surface_id;

        if let Some(surface_id) = surface_id {
            let mut keycodes: Vec<u32> = self.pressed_keycodes.iter().copied().collect();
            keycodes.sort_unstable();
            let serial = self.next_serial();
            self.serializer
                .writer()
                .send(SendType::Object(proto::Event::KeyboardEvent(
                    crate::protocols::wprs::wayland::KeyboardEvent::Enter {
                        serial,
                        surface_id,
                        keycodes,
                        keysyms: Vec::new(),
                    },
                )));
        }
    }

    fn send_modifiers(&mut self, modifiers: winit::keyboard::ModifiersState) {
        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::KeyboardEvent(
                crate::protocols::wprs::wayland::KeyboardEvent::Modifiers {
                    modifier_state: crate::protocols::wprs::wayland::ModifierState {
                        ctrl: modifiers.control_key(),
                        alt: modifiers.alt_key(),
                        shift: modifiers.shift_key(),
                        // winit doesn't expose lock states in ModifiersState.
                        caps_lock: false,
                        logo: modifiers.super_key(),
                        num_lock: false,
                    },
                    layout_index: 0,
                },
            )));
    }

    fn send_key(&mut self, keycode: u32, state: crate::protocols::wprs::wayland::KeyState) {
        if self.focused_surface.is_none() {
            return;
        }
        let serial = self.next_serial();
        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::KeyboardEvent(
                crate::protocols::wprs::wayland::KeyboardEvent::Key(
                    crate::protocols::wprs::wayland::KeyInner {
                        serial,
                        raw_code: keycode,
                        state,
                    },
                ),
            )));
    }

    fn linux_button_from_winit(button: MouseButton) -> Option<u32> {
        match button {
            MouseButton::Left => Some(272),
            MouseButton::Right => Some(273),
            MouseButton::Middle => Some(274),
            MouseButton::Back => Some(275),
            MouseButton::Forward => Some(276),
            MouseButton::Other(_) => None,
        }
    }

    fn linux_keycode_from_winit(code: winit::keyboard::KeyCode) -> Option<u32> {
        use winit::keyboard::KeyCode;

        Some(match code {
            KeyCode::Escape => 1,
            KeyCode::Digit1 => 2,
            KeyCode::Digit2 => 3,
            KeyCode::Digit3 => 4,
            KeyCode::Digit4 => 5,
            KeyCode::Digit5 => 6,
            KeyCode::Digit6 => 7,
            KeyCode::Digit7 => 8,
            KeyCode::Digit8 => 9,
            KeyCode::Digit9 => 10,
            KeyCode::Digit0 => 11,
            KeyCode::Minus => 12,
            KeyCode::Equal => 13,
            KeyCode::Backspace => 14,
            KeyCode::Tab => 15,
            KeyCode::KeyQ => 16,
            KeyCode::KeyW => 17,
            KeyCode::KeyE => 18,
            KeyCode::KeyR => 19,
            KeyCode::KeyT => 20,
            KeyCode::KeyY => 21,
            KeyCode::KeyU => 22,
            KeyCode::KeyI => 23,
            KeyCode::KeyO => 24,
            KeyCode::KeyP => 25,
            KeyCode::BracketLeft => 26,
            KeyCode::BracketRight => 27,
            KeyCode::Enter => 28,
            KeyCode::ControlLeft => 29,
            KeyCode::KeyA => 30,
            KeyCode::KeyS => 31,
            KeyCode::KeyD => 32,
            KeyCode::KeyF => 33,
            KeyCode::KeyG => 34,
            KeyCode::KeyH => 35,
            KeyCode::KeyJ => 36,
            KeyCode::KeyK => 37,
            KeyCode::KeyL => 38,
            KeyCode::Semicolon => 39,
            KeyCode::Quote => 40,
            KeyCode::Backquote => 41,
            KeyCode::ShiftLeft => 42,
            KeyCode::Backslash => 43,
            KeyCode::KeyZ => 44,
            KeyCode::KeyX => 45,
            KeyCode::KeyC => 46,
            KeyCode::KeyV => 47,
            KeyCode::KeyB => 48,
            KeyCode::KeyN => 49,
            KeyCode::KeyM => 50,
            KeyCode::Comma => 51,
            KeyCode::Period => 52,
            KeyCode::Slash => 53,
            KeyCode::ShiftRight => 54,
            KeyCode::NumpadMultiply => 55,
            KeyCode::AltLeft => 56,
            KeyCode::Space => 57,
            KeyCode::CapsLock => 58,
            KeyCode::F1 => 59,
            KeyCode::F2 => 60,
            KeyCode::F3 => 61,
            KeyCode::F4 => 62,
            KeyCode::F5 => 63,
            KeyCode::F6 => 64,
            KeyCode::F7 => 65,
            KeyCode::F8 => 66,
            KeyCode::F9 => 67,
            KeyCode::F10 => 68,
            KeyCode::NumLock => 69,
            KeyCode::ScrollLock => 70,
            KeyCode::Numpad7 => 71,
            KeyCode::Numpad8 => 72,
            KeyCode::Numpad9 => 73,
            KeyCode::NumpadSubtract => 74,
            KeyCode::Numpad4 => 75,
            KeyCode::Numpad5 => 76,
            KeyCode::Numpad6 => 77,
            KeyCode::NumpadAdd => 78,
            KeyCode::Numpad1 => 79,
            KeyCode::Numpad2 => 80,
            KeyCode::Numpad3 => 81,
            KeyCode::Numpad0 => 82,
            KeyCode::NumpadDecimal => 83,
            KeyCode::F11 => 87,
            KeyCode::F12 => 88,
            KeyCode::NumpadEnter => 96,
            KeyCode::ControlRight => 97,
            KeyCode::NumpadDivide => 98,
            KeyCode::AltRight => 100,
            KeyCode::Home => 102,
            KeyCode::ArrowUp => 103,
            KeyCode::PageUp => 104,
            KeyCode::ArrowLeft => 105,
            KeyCode::ArrowRight => 106,
            KeyCode::End => 107,
            KeyCode::ArrowDown => 108,
            KeyCode::PageDown => 109,
            KeyCode::Insert => 110,
            KeyCode::Delete => 111,
            KeyCode::SuperLeft => 125,
            KeyCode::SuperRight => 126,
            _ => return None,
        })
    }

    fn handle_server_message(
        &mut self,
        event_loop: &ActiveEventLoop,
        msg: RecvType<Request>,
    ) -> Result<()> {
        match msg {
            RecvType::RawBuffer(buf) => {
                self.buffer_cache = Some(UncompressedBufferData(buf.into()));
                Ok(())
            },
            RecvType::Object(Request::Surface(surface)) => self.handle_surface(event_loop, surface),
            RecvType::Object(Request::DisplayConfig(cfg)) => {
                if self.server_display_config.is_none() {
                    info!(
                        "server display config: scale_factor={} dpi={:?}",
                        cfg.scale_factor, cfg.dpi
                    );
                }
                self.server_display_config = Some(cfg);
                Ok(())
            },
            RecvType::Object(Request::CursorImage(cursor)) => {
                self.handle_cursor_image(cursor);
                Ok(())
            },
            _ => Ok(()),
        }
    }

    fn send_configure_for_surface(&mut self, surface_id: WlSurfaceId) {
        let Some(renderer) = self.windows.get(&surface_id) else {
            return;
        };
        let size = renderer.window.inner_size();
        let logical: winit::dpi::LogicalSize<f64> = size.to_logical(renderer.window.scale_factor());
        let server_logical_w = (logical.width / self.ui_scale()).round().max(1.0) as u32;
        let server_logical_h = (logical.height / self.ui_scale()).round().max(1.0) as u32;
        let configure = ToplevelConfigure {
            surface_id,
            new_size: Size {
                w: NonZeroU32::new(server_logical_w),
                h: NonZeroU32::new(server_logical_h),
            },
            suggested_bounds: None,
            decoration_mode: DecorationMode::Server,
            state: WindowState::from_bits(0),
        };
        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::Toplevel(
                ToplevelEvent::Configure(configure),
            )));
    }

    fn handle_surface(
        &mut self,
        event_loop: &ActiveEventLoop,
        surface: SurfaceRequest,
    ) -> Result<()> {
        let surface_id = surface.surface;
        match surface.payload {
            SurfaceRequestPayload::Destroyed => {
                if let Some(renderer) = self.windows.remove(&surface_id) {
                    self.surface_by_window.remove(&renderer.window.id());
                }
                self.popup_state_by_surface.remove(&surface_id);
                return Ok(());
            },
            SurfaceRequestPayload::Commit(mut state) => {
                self.surface_scale_factor
                    .insert(surface_id, state.buffer_scale.max(1));

                let Some(role) = &state.role else {
                    return Ok(());
                };
                let toplevel = role.as_xdg_toplevel();
                let popup = role.as_xdg_popup();
                if toplevel.is_none() && popup.is_none() {
                    return Ok(());
                }

                if let Some(popup) = popup {
                    self.popup_state_by_surface
                        .insert(surface_id, popup.clone());
                } else {
                    self.popup_state_by_surface.remove(&surface_id);
                }

                if !self.windows.contains_key(&surface_id) {
                    let mut attrs = if let Some(toplevel) = toplevel {
                        let title = toplevel.title.clone().unwrap_or_else(|| "wprs".to_string());
                        Window::default_attributes().with_title(title)
                    } else {
                        Window::default_attributes()
                            .with_decorations(false)
                            .with_resizable(false)
                            .with_window_level(WindowLevel::AlwaysOnTop)
                    };

                    if let Some(BufferAssignment::New(buf)) = &state.buffer {
                        let w = buf.metadata.width.max(1) as u32;
                        let h = buf.metadata.height.max(1) as u32;
                        let server_scale = state.buffer_scale.max(1) as f64;
                        let logical_w = (f64::from(w) / server_scale).max(1.0);
                        let logical_h = (f64::from(h) / server_scale).max(1.0);
                        attrs = attrs.with_inner_size(LogicalSize::new(
                            logical_w * self.ui_scale(),
                            logical_h * self.ui_scale(),
                        ));

                        info!(
                            "creating window: surface={surface_id:?} kind={} buffer_px=({w}x{h}) buffer_scale={} ui_scale_factor={}",
                            if toplevel.is_some() {
                                "toplevel"
                            } else {
                                "popup"
                            },
                            state.buffer_scale,
                            self.ui_scale_factor
                        );
                    } else if let Some(popup) = popup {
                        attrs = attrs.with_inner_size(LogicalSize::new(
                            (popup.positioner.width.max(1) as f64) * self.ui_scale(),
                            (popup.positioner.height.max(1) as f64) * self.ui_scale(),
                        ));

                        info!(
                            "creating window: surface={surface_id:?} kind=popup positioner_px=({}x{}) ui_scale_factor={}",
                            popup.positioner.width, popup.positioner.height, self.ui_scale_factor
                        );
                    }

                    let window = Arc::new(event_loop.create_window(attrs).location(loc!())?);
                    let renderer = WindowRenderer::new(window.clone()).location(loc!())?;
                    let window_id = window.id();
                    self.surface_by_window.insert(window_id, surface_id);
                    self.windows.insert(surface_id, renderer);

                    if let Some(popup) = popup {
                        if let Some(pos) = self.compute_popup_position(popup) {
                            let renderer = self.windows.get(&surface_id).expect("renderer");
                            renderer.window.set_outer_position(pos);
                        }
                    }

                    if toplevel.is_some() {
                        self.send_configure_for_surface(surface_id);
                    }
                }

                if let Some(popup) = popup {
                    self.update_popup_position(surface_id, popup);
                }

                if let Some(BufferAssignment::New(mut buf)) = state.buffer.take() {
                    if buf.data.is_external() {
                        if let Some(cache) = self.buffer_cache.take() {
                            buf.data = BufferData::Uncompressed(cache);
                        }
                    }
                    let filtered = match buf.data {
                        BufferData::Uncompressed(data) => data.0,
                        BufferData::Compressed(_) => {
                            warn!(
                                "Received buffer commit with inline Compressed data; skipping frame for {surface_id:?}"
                            );
                            return Ok(());
                        },
                        BufferData::External => {
                            if self.surfaces_with_frame.contains(&surface_id) {
                                warn!(
                                    "Received buffer commit with External data (no cached RawBuffer); skipping frame for {surface_id:?}"
                                );
                            } else {
                                debug!(
                                    "Received initial External buffer commit without a cached RawBuffer; waiting for first frame for {surface_id:?}"
                                );
                            }
                            return Ok(());
                        },
                    };
                    self.schedule_decode(surface_id, buf.metadata, filtered);
                }
                Ok(())
            },
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.outputs_sent {
            return;
        }
        self.outputs_sent = true;

        self.maybe_send_keymap();
        for (idx, monitor) in event_loop.available_monitors().enumerate() {
            self.serializer
                .writer()
                .send(SendType::Object(proto::Event::Output(OutputEvent::New(
                    output_info_from_monitor(idx as u32, &monitor),
                ))));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::ServerMessage(msg) => {
                self.handle_server_message(event_loop, msg)
                    .log_and_ignore(loc!());
            },
            UserEvent::DecodedFrame(frame) => {
                let Some(renderer) = self.windows.get_mut(&frame.surface_id) else {
                    return;
                };
                renderer.update_frame(&frame).log_and_ignore(loc!());
                renderer.window.request_redraw();
                self.surfaces_with_frame.insert(frame.surface_id);
            },
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let surface_id = self.surface_by_window.get(&window_id).copied();
        if surface_id.is_none() {
            return;
        }
        let surface_id = surface_id.unwrap();

        if let Some(renderer) = self.windows.get_mut(&surface_id) {
            match &event {
                WindowEvent::Resized(size) => {
                    renderer.resize_surface(*size).log_and_ignore(loc!());
                    debug!("winit window resized: surface={surface_id:?} size={size:?}");
                    self.send_configure_for_surface(surface_id);
                    self.update_popups_for_parent(surface_id);
                },
                WindowEvent::ScaleFactorChanged { .. } => {
                    debug!("winit window scale factor changed: surface={surface_id:?}");
                    self.send_configure_for_surface(surface_id);
                    self.update_popups_for_parent(surface_id);
                },
                WindowEvent::Moved(_) => {
                    self.update_popups_for_parent(surface_id);
                },
                WindowEvent::RedrawRequested => {
                    renderer.render().log_and_ignore(loc!());
                },
                WindowEvent::CloseRequested => {
                    debug!("winit close requested: surface={surface_id:?}");
                    self.serializer
                        .writer()
                        .send(SendType::Object(proto::Event::Toplevel(
                            ToplevelEvent::Close(ToplevelClose { surface_id }),
                        )));
                    self.windows.remove(&surface_id);
                    self.surface_by_window.remove(&window_id);
                    if self.focused_surface == Some(surface_id) {
                        self.set_keyboard_focus(None);
                    }
                },
                _ => {},
            }
        }

        match event {
            WindowEvent::Focused(true) => {
                self.set_keyboard_focus(Some(surface_id));
            },
            WindowEvent::Focused(false) => {
                if self.focused_surface == Some(surface_id) {
                    self.set_keyboard_focus(None);
                }
            },
            WindowEvent::ModifiersChanged(modifiers) => {
                self.send_modifiers(modifiers.state());
            },
            WindowEvent::KeyboardInput { event, .. } => {
                let winit::keyboard::PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let Some(linux_keycode) = Self::linux_keycode_from_winit(code) else {
                    debug!("unmapped keycode {code:?}");
                    return;
                };

                let state = match (event.state, event.repeat) {
                    (ElementState::Pressed, true) => {
                        crate::protocols::wprs::wayland::KeyState::Repeated
                    },
                    (ElementState::Pressed, false) => {
                        crate::protocols::wprs::wayland::KeyState::Pressed
                    },
                    (ElementState::Released, _) => {
                        crate::protocols::wprs::wayland::KeyState::Released
                    },
                };

                match state {
                    crate::protocols::wprs::wayland::KeyState::Pressed
                    | crate::protocols::wprs::wayland::KeyState::Repeated => {
                        self.pressed_keycodes.insert(linux_keycode);
                    },
                    crate::protocols::wprs::wayland::KeyState::Released => {
                        self.pressed_keycodes.remove(&linux_keycode);
                    },
                }
                self.send_key(linux_keycode, state);
            },
            WindowEvent::CursorMoved { position, .. } => {
                let Some(renderer) = self.windows.get(&surface_id) else {
                    return;
                };
                let logical = position.to_logical::<f64>(renderer.window.scale_factor());
                let window_pos = crate::protocols::wprs::geometry::Point {
                    x: logical.x,
                    y: logical.y,
                };
                let pos = self.to_remote_surface_coords(window_pos);
                self.last_cursor_pos.insert(window_id, pos);
                self.pointer_surface = Some(surface_id);

                if self.pointer_inside.insert(window_id) {
                    let serial = self.next_serial();
                    self.send_pointer_event(
                        surface_id,
                        pos,
                        crate::protocols::wprs::wayland::PointerEventKind::Enter { serial },
                    );
                }
                self.send_pointer_event(
                    surface_id,
                    pos,
                    crate::protocols::wprs::wayland::PointerEventKind::Motion,
                );
            },
            WindowEvent::CursorEntered { .. } => {
                let pos = self.cursor_pos_for(window_id);
                let serial = self.next_serial();
                self.pointer_inside.insert(window_id);
                self.pointer_surface = Some(surface_id);
                self.send_pointer_event(
                    surface_id,
                    pos,
                    crate::protocols::wprs::wayland::PointerEventKind::Enter { serial },
                );
            },
            WindowEvent::CursorLeft { .. } => {
                let pos = self.cursor_pos_for(window_id);
                let serial = self.next_serial();
                self.pointer_inside.remove(&window_id);
                if self.pointer_surface == Some(surface_id) {
                    self.pointer_surface = None;
                }
                self.send_pointer_event(
                    surface_id,
                    pos,
                    crate::protocols::wprs::wayland::PointerEventKind::Leave { serial },
                );
            },
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = Self::linux_button_from_winit(button) else {
                    return;
                };
                let pos = self.cursor_pos_for(window_id);
                let kind = match state {
                    ElementState::Pressed => {
                        crate::protocols::wprs::wayland::PointerEventKind::Press {
                            button,
                            serial: self.next_serial(),
                        }
                    },
                    ElementState::Released => {
                        crate::protocols::wprs::wayland::PointerEventKind::Release {
                            button,
                            serial: self.next_serial(),
                        }
                    },
                };
                self.send_pointer_event(surface_id, pos, kind);
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let (h_abs, v_abs, h_discrete, v_discrete, source) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        let v120_x = (x * 120.0) as i32;
                        let v120_y = (y * 120.0) as i32;
                        (
                            f64::from(x) * 15.0,
                            f64::from(y) * 15.0,
                            v120_x,
                            v120_y,
                            Some(AxisSource::Wheel),
                        )
                    },
                    MouseScrollDelta::PixelDelta(pos) => {
                        let Some(renderer) = self.windows.get(&surface_id) else {
                            return;
                        };
                        let logical = pos.to_logical::<f64>(renderer.window.scale_factor());
                        let dx = logical.x / self.ui_scale();
                        let dy = logical.y / self.ui_scale();
                        (dx, dy, 0, 0, Some(AxisSource::Finger))
                    },
                };
                let pos = self.cursor_pos_for(window_id);

                debug!(
                    "scroll: surface={surface_id:?} source={source:?} h_abs={h_abs:.2} v_abs={v_abs:.2} h_discrete={h_discrete} v_discrete={v_discrete}"
                );
                self.send_pointer_event(
                    surface_id,
                    pos,
                    crate::protocols::wprs::wayland::PointerEventKind::Axis {
                        horizontal: AxisScroll {
                            absolute: h_abs,
                            discrete: h_discrete,
                            stop: false,
                        },
                        vertical: AxisScroll {
                            absolute: v_abs,
                            discrete: v_discrete,
                            stop: false,
                        },
                        source,
                    },
                );
            },
            _ => {},
        }
    }
}

pub fn run(
    mut serializer: Serializer<proto::Event, Request>,
    options: WinitPixelsOptions,
) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    let (decode_tx, decode_rx) = std::sync::mpsc::channel::<DecodeJob>();
    {
        let proxy = proxy.clone();
        thread::spawn(move || {
            while let Ok(job) = decode_rx.recv() {
                let pixels = decode_filtered_to_rgba(&job.metadata, &job.filtered);
                let _ = proxy.send_event(UserEvent::DecodedFrame(DecodedFrame {
                    surface_id: job.surface_id,
                    width: job.metadata.width.max(1) as u32,
                    height: job.metadata.height.max(1) as u32,
                    pixels,
                }));
            }
        });
    }

    let reader = serializer.reader().location(loc!())?;
    let proxy_for_reader = proxy.clone();
    thread::spawn(move || {
        let mut loop_: CalloopEventLoop<()> = CalloopEventLoop::try_new().expect("calloop init");
        loop_
            .handle()
            .insert_source(reader, move |event, _metadata, _state| {
                if let CalloopChannelEvent::Msg(msg) = event {
                    proxy_for_reader.send_event(UserEvent::ServerMessage(msg)).ok();
                }
            })
            .expect("insert serializer reader");

        loop_.run(None, &mut (), |_| {}).ok();
    });

    let mut app = App {
        serializer,
        decode_tx,
        buffer_cache: None,
        windows: HashMap::new(),
        surface_by_window: HashMap::new(),
        outputs_sent: false,

        server_display_config: None,
        surface_scale_factor: HashMap::new(),

        keyboard_mode: options.keyboard_mode,
        xkb_keymap_sent: false,
        xkb_keymap_file: options.xkb_keymap_file,
        ui_scale_factor: options.ui_scale_factor,

        serial_counter: 1,
        focused_surface: None,
        surfaces_with_frame: HashSet::new(),
        pressed_keycodes: HashSet::new(),
        last_cursor_pos: HashMap::new(),
        pointer_inside: HashSet::new(),
        pointer_surface: None,

        popup_state_by_surface: HashMap::new(),
    };

    event_loop.run_app(&mut app)?;
    Ok(())
}

#[derive(Clone, Debug)]
struct OwnedDisplay {
    raw: RawDisplayHandle,
}

// Raw display handles are just opaque pointers/IDs.
unsafe impl Send for OwnedDisplay {}
unsafe impl Sync for OwnedDisplay {}

impl OwnedDisplay {
    fn new(window: &winit::window::Window) -> Result<Self> {
        let handle = window
            .display_handle()
            .map_err(|err| anyhow!("display handle failed: {err:?}"))?;
        Ok(Self {
            raw: handle.as_raw(),
        })
    }
}

impl HasDisplayHandle for OwnedDisplay {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe { DisplayHandle::borrow_raw(self.raw) })
    }
}

#[derive(Clone, Debug)]
struct OwnedWindow {
    raw: RawWindowHandle,
}

// Raw window handles are just opaque pointers/IDs.
unsafe impl Send for OwnedWindow {}
unsafe impl Sync for OwnedWindow {}

impl OwnedWindow {
    fn new(window: &winit::window::Window) -> Result<Self> {
        let handle = window
            .window_handle()
            .map_err(|err| anyhow!("window handle failed: {err:?}"))?;
        Ok(Self {
            raw: handle.as_raw(),
        })
    }
}

impl HasWindowHandle for OwnedWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe { WindowHandle::borrow_raw(self.raw) })
    }
}

#[derive(Clone, Debug)]
struct OwnedSurface {
    window: OwnedWindow,
    display: OwnedDisplay,
}

// `pixels`/`wgpu` requires the window handle provider to be Send + Sync.
unsafe impl Send for OwnedSurface {}
unsafe impl Sync for OwnedSurface {}

impl OwnedSurface {
    fn new(window: &winit::window::Window) -> Result<Self> {
        Ok(Self {
            window: OwnedWindow::new(window).location(loc!())?,
            display: OwnedDisplay::new(window).location(loc!())?,
        })
    }
}

impl HasWindowHandle for OwnedSurface {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for OwnedSurface {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        self.display.display_handle()
    }
}

#[derive(Debug, Clone)]
pub struct WinitPixelsClientBackend {
    options: WinitPixelsOptions,
}

impl WinitPixelsClientBackend {
    pub fn new(config: crate::client::backend::ClientBackendConfig) -> Self {
        Self {
            options: WinitPixelsOptions {
                keyboard_mode: config.keyboard_mode,
                xkb_keymap_file: config.xkb_keymap_file,
                ui_scale_factor: config.ui_scale_factor,
            },
        }
    }
}

impl crate::client::backend::ClientBackend for WinitPixelsClientBackend {
    fn name(&self) -> &'static str {
        "winit-pixels"
    }

    fn run(self: Box<Self>, serializer: Serializer<proto::Event, proto::Request>) -> Result<()> {
        run(serializer, self.options).location(loc!())
    }
}
