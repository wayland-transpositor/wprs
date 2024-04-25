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
use std::sync::Arc;

use bimap::BiMap;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::FilterResult;
use smithay::output::Output;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::backend::ObjectId as CompositorObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as CompositorWlSurface;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::xwayland::xwm::WmWindowType;
use smithay::xwayland::X11Surface;
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::compositor::Surface;
use smithay_client_toolkit::compositor::SurfaceData;
use smithay_client_toolkit::reexports::client::backend::ObjectId as ClientObjectId;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface as ClientWlSurface;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::subcompositor::SubcompositorState;
use tracing::Span;

use crate::args;
use crate::prelude::*;
use crate::xwayland_xdg_shell::client::XWaylandSubSurface;

pub mod client;
pub mod compositor;
pub mod decoration;
pub mod wmname;
pub mod xwayland;

use client::Role;
use client::WprsClientState;
use client::XWaylandBuffer;
use client::XWaylandXdgPopup;
use client::XWaylandXdgToplevel;
use compositor::DecorationBehavior;
use compositor::WprsCompositorState;
use compositor::X11Parent;

#[derive(Debug, Default)]
pub struct XWaylandSurface {
    pub(crate) x11_surface: Option<X11Surface>,
    pub(crate) buffer: Option<XWaylandBuffer>,
    pub(crate) buffer_attached: bool,
    // None when the surface is owned by a role object (e.g., a Window).
    pub(crate) local_surface: Option<Surface>,
    pub(crate) role: Option<Role>,
    pub(crate) parent: Option<X11Parent>,
    pub(crate) children: HashSet<CompositorObjectId>,
}

impl XWaylandSurface {
    pub fn get_x11_surface(&self) -> Result<&X11Surface> {
        self.x11_surface
            .as_ref()
            .ok_or(anyhow!("x11_surface was None"))
    }

    pub fn new(
        compositor_wl_surface: &CompositorWlSurface,
        compositor_state: &CompositorState,
        qh: &QueueHandle<WprsState>,
        surface_bimap: &mut BiMap<CompositorObjectId, ClientObjectId>,
    ) -> Result<Self> {
        let local_surface = Surface::new(compositor_state, qh).location(loc!())?;
        surface_bimap.insert(compositor_wl_surface.id(), local_surface.wl_surface().id());

        Ok(Self {
            x11_surface: None,
            buffer: None,
            buffer_attached: false,
            local_surface: Some(local_surface),
            role: None,
            parent: None,
            children: HashSet::new(),
        })
    }

    fn update_local_surface(
        &mut self,
        compositor_wl_surface: &CompositorWlSurface,
        parent: Option<&ClientWlSurface>,
        compositor_state: &CompositorState,
        qh: &QueueHandle<WprsState>,
        surface_bimap: &mut BiMap<CompositorObjectId, ClientObjectId>,
    ) -> Result<()> {
        let local_surface =
            Surface::with_data(compositor_state, qh, SurfaceData::new(parent.cloned(), 1))
                .location(loc!())?;

        surface_bimap.insert(compositor_wl_surface.id(), local_surface.wl_surface().id());
        self.local_surface = Some(local_surface);

        Ok(())
    }

    fn ready(&self) -> bool {
        match &self.role {
            Some(Role::XdgToplevel(toplevel)) if !toplevel.configured => false,
            Some(Role::XdgPopup(popup)) if !popup.configured => false,
            _ => self.x11_surface.is_some() || matches!(self.role, Some(Role::Cursor)),
        }
    }

    fn needs_configure(&self) -> bool {
        match &self.role {
            Some(Role::XdgToplevel(toplevel)) if !toplevel.configured => true,
            Some(Role::XdgPopup(popup)) if !popup.configured => true,
            _ => false,
        }
    }

    fn try_attach_buffer(&mut self) {
        if !self.buffer_attached {
            if let Some(buffer) = &self.buffer {
                let surface = self.wl_surface();
                // The only possible error here is AlreadyActive, which we can
                // ignore.
                _ = buffer.active_buffer.attach_to(surface);
                surface.damage_buffer(0, 0, i32::MAX, i32::MAX);

                self.buffer_attached = true;
            }
        }
    }

    fn commit_buffer(&mut self, qh: &QueueHandle<WprsState>) {
        if !self.buffer_attached && self.buffer.is_some() {
            self.try_attach_buffer();

            self.frame(qh);
            self.commit();
        }
    }

    #[instrument(skip(xdg_shell_state, qh), level = "debug")]
    fn update_x11_surface(
        &mut self,
        x11_surface: X11Surface,
        parent: Option<X11Parent>,
        fallback_parent: &Option<X11Parent>,
        xdg_shell_state: &XdgShell,
        shm_state: &Shm,
        subcompositor_state: Arc<SubcompositorState>,
        qh: &QueueHandle<WprsState>,
        decoration_behavior: DecorationBehavior,
    ) -> Result<()> {
        self.x11_surface = Some(x11_surface);
        if self.role.is_some() {
            return Ok(());
        }

        let x11_surface = self.get_x11_surface().location(loc!())?;

        // https://specifications.freedesktop.org/wm-spec/wm-spec-latest.html#idm45317634120064
        let window_type = x11_surface.window_type().unwrap_or_else(|| {
            if x11_surface.is_override_redirect() {
                WmWindowType::Normal
            } else if x11_surface.is_transient_for().is_some() {
                WmWindowType::Dialog
            } else {
                WmWindowType::Normal
            }
        });

        enum WaylandWindowType {
            Toplevel,
            Popup,
            SubSurface,
        }

        let wayland_window_type = if parent.is_some() {
            // X11 child windows will try to place their location relative to their parent.
            // We use subsurfaces to let them be placed outside the bounds of their toplevel
            // window.

            WaylandWindowType::SubSurface
        } else {
            match window_type {
                // Java uses Dialog with override-redirect for dropbown menus.
                WmWindowType::Dialog if x11_surface.is_override_redirect() => {
                    WaylandWindowType::Popup
                },
                // gvim uses Normal with override-redirect for tooltips.
                WmWindowType::Normal if x11_surface.is_override_redirect() => {
                    WaylandWindowType::Popup
                },
                // Firefox uses Utility with override-redirect for its hamburger
                // menu.
                WmWindowType::Utility if x11_surface.is_override_redirect() => {
                    WaylandWindowType::Popup
                },
                WmWindowType::Dialog
                | WmWindowType::Normal
                | WmWindowType::Splash
                | WmWindowType::Utility => WaylandWindowType::Toplevel,
                WmWindowType::DropdownMenu
                | WmWindowType::Menu
                | WmWindowType::Notification
                | WmWindowType::PopupMenu
                | WmWindowType::Toolbar
                | WmWindowType::Tooltip => WaylandWindowType::Popup,
            }
        };

        let parent_if_toplevel = parent.clone();
        let parent_if_popup = parent.clone().or_else(|| fallback_parent.clone());
        let parent_if_subsurface = parent.or_else(|| fallback_parent.clone());

        match wayland_window_type {
            WaylandWindowType::Toplevel => {
                debug!("creating xdg_toplevel for {self:?}");
                self.parent.clone_from(&parent_if_toplevel);
                XWaylandXdgToplevel::set_role(
                    self,
                    parent_if_toplevel.and_then(|p| p.for_toplevel).as_ref(),
                    xdg_shell_state,
                    shm_state,
                    subcompositor_state,
                    qh,
                    decoration_behavior,
                )
                .location(loc!())?;
            },
            WaylandWindowType::Popup if parent_if_popup.is_none() => {
                debug!(
                    "creating xdg_toplevel for {self:?} instead of popup because parent was None"
                );
                self.parent = None;
                XWaylandXdgToplevel::set_role(
                    self,
                    None,
                    xdg_shell_state,
                    shm_state,
                    subcompositor_state,
                    qh,
                    decoration_behavior,
                )
                .location(loc!())?;
            },
            WaylandWindowType::Popup if parent_if_popup.clone().unwrap().for_popup.is_none() => {
                debug!("creating subsurface for {self:?} instead of popup because parent was subsurface");
                self.parent.clone_from(&parent_if_subsurface);
                XWaylandSubSurface::set_role(
                    self,
                    parent_if_subsurface.unwrap().for_subsurface,
                    shm_state,
                    subcompositor_state,
                    qh,
                )
                .location(loc!())?;
            },
            WaylandWindowType::Popup => {
                debug!("creating xdg_popup for {self:?}");
                self.parent.clone_from(&parent_if_popup);
                XWaylandXdgPopup::set_role(
                    self,
                    &parent_if_popup.unwrap().for_popup.unwrap(),
                    xdg_shell_state,
                    qh,
                )
                .location(loc!())?;
            },
            WaylandWindowType::SubSurface => {
                debug!("creating subsurface for {self:?}");
                self.parent.clone_from(&parent_if_subsurface);
                XWaylandSubSurface::set_role(
                    self,
                    parent_if_subsurface.unwrap().for_subsurface,
                    shm_state,
                    subcompositor_state,
                    qh,
                )
                .location(loc!())?;
            },
            // TODO: do we need a None for hidden helper windows?
        }

        Ok(())
    }
}

impl WaylandSurface for XWaylandSurface {
    fn wl_surface(&self) -> &ClientWlSurface {
        match &self.role {
            None | Some(Role::Cursor) => self.local_surface.as_ref().unwrap().wl_surface(),
            Some(Role::XdgToplevel(remote_xdg_toplevel)) => {
                remote_xdg_toplevel.local_window.wl_surface()
            },
            Some(Role::XdgPopup(remote_xdg_popup)) => remote_xdg_popup.local_popup.wl_surface(),
            Some(Role::SubSurface(remote_subsurface)) => remote_subsurface.wl_surface(),
        }
    }
}

#[derive(Debug)]
pub struct WprsState {
    pub dh: DisplayHandle,
    pub event_loop_handle: LoopHandle<'static, Self>,
    pub client_state: WprsClientState,
    pub compositor_state: WprsCompositorState,
    pub surface_bimap: BiMap<CompositorObjectId, ClientObjectId>,
    pub surfaces: HashMap<CompositorObjectId, XWaylandSurface>,
    pub outputs: HashMap<u32, Output>,
}

impl WprsState {
    pub fn new(
        dh: DisplayHandle,
        globals: &GlobalList,
        qh: QueueHandle<Self>,
        conn: Connection,
        event_loop_handle: LoopHandle<'static, Self>,
        decoration_behavior: DecorationBehavior,
    ) -> Result<Self> {
        Ok(Self {
            dh: dh.clone(),
            event_loop_handle: event_loop_handle.clone(),
            client_state: WprsClientState::new(globals, qh, conn).location(loc!())?,
            compositor_state: WprsCompositorState::new(dh, event_loop_handle, decoration_behavior),
            surface_bimap: BiMap::new(),
            surfaces: HashMap::new(),
            outputs: HashMap::new(),
        })
    }

    #[instrument(skip(self), level = "debug")]
    pub fn remove_surface(&mut self, surface_id: &CompositorObjectId) {
        let children = match self.surfaces.get(surface_id) {
            Some(surface) => surface.children.clone(),
            None => HashSet::new(),
        };

        for child in children {
            self.remove_surface(&child);
        }

        self.surface_bimap.remove_by_left(surface_id);
        if let Some(xwayland_surface) = self.surfaces.remove(surface_id) {
            if let Some(parent) = xwayland_surface.parent {
                let parent_xwayland_surface = self.surfaces.get_mut(&parent.surface_id).unwrap();
                parent_xwayland_surface
                    .children
                    .retain(|child_surface_id| child_surface_id != surface_id);
            }

            // last_focused_window holds a handle to the window, not just an id, so
            // if we don't do this, the window doesn't get destroyed until a
            // different window is focused.
            if let (
                Some(Role::XdgToplevel(toplevel)),
                Some(X11Parent {
                    for_toplevel: Some(window),
                    ..
                }),
            ) = (
                xwayland_surface.role,
                &self.client_state.last_focused_window,
            ) {
                if window == &toplevel.local_window {
                    self.client_state.last_focused_window = None;
                }
            }
        }
    }

    #[instrument(
        skip(self, keycode, state),
        fields(keycode = "<redacted>", state = "<redacted>"),
        level = "debug"
    )]
    pub(crate) fn set_key_state(
        &mut self,
        keycode: u32,
        state: KeyState,
        serial: Serial,
    ) -> Result<()> {
        let keyboard = self.compositor_state.seat.get_keyboard().location(loc!())?;

        if args::get_log_priv_data() {
            Span::current().record("keycode", field::debug(&keycode));
            Span::current().record("state", field::debug(&state));
        }

        keyboard.input::<(), _>(
            self,
            keycode,
            state,
            serial,
            self.compositor_state.start_time.elapsed().as_millis() as u32,
            |_, &modifiers_state, keysym| {
                if args::get_log_priv_data() {
                    Span::current().record("modifiers_state", field::debug(&modifiers_state));
                    Span::current().record("keysym", field::debug(&keysym));
                }
                FilterResult::Forward
            },
        );
        match state {
            KeyState::Pressed => {
                self.compositor_state.pressed_keys.insert(keycode);
            },
            KeyState::Released => {
                self.compositor_state.pressed_keys.remove(&keycode);
            },
        }

        Ok(())
    }
}

pub fn xsurface_from_client_surface<'a>(
    surface_bimap: &BiMap<CompositorObjectId, ClientObjectId>,
    surfaces: &'a mut HashMap<CompositorObjectId, XWaylandSurface>,
    surface: &ClientWlSurface,
) -> Option<&'a mut XWaylandSurface> {
    debug!(
        "xsurface_from_client_surface, {:?}, {:?}",
        surface.id(),
        surfaces
    );
    let compositor_surface_id = surface_bimap.get_by_right(&surface.id())?;
    surfaces.get_mut(compositor_surface_id)
}

pub fn xsurface_from_x11_surface<'a>(
    surfaces: &'a mut HashMap<CompositorObjectId, XWaylandSurface>,
    surface: &X11Surface,
) -> Option<&'a mut XWaylandSurface> {
    surfaces.values_mut().find(|xws| {
        xws.x11_surface
            .as_ref()
            .map(|s| s == surface)
            .unwrap_or(false)
    })
}
