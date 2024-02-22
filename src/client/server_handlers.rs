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

/// Handlers for events from the wprs server.
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::thread;

use smithay_client_toolkit::shell::WaylandSurface;

use crate::client::subsurface;
use crate::client::subsurface::RemoteSubSurface;
use crate::client::RemoteCursor;
use crate::client::RemoteSurface;
use crate::client::RemoteXdgPopup;
use crate::client::RemoteXdgToplevel;
use crate::client::Role;
use crate::client::WprsClientState;
use crate::fallible_entry::FallibleEntryExt;
use crate::prelude::*;
use crate::serialization::tuple::Tuple2;
use crate::serialization::wayland;
use crate::serialization::wayland::ClientSurface;
use crate::serialization::wayland::CursorImage;
use crate::serialization::wayland::CursorImageStatus;
use crate::serialization::wayland::DataDestinationRequest;
use crate::serialization::wayland::DataEvent;
use crate::serialization::wayland::DataRequest;
use crate::serialization::wayland::DataSource;
use crate::serialization::wayland::DataSourceRequest;
use crate::serialization::wayland::DataToTransfer;
use crate::serialization::wayland::SurfaceRequest;
use crate::serialization::wayland::SurfaceRequestPayload;
use crate::serialization::wayland::SurfaceState;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::xdg_shell;
use crate::serialization::xdg_shell::PopupRequest;
use crate::serialization::xdg_shell::PopupRequestPayload;
use crate::serialization::xdg_shell::ToplevelRequest;
use crate::serialization::xdg_shell::ToplevelRequestPayload;
use crate::serialization::Capabilities;
use crate::serialization::ClientId;
use crate::serialization::Event;
use crate::serialization::RecvType;
use crate::serialization::Request;
use crate::serialization::SendType;

impl WprsClientState {
    #[instrument(skip(self), level = "debug")]
    fn handle_commit(
        &mut self,
        client_id: ClientId,
        surface_id: WlSurfaceId,
        mut surface_state: SurfaceState,
    ) -> Result<()> {
        let client = self.remote_display.client(&client_id);
        let surfaces = &mut client.surfaces;

        let frame_callback_completed = {
            let remote_surface = surfaces
                .entry(surface_id)
                .or_insert_with_result(|| {
                    RemoteSurface::new(
                        client.id,
                        surface_id,
                        &self.compositor_state,
                        &self.qh,
                        &mut self.object_bimap,
                    )
                })
                .location(loc!())?;

            remote_surface
                .apply_buffer(
                    surface_state.buffer.take(),
                    &mut self.buffer_cache,
                    &mut self.pool,
                )
                .location(loc!())?;

            remote_surface.set_transformation(surface_state.buffer_scale);

            remote_surface
                .set_input_region(surface_state.input_region.take(), &self.compositor_state)
                .location(loc!())?;
            remote_surface
                .set_opaque_region(surface_state.opaque_region.take(), &self.compositor_state)
                .location(loc!())?;

            remote_surface.set_title_prefix(&self.title_prefix);

            remote_surface.frame_callback_completed
        };

        subsurface::populate_subsurfaces(
            client.id,
            surface_id,
            surfaces,
            &self.compositor_state,
            &self.subcompositor,
            &self.qh,
            &mut self.object_bimap,
        )
        .location(loc!())?;
        subsurface::reorder_subsurfaces(surface_id, &surface_state, surfaces).location(loc!())?;

        match &surface_state.role {
            Some(wayland::Role::Cursor(_)) => {},
            Some(wayland::Role::SubSurface(_)) => RemoteSubSurface::apply(
                client.id,
                surface_state,
                surface_id,
                surfaces,
                &self.compositor_state,
                &self.subcompositor,
                &self.qh,
                &mut self.object_bimap,
            )
            .location(loc!())?,
            Some(wayland::Role::XdgToplevel(_)) => RemoteXdgToplevel::apply(
                client.id,
                surface_state,
                surface_id,
                surfaces,
                &self.xdg_shell_state,
                &self.qh,
                &mut self.object_bimap,
            )
            .location(loc!())?,
            Some(wayland::Role::XdgPopup(_)) => RemoteXdgPopup::apply(
                client.id,
                surface_state,
                surface_id,
                surfaces,
                &self.xdg_shell_state,
                &self.qh,
                &mut self.object_bimap,
            )
            .location(loc!())?,
            None => {},
        }

        if frame_callback_completed {
            subsurface::commit_sync_children(surface_id, surfaces).location(loc!())?;
            let remote_surface = surfaces.get_mut(&surface_id).location(loc!())?;
            match &remote_surface.role {
                Some(Role::SubSurface(subsurface)) if subsurface.sync => {},
                Some(Role::XdgToplevel(toplevel)) if !toplevel.configured => {
                    toplevel.commit();
                },
                Some(Role::XdgPopup(popup)) if !popup.configured => {
                    popup.commit();
                },
                _ => remote_surface
                    .attach_damage_frame_commit(&self.qh)
                    .location(loc!())?,
            }
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_surface_destroy(
        &mut self,
        client_id: ClientId,
        surface_id: WlSurfaceId,
    ) -> Result<()> {
        let client = self.remote_display.client(&client_id);
        if let Some(surface) = client.surfaces.remove(&surface_id) {
            if let Ok(Role::SubSurface(subsurface)) = surface.get_role() {
                // The parent surface may have already been destroyed.
                if let Some(parent) = client.surfaces.get_mut(&subsurface.parent) {
                    parent
                        .z_ordered_children
                        .retain(|child| child.id != surface.id);
                }
            }
        };
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_surface(&mut self, request: SurfaceRequest) -> Result<()> {
        if (matches!(request.payload, SurfaceRequestPayload::Destroyed)
            && !self.remote_display.clients.contains_key(&request.client))
        {
            // Client already disconnected, nothing to do.
            return Ok(());
        }

        let surface_id = request.surface;
        match request.payload {
            SurfaceRequestPayload::Commit(surface_state) => {
                self.handle_commit(request.client, surface_id, surface_state)
                    .location(loc!())?;
            },
            SurfaceRequestPayload::Destroyed => {
                self.handle_surface_destroy(request.client, surface_id)
                    .location(loc!())?;
            },
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_toplevel(&mut self, request: ToplevelRequest) -> Result<()> {
        if (matches!(request.payload, ToplevelRequestPayload::Destroyed)
            && !self.remote_display.clients.contains_key(&request.client))
        {
            // Client already disconnected, nothing to do.
            return Ok(());
        }

        let client = self.remote_display.client(&request.client);
        // TODO: these properties aren't in the double-buffered state in
        // smithay, but still only take affect on commit. That seems wrong. In
        // the meantime though, we can get these before the initial commit.
        let Ok(surface) = client.surface(&request.surface) else {
            warn!("received request for unknown surface");
            return Ok(());
        };
        match request.payload {
            ToplevelRequestPayload::Destroyed => {
                surface.role = None;
            },
            ToplevelRequestPayload::SetMaximized => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .set_maximized();
            },
            ToplevelRequestPayload::UnsetMaximized => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .unset_maximized();
            },
            ToplevelRequestPayload::SetFullscreen => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .set_fullscreen(None);
            },
            ToplevelRequestPayload::UnsetFullscreen => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .unset_fullscreen();
            },
            ToplevelRequestPayload::SetMinimized => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .set_minimized();
            },
            ToplevelRequestPayload::Move(xdg_shell::Move { serial }) => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .xdg_toplevel()
                    ._move(&self.seat_state.seats().next().location(loc!())?, serial);
            },
            ToplevelRequestPayload::Resize(xdg_shell::Resize { serial, edge }) => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .xdg_toplevel()
                    .resize(
                        &self.seat_state.seats().next().location(loc!())?,
                        serial,
                        // The error type is (). :(
                        edge.try_into()
                            .map_err(|_| anyhow!("invalid edge"))
                            .location(loc!())?,
                    );
            },
            ToplevelRequestPayload::Decoration(mode) => {
                surface
                    .xdg_toplevel()
                    .location(loc!())?
                    .local_window
                    .request_decoration_mode(mode.map(Into::into));
            },
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_popup(&mut self, request: PopupRequest) -> Result<()> {
        if (matches!(request.payload, PopupRequestPayload::Destroyed)
            && !self.remote_display.clients.contains_key(&request.client))
        {
            // Client already disconnected, nothing to do.
            return Ok(());
        }

        let client = self.remote_display.client(&request.client);
        let surface = client.surface(&request.surface).location(loc!())?;
        match request.payload {
            PopupRequestPayload::Destroyed => {
                surface.role = None;
            },
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_cursor_image(&mut self, cursor_image: CursorImage) -> Result<()> {
        // TODO: support multiple seats
        let Some(themed_pointer) = self.seat_objects.last().location(loc!())?.pointer.as_ref()
        else {
            warn!("State has no pointer capability, ignoring cursor image.");
            return Ok(());
        };

        match cursor_image.status {
            CursorImageStatus::Named(name) => {
                themed_pointer
                    .set_cursor(
                        &self.conn,
                        name.parse()
                            .with_context(loc!(), || format!("Unknown cursor name {name:?}."))?,
                    )
                    .location(loc!())?;
            },
            CursorImageStatus::Surface {
                client_surface: ClientSurface { client, surface },
                hotspot,
            } => {
                let client = self
                    .remote_display
                    .clients
                    .get_mut(&client)
                    .location(loc!())?;
                let remote_surface = client
                    .surfaces
                    .entry(surface)
                    .or_insert_with_result(|| {
                        RemoteSurface::new(
                            client.id,
                            surface,
                            &self.compositor_state,
                            &self.qh,
                            &mut self.object_bimap,
                        )
                    })
                    .location(loc!())?;
                RemoteCursor::set_role(client.id, remote_surface);
                themed_pointer.pointer().set_cursor(
                    self.last_enter_serial,
                    Some(remote_surface.wl_surface()),
                    hotspot.x,
                    hotspot.y,
                );
            },
            CursorImageStatus::Hidden => {
                themed_pointer.hide_cursor().location(loc!())?;
            },
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_data(&mut self, data: DataRequest) -> Result<()> {
        match data {
            DataRequest::SourceRequest(DataSourceRequest::StartDrag(mut source_metadata, icon)) => {
                let icon_surface = match icon {
                    None => None,
                    Some(Tuple2(client, surface)) => {
                        let client = self.remote_display.client(&client);
                        let remote_surface = client
                            .surfaces
                            .entry(surface)
                            .or_insert_with_result(|| {
                                RemoteSurface::new(
                                    client.id,
                                    surface,
                                    &self.compositor_state,
                                    &self.qh,
                                    &mut self.object_bimap,
                                )
                            })
                            .location(loc!())?;
                        Some(remote_surface.wl_surface())
                    },
                };

                // TODO: support multiple seats
                if let (Some(seat_obj), Some(serial)) = (
                    self.seat_objects.iter().last(),
                    self.last_mouse_down_serial.take(),
                ) {
                    source_metadata.mime_types.push("_wprs_marker".to_owned());
                    let source = self.data_device_manager_state.create_drag_and_drop_source(
                        &self.qh,
                        source_metadata.mime_types.iter().map(String::as_str),
                        // The error type is (). :(
                        source_metadata
                            .dnd_actions
                            .try_into()
                            .map_err(|_| anyhow!("invalid dnd actions"))
                            .location(loc!())?,
                    );
                    source.start_drag(
                        &seat_obj.data_device,
                        self.current_focus.as_ref().location(loc!())?,
                        icon_surface,
                        serial,
                    );
                    self.dnd_source = Some(source);
                }
            },
            DataRequest::SourceRequest(DataSourceRequest::SetSelection(
                source,
                mut source_metadata,
            )) => {
                match source {
                    DataSource::Selection => {
                        // TODO: support multiple seats
                        if let (Some(seat_obj), Some(serial)) = (
                            self.seat_objects.iter().last(),
                            self.last_implicit_grab_serial.take(),
                        ) {
                            source_metadata.mime_types.push("_wprs_marker".to_string());
                            let mime_types = source_metadata.mime_types.iter().map(String::as_str);
                            let source = self
                                .data_device_manager_state
                                .create_copy_paste_source(&self.qh, mime_types);
                            source.set_selection(&seat_obj.data_device, serial);
                            self.selection_source = Some(source);
                        }
                    },
                    DataSource::Primary => {
                        if let (Some(seat_obj), Some(serial)) = (
                            self.seat_objects.iter().last(),
                            self.last_mouse_down_serial.take(),
                        ) {
                            if let (
                                Some(primary_selection_manager_state),
                                Some(primary_selection_device),
                            ) = (
                                &self.primary_selection_manager_state,
                                &seat_obj.primary_selection_device,
                            ) {
                                source_metadata.mime_types.push("_wprs_marker".to_string());
                                let mime_types =
                                    source_metadata.mime_types.iter().map(String::as_str);
                                let source = primary_selection_manager_state
                                    .create_selection_source(&self.qh, mime_types);
                                source.set_selection(primary_selection_device, serial);
                                self.primary_selection_source = Some(source);
                            }
                        }
                    },
                    DataSource::DnD => {},
                }
            },
            DataRequest::DestinationRequest(DataDestinationRequest::DnDAcceptMimeType(
                mime_type,
            )) => {
                if let Some(dnd_offer) = &self.dnd_offer {
                    dnd_offer.accept_mime_type(self.dnd_accept_counter, mime_type);
                    self.dnd_accept_counter += 1;
                }
            },
            DataRequest::DestinationRequest(DataDestinationRequest::RequestDataTransfer(
                source,
                mime_type,
            )) => {
                let read_pipe = match source {
                    DataSource::Primary => {
                        let cur_offer = self
                            .primary_selection_offer
                            .clone()
                            .ok_or(anyhow!("primary_selection_offer was empty"))?;

                        cur_offer.receive(mime_type.clone()).ok()
                    },
                    DataSource::Selection => {
                        let cur_offer = self
                            .selection_offer
                            .clone()
                            .ok_or(anyhow!("selection_offer was empty"))?;

                        cur_offer.receive(mime_type.clone()).ok()
                    },
                    DataSource::DnD => {
                        let cur_offer = self
                            .dnd_offer
                            .clone()
                            .ok_or(anyhow!("dnd_offer was empty"))?;

                        cur_offer.receive(mime_type.clone()).ok()
                    },
                };
                if let Some(mut read_pipe) = read_pipe {
                    debug!("spawning receive thread for mime {mime_type}");
                    let writer = self.serializer.writer().clone().into_inner();
                    // The data source application will write to the other end
                    // of read_pipe at its convenience and then close the file
                    // descriptor, so spawn off a thread to perform that read
                    // and send the data to the server whenever the read is
                    // completed. The thread will then terminate.
                    thread::spawn(move || -> Result<()> {
                        debug!("in receive thread for mime {mime_type}");
                        let mut buf = Vec::new();
                        let bytes_read = read_pipe.read_to_end(&mut buf).location(loc!())?;
                        debug!("read selection ({bytes_read:?} bytes): {buf:?}");
                        writer.send(SendType::Object(Event::Data(DataEvent::TransferData(
                            source,
                            DataToTransfer(buf),
                        ))))
                            // This should be infallible, writer is an
                            // InfallibleWriter, but we can't prove that to the
                            // compiler for thread lifetime reasons.
                            .unwrap();
                        Ok(())
                    });
                }
            },
            DataRequest::DestinationRequest(DataDestinationRequest::DnDSetDestinationActions(
                action,
            )) => {
                if let Some(dnd_offer) = &self.dnd_offer {
                    let action = action
                        .try_into()
                        .map_err(|_| anyhow!("invalid dnd action"))
                        .location(loc!())?;
                    dnd_offer.set_actions(action, action);
                }
            },
            DataRequest::DestinationRequest(DataDestinationRequest::DnDFinish) => {
                if let Some(dnd_offer) = &self.dnd_offer {
                    dnd_offer.finish();
                }
            },
            DataRequest::TransferData(source, data) => {
                let write_pipe = match source {
                    DataSource::Primary => self.primary_selection_pipe.take().location(loc!())?,
                    DataSource::Selection => self.selection_pipe.take().location(loc!())?, // TODO
                    DataSource::DnD => self.dnd_pipe.take().location(loc!())?,             // TODO
                };
                let fd = OwnedFd::from(write_pipe);
                let mut f = File::from(fd);
                // If data is large, the write may block if the reader (the
                // application requesting the data) isn't reading it quickly
                // enough, so do the write in a separate thread to avoid
                // blocking the event loop. The thread will then terminate.
                thread::spawn(move || {
                    f.write_all(&data.0).log_and_ignore(loc!());
                });
            },
        }
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_client_disconnected(&mut self, client: ClientId) -> Result<()> {
        self.remote_display.clients.remove(&client);
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn handle_capabilities(&mut self, caps: Capabilities) -> Result<()> {
        self.capabilities
            .set(caps)
            .map_err(|_| anyhow!("attempted to set capabilities more than once"))
            .location(loc!())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_buffer(&mut self, buffer: Vec<u8>) -> Result<()> {
        self.buffer_cache = Some(Arc::new(buffer.into()));
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub fn handle_request(&mut self, request: RecvType<Request>) {
        match request {
            RecvType::Object(Request::Surface(surface)) => self.handle_surface(surface),
            RecvType::Object(Request::Toplevel(toplevel)) => self.handle_toplevel(toplevel),
            RecvType::Object(Request::Popup(popup)) => self.handle_popup(popup),
            RecvType::Object(Request::CursorImage(cursor_image)) => {
                self.handle_cursor_image(cursor_image)
            },
            RecvType::Object(Request::Data(data)) => self.handle_data(data),
            RecvType::Object(Request::ClientDisconnected(client)) => {
                self.handle_client_disconnected(client)
            },
            RecvType::Object(Request::Capabilities(caps)) => self.handle_capabilities(caps),
            RecvType::RawBuffer(buffer) => self.handle_buffer(buffer),
        }
        .log_and_ignore(loc!())
        // TODO: maybe send errors back to the server.
    }
}
