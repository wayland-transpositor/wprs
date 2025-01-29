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

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::mem;
use std::os::fd::OwnedFd;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use serde_derive::Deserialize;
use serde_derive::Serialize;
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::input::pointer::CursorImageStatus;
use smithay::input::pointer::CursorImageSurfaceData;
use smithay::input::Seat;
use smithay::input::SeatHandler;
use smithay::input::SeatState;
use smithay::output::Output;
use smithay::output::PhysicalProperties;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor;
use smithay::wayland::compositor::BufferAssignment;
use smithay::wayland::compositor::CompositorClientState;
use smithay::wayland::compositor::CompositorHandler;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::compositor::Damage;
use smithay::wayland::compositor::SurfaceAttributes;
use smithay::wayland::compositor::SurfaceData;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::ClientDndGrabHandler;
use smithay::wayland::selection::data_device::DataDeviceHandler;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::data_device::ServerDndGrabHandler;
use smithay::wayland::selection::primary_selection::PrimarySelectionHandler;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::SelectionSource;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::shm::ShmHandler;
use smithay::wayland::shm::ShmState;
use smithay::wayland::xwayland_shell::XWaylandShellHandler;
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::xwm::XwmId;
use smithay::xwayland::X11Surface;
use smithay::xwayland::X11Wm;
use smithay::xwayland::XWayland;
use smithay::xwayland::XWaylandClientData;
use smithay::xwayland::XWaylandEvent;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface as SctkWlSurface;
use smithay_client_toolkit::reexports::csd_frame::DecorationsFrame;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_surface;
use smithay_client_toolkit::shell::xdg::XdgSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use wprs_common::utils::SerialMap;
use wprs_protocol::serialization::geometry::Point;
use wprs_protocol::serialization::wayland::OutputInfo;

use crate::compositor_utils;
use crate::fallible_entry::FallibleEntryExt;
use crate::prelude::*;
use crate::xwayland_xdg_shell::client::Role;
use crate::xwayland_xdg_shell::wmname;
use crate::xwayland_xdg_shell::WprsState;
use crate::xwayland_xdg_shell::XWaylandSurface;

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
pub enum DecorationBehavior {
    #[default]
    Auto,
    AlwaysEnabled,
    AlwaysDisabled,
}

pub struct XwaylandOptions<K, V, I>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    pub display: Option<u32>,
    pub env: I,
}

#[derive(Debug)]
pub struct WprsCompositorState {
    pub dh: DisplayHandle,
    pub compositor_state: CompositorState,
    pub start_time: Instant,
    pub shm_state: ShmState,
    pub seat_state: SeatState<WprsState>,
    pub data_device_state: DataDeviceState,
    pub xwayland_shell_state: XWaylandShellState,
    pub primary_selection_state: PrimarySelectionState,
    pub decoration_behavior: DecorationBehavior,

    pub seat: Seat<WprsState>,

    pub outputs: HashMap<u32, (Output, GlobalId)>,
    pub(crate) serial_map: SerialMap,
    pub(crate) pressed_keys: HashSet<u32>,

    pub xwm: Option<X11Wm>,

    pub x11_screen_offset: Option<Point<i32>>,

    /// unpaired x11 surfaces
    pub x11_surfaces: Vec<X11Surface>,
}

impl WprsCompositorState {
    /// # Panics
    /// On failure launching xwayland.
    pub fn new<K, V, I>(
        dh: DisplayHandle,
        event_loop_handle: LoopHandle<'static, WprsState>,
        decoration_behavior: DecorationBehavior,
        xwayland_options: XwaylandOptions<K, V, I>,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "wprs");

        let (xwayland, client) = XWayland::spawn(
            &dh,
            xwayland_options.display,
            xwayland_options.env,
            false,
            Stdio::inherit(),
            Stdio::inherit(),
            |_| {},
        )
        .expect("failed to start xwayland.");

        let ret = event_loop_handle.insert_source(xwayland, move |event, _, data| match event {
            XWaylandEvent::Ready {
                x11_socket,
                display_number,
            } => {
                let wm =
                    X11Wm::start_wm(data.event_loop_handle.clone(), x11_socket, client.clone())
                        .expect("Failed to attach X11 Window Manager.");

                // Oh Java...
                wmname::set_wmname(Some(&format!(":{}", display_number)), "LG3D")
                    .expect("Failed to set WM name.");

                data.compositor_state.xwm = Some(wm);
            },
            XWaylandEvent::Error => {
                let _ = data.compositor_state.xwm.take();
            },
        });
        if let Err(e) = ret {
            error!(
                "Failed to insert the XWaylandSource into the event loop: {}",
                e
            );
        }

        Self {
            dh: dh.clone(),
            compositor_state: CompositorState::new::<WprsState>(&dh),
            start_time: Instant::now(),
            shm_state: ShmState::new::<WprsState>(&dh, Vec::new()),
            seat_state,
            xwayland_shell_state: XWaylandShellState::new::<WprsState>(&dh),
            data_device_state: DataDeviceState::new::<WprsState>(&dh),
            primary_selection_state: PrimarySelectionState::new::<WprsState>(&dh),
            decoration_behavior,
            seat,
            outputs: HashMap::new(),
            serial_map: SerialMap::new(),
            pressed_keys: HashSet::new(),
            xwm: None,
            x11_screen_offset: None,
            x11_surfaces: Vec::new(),
        }
    }

    #[instrument(skip(self), level = "debug")]
    pub(crate) fn new_output(&mut self, output: OutputInfo) {
        let (local_output, _) = self.outputs.entry(output.id).or_insert_with_key(|id| {
            let new_output = Output::new(
                format!(
                    "{}_{}",
                    id,
                    output.name.clone().unwrap_or("None".to_string())
                ),
                PhysicalProperties {
                    size: output.physical_size.into(),
                    subpixel: output.subpixel.into(),
                    make: output.make.clone(),
                    model: output.model.clone(),
                },
            );
            let global_id = new_output.create_global::<WprsState>(&self.dh);
            (new_output, global_id)
        });

        // We are lying to xwayland about the size of the display and offsetting all our x11 windows
        // by the accordingly. This is because xwayland will not let us move cursors beyond the bounds of the
        // screen. Since wayland surfaces do not know where they are placed, we will sometimes receive
        // events that either enter the negative coordinate space (because the wayland window is not aligned
        // with the topleft corner) or are beyond the size of the screen (because the window partially overlaps
        // the edge of the screen.)
        // However, Xwayland seems to run into performance bottlenecks as we increase the screen size,
        // even if an app's window size doesn't change. So we want to choose the minimal size possible.
        let mut expanded_output = output.clone();
        expanded_output.mode.dimensions =
            (output.mode.dimensions.w * 3, output.mode.dimensions.h * 3).into();
        self.x11_screen_offset =
            Some((-output.mode.dimensions.w, -output.mode.dimensions.h).into());

        compositor_utils::update_output(local_output, expanded_output);
    }

    #[instrument(skip(self), level = "debug")]
    pub(crate) fn update_output(&mut self, output: OutputInfo) {
        let (local_output, _) = match self.outputs.entry(output.id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(_) => {
                warn!("update to unknown display {:?}", output.id);
                return;
            },
        };

        let mut expanded_output = output.clone();
        expanded_output.mode.dimensions = (
            expanded_output.mode.dimensions.w * 3,
            expanded_output.mode.dimensions.h * 3,
        )
            .into();
        self.x11_screen_offset =
            Some((-output.mode.dimensions.w, -output.mode.dimensions.h).into());

        compositor_utils::update_output(local_output, expanded_output);
    }

    #[instrument(skip(self), level = "debug")]
    pub(crate) fn destroy_output(&mut self, output: OutputInfo) {
        if let Some((_, (_, global_id))) = self.outputs.remove_entry(&output.id) {
            self.dh.remove_global::<WprsState>(global_id);
        }
    }
}

impl BufferHandler for WprsState {
    #[instrument(skip(self), level = "debug")]
    fn buffer_destroyed(&mut self, buffer: &WlBuffer) {}
}

impl SelectionHandler for WprsState {
    type SelectionUserData = ();

    // We need to implement this trait for copying to clients, but all our
    // clients are xwayland clients and so the methods below should never be
    // called.

    #[instrument(skip(self, _seat), level = "debug")]
    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        error!("new_selection called");
    }

    #[instrument(skip(self, _fd, _seat, _user_data), level = "debug")]
    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        _fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        error!("new_selection called");
    }
}

impl DataDeviceHandler for WprsState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.compositor_state.data_device_state
    }
}

impl PrimarySelectionHandler for WprsState {
    fn primary_selection_state(
        &self,
    ) -> &smithay::wayland::selection::primary_selection::PrimarySelectionState {
        &self.compositor_state.primary_selection_state
    }
}

impl ClientDndGrabHandler for WprsState {}
impl ServerDndGrabHandler for WprsState {}

impl XWaylandShellHandler for WprsState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.compositor_state.xwayland_shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, wl_surface: WlSurface, surface: X11Surface) {
        // TODO: we should implement this and get rid of the deferring commit logic below
        debug!(
            "X11 window {:?} associated with surface {:?}",
            surface, wl_surface
        )
    }
}

fn execute_or_defer_commit(state: &mut WprsState, surface: WlSurface) -> Result<()> {
    commit(&surface, state).location(loc!())?;

    let xwayland_surface = state.surfaces.get(&surface.id());

    // we may not have matched an X11 surface to the wayland surface yet.
    // defer if that is the case.
    if !xwayland_surface.is_some_and(XWaylandSurface::ready) {
        debug!("deferring commit");
        state.event_loop_handle.insert_idle(|state| {
            execute_or_defer_commit(state, surface).log_and_ignore(loc!());
        });
    }
    Ok(())
}

impl CompositorHandler for WprsState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<XWaylandClientData>()
            .unwrap()
            .compositor_state
    }

    #[instrument(skip(self), level = "debug")]
    fn commit(&mut self, surface: &WlSurface) {
        execute_or_defer_commit(self, surface.clone()).log_and_ignore(loc!());
    }
}

#[instrument(skip(state), level = "debug")]
pub fn commit(surface: &WlSurface, state: &mut WprsState) -> Result<()> {
    compositor::with_states(surface, |surface_data| -> Result<()> {
        commit_inner(surface, surface_data, state).location(loc!())?;
        Ok(())
    })
    .location(loc!())?;
    on_commit_buffer_handler::<WprsState>(surface);
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct X11ParentForPopup {
    pub(crate) surface_id: ObjectId,
    pub(crate) xdg_surface: xdg_surface::XdgSurface,
    pub(crate) x11_offset: Point<i32>,
    pub(crate) wl_offset: Point<i32>,
}

#[derive(Debug, Clone)]
pub(crate) struct X11ParentForSubsurface {
    pub(crate) surface: SctkWlSurface,
    pub(crate) x11_offset: Point<i32>,
}

#[derive(Debug, Clone)]
pub(crate) struct X11Parent {
    pub(crate) surface_id: ObjectId,
    pub(crate) for_popup: Option<X11ParentForPopup>,
    pub(crate) for_subsurface: X11ParentForSubsurface,
}

pub(crate) fn find_x11_parent(
    state: &WprsState,
    x11_surface: Option<X11Surface>,
) -> Option<X11Parent> {
    if let Some(x11_surface) = &x11_surface {
        if let Some(parent_id) = x11_surface.is_transient_for() {
            let (parent_id, parent) = state
                .surfaces
                .iter()
                .find(|(_, xwls)| {
                    xwls.x11_surface
                        .as_ref()
                        .is_some_and(|s| s.window_id() == parent_id)
                })
                .unwrap();

            let Ok(parent_x11_surface) = parent.get_x11_surface() else {
                error!("parent {parent:?} has no attached x11 surface");
                return None;
            };
            let parent_geo = parent_x11_surface.geometry();

            match &parent.role {
                Some(Role::XdgToplevel(toplevel)) => Some(X11Parent {
                    surface_id: parent_id.clone(),
                    for_popup: Some(X11ParentForPopup {
                        surface_id: parent_id.clone(),
                        xdg_surface: toplevel.xdg_surface().clone(),
                        x11_offset: (
                            -parent_geo.loc.x + toplevel.frame_offset.x,
                            -parent_geo.loc.y + toplevel.frame_offset.y,
                        )
                            .into(),
                        wl_offset: (
                            -parent_geo.loc.x + toplevel.frame_offset.x - toplevel.x11_offset.x,
                            -parent_geo.loc.y + toplevel.frame_offset.y - toplevel.x11_offset.y,
                        )
                            .into(),
                    }),
                    for_subsurface: X11ParentForSubsurface {
                        surface: toplevel.wl_surface().clone(),
                        x11_offset: (-parent_geo.loc.x, -parent_geo.loc.y).into(),
                    },
                }),
                Some(Role::XdgPopup(popup)) => Some(X11Parent {
                    surface_id: parent_id.clone(),
                    for_popup: Some(X11ParentForPopup {
                        surface_id: parent_id.clone(),
                        xdg_surface: popup.xdg_surface().clone(),
                        x11_offset: (-parent_geo.loc.x, -parent_geo.loc.y).into(),
                        wl_offset: (-parent_geo.loc.x, -parent_geo.loc.y).into(),
                    }),
                    for_subsurface: X11ParentForSubsurface {
                        surface: popup.wl_surface().clone(),
                        x11_offset: (-parent_geo.loc.x, -parent_geo.loc.y).into(),
                    },
                }),
                Some(Role::SubSurface(subsurface)) => Some(X11Parent {
                    surface_id: parent_id.clone(),
                    for_popup: None, // subsurface cannot be parent to popup
                    for_subsurface: X11ParentForSubsurface {
                        surface: subsurface.wl_surface().clone(),
                        x11_offset: (-parent_geo.loc.x, -parent_geo.loc.y).into(),
                    },
                }),
                Some(Role::Cursor) => unreachable!("Cursors cannot have child surfaces."),
                // TODO: fix this
                None => unreachable!(
                    "Parent doesn't yet have a role assigned. This is a race condition."
                ),
            }
        } else {
            None
        }
    } else {
        None
    }
}

#[instrument(skip(state), level = "debug")]
pub fn commit_inner(
    surface: &WlSurface,
    surface_data: &SurfaceData,
    state: &mut WprsState,
) -> Result<()> {
    let mut guard = surface_data.cached_state.get::<SurfaceAttributes>();
    let surface_attributes = guard.current();

    let x11_surface = state
        .compositor_state
        .x11_surfaces
        .iter()
        .position(|x11s| x11s.wl_surface().map(|s| s == *surface).unwrap_or(false))
        .map(|pos| state.compositor_state.x11_surfaces.swap_remove(pos));
    debug!("matched x11 surface: {x11_surface:?}");

    let parent = find_x11_parent(state, x11_surface.clone());

    if let (Some(parent), Some(_)) = (&parent, &x11_surface) {
        debug!(
            "registering child {:?} with parent {:?}",
            surface.id(),
            &parent.surface_id
        );
        // We can still get cycles in the case of bugs in find_x11_parent, but
        // this is a start.
        assert!(
            surface.id() != parent.surface_id,
            "tried to register a surface as a child of itself"
        );
        let parent_xwayland_surface = state.surfaces.get_mut(&parent.surface_id).unwrap();
        parent_xwayland_surface.children.insert(surface.id());
    }

    let xwayland_surface = state.surfaces.entry(surface.id()).or_default();

    if let Some(x11_surface) = x11_surface {
        if xwayland_surface.local_surface.is_none() {
            xwayland_surface
                .update_local_surface(
                    surface,
                    parent.as_ref().map(|parent| &parent.for_subsurface.surface),
                    &state.client_state.compositor_state,
                    &state.client_state.qh,
                    &mut state.surface_bimap,
                )
                .location(loc!())?;
        }

        if let Some(x11_offset) = state.compositor_state.x11_screen_offset {
            xwayland_surface
                .update_x11_surface(
                    x11_surface,
                    x11_offset,
                    parent,
                    &state.client_state.last_focused_window,
                    &state.client_state.xdg_shell_state,
                    &state.client_state.shm_state,
                    state.client_state.subcompositor_state.clone(),
                    &state.client_state.qh,
                    state.compositor_state.decoration_behavior,
                )
                .location(loc!())?;
        }
    }

    debug!("buffer assignment: {:?}", &surface_attributes.buffer);

    match &surface_attributes.buffer {
        Some(BufferAssignment::NewBuffer(buffer)) => {
            compositor_utils::with_buffer_contents(buffer, |data, spec| {
                xwayland_surface.update_buffer(
                    &spec,
                    data,
                    state.client_state.pool.as_mut().location(loc!())?,
                )
            })
            .location(loc!())?
            .location(loc!())?;

            xwayland_surface.buffer_attached = false;
        },
        Some(BufferAssignment::Removed) => {
            xwayland_surface.buffer = None;
            xwayland_surface.wl_surface().attach(None, 0, 0);
        },
        None => {},
    }

    if let Some(Role::XdgToplevel(toplevel)) = &mut xwayland_surface.role {
        if toplevel.configured && toplevel.window_frame.is_dirty() {
            toplevel.window_frame.draw();
        }
    }

    if let Some(Role::SubSurface(subsurface)) = &mut xwayland_surface.role {
        if let Some(decorated_subsurface) = &mut subsurface.frame {
            if decorated_subsurface.is_dirty() {
                decorated_subsurface.draw();
            }
        }
    }

    let damage = &mut mem::take(&mut surface_attributes.damage)
        .iter()
        .map(|damage| match damage {
            Damage::Buffer(rect) => *rect,
            Damage::Surface(rect) => rect.to_buffer(
                surface_attributes.buffer_scale,
                surface_attributes.buffer_transform.into(),
                &rect.size,
            ),
        })
        .map(Into::into)
        .collect();

    if let Some(surface_damage) = &mut xwayland_surface.damage {
        surface_damage.append(damage);
    } else {
        xwayland_surface.damage = Some(damage.to_vec());
    }

    if xwayland_surface.ready() {
        if let Some(Role::SubSurface(subsurface)) = &mut xwayland_surface.role {
            if !subsurface.pending_frame_callback {
                xwayland_surface.frame(&state.client_state.qh);
            }
        } else {
            xwayland_surface.frame(&state.client_state.qh);
        }

        xwayland_surface.try_draw_buffer();
    }

    if xwayland_surface.ready() || xwayland_surface.needs_configure() {
        xwayland_surface.commit();
    }

    if xwayland_surface.x11_surface.is_none() || matches!(xwayland_surface.role, Some(Role::Cursor))
    {
        compositor_utils::send_frames(
            surface,
            &surface_data.data_map,
            surface_attributes,
            state.compositor_state.start_time.elapsed(),
            Duration::ZERO,
        )
        .location(loc!())?;
    }
    Ok(())
}

impl ShmHandler for WprsState {
    fn shm_state(&self) -> &ShmState {
        &self.compositor_state.shm_state
    }
}

impl SeatHandler for WprsState {
    type KeyboardFocus = X11Surface;
    type PointerFocus = X11Surface;
    type TouchFocus = X11Surface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.compositor_state.seat_state
    }

    #[instrument(skip(self, _seat), level = "debug")]
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        // TODO: support multiple seats
        let themed_pointer = self
            .client_state
            .seat_objects
            .last()
            .unwrap()
            .pointer
            .as_ref()
            .unwrap();
        let pointer = themed_pointer.pointer();

        // TODO: move to a fn on serialization::CursorImaveStatus
        match image {
            CursorImageStatus::Hidden => {
                themed_pointer.hide_cursor().log_and_ignore(loc!());
            },
            CursorImageStatus::Surface(surface) => {
                let hotspot = compositor::with_states(&surface, |surface_data| {
                    surface_data
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .hotspot
                });

                let xwayland_surface = log_and_return!(self
                    .surfaces
                    .entry(surface.id())
                    .or_insert_with_result(|| {
                        XWaylandSurface::new(
                            &surface,
                            &self.client_state.compositor_state,
                            &self.client_state.qh,
                            &mut self.surface_bimap,
                        )
                    }));

                xwayland_surface.role = Some(Role::Cursor);

                // TODO: expose serial to this function, then remove
                // last_enter_serial on client.
                pointer.set_cursor(
                    self.client_state.last_enter_serial,
                    Some(xwayland_surface.wl_surface()),
                    hotspot.x,
                    hotspot.y,
                );
            },
            CursorImageStatus::Named(name) => {
                themed_pointer
                    .set_cursor(&self.client_state.conn, name)
                    .log_and_ignore(loc!());
            },
        }
    }
}

impl OutputHandler for WprsState {}

smithay::delegate_compositor!(WprsState);
smithay::delegate_shm!(WprsState);
smithay::delegate_seat!(WprsState);
smithay::delegate_data_device!(WprsState);
smithay::delegate_output!(WprsState);
smithay::delegate_primary_selection!(WprsState);
smithay::delegate_xwayland_shell!(WprsState);
