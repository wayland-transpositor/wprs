use std::time::Duration;

use smithay::utils::Serial;
use smithay::xwayland::X11Surface;
use smithay_client_toolkit::compositor::SurfaceData;
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::csd_frame::CursorIcon;
use smithay_client_toolkit::reexports::csd_frame::DecorationsFrame;
use smithay_client_toolkit::reexports::csd_frame::FrameAction;
use smithay_client_toolkit::reexports::csd_frame::FrameClick;
use smithay_client_toolkit::reexports::csd_frame::ResizeEdge;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge as SctkResizeEdge;
use smithay_client_toolkit::seat::pointer::PointerData;
use smithay_client_toolkit::seat::pointer::PointerEvent;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::seat::pointer::BTN_LEFT;
use smithay_client_toolkit::seat::pointer::BTN_RIGHT;
use smithay_client_toolkit::shell::xdg::fallback_frame::FallbackFrame;
use tracing::warn;

use crate::prelude::*;
use crate::xwayland_xdg_shell::client::Role;
use crate::xwayland_xdg_shell::client::WprsClientState;
use crate::xwayland_xdg_shell::client::XWaylandSubSurface;
use crate::xwayland_xdg_shell::client::XWaylandXdgToplevel;
use crate::xwayland_xdg_shell::xsurface_from_client_surface;
use crate::xwayland_xdg_shell::WprsState;

fn parent(surface: &WlSurface) -> Option<&WlSurface> {
    surface.data::<SurfaceData>()?.parent_surface()
}

fn surface_tree_root(surface: &WlSurface) -> &WlSurface {
    let mut surface = surface;
    while let Some(parent) = parent(surface) {
        surface = parent;
    }
    surface
}

#[instrument(skip(state, conn, pointer), level = "debug")]
pub fn handle_window_frame_pointer_event(
    state: &mut WprsState,
    conn: &Connection,
    qh: &QueueHandle<WprsState>,
    pointer: &WlPointer,
    events: &[PointerEvent],
) -> Result<()> {
    for event in events {
        let parent_surface = parent(&event.surface).unwrap_or(surface_tree_root(&event.surface));
        let Some(xwayland_surface) =
            xsurface_from_client_surface(&state.surface_bimap, &mut state.surfaces, parent_surface)
        else {
            info!("frame owner not found");
            return Ok(());
        };

        let x11_surface = &xwayland_surface.x11_surface.as_ref().unwrap().clone();
        match &mut xwayland_surface.role {
            Some(Role::SubSurface(subsurface)) => {
                subsurface
                    .handle_pointer_event(
                        &mut state.client_state,
                        x11_surface,
                        conn,
                        qh,
                        pointer,
                        event,
                    )
                    .location(loc!())?;
            },
            Some(Role::XdgToplevel(toplevel)) => {
                toplevel
                    .handle_pointer_event(
                        &mut state.client_state,
                        x11_surface,
                        conn,
                        qh,
                        pointer,
                        event,
                    )
                    .location(loc!())?;
            },
            _ => unreachable!(
                "expected role xdg_toplevel or subsurface, found role {:?}",
                &xwayland_surface.role
            ),
        }
    }
    Ok(())
}

pub trait FramedSurface {
    fn frame_action(
        &mut self,
        x11_surface: &X11Surface,
        client_pointer: &WlPointer,
        serial: Serial,
        action: FrameAction,
        position: (f64, f64),
    ) -> Result<()>;

    fn frame(&mut self) -> &mut FallbackFrame<WprsState>;

    fn handle_pointer_event_inner(
        &mut self,
        client_state: &mut WprsClientState,
        x11_surface: &X11Surface,
        qh: &QueueHandle<WprsState>,
        pointer: &WlPointer,
        event: &PointerEvent,
    ) -> Result<Option<CursorIcon>>;

    fn handle_pointer_event(
        &mut self,
        client_state: &mut WprsClientState,
        x11_surface: &X11Surface,
        conn: &Connection,
        qh: &QueueHandle<WprsState>,
        pointer: &WlPointer,
        event: &PointerEvent,
    ) -> Result<()> {
        let new_cursor = self
            .handle_pointer_event_inner(client_state, x11_surface, qh, pointer, event)
            .location(loc!())?;

        if let (Some(new_cursor), cur_cursor) =
            (new_cursor, client_state.cursor_icon.unwrap_or_default())
        {
            if new_cursor != cur_cursor {
                client_state.cursor_icon = Some(new_cursor);
                let _ = client_state
                    .seat_objects
                    .last()
                    .unwrap()
                    .pointer
                    .as_ref()
                    .unwrap()
                    .set_cursor(conn, new_cursor);
            }
        }

        let frame = self.frame();

        if frame.is_dirty() {
            frame.draw();
        }

        Ok(())
    }
}

impl FramedSurface for XWaylandXdgToplevel {
    fn frame_action(
        &mut self,
        x11_surface: &X11Surface,
        client_pointer: &WlPointer,
        serial: Serial,
        action: FrameAction,
        _position: (f64, f64),
    ) -> Result<()> {
        let window = &self.local_window;
        let pointer_data = client_pointer.data::<PointerData>().unwrap();
        let client_seat = pointer_data.seat();
        match action {
            FrameAction::Close => {
                x11_surface.close().location(loc!())?;
            },
            FrameAction::Minimize => {
                window.set_minimized();
            },
            FrameAction::Maximize => {
                window.set_maximized();
            },
            FrameAction::UnMaximize => {
                window.unset_maximized();
            },
            FrameAction::ShowMenu(x, y) => {
                window.show_window_menu(client_seat, serial.into(), (x, y));
            },
            FrameAction::Resize(edge) => {
                let edge = match edge {
                    ResizeEdge::None => SctkResizeEdge::None,
                    ResizeEdge::Top => SctkResizeEdge::Top,
                    ResizeEdge::Bottom => SctkResizeEdge::Bottom,
                    ResizeEdge::Left => SctkResizeEdge::Left,
                    ResizeEdge::TopLeft => SctkResizeEdge::TopLeft,
                    ResizeEdge::BottomLeft => SctkResizeEdge::BottomLeft,
                    ResizeEdge::Right => SctkResizeEdge::Right,
                    ResizeEdge::TopRight => SctkResizeEdge::TopRight,
                    ResizeEdge::BottomRight => SctkResizeEdge::BottomRight,
                    _ => SctkResizeEdge::None,
                };
                window.resize(client_seat, serial.into(), edge);
            },
            FrameAction::Move => {
                window.move_(client_seat, serial.into());
            },
            _ => {
                warn!("Unknown frame action: {:?}", action);
            },
        }
        Ok(())
    }

    fn frame(&mut self) -> &mut FallbackFrame<WprsState> {
        &mut self.window_frame
    }

    fn handle_pointer_event_inner(
        &mut self,
        client_state: &mut WprsClientState,
        x11_surface: &X11Surface,
        _qh: &QueueHandle<WprsState>,
        pointer: &WlPointer,
        event: &PointerEvent,
    ) -> Result<Option<CursorIcon>> {
        let (x, y) = event.position;
        let frame = &mut self.window_frame;
        let mut new_cursor = None;
        match event.kind {
            PointerEventKind::Enter { serial } => {
                new_cursor = Some(
                    frame
                        .click_point_moved(Duration::ZERO, &event.surface.id(), x, y)
                        .unwrap_or(CursorIcon::Default),
                );
                client_state.last_enter_serial = serial;
            },
            PointerEventKind::Leave { serial: _ } => {
                frame.click_point_left();
            },
            PointerEventKind::Motion { time: _ } => {
                new_cursor = frame.click_point_moved(Duration::ZERO, &event.surface.id(), x, y);
            },
            PointerEventKind::Press { button, serial, .. }
            | PointerEventKind::Release { button, serial, .. } => {
                let kind = &event.kind;
                let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                let click = match button {
                    BTN_LEFT => FrameClick::Normal,
                    BTN_RIGHT => FrameClick::Alternate,
                    _ => return Ok(None),
                };

                if let Some(action) = frame.on_click(Duration::ZERO, click, pressed) {
                    debug!("button: {click:?}, kind: {kind:?}, action {action:?}");

                    self.frame_action(x11_surface, pointer, serial.into(), action, (x, y))
                        .location(loc!())?;
                }
            },
            PointerEventKind::Axis { .. } => {},
        }

        Ok(new_cursor)
    }
}

impl FramedSurface for XWaylandSubSurface {
    fn frame_action(
        &mut self,
        x11_surface: &X11Surface,
        _client_pointer: &WlPointer,
        _serial: Serial,
        action: FrameAction,
        position: (f64, f64),
    ) -> Result<()> {
        match action {
            FrameAction::Close => {
                x11_surface.close().location(loc!())?;
            },
            FrameAction::Resize(_edge) => {
                // TODO
            },
            FrameAction::Move => {
                self.move_pointer_location = position;
                self.move_active = true;
            },
            _ => {},
        };

        Ok(())
    }

    fn frame(&mut self) -> &mut FallbackFrame<WprsState> {
        self.frame.as_mut().unwrap()
    }

    fn handle_pointer_event_inner(
        &mut self,
        client_state: &mut WprsClientState,
        x11_surface: &X11Surface,
        qh: &QueueHandle<WprsState>,
        pointer: &WlPointer,
        event: &PointerEvent,
    ) -> Result<Option<CursorIcon>> {
        let frame = self.frame.as_mut().unwrap();
        let mut new_cursor: Option<CursorIcon> = None;

        let (x, y) = event.position;
        match event.kind {
            PointerEventKind::Enter { serial } => {
                new_cursor = Some(
                    frame
                        .click_point_moved(Duration::ZERO, &event.surface.id(), x, y)
                        .unwrap_or(CursorIcon::Default),
                );
                client_state.last_enter_serial = serial;
            },
            PointerEventKind::Leave { serial: _ } => {
                frame.click_point_left();
            },
            PointerEventKind::Motion { time: _ } => {
                new_cursor = frame.click_point_moved(Duration::ZERO, &event.surface.id(), x, y);

                if self.move_active {
                    new_cursor = Some(CursorIcon::Move);
                }

                if self.move_active && !self.pending_frame_callback {
                    let (init_x, init_y) = self.move_pointer_location;
                    let offset_x = (x - init_x).round() as i32;
                    let offset_y = (y - init_y).round() as i32;

                    if (offset_x, offset_y) != (0, 0) {
                        let mut geo = x11_surface.geometry();
                        geo.loc.x += offset_x;
                        geo.loc.y += offset_y;

                        // frame() will cause the compositor to send us a frame callback on the next commit.
                        // So if we commit() the parent and call frame(), then we know that the compositor
                        // has processed our last set_position call, and therefore the next set of motion
                        // coordinates will be relative to it.  Without this, we won't know if the coordinates
                        // receive are relative to our current position or a previous position.
                        self.move_(geo.loc.x, geo.loc.y, qh);

                        x11_surface.configure(geo).location(loc!())?;
                    }
                }
            },
            PointerEventKind::Press { button, serial, .. }
            | PointerEventKind::Release { button, serial, .. } => {
                let kind = &event.kind;
                let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                let click = match button {
                    BTN_LEFT => FrameClick::Normal,
                    BTN_RIGHT => FrameClick::Alternate,
                    _ => return Ok(None),
                };

                if let Some(action) = frame.on_click(Duration::ZERO, click, pressed) {
                    debug!("button: {click:?}, kind: {kind:?}, action {action:?}");

                    self.frame_action(x11_surface, pointer, serial.into(), action, event.position)
                        .location(loc!())?;
                } else {
                    self.move_active = false;
                }
            },
            PointerEventKind::Axis { .. } => {},
        }

        Ok(new_cursor)
    }
}
