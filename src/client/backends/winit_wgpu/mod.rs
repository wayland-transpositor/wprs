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
use std::num::NonZeroU32;
use std::sync::Arc;
use std::thread;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::Window;

use calloop::EventLoop as CalloopEventLoop;
use calloop::channel::Event as CalloopChannelEvent;
use tracing::{debug, warn};

use crate::client::config::KeyboardMode;
use crate::filtering;
use crate::prelude::*;
use crate::protocols::wprs as proto;
use crate::protocols::wprs::RecvType;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::SendType;
use crate::protocols::wprs::Serializer;
use crate::protocols::wprs::geometry::{Point, Size};
use crate::protocols::wprs::wayland::{
    AxisScroll, AxisSource, KeyInner, KeyState, KeyboardEvent, ModifierState, PointerEvent,
    PointerEventKind,
};
use crate::protocols::wprs::wayland::{
    BufferAssignment, BufferData, Mode, OutputEvent, OutputInfo, Subpixel, SurfaceRequest,
    SurfaceRequestPayload, Transform, UncompressedBufferData, WlSurfaceId,
};
use crate::protocols::wprs::xdg_shell::{
    DecorationMode, ToplevelClose, ToplevelConfigure, ToplevelEvent, WindowState,
};

#[derive(Clone, Debug)]
pub struct WinitWgpuOptions {
    pub keyboard_mode: KeyboardMode,
    pub xkb_keymap_file: Option<std::path::PathBuf>,
}

#[derive(Debug)]
pub enum UserEvent {
    ServerMessage(RecvType<Request>),
}

#[derive(Clone)]
struct WgpuShared {
    instance: Arc<wgpu::Instance>,
    adapter: Arc<wgpu::Adapter>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

struct WindowRenderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,

    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    texture: Option<wgpu::Texture>,
    texture_size: Option<(u32, u32)>,

    scratch_unfiltered: Vec<u8>,
    scratch_padded: Vec<u8>,
}

impl WindowRenderer {
    fn new(shared: &WgpuShared, window: Arc<Window>) -> Result<Self> {
        let surface = shared
            .instance
            .create_surface(window.clone())
            .location(loc!())?;

        let caps = surface.get_capabilities(&shared.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&shared.device, &config);

        let shader = shared
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("wprs_winit_wgpu_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
            });

        let bind_group_layout =
            shared
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("wprs_winit_wgpu_bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                multisampled: false,
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let sampler = shared.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wprs_winit_wgpu_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let pipeline_layout =
            shared
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("wprs_winit_wgpu_pipeline_layout"),
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                });

        let pipeline = shared
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("wprs_winit_wgpu_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 1,
                            },
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let vertex_data: &[f32] = &[
            // pos(x,y) uv(u,v)
            -1.0, -1.0, 0.0, 1.0, //
            1.0, -1.0, 1.0, 1.0, //
            1.0, 1.0, 1.0, 0.0, //
            -1.0, -1.0, 0.0, 1.0, //
            1.0, 1.0, 1.0, 0.0, //
            -1.0, 1.0, 0.0, 0.0, //
        ];
        let vertex_buffer = shared.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wprs_winit_wgpu_vertex_buffer"),
            size: (vertex_data.len() * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        shared
            .queue
            .write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(vertex_data));

        Ok(Self {
            window,
            surface,
            config,
            pipeline,
            vertex_buffer,
            bind_group_layout,
            sampler,
            bind_group: None,
            texture: None,
            texture_size: None,
            scratch_unfiltered: Vec::new(),
            scratch_padded: Vec::new(),
        })
    }

    fn resize(&mut self, shared: &WgpuShared, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&shared.device, &self.config);
    }

    fn update_texture_from_filtered_bgra(
        &mut self,
        shared: &WgpuShared,
        metadata: &crate::protocols::wprs::wayland::BufferMetadata,
        filtered_data: &crate::vec4u8::Vec4u8s,
    ) {
        let width = metadata.width as u32;
        let height = metadata.height as u32;
        let src_stride = metadata.stride as usize;
        let row_bytes = metadata.width as usize * 4;

        self.scratch_unfiltered.resize(metadata.len(), 0);
        filtering::unfilter(filtered_data, &mut self.scratch_unfiltered);

        let padded_row_bytes = align_up(row_bytes, 256);
        self.scratch_padded
            .resize(padded_row_bytes * height as usize, 0);
        for y in 0..height as usize {
            let src = &self.scratch_unfiltered[y * src_stride..y * src_stride + row_bytes];
            let dst =
                &mut self.scratch_padded[y * padded_row_bytes..y * padded_row_bytes + row_bytes];
            dst.copy_from_slice(src);
        }

        if self.texture_size != Some((width, height)) {
            let texture = shared.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("wprs_remote_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = shared.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("wprs_remote_texture_bg"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.texture = Some(texture);
            self.bind_group = Some(bind_group);
            self.texture_size = Some((width, height));
        }

        let texture = self.texture.as_ref().unwrap();
        shared.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.scratch_padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes as u32),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn render(&mut self, shared: &WgpuShared) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated) | Err(wgpu::SurfaceError::Lost) => {
                self.surface.configure(&shared.device, &self.config);
                return Ok(());
            },
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(err) => return Err(anyhow!("surface acquire failed: {err:?}")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = shared
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wprs_winit_wgpu_encoder"),
            });
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wprs_winit_wgpu_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            if let Some(bg) = &self.bind_group {
                rp.set_bind_group(0, bg, &[]);
            }
            rp.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            rp.draw(0..6, 0..1);
        }
        shared.queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

fn align_up(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
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
    shared: WgpuShared,
    serializer: Serializer<proto::Event, Request>,
    buffer_cache: Option<UncompressedBufferData>,
    windows: HashMap<WlSurfaceId, WindowRenderer>,
    surface_by_window: HashMap<winit::window::WindowId, WlSurfaceId>,
    outputs_sent: bool,

    keyboard_mode: KeyboardMode,
    xkb_keymap_sent: bool,
    xkb_keymap_file: Option<std::path::PathBuf>,

    serial_counter: u32,
    focused_surface: Option<WlSurfaceId>,
    surfaces_with_frame: HashSet<WlSurfaceId>,
    pressed_keycodes: HashSet<u32>,
    last_cursor_pos: HashMap<winit::window::WindowId, crate::protocols::wprs::geometry::Point<f64>>,
    pointer_inside: HashSet<winit::window::WindowId>,
}

impl App {
    fn logical_pos(
        window: &winit::window::Window,
        position: winit::dpi::PhysicalPosition<f64>,
    ) -> crate::protocols::wprs::geometry::Point<f64> {
        let scale = window.scale_factor();
        let logical: winit::dpi::LogicalPosition<f64> = position.to_logical(scale);
        crate::protocols::wprs::geometry::Point {
            x: logical.x,
            y: logical.y,
        }
    }

    fn logical_size(window: &winit::window::Window) -> Size<Option<NonZeroU32>> {
        let scale = window.scale_factor();
        let size = window.inner_size();
        let logical: winit::dpi::LogicalSize<f64> = size.to_logical(scale);

        // Wayland sizes are logical surface units (not physical pixels).
        let w = (logical.width.round() as u32).max(1);
        let h = (logical.height.round() as u32).max(1);

        Size {
            w: NonZeroU32::new(w),
            h: NonZeroU32::new(h),
        }
    }

    fn generate_keymap_from_tools() -> Result<String> {
        // Preferred path: ask X11 for the active keymap (works under Xwayland/X11).
        // `setxkbmap -print` emits an XKB config, `xkbcomp -xkb - -` compiles it to a keymap.
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
                KeyboardEvent::Keymap(keymap),
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
        kind: PointerEventKind,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::PointerFrame(vec![
                PointerEvent {
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
                    KeyboardEvent::Leave { serial },
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
                    KeyboardEvent::Enter {
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
                KeyboardEvent::Modifiers {
                    modifier_state: ModifierState {
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

    fn send_key(&mut self, keycode: u32, state: KeyState) {
        // Keyboard events are applied to the currently-focused surface on the server side.
        if self.focused_surface.is_none() {
            return;
        }
        let serial = self.next_serial();
        self.serializer
            .writer()
            .send(SendType::Object(proto::Event::KeyboardEvent(
                KeyboardEvent::Key(KeyInner {
                    serial,
                    raw_code: keycode,
                    state,
                }),
            )));
    }

    fn linux_button_from_winit(button: MouseButton) -> Option<u32> {
        // linux/input-event-codes.h
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

        // linux/input-event-codes.h keycodes (evdev)
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
            // Not yet handled in this backend.
            _ => Ok(()),
        }
    }

    fn send_configure_for_surface(&mut self, surface_id: WlSurfaceId) {
        let Some(renderer) = self.windows.get(&surface_id) else {
            return;
        };
        let configure = ToplevelConfigure {
            surface_id,
            new_size: Self::logical_size(&renderer.window),
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
                return Ok(());
            },
            SurfaceRequestPayload::Commit(mut state) => {
                let Some(role) = &state.role else {
                    return Ok(());
                };
                let Some(toplevel) = role.as_xdg_toplevel() else {
                    return Ok(());
                };

                // Ensure we have a window for this toplevel.
                if !self.windows.contains_key(&surface_id) {
                    let title = toplevel.title.clone().unwrap_or_else(|| "wprs".to_string());
                    let mut attrs = Window::default_attributes().with_title(title);

                    if let Some(BufferAssignment::New(buf)) = &state.buffer {
                        let w = buf.metadata.width.max(1) as u32;
                        let h = buf.metadata.height.max(1) as u32;
                        attrs = attrs.with_inner_size(PhysicalSize::new(w, h));
                    }

                    let window = Arc::new(event_loop.create_window(attrs).location(loc!())?);
                    let renderer =
                        WindowRenderer::new(&self.shared, window.clone()).location(loc!())?;
                    self.surface_by_window.insert(window.id(), surface_id);
                    self.windows.insert(surface_id, renderer);

                    // Send an initial configure so apps can begin drawing.
                    self.send_configure_for_surface(surface_id);
                }

                // Apply buffer if present.
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
                        }
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
                        }
                    };
                    let renderer = self.windows.get_mut(&surface_id).unwrap();
                    renderer.update_texture_from_filtered_bgra(
                        &self.shared,
                        &buf.metadata,
                        &filtered,
                    );
                    renderer.window.request_redraw();
                    self.surfaces_with_frame.insert(surface_id);
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
                .send(SendType::Object(proto::Event::Output(
                    OutputEvent::New(output_info_from_monitor(idx as u32, &monitor)),
                )));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::ServerMessage(msg) => {
                self.handle_server_message(event_loop, msg)
                    .log_and_ignore(loc!());
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
                    renderer.resize(&self.shared, *size);
                    self.send_configure_for_surface(surface_id);
                },
                WindowEvent::ScaleFactorChanged { .. } => {
                    // Winit reports physical sizes; propagate the logical size.
                    self.send_configure_for_surface(surface_id);
                },
                WindowEvent::RedrawRequested => {
                    renderer.render(&self.shared).log_and_ignore(loc!());
                },
                WindowEvent::CloseRequested => {
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
                    (ElementState::Pressed, true) => KeyState::Repeated,
                    (ElementState::Pressed, false) => KeyState::Pressed,
                    (ElementState::Released, _) => KeyState::Released,
                };

                match state {
                    KeyState::Pressed | KeyState::Repeated => {
                        self.pressed_keycodes.insert(linux_keycode);
                    },
                    KeyState::Released => {
                        self.pressed_keycodes.remove(&linux_keycode);
                    },
                }
                self.send_key(linux_keycode, state);
            },
            WindowEvent::CursorMoved { position, .. } => {
                let Some(renderer) = self.windows.get(&surface_id) else {
                    return;
                };
                let pos = Self::logical_pos(&renderer.window, position);
                self.last_cursor_pos.insert(window_id, pos);

                // Some platforms don't emit CursorEntered if the cursor is
                // already inside the window when it is created.
                if self.pointer_inside.insert(window_id) {
                    let serial = self.next_serial();
                    self.send_pointer_event(surface_id, pos, PointerEventKind::Enter { serial });
                }
                self.send_pointer_event(surface_id, pos, PointerEventKind::Motion);
            },
            WindowEvent::CursorEntered { .. } => {
                let pos = self.cursor_pos_for(window_id);
                let serial = self.next_serial();
                self.pointer_inside.insert(window_id);
                self.send_pointer_event(surface_id, pos, PointerEventKind::Enter { serial });
            },
            WindowEvent::CursorLeft { .. } => {
                let pos = self.cursor_pos_for(window_id);
                let serial = self.next_serial();
                self.pointer_inside.remove(&window_id);
                self.send_pointer_event(surface_id, pos, PointerEventKind::Leave { serial });
            },
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = Self::linux_button_from_winit(button) else {
                    return;
                };
                let pos = self.cursor_pos_for(window_id);
                let kind = match state {
                    ElementState::Pressed => PointerEventKind::Press {
                        button,
                        serial: self.next_serial(),
                    },
                    ElementState::Released => PointerEventKind::Release {
                        button,
                        serial: self.next_serial(),
                    },
                };
                self.send_pointer_event(surface_id, pos, kind);
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let (h_abs, v_abs, h_discrete, v_discrete) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        let v120_x = (x * 120.0) as i32;
                        let v120_y = (y * 120.0) as i32;
                        (f64::from(x) * 15.0, f64::from(y) * 15.0, v120_x, v120_y)
                    },
                    MouseScrollDelta::PixelDelta(pos) => {
                        let Some(renderer) = self.windows.get(&surface_id) else {
                            return;
                        };
                        let scale = renderer.window.scale_factor();
                        let logical: winit::dpi::LogicalPosition<f64> = pos.to_logical(scale);
                        (logical.x, logical.y, 0, 0)
                    }
                };
                let pos = self.cursor_pos_for(window_id);
                self.send_pointer_event(
                    surface_id,
                    pos,
                    PointerEventKind::Axis {
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
                        source: Some(AxisSource::Wheel),
                    },
                );
            },
            _ => {},
        }
    }
}

pub fn run(
    mut serializer: Serializer<proto::Event, Request>,
    options: WinitWgpuOptions,
) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    // Forward serializer messages to the winit event loop.
    let reader = serializer.reader().location(loc!())?;
    thread::spawn(move || {
        let mut loop_: CalloopEventLoop<()> = CalloopEventLoop::try_new().expect("calloop init");
        loop_
            .handle()
            .insert_source(reader, move |event, _metadata, _state| {
                if let CalloopChannelEvent::Msg(msg) = event {
                    proxy.send_event(UserEvent::ServerMessage(msg)).ok();
                }
            })
            .expect("insert serializer reader");

        loop_.run(None, &mut (), |_| {}).ok();
    });

    // Init shared wgpu context.
    let instance = Arc::new(wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    }));

    let adapter = Arc::new(
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .location(loc!())?,
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("wprs_winit_wgpu_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        ..Default::default()
    }))
    .location(loc!())?;

    debug!("wgpu adapter: {:?}", adapter.get_info());

    let shared = WgpuShared {
        instance,
        adapter,
        device: Arc::new(device),
        queue: Arc::new(queue),
    };

    let mut app = App {
        shared,
        serializer,
        buffer_cache: None,
        windows: HashMap::new(),
        surface_by_window: HashMap::new(),
        outputs_sent: false,

        keyboard_mode: options.keyboard_mode,
        xkb_keymap_sent: false,
        xkb_keymap_file: options.xkb_keymap_file,

        serial_counter: 1,
        focused_surface: None,
        surfaces_with_frame: HashSet::new(),
        pressed_keycodes: HashSet::new(),
        last_cursor_pos: HashMap::new(),
        pointer_inside: HashSet::new(),
    };

    event_loop.run_app(&mut app)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct WinitWgpuClientBackend {
    options: WinitWgpuOptions,
}

impl WinitWgpuClientBackend {
    pub fn new(config: crate::client::backend::ClientBackendConfig) -> Self {
        Self {
            options: WinitWgpuOptions {
                keyboard_mode: config.keyboard_mode,
                xkb_keymap_file: config.xkb_keymap_file,
            },
        }
    }
}

impl crate::client::backend::ClientBackend for WinitWgpuClientBackend {
    fn name(&self) -> &'static str {
        "winit-wgpu"
    }

    fn run(
        self: Box<Self>,
        serializer: Serializer<proto::Event, proto::Request>,
    ) -> Result<()> {
        run(serializer, self.options).location(loc!())
    }
}
