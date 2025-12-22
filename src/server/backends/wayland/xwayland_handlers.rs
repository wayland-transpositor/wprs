use std::process::Stdio;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor;
use smithay::wayland::xwayland_shell::XWaylandShellHandler;
use smithay::xwayland::X11Surface;
use smithay::xwayland::X11Wm;
use smithay::xwayland::XWayland;
use smithay::xwayland::XWaylandEvent;
use smithay::xwayland::XwmHandler;
use smithay::xwayland::xwm::XwmId;

use crate::prelude::*;
use crate::protocols::wprs::wayland::Role;
use crate::protocols::wprs::xdg_shell::XdgToplevelId;
use crate::protocols::wprs::xdg_shell::XdgToplevelState;

use super::LockedSurfaceState;
use super::WprsServerState;

#[derive(Debug, Clone)]
pub(crate) struct XwaylandSurfaceData {
    pub(crate) x11_surface: X11Surface,
}

impl WprsServerState {
    pub fn start_xwayland(&mut self, wayland_debug: bool, preferred_display: Option<u32>) -> Result<()> {
        if self.xwm.is_some() {
            return Ok(());
        }

        let env = vec![("WAYLAND_DEBUG", if wayland_debug { "1" } else { "0" })];

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
            },
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

        let _ = token;
        Ok(())
    }
}

impl XWaylandShellHandler for WprsServerState {
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
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
}

