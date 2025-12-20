use std::sync::Arc;

use crate::prelude::*;
use crate::protocols::wprs::Capabilities;
use crate::protocols::wprs::DisplayConfig;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::wayland;
use crate::protocols::wprs::wayland::Buffer;
use crate::protocols::wprs::wayland::BufferAssignment;
use crate::protocols::wprs::wayland::BufferData;
use crate::protocols::wprs::wayland::BufferMetadata;
use crate::protocols::wprs::wayland::SurfaceState;
use crate::protocols::wprs::wayland::WlSurfaceId;
use crate::protocols::wprs::xdg_shell;
use crate::server::runtime::backend::BackendObservation;
use crate::server::runtime::backend::PollingBackend;
use crate::server::runtime::backend::SurfaceSnapshot;

#[derive(Debug)]
pub struct MacosFullscreenBackend {
    surface_state: SurfaceState,
    pressed_buttons: u32,
    last_pos: (f64, f64),
    display_config: DisplayConfig,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MacosFullscreenBackendConfig {
    pub dpi: Option<u32>,
}

impl MacosFullscreenBackend {
    pub fn new(config: MacosFullscreenBackendConfig) -> Self {
        let (detected_scale_factor, detected_dpi) =
            display_scale_factor_and_dpi().unwrap_or((1, None));
        let scale_factor = detected_scale_factor.max(1);
        let dpi = config.dpi.or(detected_dpi);
        let display_config = DisplayConfig {
            scale_factor,
            dpi,
        };

        // A single synthetic surface representing the main display.
        let toplevel = xdg_shell::XdgToplevelState {
            id: xdg_shell::XdgToplevelId(1),
            parent: None,
            title: Some("wprs (macOS)".to_string()),
            app_id: Some("wprs".to_string()),
            decoration_mode: None,
            maximized: Some(true),
            fullscreen: Some(true),
        };

        let surface_state = SurfaceState {
            client: ClientId(1),
            id: WlSurfaceId(1),
            buffer: None,
            role: Some(wayland::Role::XdgToplevel(toplevel)),
            buffer_scale: display_config.scale_factor,
            buffer_transform: None,
            opaque_region: None,
            input_region: None,
            z_ordered_children: Vec::new(),
            damage: None,
            output_ids: Vec::new(),
            viewport_state: None,
            xdg_surface_state: Some(xdg_shell::XdgSurfaceState::default()),
        };

        Self {
            surface_state,
            pressed_buttons: 0,
            last_pos: (0.0, 0.0),
            display_config,
        }
    }
}

impl PollingBackend for MacosFullscreenBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities { xwayland: false }
    }

    fn display_config(&self) -> DisplayConfig {
        self.display_config.clone()
    }

    fn initial_snapshot(&mut self) -> Result<Vec<SurfaceSnapshot>> {
        let (metadata, _bgra) = capture_main_display_bgra().location(loc!())?;
        self.surface_state.buffer = Some(BufferAssignment::New(Buffer {
            metadata,
            data: BufferData::External,
        }));

        Ok(vec![SurfaceSnapshot {
            state: self.surface_state.clone(),
        }])
    }

    fn poll(&mut self) -> Result<Vec<BackendObservation>> {
        let (metadata, bgra) = capture_main_display_bgra().location(loc!())?;
        self.surface_state.buffer = Some(BufferAssignment::New(Buffer {
            metadata,
            data: BufferData::External,
        }));

        Ok(vec![BackendObservation::SurfaceCommit {
            state: self.surface_state.clone(),
            bgra: Some(Arc::from(bgra.into_boxed_slice())),
        }])
    }

    fn handle_client_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::PointerFrame(events) => {
                for e in events {
                    self.handle_pointer_event(e).log_and_ignore(loc!());
                }
            }
            // Keyboard input injection is currently best-effort.
            // wprsc primarily emits Linux evdev raw codes, which don't map 1:1
            // to macOS CGKeyCode.
            Event::KeyboardEvent(_) => {}
            _ => {}
        }
        Ok(())
    }
}

impl MacosFullscreenBackend {
    fn handle_pointer_event(&mut self, e: wayland::PointerEvent) -> Result<()> {
        let x = e.position.x;
        let y = e.position.y;
        self.last_pos = (x, y);

        match e.kind {
            wayland::PointerEventKind::Enter { .. } | wayland::PointerEventKind::Leave { .. } => {
                Ok(())
            }
            wayland::PointerEventKind::Motion => {
                post_mouse_motion(self.pressed_buttons, x, y).location(loc!())
            }
            wayland::PointerEventKind::Press { button, .. } => {
                let mask = button_mask(button);
                self.pressed_buttons |= mask;
                post_mouse_button(true, button, x, y).location(loc!())
            }
            wayland::PointerEventKind::Release { button, .. } => {
                let mask = button_mask(button);
                self.pressed_buttons &= !mask;
                post_mouse_button(false, button, x, y).location(loc!())
            }
            wayland::PointerEventKind::Axis { horizontal, vertical, .. } => {
                // Prefer discrete wheel steps when available.
                post_scroll(horizontal.discrete, vertical.discrete).location(loc!())
            }
        }
    }
}

fn button_mask(button: u32) -> u32 {
    // Linux input-event-codes: BTN_LEFT=272, BTN_RIGHT=273, BTN_MIDDLE=274
    match button {
        272 => 1 << 0,
        273 => 1 << 1,
        274 => 1 << 2,
        _ => 0,
    }
}

fn display_scale_factor_and_dpi() -> Result<(i32, Option<u32>)> {
    #[cfg(target_os = "macos")]
    {
        macos::main_display_scale_factor_and_dpi().location(loc!())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok((1, None))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use anyhow::ensure;
    use std::ffi::c_void;
    use std::ptr;

    type CFIndex = isize;
    type CFTypeRef = *const c_void;
    type CFDataRef = *const c_void;
    type CGImageRef = *const c_void;
    type CGDataProviderRef = *const c_void;
    type CGDirectDisplayID = u32;
    type CGDisplayModeRef = *const c_void;
    type CGEventRef = *const c_void;
    type CGEventType = u32;
    type CGMouseButton = u32;
    type CGEventTapLocation = u32;

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> CGDirectDisplayID;
        fn CGDisplayCopyDisplayMode(display_id: CGDirectDisplayID) -> CGDisplayModeRef;
        fn CGDisplayModeGetWidth(mode: CGDisplayModeRef) -> usize;
        fn CGDisplayModeGetHeight(mode: CGDisplayModeRef) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: CGDisplayModeRef) -> usize;
        fn CGDisplayModeGetPixelHeight(mode: CGDisplayModeRef) -> usize;
        fn CGDisplayScreenSize(display_id: CGDirectDisplayID) -> CGSize;

        fn CGDisplayCreateImage(display_id: CGDirectDisplayID) -> CGImageRef;
        fn CGImageGetWidth(image: CGImageRef) -> usize;
        fn CGImageGetHeight(image: CGImageRef) -> usize;
        fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
        fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
        fn CGImageGetBitsPerComponent(image: CGImageRef) -> usize;
        fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
        fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;

        fn CGEventCreateMouseEvent(
            source: *const c_void,
            event_type: CGEventType,
            mouse_cursor_position: CGPoint,
            mouse_button: CGMouseButton,
        ) -> CGEventRef;

        fn CGEventCreateScrollWheelEvent(
            source: *const c_void,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
        ) -> CGEventRef;

        fn CGEventPost(tap: CGEventTapLocation, event: CGEventRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataGetLength(the_data: CFDataRef) -> CFIndex;
        fn CFDataGetBytePtr(the_data: CFDataRef) -> *const u8;
        fn CFRelease(cf: CFTypeRef);
    }

    const K_CG_EVENT_TAP_HID: CGEventTapLocation = 0;

    const K_CG_EVENT_MOUSE_MOVED: CGEventType = 5;
    const K_CG_EVENT_LEFT_MOUSE_DOWN: CGEventType = 1;
    const K_CG_EVENT_LEFT_MOUSE_UP: CGEventType = 2;
    const K_CG_EVENT_RIGHT_MOUSE_DOWN: CGEventType = 3;
    const K_CG_EVENT_RIGHT_MOUSE_UP: CGEventType = 4;
    const K_CG_EVENT_OTHER_MOUSE_DOWN: CGEventType = 25;
    const K_CG_EVENT_OTHER_MOUSE_UP: CGEventType = 26;
    const K_CG_EVENT_LEFT_MOUSE_DRAGGED: CGEventType = 6;
    const K_CG_EVENT_RIGHT_MOUSE_DRAGGED: CGEventType = 7;
    const K_CG_EVENT_OTHER_MOUSE_DRAGGED: CGEventType = 27;

    const K_CG_MOUSE_BUTTON_LEFT: CGMouseButton = 0;
    const K_CG_MOUSE_BUTTON_RIGHT: CGMouseButton = 1;
    const K_CG_MOUSE_BUTTON_CENTER: CGMouseButton = 2;

    const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;

    pub(super) fn capture_main_display_bgra() -> Result<(BufferMetadata, Vec<u8>)> {
        unsafe {
            let display = CGMainDisplayID();
            let image = CGDisplayCreateImage(display);
            ensure!(!image.is_null(), "CGDisplayCreateImage returned null (Screen Recording permission?)");

            let width = CGImageGetWidth(image) as i32;
            let height = CGImageGetHeight(image) as i32;
            let stride = CGImageGetBytesPerRow(image) as i32;
            let bpp = CGImageGetBitsPerPixel(image);
            let bpc = CGImageGetBitsPerComponent(image);

            let provider = CGImageGetDataProvider(image);
            ensure!(!provider.is_null(), "CGImageGetDataProvider returned null");
            let cf_data = CGDataProviderCopyData(provider);
            ensure!(!cf_data.is_null(), "CGDataProviderCopyData returned null");

            let len = CFDataGetLength(cf_data) as usize;
            let ptr = CFDataGetBytePtr(cf_data);
            ensure!(!ptr.is_null(), "CFDataGetBytePtr returned null");

            // Best-effort: most systems will produce a 32bpp image. If not,
            // bail with a clear message rather than silently corrupting.
            ensure!(bpp == 32 && bpc == 8, "unsupported capture format: bpp={bpp}, bpc={bpc}");
            ensure!(stride > 0 && width > 0 && height > 0, "invalid captured dimensions");
            ensure!(len >= (height as usize) * (stride as usize), "captured buffer is smaller than expected");

            let bytes = std::slice::from_raw_parts(ptr, (height as usize) * (stride as usize));
            let out = bytes.to_vec();

            CFRelease(cf_data as CFTypeRef);
            CFRelease(image as CFTypeRef);

            let metadata = BufferMetadata {
                width,
                height,
                stride,
                format: wayland::BufferFormat::Argb8888,
            };
            Ok((metadata, out))
        }
    }

    pub(super) fn post_mouse_motion(button_mask: u32, x: f64, y: f64) -> Result<()> {
        unsafe {
            let (_width, height) = display_size().location(loc!())?;
            let p = CGPoint {
                x,
                // Quartz global coordinates are origin-at-bottom-left.
                y: (height as f64) - y,
            };

            let (event_type, mouse_button) = if button_mask & (1 << 0) != 0 {
                (K_CG_EVENT_LEFT_MOUSE_DRAGGED, K_CG_MOUSE_BUTTON_LEFT)
            } else if button_mask & (1 << 1) != 0 {
                (K_CG_EVENT_RIGHT_MOUSE_DRAGGED, K_CG_MOUSE_BUTTON_RIGHT)
            } else if button_mask & (1 << 2) != 0 {
                (K_CG_EVENT_OTHER_MOUSE_DRAGGED, K_CG_MOUSE_BUTTON_CENTER)
            } else {
                (K_CG_EVENT_MOUSE_MOVED, K_CG_MOUSE_BUTTON_LEFT)
            };

            let ev = CGEventCreateMouseEvent(ptr::null(), event_type, p, mouse_button);
            ensure!(!ev.is_null(), "CGEventCreateMouseEvent returned null");
            CGEventPost(K_CG_EVENT_TAP_HID, ev);
            CFRelease(ev as CFTypeRef);
            Ok(())
        }
    }

    pub(super) fn post_mouse_button(down: bool, button: u32, x: f64, y: f64) -> Result<()> {
        unsafe {
            let (_width, height) = display_size().location(loc!())?;
            let p = CGPoint {
                x,
                y: (height as f64) - y,
            };

            let (event_type, mouse_button) = match button {
                272 => (
                    if down { K_CG_EVENT_LEFT_MOUSE_DOWN } else { K_CG_EVENT_LEFT_MOUSE_UP },
                    K_CG_MOUSE_BUTTON_LEFT,
                ),
                273 => (
                    if down { K_CG_EVENT_RIGHT_MOUSE_DOWN } else { K_CG_EVENT_RIGHT_MOUSE_UP },
                    K_CG_MOUSE_BUTTON_RIGHT,
                ),
                274 => (
                    if down { K_CG_EVENT_OTHER_MOUSE_DOWN } else { K_CG_EVENT_OTHER_MOUSE_UP },
                    K_CG_MOUSE_BUTTON_CENTER,
                ),
                _ => return Ok(()),
            };

            let ev = CGEventCreateMouseEvent(ptr::null(), event_type, p, mouse_button);
            ensure!(!ev.is_null(), "CGEventCreateMouseEvent returned null");
            CGEventPost(K_CG_EVENT_TAP_HID, ev);
            CFRelease(ev as CFTypeRef);
            Ok(())
        }
    }

    pub(super) fn post_scroll(horizontal_discrete: i32, vertical_discrete: i32) -> Result<()> {
        unsafe {
            // Wayland positive Y scroll is "down" for most toolkits; Quartz
            // line scrolling is positive "up". Flip vertical.
            let ev = CGEventCreateScrollWheelEvent(
                ptr::null(),
                K_CG_SCROLL_EVENT_UNIT_LINE,
                2,
                -vertical_discrete,
                horizontal_discrete,
            );
            ensure!(!ev.is_null(), "CGEventCreateScrollWheelEvent returned null");
            CGEventPost(K_CG_EVENT_TAP_HID, ev);
            CFRelease(ev as CFTypeRef);
            Ok(())
        }
    }

    fn display_size() -> Result<(usize, usize)> {
        unsafe {
            let display = CGMainDisplayID();
            let image = CGDisplayCreateImage(display);
            ensure!(!image.is_null(), "CGDisplayCreateImage returned null");
            let w = CGImageGetWidth(image);
            let h = CGImageGetHeight(image);
            CFRelease(image as CFTypeRef);
            Ok((w, h))
        }
    }

    pub(super) fn main_display_scale_factor_and_dpi() -> Result<(i32, Option<u32>)> {
        unsafe {
            let display = CGMainDisplayID();
            let mode = CGDisplayCopyDisplayMode(display);
            ensure!(!mode.is_null(), "CGDisplayCopyDisplayMode returned null");

            let width_points = CGDisplayModeGetWidth(mode) as f64;
            let height_points = CGDisplayModeGetHeight(mode) as f64;
            let width_pixels = CGDisplayModeGetPixelWidth(mode) as f64;
            let height_pixels = CGDisplayModeGetPixelHeight(mode) as f64;

            // CGDisplayModeRef is a CFType.
            CFRelease(mode as CFTypeRef);

            ensure!(width_points > 0.0 && height_points > 0.0, "invalid display mode size");
            ensure!(width_pixels > 0.0 && height_pixels > 0.0, "invalid display mode pixel size");

            let scale_w = width_pixels / width_points;
            let scale_h = height_pixels / height_points;
            let mut scale = scale_w;
            // Prefer width-based scale; if height differs significantly, fall back to average.
            if (scale_w - scale_h).abs() > 0.1 {
                scale = (scale_w + scale_h) / 2.0;
            }
            let scale_factor = (scale.round() as i32).max(1);

            // Best-effort DPI calculation.
            let screen_mm = CGDisplayScreenSize(display);
            let dpi = if screen_mm.width > 0.0 {
                let inches = screen_mm.width / 25.4;
                if inches > 0.0 {
                    Some((width_pixels / inches).round() as u32)
                } else {
                    None
                }
            } else {
                None
            };

            Ok((scale_factor, dpi))
        }
    }
}

#[cfg(target_os = "macos")]
use macos::capture_main_display_bgra;
#[cfg(target_os = "macos")]
use macos::post_mouse_button;
#[cfg(target_os = "macos")]
use macos::post_mouse_motion;
#[cfg(target_os = "macos")]
use macos::post_scroll;

#[cfg(not(target_os = "macos"))]
fn capture_main_display_bgra() -> Result<(BufferMetadata, Vec<u8>)> {
    bail!("macOS fullscreen backend is only supported on macOS")
}

#[cfg(not(target_os = "macos"))]
fn post_mouse_motion(_button_mask: u32, _x: f64, _y: f64) -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn post_mouse_button(_down: bool, _button: u32, _x: f64, _y: f64) -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn post_scroll(_horizontal_discrete: i32, _vertical_discrete: i32) -> Result<()> {
    Ok(())
}
