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

use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::reexports::client::protocol::wl_subcompositor::WlSubcompositor;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::shell::WaylandSurface;

use crate::client::smithay_handlers::SubSurfaceData;
use crate::client::ObjectBimap;
use crate::client::RemoteSurface;
use crate::client::Role;
use crate::client::WprsClientState;
use crate::fallible_entry::FallibleEntryExt;
use crate::prelude::*;
use crate::serialization::wayland::SubSurfaceState;
use crate::serialization::wayland::SubsurfacePosition;
use crate::serialization::wayland::SurfaceState;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::ClientId;

pub(crate) fn populate_subsurfaces(
    client_id: ClientId,
    surface_id: WlSurfaceId,
    surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
    compositor: &CompositorState,
    subcompositor: &WlSubcompositor,
    qh: &QueueHandle<WprsClientState>,
    object_bimap: &mut ObjectBimap,
) -> Result<()> {
    let children = surfaces
        .get(&surface_id)
        .location(loc!())?
        .z_ordered_children
        .clone();
    for child in children.into_iter().filter(|c| c.id != surface_id) {
        surfaces.entry(child.id).or_insert_with_result(|| {
            RemoteSurface::new(client_id, surface_id, compositor, qh, object_bimap)
        })?;

        RemoteSubSurface::set_role(
            client_id,
            surface_id,
            child.id,
            surfaces,
            compositor,
            subcompositor,
            qh,
            object_bimap,
        )
        .location(loc!())?;
    }
    Ok(())
}

fn commit_sync_children_impl(
    surface_id: WlSurfaceId,
    surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
    parent_is_sync: bool,
) -> Result<()> {
    let remote_surface = surfaces.get_mut(&surface_id).location(loc!())?;
    let surface_is_sync = remote_surface
        .role
        .as_ref()
        .location(loc!())?
        .as_sub_surface()
        .location(loc!())?
        .sync;
    let is_sync = parent_is_sync | surface_is_sync;
    if is_sync {
        remote_surface.attach_damage_commit()?;
    }

    let children = surfaces
        .get(&surface_id)
        .location(loc!())?
        .z_ordered_children
        .clone();
    for child in children.into_iter().filter(|c| c.id != surface_id) {
        commit_sync_children_impl(child.id, surfaces, is_sync).location(loc!())?;
    }
    Ok(())
}

pub(crate) fn commit_sync_children(
    surface_id: WlSurfaceId,
    surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
) -> Result<()> {
    let Some(surface) = surfaces.get(&surface_id) else {
        // TODO: should this be an error?
        return Ok(());
    };

    // If the current surface is a sync subsurface, then don't process its
    // children and let them be processed when the closest desync ancestor is
    // being processed.
    if let Some(Role::SubSurface(subsurface)) = &surface.role {
        if subsurface.sync {
            return Ok(());
        }
    }

    let children = surface.z_ordered_children.clone();
    for child in children.into_iter().filter(|c| c.id != surface_id) {
        commit_sync_children_impl(child.id, surfaces, false)?;
    }
    Ok(())
}

pub(crate) fn reorder_subsurfaces(
    surface_id: WlSurfaceId,
    surface_state: &SurfaceState,
    surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
) -> Result<()> {
    let moves = {
        let remote_surface = surfaces.get_mut(&surface_id).location(loc!())?;
        remote_surface.reorder_children(&surface_state.z_ordered_children)
    };

    let moves: Result<Vec<_>> = moves
        .iter()
        .map(
            |(surf_to_move, surf_to_move_to)| -> Result<(&WlSubsurface, &WlSurface)> {
                Ok((
                    &surfaces
                        .get(surf_to_move)
                        .location(loc!())?
                        .get_role()
                        .location(loc!())?
                        .as_sub_surface()
                        .location(loc!())?
                        .local_subsurface,
                    surfaces.get(surf_to_move_to).location(loc!())?.wl_surface(),
                ))
            },
        )
        .collect();

    for (surf_to_move, surf_to_move_to) in moves.location(loc!())? {
        surf_to_move.place_above(surf_to_move_to);
    }

    let children = surfaces
        .get(&surface_id)
        .location(loc!())?
        .z_ordered_children
        .clone();

    for child in children.into_iter().filter(|c| c.id != surface_id) {
        debug!(
            "Setting subsurface {:?} position to {:?}.",
            child.id,
            (child.position.x, child.position.y)
        );
        surfaces
            .get_mut(&child.id)
            .location(loc!())?
            .role
            .as_mut()
            .location(loc!())?
            .as_sub_surface_mut()
            .location(loc!())?
            .local_subsurface
            .set_position(child.position.x, child.position.y);
    }

    Ok(())
}

#[derive(Debug)]
pub struct RemoteSubSurface {
    pub(crate) parent: WlSurfaceId,
    pub(crate) sync: bool,
    local_subsurface: WlSubsurface,
}

impl RemoteSubSurface {
    pub fn set_role(
        client_id: ClientId,
        parent_id: WlSurfaceId,
        surface_id: WlSurfaceId,
        surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
        compositor: &CompositorState,
        subcompositor: &WlSubcompositor,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<()> {
        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        if surface.role.is_some() {
            return Ok(());
        }

        let parent = {
            // It's possible that on initial sync, the server sent over a
            // subsurface before the parent surface, so insert an entry for the
            // parent here if we need to. The parent state will be sent forthwith.
            let remote_parent_surface = surfaces
                .entry(parent_id)
                .or_insert_with_result(|| {
                    RemoteSurface::new(client_id, parent_id, compositor, qh, object_bimap)
                })
                .location(loc!())?;

            if !remote_parent_surface
                .z_ordered_children
                .iter()
                .any(|child| child.id == surface_id)
            {
                remote_parent_surface
                    .z_ordered_children
                    .push(SubsurfacePosition {
                        id: surface_id,
                        position: (0, 0).into(),
                    });
            }

            remote_parent_surface.wl_surface().clone()
        };

        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        if surface.role.is_some() {
            return Ok(());
        }

        let local_subsurface = subcompositor.get_subsurface(
            surface
                .local_surface
                .as_ref()
                .location(loc!())?
                .wl_surface(),
            &parent,
            qh,
            SubSurfaceData,
        );

        let remote_subsurface = Self {
            parent: parent_id,
            sync: true,
            local_subsurface,
        };
        surface.role = Some(Role::SubSurface(remote_subsurface));
        Ok(())
    }

    pub fn update(subsurface_state: &SubSurfaceState, surface: &mut RemoteSurface) -> Result<()> {
        let role = &mut surface
            .role
            .as_mut()
            .location(loc!())?
            .as_sub_surface_mut()
            .location(loc!())?;
        if role.sync == subsurface_state.sync {
            return Ok(());
        }

        role.sync = subsurface_state.sync;

        if role.sync {
            role.local_subsurface.set_sync();
        } else {
            role.local_subsurface.set_desync();
        }

        Ok(())
    }

    pub fn apply(
        client_id: ClientId,
        surface_state: SurfaceState,
        surface_id: WlSurfaceId,
        surfaces: &mut HashMap<WlSurfaceId, RemoteSurface>,
        compositor: &CompositorState,
        subcompositor: &WlSubcompositor,
        qh: &QueueHandle<WprsClientState>,
        object_bimap: &mut ObjectBimap,
    ) -> Result<()> {
        let subsurface_state = surface_state
            .role
            .as_ref()
            .location(loc!())?
            .as_sub_surface()
            .location(loc!())?;
        Self::set_role(
            client_id,
            subsurface_state.parent,
            surface_id,
            surfaces,
            compositor,
            subcompositor,
            qh,
            object_bimap,
        )
        .location(loc!())?;
        let surface = surfaces.get_mut(&surface_id).location(loc!())?;
        Self::update(subsurface_state, surface).location(loc!())?;
        Ok(())
    }
}
