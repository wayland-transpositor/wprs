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
use std::os::fd::OwnedFd;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use smithay::input::Seat;
use smithay::input::SeatState;
use smithay::output::Output;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::compositor;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::compositor::SurfaceData;
use smithay::wayland::compositor::TraversalAction;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::kde::decoration::KdeDecorationState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shm::ShmState;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as KdeDecorationMode;
use smithay::wayland::viewporter::ViewporterState;

use crate::prelude::*;
use crate::serialization::wayland::SurfaceRequest;
use crate::serialization::wayland::SurfaceRequestPayload;
use crate::serialization::wayland::SurfaceState;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::Event;
use crate::serialization::Request;
use crate::serialization::SendType;
use crate::serialization::Serializer;
use crate::utils::SerialMap;

pub mod client_handlers;
pub mod smithay_handlers;

struct LockedSurfaceState(Mutex<SurfaceState>);

fn surface_destruction_callback(state: &mut WprsServerState, surface: &WlSurface) {
    compositor::with_states(surface, |surface_data| {
        let surface_state = surface_data
            .data_map
            .get::<LockedSurfaceState>()
            .unwrap()
            .0
            .lock()
            .unwrap();

        let writer = state.serializer.writer();

        writer.send(SendType::Object(Request::Surface(SurfaceRequest {
            client: surface_state.client,
            surface: surface_state.id,
            payload: SurfaceRequestPayload::Destroyed,
        })));

        state.object_map.remove(&surface_state.id);
    });
}

pub struct WprsServerState {
    pub dh: DisplayHandle,
    pub lh: LoopHandle<'static, Self>,
    pub compositor_state: CompositorState,
    pub start_time: Instant,
    pub frame_interval: Duration,
    pub xwayland_enabled: bool,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    // TODO(https://gitlab.gnome.org/GNOME/gtk/-/merge_requests/6398): rip this
    // out once GTK switches to xdg-decoration-protocol and applications/distros
    // move to GTK4.
    pub kde_decoration_state: KdeDecorationState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub viewporter_state: ViewporterState,

    pub seat: Seat<Self>,

    pub serializer: Serializer<Request, Event>,
    /// Reverse map from WlSurfaceId, which is the hash of ObjectId, back to its
    /// source ObjectId. We can't put this in SurfaceState because is
    /// serializable, while this only has meaning locally. We need this for
    /// finding the local surface associated with a remote pointer/keyboard
    /// event. Left is the serialized surface id and right is the local, native
    /// surface id.
    // object_map:
    // left: serialized surface id, right: local native surface id
    pub object_map: HashMap<WlSurfaceId, ObjectId>,
    pub outputs: HashMap<u32, (Output, GlobalId)>,
    serial_map: SerialMap,
    pressed_keys: HashSet<u32>,
    pressed_buttons: HashSet<u32>,

    selection_pipe: Option<OwnedFd>,
    dnd_source: Option<WlDataSource>,
    dnd_pipe: Option<OwnedFd>,
    primary_selection_pipe: Option<OwnedFd>,
}

impl WprsServerState {
    pub fn new(
        dh: DisplayHandle,
        lh: LoopHandle<'static, Self>,
        serializer: Serializer<Request, Event>,
        xwayland_enabled: bool,
        frame_interval: Duration,
        kde_server_side_decorations: bool,
    ) -> Self {
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "wprs");
        let kde_default_decoration_mode = if kde_server_side_decorations {
            KdeDecorationMode::Server
        } else {
            KdeDecorationMode::Client
        };

        Self {
            dh: dh.clone(),
            lh,
            compositor_state: CompositorState::new::<Self>(&dh),
            start_time: Instant::now(),
            xwayland_enabled,
            frame_interval,
            xdg_shell_state: XdgShellState::new::<Self>(&dh),
            xdg_decoration_state: XdgDecorationState::new::<Self>(&dh),
            kde_decoration_state: KdeDecorationState::new::<Self>(&dh, kde_default_decoration_mode),
            shm_state: ShmState::new::<Self>(&dh, Vec::new()),
            seat_state,
            data_device_state: DataDeviceState::new::<Self>(&dh),
            primary_selection_state: PrimarySelectionState::new::<Self>(&dh),
            viewporter_state: ViewporterState::new::<Self>(&dh),
            seat,
            serializer,
            object_map: HashMap::new(),
            outputs: HashMap::new(),
            serial_map: SerialMap::new(),
            pressed_keys: HashSet::new(),
            pressed_buttons: HashSet::new(),
            selection_pipe: None,
            dnd_source: None,
            dnd_pipe: None,
            primary_selection_pipe: None,
        }
    }

    #[instrument(skip(self), level = "debug")]
    pub fn insert_surface(&mut self, surface: &WlSurface) -> Result<()> {
        self.object_map
            .insert(WlSurfaceId::new(surface), surface.id());

        let newly_inserted = compositor::with_states(surface, |surface_data| {
            surface_data.data_map.insert_if_missing_threadsafe(|| {
                LockedSurfaceState(Mutex::new(SurfaceState::new(surface, None).unwrap()))
            })
        });
        // TODO: https://github.com/Smithay/smithay/issues/538 - move into block
        // above.
        if newly_inserted {
            compositor::add_destruction_hook(surface, surface_destruction_callback);
        }
        Ok(())
    }

    pub fn for_each_surface<F>(&self, mut processor: F)
    where
        F: FnMut(&WlSurface, &SurfaceData),
    {
        for surface in self.xdg_shell_state.toplevel_surfaces() {
            compositor::with_surface_tree_downward(
                surface.wl_surface(),
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |surface, surface_data, _| processor(surface, surface_data),
                |_, _, _| true,
            );
        }

        for surface in self.xdg_shell_state.popup_surfaces() {
            compositor::with_surface_tree_downward(
                surface.wl_surface(),
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |surface, surface_data, _| processor(surface, surface_data),
                |_, _, _| true,
            )
        }
    }
}
