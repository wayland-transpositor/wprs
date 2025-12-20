use std::process::Stdio;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::compositor;
use smithay::wayland::xwayland_shell::XWaylandShellHandler;
use smithay::xwayland::X11Surface;
use smithay::xwayland::X11Wm;
use smithay::xwayland::XWayland;
use smithay::xwayland::XWaylandEvent;
use smithay::xwayland::XwmHandler;
use smithay::xwayland::xwm::Reorder;
use smithay::xwayland::xwm::ResizeEdge as X11ResizeEdge;
use smithay::xwayland::xwm::WmWindowProperty;
use smithay::xwayland::xwm::XwmId;

use crate::prelude::*;
use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::SendType;
use crate::protocols::wprs::wayland::Role;
use crate::protocols::wprs::wayland::WlSurfaceId;
use crate::protocols::wprs::xdg_shell::ToplevelRequest;
use crate::protocols::wprs::xdg_shell::ToplevelRequestPayload;
use crate::protocols::wprs::xdg_shell::XdgToplevelId;
use crate::protocols::wprs::xdg_shell::XdgToplevelState;
use crate::server::config::XwaylandMode;

use super::LockedSurfaceState;
use super::WprsServerState;

#[derive(Debug, Clone)]
pub(crate) struct XwaylandSurfaceData {
    pub(crate) x11_surface: X11Surface,
}

impl WprsServerState {
    pub fn start_xwayland_inline_proxy(
        &mut self,
        wayland_debug: bool,
        preferred_display: Option<u32>,
    ) -> Result<()> {
        if self.xwayland_mode != XwaylandMode::InlineProxy {
            bail!("start_xwayland_inline_proxy called when xwayland_mode != inline-proxy");
        }
        if self.xwm.is_some() {
            return Ok(());
        }

        let env = vec![(
            "WAYLAND_DEBUG",
            if wayland_debug { "1" } else { "0" },
        )];

        let (xwayland, client) = match XWayland::spawn(
            &self.dh,
            preferred_display,
            env.clone(),
            false,
            Stdio::inherit(),
            Stdio::inherit(),
            |_| {},
        ) {
            Ok(v) => v,
            Err(err) => {
                if preferred_display.is_some() {
                    warn!(
                        "failed to start Xwayland on preferred display {preferred_display:?}: {err:?}; falling back to auto-pick"
                    );
                    XWayland::spawn(
                        &self.dh,
                        None,
                        env,
                        false,
                        Stdio::inherit(),
                        Stdio::inherit(),
                        |_| {},
                    )
                    .map_err(|e| anyhow!("failed to start Xwayland: {e:?}"))
                    .location(loc!())?
                } else {
                    return Err(anyhow!("failed to start Xwayland: {err:?}")).location(loc!());
                }
            }
        };

        let token = self
            .lh
            .insert_source(xwayland, move |event, _, state| {
                match event {
                    XWaylandEvent::Ready {
                        x11_socket,
                        display_number,
                    } => {
                        info!(
                            "Xwayland ready: set DISPLAY=:{display_number} to run X11 apps in this session"
                        );

                        match X11Wm::start_wm(state.lh.clone(), x11_socket, client.clone()) {
                            Ok(wm) => {
                                state.xwm = Some(wm);
                            },
                            Err(err) => {
                                error!("failed to attach X11 Window Manager: {err:?}");
                                state.xwm = None;
                            },
                        }
                    },
                    XWaylandEvent::Error => {
                        error!("Xwayland error: X11 apps will not work (Xwayland is down)");
                        state.xwm = None;
                    },
                };
            })
            .location(loc!())?;

        // Keep the source registered for the lifetime of the compositor.
        let _ = token;
        Ok(())
    }

    pub(crate) fn x11_surface_for_wl_surface_id(&self, surface_id: &WlSurfaceId) -> Option<X11Surface> {
        let object_id = self.object_map.get(surface_id)?.clone();
        let wl_surface = WlSurface::from_id(&self.dh, object_id).ok()?;
        compositor::with_states(&wl_surface, |surface_data| {
            surface_data
                .data_map
                .get::<XwaylandSurfaceData>()
                .map(|data| data.x11_surface.clone())
        })
    }

    pub(crate) fn send_toplevel_request_for_surface(
        &self,
        surface: &WlSurface,
        payload: ToplevelRequestPayload,
    ) {
        let Some(client) = surface.client() else {
            return;
        };
        self.serializer
            .writer()
            .send(SendType::Object(Request::Toplevel(ToplevelRequest {
                client: ClientId::new(&client),
                surface: WlSurfaceId::new(surface),
                payload,
            })));
    }
}

impl XWaylandShellHandler for WprsServerState {
    fn xwayland_shell_state(&mut self) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, wl_surface: WlSurface, surface: X11Surface) {
        self.insert_surface(&wl_surface).log_and_ignore(loc!());

        self.xwayland_surfaces.insert(wl_surface.id());

        compositor::with_states(&wl_surface, |surface_data| {
            surface_data
                .data_map
                .insert_if_missing_threadsafe(|| XwaylandSurfaceData {
                    x11_surface: surface.clone(),
                });

            let title = match surface.title().as_str() {
                "" => None,
                other => Some(other.to_string()),
            };
            let app_id = match surface.class().as_str() {
                "" => None,
                other => Some(other.to_string()),
            };

            let toplevel_state = XdgToplevelState {
                id: XdgToplevelId::from(&wl_surface.id()),
                parent: None,
                title,
                app_id,
                decoration_mode: None,
                maximized: None,
                fullscreen: None,
            };

            let surface_state = &mut surface_data
                .data_map
                .get::<LockedSurfaceState>()
                .unwrap()
                .0
                .lock()
                .unwrap();

            surface_state.role = Some(Role::XdgToplevel(toplevel_state));
        });
    }
}

impl XwmHandler for WprsServerState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).log_and_ignore(loc!());
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(wl_surface) = window.wl_surface() {
            self.xwayland_surfaces.remove(&wl_surface.id());
            compositor::with_states(&wl_surface, |surface_data| {
                if let Some(surface_state) =
                    surface_data.data_map.get::<LockedSurfaceState>()
                {
                    surface_state.0.lock().unwrap().role = None;
                }
            });
        }

        if !window.is_override_redirect() {
            window.set_mapped(false).log_and_ignore(loc!());
        }
    }

    fn destroyed_window(&mut self, xwm: XwmId, window: X11Surface) {
        self.unmapped_window(xwm, window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geo = window.geometry();

        if let Some(x) = x {
            geo.loc.x = x;
        }
        if let Some(y) = y {
            geo.loc.y = y;
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }

        if window.is_mapped() {
            let mut hack_geo = geo;
            hack_geo.size.w -= 1;
            window.configure(hack_geo).log_and_ignore(loc!());
        }
        window.configure(geo).log_and_ignore(loc!());
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        _above: Option<u32>,
    ) {
    }

    fn property_notify(&mut self, _xwm: XwmId, _window: X11Surface, _property: WmWindowProperty) {}

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(wl_surface) = window.wl_surface() {
            self.send_toplevel_request_for_surface(&wl_surface, ToplevelRequestPayload::SetMaximized);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(wl_surface) = window.wl_surface() {
            self.send_toplevel_request_for_surface(
                &wl_surface,
                ToplevelRequestPayload::UnsetMaximized,
            );
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(wl_surface) = window.wl_surface() {
            self.send_toplevel_request_for_surface(&wl_surface, ToplevelRequestPayload::SetFullscreen);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(wl_surface) = window.wl_surface() {
            self.send_toplevel_request_for_surface(
                &wl_surface,
                ToplevelRequestPayload::UnsetFullscreen,
            );
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edges: X11ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    #[instrument(skip(self), level = "debug")]
    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: smithay::wayland::selection::SelectionTarget) -> bool {
        // TODO: selection bridging between Wayland and X11.
        false
    }
}
