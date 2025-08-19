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
use std::ffi::OsStr;
use std::sync::Arc;

use bimap::BiMap;
use calloop::RegistrationToken;
use smithay::backend::input::KeyState as SmithayKeyState;
use smithay::input::keyboard::FilterResult;
use smithay::input::keyboard::KeysymHandle;
use smithay::input::keyboard::ModifiersState;
use smithay::output::Output;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId as CompositorObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as CompositorWlSurface;
use smithay::utils::Serial;
use smithay::xwayland::X11Surface;
use smithay::xwayland::xwm::WmWindowType;
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::compositor::Surface;
use smithay_client_toolkit::compositor::SurfaceData;
use smithay_client_toolkit::output::OutputData;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::client::backend::ObjectId as ClientObjectId;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface as ClientWlSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::subcompositor::SubcompositorState;
use tracing::Span;

use crate::args;
use crate::compositor_utils;
use crate::constants;
use crate::prelude::*;
use crate::serialization::geometry::Point;
use crate::serialization::geometry::Rectangle;
use crate::serialization::wayland::KeyState;
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
use compositor::XwaylandOptions;

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
    pub(crate) output_ids: HashSet<u32>,
    pub(crate) damage: Option<Vec<Rectangle<i32>>>,
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
            output_ids: HashSet::new(),
            damage: None,
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

    fn try_draw_buffer(&mut self) {
        if !self.buffer_attached
            && let Some(buffer) = &self.buffer
        {
            let surface = self.wl_surface().clone();
            // The only possible error here is AlreadyActive, which we can
            // ignore.
            _ = buffer.active_buffer.attach_to(&surface);
            if let Some(damage_rects) = &self.damage.take() {
                // avoid overwhelming wayland connection
                if damage_rects.len() < constants::SENT_DAMAGE_LIMIT {
                    for damage_rect in damage_rects {
                        surface.damage_buffer(
                            damage_rect.loc.x,
                            damage_rect.loc.y,
                            damage_rect.size.w,
                            damage_rect.size.h,
                        );
                    }
                } else {
                    surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                }
            } else {
                surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
            }

            self.buffer_attached = true;
        }
    }

    fn commit_buffer(&mut self, qh: &QueueHandle<WprsState>) {
        if !self.buffer_attached && self.buffer.is_some() {
            self.try_draw_buffer();

            self.frame(qh);
            self.commit();
        }
    }

    #[instrument(skip(xdg_shell_state, qh), level = "debug")]
    fn update_x11_surface(
        &mut self,
        x11_surface: X11Surface,
        x11_offset: Point<i32>,
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
                    x11_offset,
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
                    x11_offset,
                    xdg_shell_state,
                    shm_state,
                    subcompositor_state,
                    qh,
                    decoration_behavior,
                )
                .location(loc!())?;
            },
            WaylandWindowType::Popup if parent_if_popup.clone().unwrap().for_popup.is_none() => {
                debug!(
                    "creating subsurface for {self:?} instead of popup because parent was subsurface"
                );
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
    pub registration_tokens: Vec<RegistrationToken>,
    pub client_state: WprsClientState,
    pub compositor_state: WprsCompositorState,
    pub surface_bimap: BiMap<CompositorObjectId, ClientObjectId>,
    pub surfaces: HashMap<CompositorObjectId, XWaylandSurface>,
    pub outputs: HashMap<u32, Output>,
}

impl WprsState {
    pub fn new<K, V, I>(
        dh: DisplayHandle,
        globals: &GlobalList,
        qh: QueueHandle<Self>,
        conn: Connection,
        event_loop_handle: LoopHandle<'static, Self>,
        decoration_behavior: DecorationBehavior,
        xwayland_options: XwaylandOptions<K, V, I>,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut registration_tokens = vec![];
        Ok(Self {
            dh: dh.clone(),
            event_loop_handle: event_loop_handle.clone(),
            client_state: WprsClientState::new(globals, qh, conn).location(loc!())?,
            compositor_state: WprsCompositorState::new(
                dh,
                &event_loop_handle,
                decoration_behavior,
                xwayland_options,
                &mut registration_tokens,
            ),
            surface_bimap: BiMap::new(),
            surfaces: HashMap::new(),
            outputs: HashMap::new(),
            registration_tokens,
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

        if let Some(xwayland_surface) = self.surfaces.remove(surface_id)
            && let Some(parent) = xwayland_surface.parent
        {
            let parent_xwayland_surface = self.surfaces.get_mut(&parent.surface_id).unwrap();
            parent_xwayland_surface
                .children
                .retain(|child_surface_id| child_surface_id != surface_id);
        }

        // this MUST come after removing xwayland_surface, because xwayland_surface's role needs
        // to be destroyed before it's client wl_surface.
        // ultimately, the wayland object should be destroyed in order from:
        // xdg_popup/xdg_toplevel -> xdg_surface -> wl_surface
        self.surface_bimap.remove_by_left(surface_id);
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

        fn filter(
            _: &mut WprsState,
            modifiers_state: &ModifiersState,
            keysym: KeysymHandle,
        ) -> FilterResult<()> {
            if args::get_log_priv_data() {
                Span::current().record("modifiers_state", field::debug(&modifiers_state));
                Span::current().record("keysym", field::debug(&keysym));
            }
            FilterResult::Forward
        }

        // our keycode is getting offset by 8 for reasons
        // see https://github.com/Smithay/smithay/pull/1536
        let x11_keycode = (keycode + 8).into();
        let time = self.compositor_state.start_time.elapsed().as_millis() as u32;
        match state {
            KeyState::Pressed => {
                keyboard.input::<(), _>(
                    self,
                    x11_keycode,
                    SmithayKeyState::Pressed,
                    serial,
                    time,
                    filter,
                );
                self.compositor_state.pressed_keys.insert(keycode);
            },
            KeyState::Released => {
                keyboard.input::<(), _>(
                    self,
                    x11_keycode,
                    SmithayKeyState::Released,
                    serial,
                    time,
                    filter,
                );
                self.compositor_state.pressed_keys.remove(&keycode);
            },
            KeyState::Repeated => {
                // Map repeated to released + pressed
                // Smithay 0.7 keystates don't support repetition
                keyboard.input::<(), _>(
                    self,
                    x11_keycode,
                    SmithayKeyState::Released,
                    serial,
                    time,
                    filter,
                );
                keyboard.input::<(), _>(
                    self,
                    x11_keycode,
                    SmithayKeyState::Pressed,
                    serial,
                    time,
                    filter,
                );
            },
        }

        Ok(())
    }

    pub fn compositor_surface_from_client_surface(
        &self,
        client_surface: &ClientWlSurface,
    ) -> Option<CompositorWlSurface> {
        let compositor_surface_id = self.surface_bimap.get_by_right(&client_surface.id())?;

        let Ok(client) = self.dh.get_client(compositor_surface_id.clone()) else {
            return None;
        };

        let Ok(surface) =
            client.object_from_protocol_id(&self.dh, compositor_surface_id.protocol_id())
        else {
            return None;
        };

        Some(surface)
    }

    pub fn sync_surface_outputs(&mut self, surface: &ClientWlSurface) {
        let (Some(compositor_surface), Some(xwayland_surface), Some(outputs)) = (
            self.compositor_surface_from_client_surface(surface),
            xsurface_from_client_surface(&self.surface_bimap, &mut self.surfaces, surface),
            surface.data::<SurfaceData>().map(SurfaceData::outputs),
        ) else {
            return;
        };

        let new_ids: HashSet<u32> = HashSet::from_iter(outputs.filter_map(|output| {
            output
                .data::<OutputData>()
                .map(|data| data.with_output_info(|info| info.id))
        }));

        compositor_utils::update_surface_outputs(
            &compositor_surface,
            &new_ids,
            &xwayland_surface.output_ids,
            |id| self.outputs.get(id),
        );

        xwayland_surface.output_ids = new_ids;
    }
}

impl Drop for WprsState {
    fn drop(&mut self) {
        for token in self.registration_tokens.drain(..) {
            self.event_loop_handle.remove(token);
        }
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
