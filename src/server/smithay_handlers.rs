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

/// Handlers for events from Smithay.
use std::os::fd::OwnedFd;
use std::sync::Arc;

use crossbeam_channel::Sender;
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::input::pointer::AxisFrame;
use smithay::input::pointer::ButtonEvent;
use smithay::input::pointer::CursorImageStatus as SmithayCursorImageStatus;
use smithay::input::pointer::CursorImageSurfaceData;
use smithay::input::pointer::GestureHoldBeginEvent;
use smithay::input::pointer::GestureHoldEndEvent;
use smithay::input::pointer::GesturePinchBeginEvent;
use smithay::input::pointer::GesturePinchEndEvent;
use smithay::input::pointer::GesturePinchUpdateEvent;
use smithay::input::pointer::GestureSwipeBeginEvent;
use smithay::input::pointer::GestureSwipeEndEvent;
use smithay::input::pointer::GestureSwipeUpdateEvent;
use smithay::input::pointer::GrabStartData;
use smithay::input::pointer::MotionEvent;
use smithay::input::pointer::PointerGrab;
use smithay::input::pointer::PointerInnerHandle;
use smithay::input::pointer::RelativeMotionEvent;
use smithay::input::Seat;
use smithay::input::SeatHandler;
use smithay::input::SeatState;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::Mode as KdeDecorationMode;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::OrgKdeKwinServerDecoration;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::backend::DisconnectReason;
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::WEnum;
use smithay::utils::Logical;
use smithay::utils::Point;
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor;
use smithay::wayland::compositor::BufferAssignment as SmithayBufferAssignment;
use smithay::wayland::compositor::CompositorClientState;
use smithay::wayland::compositor::CompositorHandler;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::compositor::SubsurfaceCachedState;
use smithay::wayland::compositor::SurfaceAttributes;
use smithay::wayland::compositor::SurfaceData;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::with_source_metadata;
use smithay::wayland::selection::data_device::ClientDndGrabHandler;
use smithay::wayland::selection::data_device::DataDeviceHandler;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::data_device::ServerDndGrabHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::SelectionSource;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::primary_selection::PrimarySelectionHandler;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::kde::decoration::KdeDecorationHandler;
use smithay::wayland::shell::kde::decoration::KdeDecorationState;
use smithay::wayland::shell::xdg::Configure;
use smithay::wayland::shell::xdg::PopupSurface;
use smithay::wayland::shell::xdg::PositionerState;
use smithay::wayland::shell::xdg::SurfaceCachedState;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::XdgShellHandler;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shm::ShmHandler;
use smithay::wayland::shm::ShmState;

use crate::channel_utils::DiscardingSender;
use crate::compositor_utils;
use crate::prelude::*;
use crate::serialization;
use crate::serialization::tuple::Tuple2;
use crate::serialization::wayland::BufferAssignment;
use crate::serialization::wayland::ClientSurface;
use crate::serialization::wayland::CursorImage;
use crate::serialization::wayland::CursorImageStatus;
use crate::serialization::wayland::DataDestinationRequest;
use crate::serialization::wayland::DataRequest;
use crate::serialization::wayland::DataSource;
use crate::serialization::wayland::DataSourceRequest;
use crate::serialization::wayland::Role;
use crate::serialization::wayland::SourceMetadata;
use crate::serialization::wayland::SubSurfaceState;
use crate::serialization::wayland::SubsurfacePosition;
use crate::serialization::wayland::SurfaceRequest;
use crate::serialization::wayland::SurfaceRequestPayload;
use crate::serialization::wayland::SurfaceState;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::xdg_shell::DecorationMode;
use crate::serialization::xdg_shell::Move;
use crate::serialization::xdg_shell::PopupRequest;
use crate::serialization::xdg_shell::PopupRequestPayload;
use crate::serialization::xdg_shell::Resize;
use crate::serialization::xdg_shell::ToplevelRequest;
use crate::serialization::xdg_shell::ToplevelRequestPayload;
use crate::serialization::xdg_shell::XdgPopupState;
use crate::serialization::xdg_shell::XdgPositioner;
use crate::serialization::xdg_shell::XdgSurfaceState;
use crate::serialization::xdg_shell::XdgToplevelState;
use crate::serialization::Request;
use crate::server::LockedSurfaceState;
use crate::server::WprsServerState;
use crate::serialization::SendType;
use crate::vec4u8::Vec4u8s;

impl BufferHandler for WprsServerState {
    #[instrument(skip(self), level = "debug")]
    fn buffer_destroyed(&mut self, buffer: &wl_buffer::WlBuffer) {}
}

impl WprsServerState {
    fn send_toplevel_request(&self, toplevel: ToplevelSurface, payload: ToplevelRequestPayload) {
        let surface = toplevel.wl_surface();
        self.serializer
            .writer()
            .send(SendType::Object(Request::Toplevel(ToplevelRequest {
                client: serialization::ClientId::new(&surface.client().unwrap()),
                surface: (&surface.id()).into(),
                payload,
            })))
    }
}

impl XdgShellHandler for WprsServerState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    #[instrument(skip(self), level = "debug")]
    fn new_toplevel(&mut self, toplevel: ToplevelSurface) {
        self.insert_surface(toplevel.wl_surface())
            .log_and_ignore(loc!());
        compositor::with_states(toplevel.wl_surface(), |surface_data| {
            let surface_state = &mut surface_data
                .data_map
                .get::<LockedSurfaceState>()
                .unwrap()
                .0
                .lock()
                .unwrap();
            surface_state.role = Some(Role::XdgToplevel(XdgToplevelState::new(&toplevel)));
        });

        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });
        toplevel.send_configure();
    }

    #[instrument(skip(self), level = "debug")]
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // If client() returns None, the surface was already destroyed and an
        // appropriate message would have been sent to the client, so we don't
        // need to worry about destroying the toplevel,
        if surface.wl_surface().client().is_some() {
            self.send_toplevel_request(surface, ToplevelRequestPayload::Destroyed);
        }
    }

    #[instrument(skip(self))]
    fn new_popup(&mut self, popup: PopupSurface, positioner: PositionerState) {
        self.insert_surface(popup.wl_surface())
            .log_and_ignore(loc!());

        // Uses with_states internally and with_states is not reentrant.
        let popup_state = log_and_return!(XdgPopupState::new(&popup, &positioner));
        compositor::with_states(popup.wl_surface(), |surface_data| {
            let surface_state = &mut surface_data
                .data_map
                .get::<LockedSurfaceState>()
                .unwrap()
                .0
                .lock()
                .unwrap();
            surface_state.role = Some(Role::XdgPopup(popup_state));
        });

        // TODO: this sometimes sends duplicate configures and causes "The popup
        // positioner is not reactive" errors, but without this popups break
        // completely.
        popup.send_configure().log_and_ignore(loc!());
    }

    #[instrument(skip(self), level = "debug")]
    fn popup_destroyed(&mut self, surface: PopupSurface) {
        // If client() returns None, the surface was already destroyed and an
        // appropriate message would have been sent to the client, so we don't
        // need to worry about destroying the popup,
        if let Some(client) = surface.wl_surface().client() {
            self.serializer
                .writer()
                .send(SendType::Object(Request::Popup(PopupRequest {
                    client: serialization::ClientId::new(&client),
                    surface: (&surface.wl_surface().id()).into(),
                    payload: PopupRequestPayload::Destroyed,
                })));
        };
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: this works in sway but breaks popups in mutter
        // "This means it requests to be sent a popup_done event when the pointer leaves the grab area.", do we need to do something here?
        // maybe mutter is denying the grab? maybe because we're passing 0 as the serial?

        // let mut surface_state = self
        //     .surfaces
        //     .get_mut(&serialization::wayland::WlSurfaceId::new(surface.wl_surface()))
        //     .unwrap();
        // surface_state.xdg_popup().unwrap().grab_requested = true;
    }

    fn ack_configure(&mut self, _surface: wl_surface::WlSurface, _configure: Configure) {}

    // TODO: implement ClientId from WLSurface constructor
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.send_toplevel_request(surface, ToplevelRequestPayload::SetMaximized);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.send_toplevel_request(surface, ToplevelRequestPayload::UnsetMaximized);
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        // TODO: do anything with output? Probably not, but also depends on how
        // exactly we handle output enter/exit events and updating outputs
        // between client reconnections.
        self.send_toplevel_request(surface, ToplevelRequestPayload::SetFullscreen);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.send_toplevel_request(surface, ToplevelRequestPayload::UnsetFullscreen);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.send_toplevel_request(surface, ToplevelRequestPayload::SetMinimized);
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, serial: Serial) {
        let Some(client_serial) = self.serial_map.remove(serial) else {
            warn!("Received move request with unknown serial {serial:?}.");
            return;
        };

        self.send_toplevel_request(
            surface,
            ToplevelRequestPayload::Move(Move {
                serial: client_serial,
            }),
        );
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let Some(client_serial) = self.serial_map.remove(serial) else {
            warn!("Received resize request with unknown serial {serial:?}.");
            return;
        };

        self.send_toplevel_request(
            surface,
            ToplevelRequestPayload::Resize(Resize {
                serial: client_serial,
                edge: edges.into(),
            }),
        );
    }

    fn reposition_request(&mut self, popup: PopupSurface, positioner: PositionerState, token: u32) {
        popup.send_repositioned(token);

        let surface = popup.wl_surface();
        let xdg_positioner = XdgPositioner::new(&positioner);
        compositor::with_states(surface, |surface_data| {
            let surface_state = &mut surface_data
                .data_map
                .get::<LockedSurfaceState>()
                .unwrap()
                .0
                .lock()
                .unwrap();

            if let Some(Role::XdgPopup(popup_state)) = &mut surface_state.role {
                popup_state.positioner = xdg_positioner;
            } else {
                error!("reposition called on surface that wasn't a popup");
                return;
            }

            let surface_state_to_send = surface_state.clone_without_buffer();
            self.serializer
                .writer()
                .send(SendType::Object(Request::Surface(log_and_return!(
                    SurfaceRequest::new(
                        surface,
                        SurfaceRequestPayload::Commit(surface_state_to_send),
                    )
                ))));
        });
    }

    // TODO: show_window_menu
}

impl SelectionHandler for WprsServerState {
    type SelectionUserData = ();

    #[instrument(skip(self, _seat), level = "debug")]
    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(source) = source {
            self.serializer
                .writer()
                .send(SendType::Object(Request::Data(DataRequest::SourceRequest(
                    DataSourceRequest::SetSelection(
                        match ty {
                            SelectionTarget::Clipboard => DataSource::Selection,
                            SelectionTarget::Primary => DataSource::Primary,
                        },
                        SourceMetadata::from_mime_types(source.mime_types()),
                    ),
                ))));
        }
    }

    #[instrument(skip(self, fd, _seat, _user_data), level = "debug")]
    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        let data_source = match ty {
            SelectionTarget::Clipboard => {
                self.selection_pipe = Some(fd);

                DataSource::Selection
            },
            SelectionTarget::Primary => {
                self.primary_selection_pipe = Some(fd);

                DataSource::Primary
            },
        };

        self.serializer
            .writer()
            .send(SendType::Object(Request::Data(
                DataRequest::DestinationRequest(DataDestinationRequest::RequestDataTransfer(
                    data_source,
                    mime_type,
                )),
            )));
    }
}

impl DataDeviceHandler for WprsServerState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for WprsServerState {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl ClientDndGrabHandler for WprsServerState {
    #[instrument(skip(self, _seat), level = "debug")]
    fn started(
        &mut self,
        source: Option<WlDataSource>,
        icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        self.dnd_source = source;
        if let Some(source) = &self.dnd_source {
            with_source_metadata(source, |source_metadata| {
                debug!("START DRAG: {source:?}, {source_metadata:?}");
                self.serializer
                    .writer()
                    .send(SendType::Object(Request::Data(DataRequest::SourceRequest(
                        DataSourceRequest::StartDrag(
                            source_metadata.clone().into(),
                            icon.map(|surface| {
                                Tuple2(
                                    serialization::ClientId::new(&surface.client().unwrap()),
                                    (&surface.id()).into(),
                                )
                            }),
                        ),
                    ))));
            })
            .log_and_ignore(loc!());
        }
    }

    #[instrument(skip(self, _seat), level = "debug")]
    fn dropped(&mut self, _seat: Seat<Self>) {}
}

impl ServerDndGrabHandler for WprsServerState {
    #[instrument(skip(self, _seat), level = "debug")]
    fn accept(&mut self, mime_type: Option<String>, _seat: Seat<Self>) {
        self.serializer
            .writer()
            .send(SendType::Object(Request::Data(
                DataRequest::DestinationRequest(DataDestinationRequest::DnDAcceptMimeType(
                    mime_type,
                )),
            )));
    }

    #[instrument(skip(self, _seat), level = "debug")]
    fn action(&mut self, action: DndAction, _seat: Seat<Self>) {
        self.serializer
            .writer()
            .send(SendType::Object(Request::Data(
                DataRequest::DestinationRequest(DataDestinationRequest::DnDSetDestinationActions(
                    action.into(),
                )),
            )));
    }

    #[instrument(skip(self, _seat), level = "debug")]
    fn dropped(&mut self, _seat: Seat<Self>) {}

    #[instrument(skip(self, _seat), level = "debug")]
    fn cancelled(&mut self, _seat: Seat<Self>) {}

    #[instrument(skip(self, _seat), level = "debug")]
    fn send(&mut self, mime_type: String, fd: OwnedFd, _seat: Seat<Self>) {
        self.dnd_pipe = Some(fd);
        self.serializer
            .writer()
            .send(SendType::Object(Request::Data(
                DataRequest::DestinationRequest(DataDestinationRequest::RequestDataTransfer(
                    DataSource::DnD,
                    mime_type,
                )),
            )));
    }

    #[instrument(skip(self, _seat), level = "debug")]
    fn finished(&mut self, _seat: Seat<Self>) {
        self.serializer
            .writer()
            .send(SendType::Object(Request::Data(
                DataRequest::DestinationRequest(DataDestinationRequest::DnDFinish),
            )));
    }
}

impl CompositorHandler for WprsServerState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    #[instrument(skip(self), level = "debug")]
    fn commit(&mut self, surface: &WlSurface) {
        // Send over the updated buffers from the children first so that the
        // client already has them when the parent is comitted.
        let children_dirty = commit_sync_children(self, surface, &commit).unwrap();
        commit(surface, self, children_dirty).log_and_ignore(loc!());
    }
}

#[instrument(skip(state, commit_fn), level = "debug")]
pub(crate) fn commit_sync_children<T, F>(
    state: &mut T,
    surface: &WlSurface,
    commit_fn: &F,
) -> Result<bool>
where
    F: Fn(&WlSurface, &mut T, bool) -> Result<bool>,
{
    compositor::get_children(surface)
        .iter()
        .filter(|child| compositor::is_sync_subsurface(child))
        .map(|child| {
            let children_dirty = commit_sync_children(state, child, commit_fn).location(loc!())?;
            commit_fn(child, state, children_dirty).location(loc!())
        })
        .collect::<Result<Vec<bool>>>()
        .location(loc!())?
        .into_iter()
        .reduce(|acc, elem| acc || elem)
        .map(Ok)
        .unwrap_or(Ok(false))
}

#[instrument(ret, level = "debug")]
pub fn get_child_positions(surface: &WlSurface) -> Vec<SubsurfacePosition> {
    compositor::get_children(surface)
        .iter()
        .map(|child| SubsurfacePosition {
            id: WlSurfaceId::new(child),
            position: compositor::with_states(child, |surface_data| {
                surface_data
                    .cached_state
                    .pending::<SubsurfaceCachedState>()
                    .location
                    .into()
            }),
        })
        .collect()
}

#[instrument(skip_all, level = "debug")]
pub fn commit(
    surface: &WlSurface,
    state: &mut WprsServerState,
    children_dirty: bool,
) -> Result<bool> {
    let surface_order = get_child_positions(surface);

    // TODO: https://github.com/Smithay/smithay/issues/538 - move into commit.
    let sync = compositor::is_sync_subsurface(surface);
    let parent = compositor::get_parent(surface);

    state.insert_surface(surface).log_and_ignore(loc!());

    let dirty = compositor::with_states(surface, |surface_data| {
        commit_impl(
            surface,
            surface_data,
            state,
            sync,
            parent,
            surface_order,
            children_dirty,
        )
    })
    .location(loc!())?;
    on_commit_buffer_handler::<WprsServerState>(surface);
    Ok(dirty)
}

// TODO: maybe make these methods on the relevant states

#[instrument(skip_all, level = "debug")]
pub fn set_regions(surface_attributes: &SurfaceAttributes, surface_state: &mut SurfaceState) {
    surface_state.input_region = surface_attributes.input_region.as_ref().map(Into::into);
    surface_state.opaque_region = surface_attributes.opaque_region.as_ref().map(Into::into);
}

#[instrument(skip_all, level = "debug")]
pub fn set_transformation(
    surface_attributes: &SurfaceAttributes,
    surface_state: &mut SurfaceState,
) {
    surface_state.buffer_scale = surface_attributes.buffer_scale;
}

#[instrument(skip_all, level = "debug")]
pub fn set_xdg_surface_attributes(surface_data: &SurfaceData, surface_state: &mut SurfaceState) {
    if surface_data.cached_state.has::<SurfaceCachedState>() {
        let surface_cached_state = surface_data.cached_state.current::<SurfaceCachedState>();
        let xdg_surface_state = XdgSurfaceState {
            window_geometry: surface_cached_state
                .geometry
                .as_ref()
                .map(|geometry| (*geometry).into()),
            max_size: surface_cached_state.max_size.into(),
            min_size: surface_cached_state.min_size.into(),
        };
        surface_state.xdg_surface_state = Some(xdg_surface_state);
    }
}

#[instrument(skip_all, level = "debug")]
pub fn set_xdg_toplevel_attributes(
    surface_data: &SurfaceData,
    toplevel_state: &mut XdgToplevelState,
) -> Result<()> {
    let toplevel_attributes = surface_data
        .data_map
        .get::<XdgToplevelSurfaceData>()
        .location(loc!())?
        .lock()
        .unwrap();
    // Be careful about not moving objects out of
    // toplevel_attributes here.
    toplevel_state.parent = toplevel_attributes.parent.as_ref().map(WlSurfaceId::new);
    toplevel_state.title = toplevel_attributes.title.clone();
    toplevel_state.app_id = toplevel_attributes.app_id.clone();

    // TODO: decoration mode are in wayland::shell::xdg::ToplevelState. See also
    // TODO in server_handlers::handle_toplevel.

    // toplevel_state.maximized = toplevel_attributes
    //     .current
    //     .states
    //     .contains(xdg_toplevel::State::Maximized);

    // match toplevel_attributes.current.decoration_mode {
    //     Some(
    // }

    // dbg!("TOPLEVEL ATTRIBUTES", &toplevel_attributes);
    Ok(())
}

#[instrument(skip(state), level = "debug")]
pub fn commit_impl(
    surface: &WlSurface,
    surface_data: &SurfaceData,
    state: &mut WprsServerState,
    sync: bool,
    parent: Option<WlSurface>,
    surface_order: Vec<SubsurfacePosition>,
    children_dirty: bool,
) -> Result<bool> {
    let surface_state = &mut surface_data
        .data_map
        .get::<LockedSurfaceState>()
        .location(loc!())?
        .0
        .lock()
        .unwrap();
    let prev_without_buffer = surface_state.clone_without_buffer();

    if matches!(surface_data.role, Some("subsurface")) && surface_state.role.is_none() {
        // TODO: figure out why some subsurfaces don't have parents. Probably a
        // race condition related to commits and other events.
        // let parent = parent.unwrap();  // every subsurface has a parent
        if let Some(parent) = parent {
            surface_state.role = Some(Role::SubSurface(SubSurfaceState::new(&parent)));
        } else {
            debug!("NO PARENT FOR SUBSURFACE {:?}", surface_state)
        }
    }

    surface_state.z_ordered_children = surface_order;

    // TODO: get actual surface position, for now put it at the end
    surface_state.z_ordered_children.push(SubsurfacePosition {
        id: WlSurfaceId::new(surface),
        position: (0, 0).into(),
    });

    let surface_attributes = surface_data.cached_state.current::<SurfaceAttributes>();

    set_regions(&surface_attributes, surface_state);
    set_transformation(&surface_attributes, surface_state);
    set_xdg_surface_attributes(surface_data, surface_state);

    match &mut surface_state.role {
        Some(Role::Cursor(_)) => {},
        Some(Role::SubSurface(subsurface_state)) => {
            subsurface_state.sync = sync;
        },
        Some(Role::XdgToplevel(toplevel_state)) => {
            set_xdg_toplevel_attributes(surface_data, toplevel_state).location(loc!())?;
        },
        Some(Role::XdgPopup(_)) => {},
        None => {},
    }

    // This needs to be a clone_without_buffer, the extra copy of the buffer
    // data arc will cause a deadlock otherwise.
    let mut surface_state_to_send = surface_state.clone_without_buffer();

    // TODO: make a function and dedupe with compositor.rs.
    debug!("buffer assignment: {:?}", &surface_attributes.buffer);
    match &surface_attributes.buffer {
        Some(SmithayBufferAssignment::NewBuffer(buffer)) => {
            compositor_utils::with_buffer_contents(buffer, |data, spec| {
                surface_state.set_buffer(&spec, data)
            })
            .location(loc!())?
            .location(loc!())?;

            surface_state_to_send.buffer = surface_state.buffer.clone();
            // surface_state.set_buffer (called above) sets buffer to
            // Some(BufferAssignment::New(...)), so the 4 unwraps below should
            // never fail.

            // zero-out data, see comment on wayland.rs::Buffer.
            surface_state_to_send
                .buffer
                .as_mut()
                .unwrap()
                .as_new_mut()
                .unwrap()
                .data = Arc::new(Vec4u8s::new());

            state.serializer.writer().send(SendType::RawBuffer(
                surface_state
                    .buffer
                    .as_ref()
                    .unwrap()
                    .as_new()
                    .unwrap()
                    .data
                    .clone(),
            ));
        },
        Some(SmithayBufferAssignment::Removed) => {
            surface_state.buffer = None;
            surface_state_to_send.buffer = Some(BufferAssignment::Removed);
        },
        None => {
            if (surface_state_to_send == prev_without_buffer) && !children_dirty {
                return Ok(false);
            }
            if children_dirty && sync {
                return Ok(false);
            }
        },
    }

    state
        .serializer
        .writer()
        .send(SendType::Object(Request::Surface(
            SurfaceRequest::new(
                surface,
                SurfaceRequestPayload::Commit(surface_state_to_send),
            )
            .location(loc!())?,
        )));
    Ok(true)
}

impl ShmHandler for WprsServerState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for WprsServerState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    #[instrument(skip(self, _seat), level = "debug")]
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: SmithayCursorImageStatus) {
        // TODO: move to a fn on serialization::CursorImaveStatus
        let cursor_image_status = {
            match image {
                SmithayCursorImageStatus::Hidden => CursorImageStatus::Hidden,
                SmithayCursorImageStatus::Surface(surface) => {
                    self.insert_surface(&surface).log_and_ignore(loc!());

                    let hotspot = compositor::with_states(&surface, |surface_data| {
                        let hotspot = surface_data
                            .data_map
                            .get::<CursorImageSurfaceData>()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .hotspot;

                        let surface_state = &mut surface_data
                            .data_map
                            .get::<LockedSurfaceState>()
                            .unwrap()
                            .0
                            .lock()
                            .unwrap();
                        surface_state.role = Some(Role::Cursor(hotspot.into()));

                        hotspot
                    });

                    CursorImageStatus::Surface {
                        client_surface: log_and_return!(ClientSurface::new(&surface)),
                        hotspot: hotspot.into(),
                    }
                },
                SmithayCursorImageStatus::Named(name) => {
                    CursorImageStatus::Named(name.name().to_string())
                },
            }
        };

        // TODO: expose serial to this function, then remove last_enter_serial
        // on client.
        self.serializer
            .writer()
            .send(SendType::Object(Request::CursorImage(CursorImage {
                serial: 0,
                status: cursor_image_status,
            })));
    }
}

impl WprsServerState {
    fn send_decoration_mode_request(&self, surface: &WlSurface, mode: Option<DecorationMode>) {
        self.serializer
            .writer()
            .send(SendType::Object(Request::Toplevel(ToplevelRequest {
                client: serialization::ClientId::new(&surface.client().unwrap()),
                surface: (&surface.id()).into(),
                payload: ToplevelRequestPayload::Decoration(mode),
            })))
    }
}

impl XdgDecorationHandler for WprsServerState {
    #[instrument(skip(self), level = "debug")]
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {}

    #[instrument(skip(self), level = "debug")]
    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: XdgDecorationMode) {
        let mode: Option<DecorationMode> = mode.try_into().log(loc!()).ok();
        if mode.is_some() {
            self.send_decoration_mode_request(toplevel.wl_surface(), mode);
        };
    }

    #[instrument(skip(self), level = "debug")]
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.send_decoration_mode_request(toplevel.wl_surface(), None);
    }
}

impl KdeDecorationHandler for WprsServerState {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.kde_decoration_state
    }

    #[instrument(skip(self, _surface, _decoration), level = "debug")]
    fn new_decoration(&mut self, _surface: &WlSurface, _decoration: &OrgKdeKwinServerDecoration) {}

    #[instrument(skip(self, decoration), level = "debug")]
    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<KdeDecorationMode>,
    ) {
        let mode = mode.into_result().log(loc!()).ok();
        if let Some(mode) = mode {
            decoration.mode(mode);
        }

        let mode: Option<DecorationMode> = mode.and_then(|m| m.try_into().log(loc!()).ok());
        if mode.is_some() {
            self.send_decoration_mode_request(surface, mode);
        };
    }

    #[instrument(skip(self, _decoration), level = "debug")]
    fn release(&mut self, _decoration: &OrgKdeKwinServerDecoration, surface: &WlSurface) {
        self.send_decoration_mode_request(surface, None);
    }
}

pub(crate) struct DndGrab {
    start_data: GrabStartData<WprsServerState>,
}

impl DndGrab {
    pub fn new(
        focus: Option<(
            <WprsServerState as SeatHandler>::PointerFocus,
            Point<i32, Logical>,
        )>,
        button: u32,
        location: (f64, f64),
    ) -> Self {
        Self {
            start_data: GrabStartData {
                focus,
                button,
                location: location.into(),
            },
        }
    }
}

impl PointerGrab<WprsServerState> for DndGrab {
    fn start_data(&self) -> &GrabStartData<WprsServerState> {
        &self.start_data
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn motion(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        focus: Option<(
            <WprsServerState as SeatHandler>::PointerFocus,
            Point<i32, Logical>,
        )>,
        event: &MotionEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn relative_motion(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        focus: Option<(
            <WprsServerState as SeatHandler>::PointerFocus,
            Point<i32, Logical>,
        )>,
        event: &RelativeMotionEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn button(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &ButtonEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn axis(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        details: AxisFrame,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn frame(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_swipe_begin(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GestureSwipeBeginEvent,
    );

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_swipe_update(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GestureSwipeUpdateEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_swipe_end(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GestureSwipeEndEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_pinch_begin(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GesturePinchBeginEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_pinch_update(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GesturePinchUpdateEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_pinch_end(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GesturePinchEndEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_hold_begin(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GestureHoldBeginEvent,
    ) {
    }

    #[instrument(skip(self, _data, _handle), level = "debug")]
    fn gesture_hold_end(
        &mut self,
        _data: &mut WprsServerState,
        _handle: &mut PointerInnerHandle<'_, WprsServerState>,
        event: &GestureHoldEndEvent,
    ) {
    }
}

pub struct ClientState {
    compositor_state: CompositorClientState,
    pub writer: DiscardingSender<Sender<SendType<Request>>>,
}

impl ClientState {
    pub fn new(writer: DiscardingSender<Sender<SendType<Request>>>) -> Self {
        Self {
            compositor_state: CompositorClientState::default(),
            writer,
        }
    }
}

impl ClientData for ClientState {
    #[instrument(skip(self), level = "debug")]
    fn initialized(&self, client_id: ClientId) {}

    #[instrument(skip(self), level = "debug")]
    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        self.writer
            .send(SendType::Object(Request::ClientDisconnected(client_id.into())))
            // This should be infallible, writer is an InfallibleWriter,
            // but we can't put an InfallibleWriter into ClientState
            // for ClientData trait API reasons.
            .unwrap();
    }
}

impl OutputHandler for WprsServerState {}

smithay::delegate_compositor!(WprsServerState);
smithay::delegate_xdg_shell!(WprsServerState);
smithay::delegate_xdg_decoration!(WprsServerState);
smithay::delegate_kde_decoration!(WprsServerState);
smithay::delegate_shm!(WprsServerState);
smithay::delegate_seat!(WprsServerState);
smithay::delegate_data_device!(WprsServerState);
smithay::delegate_output!(WprsServerState);
smithay::delegate_primary_selection!(WprsServerState);
