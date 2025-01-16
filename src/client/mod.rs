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
use std::sync::OnceLock;

use bimap::BiMap;
use enum_as_inner::EnumAsInner;
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::compositor::Surface;
use smithay_client_toolkit::data_device_manager::data_offer::DragOffer;
use smithay_client_toolkit::data_device_manager::data_offer::SelectionOffer;
use smithay_client_toolkit::data_device_manager::data_source::CopyPasteSource;
use smithay_client_toolkit::data_device_manager::data_source::DragSource;
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::data_device_manager::WritePipe;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::primary_selection::offer::PrimarySelectionOffer;
use smithay_client_toolkit::primary_selection::selection::PrimarySelectionSource;
use smithay_client_toolkit::primary_selection::PrimarySelectionManagerState;
use smithay_client_toolkit::reexports::client::backend::ObjectId as SctkObjectId;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_output::Transform;
use smithay_client_toolkit::reexports::client::protocol::wl_subcompositor::WlSubcompositor;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_surface;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::pointer::ThemedPointer;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::XdgSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::Buffer as SlotBuffer;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::Shm;

use crate::client_utils::SeatObject;
use crate::constants;
use crate::filtering;
use crate::prelude::*;
use crate::serialization::geometry::Point;
use crate::serialization::geometry::Rectangle;
use crate::serialization::wayland::Buffer;
use crate::serialization::wayland::BufferAssignment;
use crate::serialization::wayland::BufferMetadata;
use crate::serialization::wayland::Region;
use crate::serialization::wayland::SubsurfacePosition;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::Capabilities;
use crate::serialization::ClientId;
use crate::serialization::Event;
use crate::serialization::ObjectId;
use crate::serialization::Request;
use crate::serialization::Serializer;
use crate::vec4u8::Vec4u8s;

pub mod server_handlers;
pub mod smithay_handlers;
mod subsurface;
mod xdg_shell;

use smithay_handlers::SubCompositorData;
use subsurface::RemoteSubSurface;
use xdg_shell::RemoteXdgPopup;
use xdg_shell::RemoteXdgToplevel;

type ObjectBimap = BiMap<(ClientId, ObjectId), SctkObjectId>;

pub trait ObjectBimapExt {
    fn get_wl_surface_id(&self, key: &SctkObjectId) -> Option<(ClientId, WlSurfaceId)>;
}

impl ObjectBimapExt for ObjectBimap {
    fn get_wl_surface_id(&self, key: &SctkObjectId) -> Option<(ClientId, WlSurfaceId)> {
        match self.get_by_right(key) {
            Some((client_id, ObjectId::WlSurface(surface_id))) => Some((*client_id, *surface_id)),
            None => None,
            _ => panic!("Object corresponding to client object id {key} should be a WlSurface,"),
        }
    }
}

pub struct ClientOptions {
    pub title_prefix: String,
}

pub struct WprsClientState {
    qh: QueueHandle<WprsClientState>,
    conn: Connection,
    pub capabilities: Arc<OnceLock<Capabilities>>,

    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    subcompositor: WlSubcompositor,
    shm_state: Shm,
    xdg_shell_state: XdgShell,

    data_device_manager_state: DataDeviceManagerState,
    primary_selection_manager_state: Option<PrimarySelectionManagerState>,

    pool: SlotPool,

    seat_objects: Vec<SeatObject<ThemedPointer>>,
    selection_source: Option<CopyPasteSource>,
    selection_offer: Option<SelectionOffer>,
    selection_pipe: Option<WritePipe>,
    dnd_source: Option<DragSource>,
    dnd_offer: Option<DragOffer>,
    dnd_pipe: Option<WritePipe>,
    dnd_accept_counter: u32,
    primary_selection_source: Option<PrimarySelectionSource>,
    primary_selection_pipe: Option<WritePipe>,
    primary_selection_offer: Option<PrimarySelectionOffer>,

    serializer: Serializer<Event, Request>,
    remote_display: RemoteDisplay,
    // left: remote object IDs, right: local "native" object IDs
    pub object_bimap: ObjectBimap,

    last_enter_serial: u32,
    last_implicit_grab_serial: Option<u32>,
    last_mouse_down_serial: Option<u32>,
    current_focus: Option<WlSurface>,

    title_prefix: String,

    buffer_cache: Option<Arc<Vec4u8s>>,
}

impl WprsClientState {
    pub fn new(
        qh: QueueHandle<Self>,
        globals: GlobalList,
        conn: Connection,
        serializer: Serializer<Event, Request>,
        options: ClientOptions,
    ) -> Result<Self> {
        let shm_state = Shm::bind(&globals, &qh).context(loc!(), "wl_shm is not available")?;

        // size doesn't really matter, the pool will be automatically grown as
        // necessary.
        let pool =
            SlotPool::new(3840 * 2160, &shm_state).context(loc!(), "failed to create pool")?;

        Ok(Self {
            qh: qh.clone(),
            conn,
            capabilities: Arc::new(OnceLock::new()),
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh)
                .context(loc!(), "wl_compositor is not available")?,
            subcompositor: globals
                .bind(&qh, 1..=1, SubCompositorData)
                .context(loc!(), "wl_subcompositor is not available")?,
            shm_state,
            xdg_shell_state: XdgShell::bind(&globals, &qh)
                .context(loc!(), "xdg shell is not available")?,
            data_device_manager_state: DataDeviceManagerState::bind(&globals, &qh)
                .context(loc!(), "data device manager is not available")?,
            primary_selection_manager_state: PrimarySelectionManagerState::bind(&globals, &qh)
                .context(loc!(), "primary selection manager is not available")
                .warn(loc!())
                .ok(),

            pool,

            seat_objects: Vec::new(),
            selection_source: None,
            selection_offer: None,
            selection_pipe: None,
            dnd_source: None,
            dnd_offer: None,
            dnd_pipe: None,
            dnd_accept_counter: 0,
            primary_selection_source: None,
            primary_selection_offer: None,
            primary_selection_pipe: None,

            serializer,
            remote_display: RemoteDisplay::new(),
            object_bimap: BiMap::new(),

            last_enter_serial: 0,
            last_implicit_grab_serial: None,
            last_mouse_down_serial: None,
            current_focus: None,
            title_prefix: options.title_prefix,
            buffer_cache: None,
        })
    }
}

#[derive(Debug)]
pub struct RemoteBuffer {
    pub metadata: BufferMetadata,
    pub data: Vec4u8s,
    pub active_buffer: SlotBuffer,
    pub dirty: bool,
}

impl RemoteBuffer {
    #[allow(clippy::missing_panics_doc)]
    pub fn new(buffer_msg: Buffer, pool: &mut SlotPool) -> Result<Self> {
        let active_buffer = pool
            .create_buffer(
                buffer_msg.metadata.width,
                buffer_msg.metadata.height,
                buffer_msg.metadata.stride,
                buffer_msg.metadata.format.into(),
            )
            .location(loc!())?
            .0;

        Ok(Self {
            metadata: buffer_msg.metadata,
            // The arc is a server-side optimization and nothing else here has a
            // reference to it. The arc is here at all because the same type
            // needs to be present in both the server and the client for
            // serialization/deserialization.
            data: Arc::into_inner(buffer_msg.data).unwrap(),
            active_buffer,
            dirty: true,
        })
    }

    fn update_data(&mut self, buffer: Buffer) {
        // The arc is a server-side optimization and nothing else here has a
        // reference to it. The arc is here at all because the same type
        // needs to be present in both the server and the client for
        // serialization/deserialization.
        self.data = Arc::into_inner(buffer.data).unwrap();
        self.dirty = true;
    }

    #[instrument(skip_all, level = "debug")]
    fn write_data(&mut self, pool: &mut SlotPool) -> Result<()> {
        let canvas = match pool.canvas(&self.active_buffer) {
            Some(canvas) => canvas,
            None => {
                // This should be rare, but if the compositor has not
                // released the previous_button_state buffer, we need
                // double-buffering.
                debug!("creating new buffer");
                self.active_buffer = pool
                    .create_buffer(
                        self.metadata.width,
                        self.metadata.height,
                        self.metadata.stride,
                        self.metadata.format.into(),
                    )
                    .location(loc!())?
                    .0;
                pool.canvas(&self.active_buffer).location(loc!())?
            },
        };
        filtering::unfilter(&mut self.data, canvas);
        Ok(())
    }
}

#[derive(Debug, EnumAsInner)]
pub enum Role {
    Cursor(RemoteCursor),
    SubSurface(RemoteSubSurface),
    XdgToplevel(RemoteXdgToplevel),
    XdgPopup(RemoteXdgPopup),
}

impl WaylandSurface for RemoteSurface {
    fn wl_surface(&self) -> &WlSurface {
        match &self.role {
            None | Some(Role::Cursor(_)) => self.local_surface.as_ref().unwrap().wl_surface(),
            Some(Role::SubSurface(remote_subsurface)) => {
                remote_subsurface.local_surface.wl_surface()
            },
            Some(Role::XdgToplevel(remote_xdg_toplevel)) => {
                remote_xdg_toplevel.local_window.wl_surface()
            },
            Some(Role::XdgPopup(remote_xdg_popup)) => remote_xdg_popup.local_popup.wl_surface(),
        }
    }
}

impl WaylandSurface for RemoteXdgToplevel {
    fn wl_surface(&self) -> &WlSurface {
        self.local_window.wl_surface()
    }
}

impl XdgSurface for RemoteXdgToplevel {
    fn xdg_surface(&self) -> &xdg_surface::XdgSurface {
        self.local_window.xdg_surface()
    }
}

impl WaylandSurface for RemoteXdgPopup {
    fn wl_surface(&self) -> &WlSurface {
        self.local_popup.wl_surface()
    }
}

impl XdgSurface for RemoteXdgPopup {
    fn xdg_surface(&self) -> &xdg_surface::XdgSurface {
        self.local_popup.xdg_surface()
    }
}

#[derive(Debug)]
pub struct RemoteSurface {
    pub client: ClientId,
    pub id: WlSurfaceId,
    pub buffer: Option<RemoteBuffer>,
    // None when the surface is owned by a role object (e.g., a Window).
    pub local_surface: Option<Surface>,
    pub role: Option<Role>,
    pub opaque_region: Option<Region>,
    pub input_region: Option<Region>,
    pub z_ordered_children: Vec<SubsurfacePosition>,
    pub frame_callback_completed: bool,
    pub frame_damage: Option<Vec<Rectangle<i32>>>,
}

impl RemoteSurface {
    pub fn new(
        client_id: ClientId,
        id: WlSurfaceId,
        compositor_state: &CompositorState,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<Self> {
        let local_surface = Some(Surface::new(compositor_state, qh).location(loc!())?);

        object_bimap.insert(
            (client_id, ObjectId::WlSurface(id)),
            local_surface.as_ref().location(loc!())?.wl_surface().id(),
        );

        Ok(Self {
            client: client_id,
            id,
            buffer: None,
            local_surface,
            role: None,
            opaque_region: None,
            input_region: None,
            z_ordered_children: vec![SubsurfacePosition {
                id,
                position: (0, 0).into(),
            }],
            frame_callback_completed: true,
            frame_damage: None,
        })
    }

    fn reorder_children(
        &mut self,
        new_order: &[SubsurfacePosition],
    ) -> Vec<(WlSurfaceId, WlSurfaceId)> {
        let mut moves = Vec::new();

        let mut new_order = new_order.to_vec();
        new_order.reverse();

        debug!(
            "REORDER_CHILDREN, {:?}, {:?}, {:?}",
            &self.id, &self.z_ordered_children, &new_order
        );

        let z_ordered_children_set: HashSet<WlSurfaceId> =
            self.z_ordered_children.iter().map(|c| c.id).collect();

        let new_order: Vec<SubsurfacePosition> = new_order
            .iter()
            .filter(|elem| z_ordered_children_set.contains(&elem.id))
            .cloned()
            .collect();

        for (idx, elem) in new_order.iter().enumerate() {
            let current_elem = self.z_ordered_children[idx];
            if current_elem.id == elem.id {
                self.z_ordered_children[idx] = *elem; // position may have changed
                continue;
            }

            let current_idx = self
                .z_ordered_children
                .iter()
                .position(|x| x.id == elem.id)
                .unwrap(); // we already filtered out non-present elements

            self.z_ordered_children.remove(current_idx);
            // Insert elem instead of child because position may have changed.
            self.z_ordered_children.insert(idx, *elem);
            moves.push((elem.id, current_elem.id));
        }

        moves
    }

    pub fn write_data(&mut self, pool: &mut SlotPool) -> Result<()> {
        if let Some(buffer) = &mut self.buffer {
            buffer.write_data(pool).location(loc!())?;
        }
        Ok(())
    }

    #[instrument(skip(self, pool), level = "debug")]
    fn set_buffer(&mut self, new_buffer: Buffer, pool: &mut SlotPool) -> Result<()> {
        let buffer = match &mut self.buffer {
            // Surface was previously committed.
            Some(buffer) => {
                // Only buffer data was updated, we can reuse the buffer.
                if buffer.metadata == new_buffer.metadata {
                    buffer.update_data(new_buffer);
                    buffer
                } else {
                    // Buffer was resized or format changed, need to
                    // create a new one.
                    *buffer = RemoteBuffer::new(new_buffer, pool).location(loc!())?;
                    buffer
                }
            },
            // First commit for surface with a buffer.
            None => {
                self.buffer = Some(RemoteBuffer::new(new_buffer, pool).location(loc!())?);
                self.buffer.as_mut().unwrap() // we just set this to Some
            },
        };

        if buffer.dirty {
            buffer.write_data(pool).location(loc!())?;
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn clear_buffer(&mut self) {
        let wl_surface = self.wl_surface().clone();
        self.buffer = None;
        wl_surface.attach(None, 0, 0);
    }

    #[instrument(skip(self, buffer_cache, pool), level = "debug")]
    pub fn apply_buffer(
        &mut self,
        new_buffer: Option<BufferAssignment>,
        buffer_cache: &mut Option<Arc<Vec4u8s>>,
        pool: &mut SlotPool,
    ) -> Result<()> {
        match new_buffer {
            Some(BufferAssignment::New(mut new_buffer)) => {
                if let Some(buffer_data) = buffer_cache.take() {
                    new_buffer.data = buffer_data;
                }
                // else use the data in new_buffer as the buffer is data is
                // still sent inline on connection.

                if new_buffer.data.is_empty() {
                    // TODO: do we want to log a warning and let the rest of the
                    // commit work? Unclear that it matters.
                    return Err(anyhow!("Received buffer commit with empty data. This can if wprsc reattaches between wprsd sending a buffer message and a commit message."));
                }

                self.set_buffer(new_buffer, pool).location(loc!())?;
            },
            Some(BufferAssignment::Removed) => {
                self.clear_buffer();
            },
            None => {},
        }
        Ok(())
    }

    pub fn draw_buffer(&mut self) -> Result<()> {
        let wl_surface = &self.wl_surface().clone();
        if let Some(buffer) = &mut self.buffer {
            if buffer.dirty {
                buffer.active_buffer.attach_to(wl_surface).context(
                    loc!(),
                    "attaching a buffer failed, this probably means we're leaking buffers",
                )?;
                if let Some(damage_rects) = self.frame_damage.take() {
                    // avoid overwhelming wayland connection
                    if damage_rects.len() < constants::SENT_DAMAGE_LIMIT {
                        for damage_rect in damage_rects {
                            wl_surface.damage_buffer(
                                damage_rect.loc.x,
                                damage_rect.loc.y,
                                damage_rect.size.w,
                                damage_rect.size.h,
                            );
                        }
                    } else {
                        wl_surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                    }
                } else {
                    wl_surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                }
                buffer.dirty = false;
            }
        }
        self.commit();
        Ok(())
    }

    pub fn draw_buffer_send_frame(&mut self, qh: &QueueHandle<WprsClientState>) -> Result<()> {
        let wl_surface = &self.wl_surface().clone();
        if let Some(buffer) = &mut self.buffer {
            if buffer.dirty {
                buffer.active_buffer.attach_to(wl_surface).context(
                    loc!(),
                    "attaching a buffer failed, this probably means we're leaking buffers",
                )?;
                if let Some(damage_rects) = self.frame_damage.take() {
                    for damage_rect in damage_rects {
                        wl_surface.damage_buffer(
                            damage_rect.loc.x,
                            damage_rect.loc.y,
                            damage_rect.size.w,
                            damage_rect.size.h,
                        );
                    }
                } else {
                    wl_surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                }
                buffer.dirty = false;
                self.frame(qh);
                self.frame_callback_completed = false;
            }
        }
        self.commit();
        Ok(())
    }

    pub fn set_transformation(&mut self, scale: i32, transform: Option<Transform>) {
        self.wl_surface().set_buffer_scale(scale);
        if let Some(transform) = transform {
            self.wl_surface().set_buffer_transform(transform);
        }
    }

    pub fn set_input_region(
        &mut self,
        region: Option<Region>,
        compositor_state: &CompositorState,
    ) -> Result<()> {
        if self.input_region == region {
            return Ok(());
        }

        self.input_region = region;

        if let Some(region) = &self.input_region {
            self.wl_surface().set_input_region(Some(
                region
                    .create_compositor_region(compositor_state)
                    .location(loc!())?
                    .wl_region(),
            ));
        } else {
            self.wl_surface().set_input_region(None);
        }
        Ok(())
    }

    pub fn set_opaque_region(
        &mut self,
        region: Option<Region>,
        compositor_state: &CompositorState,
    ) -> Result<()> {
        if self.opaque_region == region {
            return Ok(());
        }

        self.opaque_region = region;

        if let Some(region) = &self.opaque_region {
            self.wl_surface().set_opaque_region(Some(
                region
                    .create_compositor_region(compositor_state)
                    .location(loc!())?
                    .wl_region(),
            ));
        } else {
            self.wl_surface().set_opaque_region(None);
        }
        Ok(())
    }

    pub fn commit(&mut self) {
        self.wl_surface().commit();
    }

    pub fn frame(&self, qh: &QueueHandle<WprsClientState>) {
        self.wl_surface().frame(qh, self.wl_surface().clone());
    }

    pub fn get_role(&self) -> Result<&Role> {
        self.role.as_ref().context(loc!(), "Role was None.")
    }

    pub fn get_mut_role(&mut self) -> Result<&mut Role> {
        self.role.as_mut().context(loc!(), "Role was None.")
    }

    pub fn xdg_surface(&self) -> Option<xdg_surface::XdgSurface> {
        match &self.role {
            Some(Role::XdgToplevel(toplevel)) => Some(toplevel.xdg_surface().clone()),
            Some(Role::XdgPopup(popup)) => Some(popup.xdg_surface().clone()),
            _ => None,
        }
    }

    pub fn xdg_toplevel(&self) -> Result<&RemoteXdgToplevel> {
        self.get_role()
            .location(loc!())?
            .as_xdg_toplevel()
            .context(loc!(), "Role was not XdgToplevel.")
    }

    pub fn xdg_popup(&self) -> Result<&RemoteXdgPopup> {
        self.get_role()
            .location(loc!())?
            .as_xdg_popup()
            .context(loc!(), "Role was not XdgPopup.")
    }

    pub fn xdg_popup_mut(&mut self) -> Result<&mut RemoteXdgPopup> {
        self.get_mut_role()
            .location(loc!())?
            .as_xdg_popup_mut()
            .context(loc!(), "Role was not XdgPopup.")
    }
}

#[derive(Debug)]
pub struct RemoteCursor {
    // TODO: can we remove client?
    pub client: ClientId,
    pub hotspot: Point<i32>,
}

impl RemoteCursor {
    pub fn set_role(client_id: ClientId, surface: &mut RemoteSurface) {
        let remote_cursor = Self {
            client: client_id,
            hotspot: Point { x: 0, y: 0 }, // TODO
        };
        surface.role = Some(Role::Cursor(remote_cursor));
    }
}

#[derive(Debug)]
pub struct RemoteClient {
    pub id: ClientId,
    pub surfaces: HashMap<WlSurfaceId, RemoteSurface>,
}

impl RemoteClient {
    pub fn new(id: ClientId) -> Self {
        Self {
            id,
            surfaces: HashMap::new(),
        }
    }

    pub fn remove_surface(&mut self, id: &WlSurfaceId, state: &mut WprsClientState) {
        let surface = self.surfaces.remove(id);
        if let Some(surface) = surface {
            // self.surface_owners.remove(&surface.id);
            state
                .object_bimap
                .remove_by_left(&(self.id, ObjectId::WlSurface(surface.id)));
        }
    }

    pub fn surface(&mut self, id: &WlSurfaceId) -> Result<&mut RemoteSurface> {
        self.surfaces
            .get_mut(id)
            .with_context(loc!(), || format!("Unknown surface id: {:?}", id))
    }
}

#[derive(Debug)]
pub struct RemoteDisplay {
    pub clients: HashMap<ClientId, RemoteClient>,
}

impl RemoteDisplay {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub fn client(&mut self, id: &ClientId) -> &mut RemoteClient {
        self.clients.entry(*id).or_insert(RemoteClient::new(*id))
    }
}

impl Default for RemoteDisplay {
    fn default() -> Self {
        Self::new()
    }
}
