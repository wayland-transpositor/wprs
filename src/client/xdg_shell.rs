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

use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner;
use smithay_client_toolkit::shell::xdg;
use smithay_client_toolkit::shell::xdg::popup;
use smithay_client_toolkit::shell::xdg::window::Window;
use smithay_client_toolkit::shell::xdg::window::WindowDecorations;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::XdgSurface;

use crate::client::ObjectBimap;
use crate::client::RemoteSurface;
use crate::client::Role;
use crate::client::WprsClientState;
use crate::prelude::*;
use crate::serialization::geometry::Size;
use crate::serialization::wayland::SurfaceState;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::xdg_shell::XdgPopupId;
use crate::serialization::xdg_shell::XdgPositioner;
use crate::serialization::xdg_shell::XdgToplevelId;
use crate::serialization::ClientId;
use crate::serialization::ObjectId;

#[derive(Debug)]
pub struct RemoteXdgToplevel {
    pub client: ClientId,
    pub id: XdgToplevelId,
    pub local_window: Window,
    // TODO: add configured field to Window, have it be set before dispatching
    // first configure;
    pub configured: bool,
    pub title: Option<String>,
    pub title_prefix: String,
    pub app_id: Option<String>,
    pub max_size: Size<i32>,
    pub min_size: Size<i32>,
}

impl RemoteXdgToplevel {
    pub fn set_role(
        client_id: ClientId,
        surface_state: &SurfaceState,
        surface_id: WlSurfaceId,
        surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
        xdg_shell_state: &XdgShell,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<()> {
        let local_surface = {
            let surface = surfaces.get_mut(&surface_id).location(loc!())?;
            if surface.role.is_some() {
                return Ok(());
            }
            surface.local_surface.take().location(loc!())?
        };
        let toplevel_state = surface_state.xdg_toplevel()?;

        let local_window =
            xdg_shell_state.create_window(local_surface, WindowDecorations::ServerDefault, qh);

        {
            let toplevel_state = surface_state
                .role
                .as_ref()
                .location(loc!())?
                .as_xdg_toplevel()
                .location(loc!())?;
            if let Some(id) = toplevel_state.parent {
                local_window.set_parent(Some(
                    &surfaces
                        .get(&id)
                        .location(loc!())?
                        .xdg_toplevel()
                        .location(loc!())?
                        .local_window,
                ));
            }
        }

        object_bimap.insert(
            (client_id, ObjectId::XdgToplevel(toplevel_state.id)),
            local_window.xdg_toplevel().id(),
        );

        let new_toplevel = Self {
            client: client_id,
            id: toplevel_state.id,
            local_window,
            configured: false,
            title: None,
            title_prefix: String::new(),
            app_id: None,
            max_size: (0, 0).into(),
            min_size: (0, 0).into(),
        };

        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        surface.role = Some(Role::XdgToplevel(new_toplevel));
        Ok(())
    }

    fn set_title(&mut self, title: Option<String>) {
        if self.title != title {
            self.title = title;
            if let Some(title) = &self.title {
                self.local_window
                    .set_title(format!("{}{}", self.title_prefix, title));
            }
        }
    }

    fn set_app_id(&mut self, app_id: Option<String>) {
        if self.app_id != app_id {
            self.app_id = app_id;
            if let Some(app_id) = &self.app_id {
                self.local_window.set_app_id(app_id);
            }
        }
    }

    fn set_max_size(&mut self, max_size: Size<i32>) {
        if self.max_size != max_size {
            self.max_size = max_size;
            self.local_window
                .set_max_size(Some((self.max_size.w as u32, self.max_size.h as u32)));
        }
    }

    fn set_min_size(&mut self, min_size: Size<i32>) {
        if self.min_size != min_size {
            self.min_size = min_size;
            self.local_window
                .set_min_size(Some((self.min_size.w as u32, self.min_size.h as u32)));
        }
    }

    pub fn update(surface_state: SurfaceState, surface: &mut RemoteSurface) -> Result<()> {
        let remote_toplevel = surface
            .role
            .as_mut()
            .location(loc!())?
            .as_xdg_toplevel_mut()
            .location(loc!())?;

        // TODO: only update if changed

        // TODO: why isn't this always set?
        // let xdg_surface_state = surface_state.xdg_surface_state.as_ref().unwrap();
        if let Some(xdg_surface_state) = &surface_state.xdg_surface_state {
            if let Some(window_geometry) = xdg_surface_state.window_geometry {
                remote_toplevel.set_window_geometry(
                    window_geometry.loc.x as u32,
                    window_geometry.loc.y as u32,
                    window_geometry.size.w as u32,
                    window_geometry.size.h as u32,
                );
                remote_toplevel.set_max_size(xdg_surface_state.max_size);
                remote_toplevel.set_min_size(xdg_surface_state.min_size);
            }
        }

        let toplevel_state = surface_state
            .role
            .location(loc!())?
            .into_xdg_toplevel()
            // The error type is the enum. :(
            .map_err(|_| anyhow!("role wasn't xdg toplevel"))
            .location(loc!())?;

        remote_toplevel.set_title(toplevel_state.title);
        remote_toplevel.set_app_id(toplevel_state.app_id);

        Ok(())
    }

    pub fn apply(
        client_id: ClientId,
        surface_state: SurfaceState,
        surface_id: WlSurfaceId,
        surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
        xdg_shell_state: &XdgShell,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<()> {
        Self::set_role(
            client_id,
            &surface_state,
            surface_id,
            surfaces,
            xdg_shell_state,
            qh,
            object_bimap,
        )
        .location(loc!())?;
        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        Self::update(surface_state, surface)
    }
}

#[derive(Debug)]
pub struct RemoteXdgPopup {
    pub client: ClientId,
    pub id: XdgPopupId,
    pub local_popup: popup::Popup,
    // TODO: add configured field to Popup, have it be set before dispatching
    // first configure;
    pub configured: bool,
    pub positioner: XdgPositioner,
}

impl RemoteXdgPopup {
    pub fn new_positioner(
        xdg_shell_state: &XdgShell,
        positioner: &XdgPositioner,
    ) -> Result<xdg::XdgPositioner> {
        let new_positioner = xdg::XdgPositioner::new(xdg_shell_state).location(loc!())?;
        new_positioner.set_size(positioner.width, positioner.height);
        new_positioner.set_anchor_rect(
            positioner.anchor_rect.loc.x,
            positioner.anchor_rect.loc.y,
            positioner.anchor_rect.size.w,
            positioner.anchor_rect.size.h,
        );
        new_positioner.set_anchor(
            xdg_positioner::Anchor::try_from(positioner.anchor_edges)
                // The error type is (). :(
                .map_err(|_| anyhow!("invalid anchor"))
                .location(loc!())?,
        );
        new_positioner.set_gravity(
            xdg_positioner::Gravity::try_from(positioner.gravity)
                // The error type is (). :(
                .map_err(|_| anyhow!("invalid anchor"))
                .location(loc!())?,
        );
        new_positioner.set_constraint_adjustment(positioner.constraint_adjustment);
        new_positioner.set_offset(positioner.offset.x, positioner.offset.y);
        if positioner.reactive {
            new_positioner.set_reactive();
        }
        if let Some(parent_size) = &positioner.parent_size {
            new_positioner.set_parent_size(parent_size.w, parent_size.h);
        };
        if let Some(parent_configure) = positioner.parent_configure {
            new_positioner.set_parent_configure(parent_configure);
        };
        Ok(new_positioner)
    }

    pub fn set_role(
        client_id: ClientId,
        surface_state: &SurfaceState,
        surface_id: WlSurfaceId,
        surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
        xdg_shell_state: &XdgShell,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<()> {
        let local_surface = {
            let surface = surfaces.get_mut(&surface_id).location(loc!())?;
            if surface.role.is_some() {
                return Ok(());
            }
            surface.local_surface.take().location(loc!())?
        };
        let popup_state = surface_state.xdg_popup().location(loc!())?;

        let parent = {
            let popup_state = surface_state
                .role
                .as_ref()
                .location(loc!())?
                .as_xdg_popup()
                .location(loc!())?;
            let parent_surface = surfaces
                .get(&popup_state.parent_surface_id)
                .location(loc!())?;
            parent_surface.xdg_surface()
        };

        let positioner =
            Self::new_positioner(xdg_shell_state, &popup_state.positioner).location(loc!())?;

        let local_popup = popup::Popup::from_surface(
            parent.as_ref(),
            &positioner,
            qh,
            local_surface,
            xdg_shell_state,
        )
        .location(loc!())?;

        // if popup_state.grab_requested {
        //     local_popup
        //         .xdg_popup()
        //         .grab(&seat_state.seats().next().location(loc!())?, 0); // TODO: serial
        // }

        object_bimap.insert(
            (client_id, ObjectId::XdgPopup(popup_state.id)),
            local_popup.xdg_popup().id(),
        );

        let new_popup = Self {
            client: client_id,
            id: popup_state.id,
            local_popup,
            configured: false,
            positioner: popup_state.positioner,
        };
        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        surface.role = Some(Role::XdgPopup(new_popup));
        Ok(())
    }

    pub fn update(
        surface_state: SurfaceState,
        surface: &RemoteSurface,
        xdg_shell_state: &XdgShell,
    ) -> Result<()> {
        let remote_popup = surface
            .role
            .as_ref()
            .location(loc!())?
            .as_xdg_popup()
            .location(loc!())?;
        // TODO: why isn't this always set?
        // let xdg_surface_state = surface_state.xdg_surface_state.as_ref().location(loc!())?;
        if let Some(xdg_surface_state) = surface_state.xdg_surface_state {
            if let Some(window_geometry) = xdg_surface_state.window_geometry {
                remote_popup.set_window_geometry(
                    window_geometry.loc.x as u32,
                    window_geometry.loc.y as u32,
                    window_geometry.size.w as u32,
                    window_geometry.size.h as u32,
                );
            }
        }

        let popup_state = surface_state.xdg_popup().location(loc!())?;
        if remote_popup.positioner != popup_state.positioner {
            let positioner =
                Self::new_positioner(xdg_shell_state, &popup_state.positioner).location(loc!())?;
            surface
                .role
                .as_ref()
                .location(loc!())?
                .as_xdg_popup()
                .location(loc!())?
                .local_popup
                .reposition(&positioner, 0);
        }

        Ok(())
    }

    pub fn apply(
        client_id: ClientId,
        surface_state: SurfaceState,
        surface_id: WlSurfaceId,
        surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
        xdg_shell_state: &XdgShell,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<()> {
        Self::set_role(
            client_id,
            &surface_state,
            surface_id,
            surfaces,
            xdg_shell_state,
            qh,
            object_bimap,
        )
        .location(loc!())?;
        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        Self::update(surface_state, surface, xdg_shell_state)
    }
}
