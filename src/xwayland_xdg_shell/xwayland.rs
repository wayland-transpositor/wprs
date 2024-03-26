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

use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::thread;

use smithay::reexports::wayland_server::Resource;
use smithay::utils::Logical;
use smithay::utils::Rectangle;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::selection::SelectionTarget;
use smithay::xwayland::xwm::Reorder;
use smithay::xwayland::xwm::ResizeEdge as X11ResizeEdge;
use smithay::xwayland::xwm::XwmId;
use smithay::xwayland::X11Surface;
use smithay::xwayland::X11Wm;
use smithay::xwayland::XwmHandler;

use crate::prelude::*;
use crate::xwayland_xdg_shell::client::Role;
use crate::xwayland_xdg_shell::xsurface_from_x11_surface;
use crate::xwayland_xdg_shell::WprsState;

impl XwmHandler for WprsState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.compositor_state.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).unwrap();

        if let Some(xwayland_surface) = xsurface_from_x11_surface(&mut self.surfaces, &window) {
            if let Some(Role::XdgPopup(_popup)) = &xwayland_surface.role {
                let mut geo = window.geometry();
                geo.loc.x = 0;
                geo.loc.y = 0;
                window.configure(geo).log_and_ignore(loc!());
            }
        }
        self.compositor_state.x11_surfaces.push(window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.compositor_state.x11_surfaces.push(window);
    }

    #[instrument(skip(self, _xwm), level = "debug")]
    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(wl_surface) = window.wl_surface() {
            // TODO: verify that we don't end up with stale entries
            let surface_id = wl_surface.id();
            self.remove_surface(&surface_id);

            // TODO: maybe do this on leave?
            // Without this, xwayland still thinks the key that triggered the
            // window close is still held down and sends key repeat events.
            if let Some(keyboard) = self.compositor_state.seat.get_keyboard() {
                if keyboard
                    .current_focus()
                    .map_or(false, |focus| focus == window)
                {
                    let serial = SERIAL_COUNTER.next_serial();
                    keyboard.set_focus(self, None, serial);
                }
            }
        }

        if !window.is_override_redirect() {
            window.set_mapped(false).unwrap();
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
            // Under Wayland, windows don't get to resize themselves. Many X apps
            // need a synthetic configure reply though. Additionally, some broken
            // toolkits (read: Java) will still render the window at the size they
            // asked for, even if the request wasn't granted, and also ignore
            // ConfigureNotify events where the size equals the current size, so
            // trigger a redraw by resizing the window by a small amount and then
            // resizing it back to the original size.
            let mut hack_geo = geo;

            hack_geo.size.w -= 1;
            window.configure(hack_geo).unwrap();
            window.configure(geo).unwrap();
        } else {
            window.configure(geo).unwrap();
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // TODO: Do we need to also reposition xdg-popups?
        if let Some(xwayland_surface) = xsurface_from_x11_surface(&mut self.surfaces, &window) {
            if let Some(Role::SubSurface(subsurface)) = &mut xwayland_surface.role {
                if !subsurface.move_active {
                    subsurface.move_(geometry.loc.x, geometry.loc.y, &self.client_state.qh);
                }
            }
        }
    }

    // For maximize and fullscreeen: send the appropriate request to the wayland
    // compositor we're running in and let the subsequent configure trigger the
    // appropriate X11 configures.
    //
    // For unmaximize and unfullscreen: the wayland compositor we're running in
    // will follow up with a configure with the geometry to use, so we don't
    // need to worry about that saving the old geometry and restoring it here.

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(xwayland_surface) = xsurface_from_x11_surface(&mut self.surfaces, &window) {
            if let Some(Role::XdgToplevel(toplevel)) = &xwayland_surface.role {
                toplevel.local_window.set_maximized();
            } else {
                warn!("Received maximize request for non-XdgToplevel surface.");
            }
        } else {
            warn!("Received maximize request for unknown surface.");
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(xwayland_surface) = xsurface_from_x11_surface(&mut self.surfaces, &window) {
            if let Some(Role::XdgToplevel(toplevel)) = &xwayland_surface.role {
                toplevel.local_window.unset_maximized();
            } else {
                warn!("Received unmaximize request for non-XdgToplevel surface.");
            }
        } else {
            warn!("Received unmaximize request for unknown surface.");
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(xwayland_surface) = xsurface_from_x11_surface(&mut self.surfaces, &window) {
            if let Some(Role::XdgToplevel(toplevel)) = &mut xwayland_surface.role {
                toplevel.local_window.set_fullscreen(None);
            } else {
                warn!("Received fullscreen request for non-XdgToplevel surface.");
            }
        } else {
            warn!("Received fullscreen request for unknown surface.");
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(xwayland_surface) = xsurface_from_x11_surface(&mut self.surfaces, &window) {
            if let Some(Role::XdgToplevel(toplevel)) = &mut xwayland_surface.role {
                toplevel.local_window.unset_fullscreen();
            } else {
                warn!("Received unfullscreen request for non-XdgToplevel surface.");
            }
        } else {
            warn!("Received unfullscreen request for unknown surface.");
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edges: X11ResizeEdge,
    ) {
        // TODO, base on frame_action, but need to get serial from somewhere.
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {
        // TODO, base on frame_action, but need to get serial from somewhere.
    }

    #[instrument(skip(self, _xwm), level = "debug")]
    fn allow_selection_access(&mut self, _xwm: XwmId, selection: SelectionTarget) -> bool {
        true
        // TODO: the below should be correct but needs to be verified.
        // !self.client_state.selection_offers.is_empty()
    }

    #[instrument(skip(self, _xwm), level = "debug")]
    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let read_pipe = match selection {
            SelectionTarget::Primary => {
                let Some(cur_offer) = self.client_state.primary_selection_offer.clone() else {
                    warn!("primary_selection_offer was empty");
                    return;
                };

                cur_offer.receive(mime_type.clone()).ok()
            },
            SelectionTarget::Clipboard => {
                let Some(cur_offer) = self.client_state.selection_offer.clone() else {
                    warn!("selection_offer was empty");
                    return;
                };
                cur_offer.receive(mime_type.clone()).ok()
            },
        };

        if let Some(mut read_pipe) = read_pipe {
            debug!("spawning send_selection thread for mime {mime_type}");
            thread::spawn(move || {
                debug!("in send_selection thread for mime {mime_type}");
                let mut f = File::from(fd);

                // NOTE: this block is useful debugging.
                // let mut buf = Vec::new();
                // let bytes_copied = read_pipe.read_to_end(&mut buf).unwrap();
                // debug!("read selection: {buf:?}");
                // f.write_all(&buf);

                let bytes_copied = io::copy(&mut read_pipe, &mut f);
                debug!("wrote selection: {bytes_copied:?} bytes");
            });
        }
    }

    #[instrument(skip(self, _xwm), level = "debug")]
    fn new_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mut mime_types: Vec<String>,
    ) {
        if let Some(seat_obj) = self.client_state.seat_objects.last() {
            mime_types.push("_xwayland_xdg_shell_marker".to_owned());

            match selection {
                SelectionTarget::Clipboard => {
                    let source = self
                        .client_state
                        .data_device_manager_state
                        .create_copy_paste_source(
                            &self.client_state.qh,
                            mime_types.iter().map(String::as_str),
                        );

                    source.set_selection(
                        &seat_obj.data_device,
                        self.client_state.last_implicit_grab_serial,
                    );

                    self.client_state.selection_source = Some(source);
                },
                SelectionTarget::Primary => {
                    if let (Some(primary_selection_manager_state), Some(primary_selection_device)) = (
                        &self.client_state.primary_selection_manager_state,
                        &seat_obj.primary_selection_device,
                    ) {
                        let source = primary_selection_manager_state.create_selection_source(
                            &self.client_state.qh,
                            mime_types.iter().map(String::as_str),
                        );

                        source.set_selection(
                            primary_selection_device,
                            self.client_state.last_implicit_grab_serial,
                        );

                        self.client_state.primary_selection_source = Some(source);
                    }
                },
            };
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, _selection: SelectionTarget) {
        // TODO
    }
}
