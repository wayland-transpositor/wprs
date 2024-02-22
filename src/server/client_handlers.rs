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

/// Handlers for events from the wprs client.
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::thread;

use nix::fcntl::OFlag;
use nix::unistd;
use smithay::backend::input::Axis;
use smithay::backend::input::ButtonState;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::FilterResult;
use smithay::input::keyboard::Layout;
use smithay::input::keyboard::XkbContext;
use smithay::input::pointer::AxisFrame;
use smithay::input::pointer::ButtonEvent;
use smithay::input::pointer::Focus;
use smithay::input::pointer::MotionEvent;
use smithay::output::Mode as OutputMode;
use smithay::output::Output;
use smithay::output::PhysicalProperties;
use smithay::output::Scale;
use smithay::utils::Rectangle;
use smithay::utils::Serial;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::selection::data_device;
use smithay::wayland::selection::data_device::SourceMetadata;
use smithay::wayland::selection::primary_selection;

use crate::args;
use crate::prelude::*;
use crate::serialization::wayland::DataDestinationEvent;
use crate::serialization::wayland::DataEvent;
use crate::serialization::wayland::DataRequest;
use crate::serialization::wayland::DataSource;
use crate::serialization::wayland::DataSourceEvent;
use crate::serialization::wayland::DataToTransfer;
use crate::serialization::wayland::KeyInner;
use crate::serialization::wayland::KeyboardEvent;
use crate::serialization::wayland::OutputInfo;
use crate::serialization::wayland::PointerEvent;
use crate::serialization::wayland::PointerEventKind;
use crate::serialization::wayland::RepeatInfo;
use crate::serialization::wayland::SurfaceRequest;
use crate::serialization::wayland::SurfaceRequestPayload;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::xdg_shell::PopupConfigure;
use crate::serialization::xdg_shell::PopupEvent;
use crate::serialization::xdg_shell::ToplevelConfigure;
use crate::serialization::xdg_shell::ToplevelEvent;
use crate::serialization::Capabilities;
use crate::serialization::Event;
use crate::serialization::RecvType;
use crate::serialization::Request;
use crate::serialization::SendType;
use crate::server::smithay_handlers::DndGrab;
use crate::server::LockedSurfaceState;
use crate::server::WprsServerState;

impl WprsServerState {
    #[instrument(skip_all, level = "debug")]
    fn handle_pointer_frame(&mut self, events: Vec<PointerEvent>) -> Result<()> {
        let pointer = self.seat.get_pointer().location(loc!())?;

        for event in events {
            let object_id = match self.object_map.get(&event.surface_id) {
                Some(object_id) => object_id.clone(),
                None => {
                    warn!(
                        "Ignoring pointer event for unknown surface {:?}",
                        &event.surface_id
                    );
                    return Ok(());
                },
            };
            let Ok(client) = self.dh.get_client(object_id.clone()) else {
                warn!("Ignoring keyboard event for unknown client of surface {object_id:?}.");
                return Ok(());
            };

            let Ok(surface) = client.object_from_protocol_id(&self.dh, object_id.protocol_id())
            else {
                warn!("Ignoring keyboard event for unknown client of surface {object_id:?}.");
                return Ok(());
            };

            let time = self.start_time.elapsed().as_millis() as u32;

            match event.kind {
                PointerEventKind::Enter { serial } => {
                    debug!("pointer entered at {:?}", event.position);
                    let serial = self.serial_map.insert(serial);
                    pointer.motion(
                        self,
                        Some((surface, (0, 0).into())),
                        &MotionEvent {
                            location: event.position.into(),
                            serial,
                            time,
                        },
                    );
                },
                PointerEventKind::Leave { serial } => {
                    debug!("pointer left");

                    // During drag/drop, we will stop receiving pointer events (i.e. we will get a pointer leave)
                    // and get data_device events instead. To prevent dropping early, we shouldn't release
                    // these keys yet.
                    if self.dnd_source.is_none() {
                        let pressed_buttons: HashSet<u32> = self.pressed_buttons.drain().collect();
                        for button in pressed_buttons {
                            debug!("releasing button {}", button);
                            pointer.button(
                                self,
                                &ButtonEvent {
                                    time,
                                    button,
                                    serial: SERIAL_COUNTER.next_serial(),
                                    state: ButtonState::Released,
                                },
                            );
                        }
                    }

                    let serial = self.serial_map.insert(serial);
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: event.position.into(),
                            serial,
                            time,
                        },
                    );
                },
                PointerEventKind::Motion => {
                    debug!("pointer moved to {:?}", event.position);
                    pointer.motion(
                        self,
                        Some((surface, (0, 0).into())),
                        &MotionEvent {
                            location: event.position.into(),
                            serial: 0.into(), // unused
                            time,
                        },
                    );
                },
                PointerEventKind::Press { serial, button } => {
                    debug!("button {:x} pressed at {:?}", button, event.position);
                    let serial = self.serial_map.insert(serial);
                    pointer.button(
                        self,
                        &ButtonEvent {
                            time,
                            button,
                            serial,
                            state: ButtonState::Pressed,
                        },
                    );
                    self.pressed_buttons.insert(button);
                },
                PointerEventKind::Release { serial, button } => {
                    debug!("button {:x} released at {:?}", button, event.position);
                    let serial = self.serial_map.insert(serial);
                    pointer.button(
                        self,
                        &ButtonEvent {
                            time,
                            button,
                            serial,
                            state: ButtonState::Released,
                        },
                    );
                    self.pressed_buttons.remove(&button);
                },
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    source,
                } => {
                    debug!("axis event: horizontal {horizontal:?}, vertical {vertical:?}, source {source:?}");
                    let axis_frame = AxisFrame::new(time)
                        .source(source.into())
                        .value(Axis::Horizontal, horizontal.absolute)
                        .value(Axis::Vertical, vertical.absolute)
                        .v120(Axis::Horizontal, horizontal.discrete)
                        .v120(Axis::Vertical, vertical.discrete);
                    if horizontal.stop {
                        axis_frame.stop(Axis::Horizontal);
                    }
                    if vertical.stop {
                        axis_frame.stop(Axis::Vertical);
                    }
                    pointer.axis(self, axis_frame);
                },
            }
        }
        pointer.frame(self);

        Ok(())
    }

    #[instrument(
        skip(self, keycode, state),
        fields(keycode = "<redacted>", state = "<redacted>"),
        level = "debug"
    )]
    fn set_key_state(&mut self, keycode: u32, state: KeyState, serial: Serial) -> Result<()> {
        let keyboard = self.seat.get_keyboard().location(loc!())?;

        if args::get_log_priv_data() {
            debug!("sending key input: code {keycode:?}, state {state:?}");
        }

        keyboard.input::<(), _>(
            self,
            keycode,
            state,
            serial,
            self.start_time.elapsed().as_millis() as u32,
            |_, &modifiers_state, keysym| {
                if args::get_log_priv_data() {
                    debug!("modifiers_state {modifiers_state:?}, keysym {keysym:?}");
                }
                FilterResult::Forward
            },
        );
        match state {
            KeyState::Pressed => {
                self.pressed_keys.insert(keycode);
            },
            KeyState::Released => {
                self.pressed_keys.remove(&keycode);
            },
        }

        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_keyboard_event(&mut self, event: KeyboardEvent) -> Result<()> {
        let keyboard = self.seat.get_keyboard().location(loc!())?;
        match event {
            KeyboardEvent::Enter {
                serial,
                surface_id,
                keycodes,
                keysyms: _,
            } => {
                // see linux/input-event-codes.h for keycodes
                let modifier_keycodes = HashSet::from([
                    /* KEY_LEFTCTRL */ 29, /* KEY_RIGHTCTRL */ 97,
                    /* KEY_LEFTALT */ 56, /* KEY_RIGHTALT */ 100,
                    /* KEY_LEFTMETA	*/ 125, /* KEY_RIGHTMETA */ 126,
                    /* KEY_LEFTSHIFT */ 42, /* KEY_RIGHTSHIFT */ 54,
                ]);

                // We simulate keycodes before focusing since that is what a normal wayland application would see.
                // Process modifier keys first so that they apply to other held keys.
                let mut delayed_keycodes = Vec::new();
                for keycode in keycodes {
                    if modifier_keycodes.contains(&keycode) {
                        self.set_key_state(
                            keycode,
                            KeyState::Pressed,
                            SERIAL_COUNTER.next_serial(),
                        )
                        .location(loc!())?;
                    } else {
                        delayed_keycodes.push(keycode);
                    }
                }
                for keycode in delayed_keycodes {
                    self.set_key_state(keycode, KeyState::Pressed, SERIAL_COUNTER.next_serial())
                        .location(loc!())?;
                }

                let serial = self.serial_map.insert(serial);
                let object_id = match self.object_map.get(&surface_id) {
                    Some(object_id) => object_id.clone(),
                    None => {
                        warn!("Ignoring keyboard event for unknown surface {surface_id:?}");
                        return Ok(());
                    },
                };
                let Ok(client) = self.dh.get_client(object_id.clone()) else {
                    warn!("Ignoring keyboard event for unknown client of surface {object_id:?}.");
                    return Ok(());
                };

                if let Ok(surface) =
                    client.object_from_protocol_id(&self.dh, object_id.protocol_id())
                {
                    debug!("setting keyboard focus to surface {surface:?}");
                    keyboard.set_focus(self, Some(surface), serial);
                    data_device::set_data_device_focus(&self.dh, &self.seat, Some(client.clone()));
                    primary_selection::set_primary_focus(&self.dh, &self.seat, Some(client));
                } else {
                    warn!("Received keyboard enter event for (probably) already-destroyed surface {surface_id:?}.");
                }
            },
            KeyboardEvent::Leave { serial } => {
                let serial = self.serial_map.insert(serial);
                keyboard.set_focus(self, None, serial);
                data_device::set_data_device_focus(&self.dh, &self.seat, None);
                primary_selection::set_primary_focus(&self.dh, &self.seat, None);

                for keycode in self.pressed_keys.clone() {
                    self.set_key_state(keycode, KeyState::Released, SERIAL_COUNTER.next_serial())
                        .location(loc!())?;
                }
            },
            KeyboardEvent::Key(KeyInner {
                serial,
                raw_code,
                state: istate,
            }) => {
                let serial = self.serial_map.insert(serial);

                self.set_key_state(raw_code, istate.into(), serial)
                    .location(loc!())?;
            },
            KeyboardEvent::RepeatInfo(info) => match info {
                RepeatInfo::Repeat { rate, delay } => {
                    keyboard.change_repeat_info(
                        i32::try_from(u32::from(rate)).location(loc!())?,
                        i32::try_from(delay).location(loc!())?,
                    );
                },
                RepeatInfo::Disable => {},
            },
            KeyboardEvent::Keymap(keymap) => keyboard
                .set_keymap_from_string(self, keymap)
                .location(loc!())?,
            KeyboardEvent::Modifiers {
                modifier_state,
                layout_index,
            } => {
                keyboard.with_xkb_state(self, |mut context: XkbContext| {
                    context.set_layout(Layout(layout_index));
                });

                // see linux/input-event-codes.h for keycodes
                let mod_state = keyboard.modifier_state();
                for (new_modifier, current_modifier, keycode) in [
                    (
                        modifier_state.caps_lock,
                        mod_state.caps_lock,
                        /* KEY_CAPSLOCK */ 58,
                    ),
                    (
                        modifier_state.num_lock,
                        mod_state.num_lock,
                        /* KEY_NUMLOCK */ 69,
                    ),
                ] {
                    if new_modifier != current_modifier {
                        self.set_key_state(
                            keycode,
                            KeyState::Pressed,
                            SERIAL_COUNTER.next_serial(),
                        )
                        .location(loc!())?;
                        self.set_key_state(
                            keycode,
                            KeyState::Released,
                            SERIAL_COUNTER.next_serial(),
                        )
                        .location(loc!())?;
                    }
                }
            },
        }

        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_toplevel_configure(&self, configure: &ToplevelConfigure) -> Result<()> {
        let surfaces = self.xdg_shell_state.toplevel_surfaces();
        // TODO: we can replace this with a hashmap lookup now
        surfaces
            .iter()
            .find(|surface| {
                let surface_id = WlSurfaceId::new(surface.wl_surface());
                debug!(
                    "inspecting surface {surface_id:?}, looking for surface {:?}",
                    configure.surface_id
                );
                surface_id == configure.surface_id
            })
            .map(|surface| {
                let surface_id = WlSurfaceId::new(surface.wl_surface());
                debug!("matched surface {surface_id:?}");
                let size = Some(
                    (
                        configure.new_size.w.map_or(0i32, |w| u32::from(w) as i32),
                        configure.new_size.h.map_or(0i32, |h| u32::from(h) as i32),
                    )
                        .into(),
                );
                surface.with_pending_state(|ref mut state| {
                    state.size = size;
                    state.states = configure.state.into();
                    // TODO: probably set this, see also other TODO related to
                    // fullscreen output.
                    state.fullscreen_output = None;
                    state.decoration_mode = Some(configure.decoration_mode.into());
                });
                surface.send_configure();
                debug!("sent configure to surface {surface:?}");
            });

        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_toplevel(&mut self, toplevel: ToplevelEvent) -> Result<()> {
        match &toplevel {
            ToplevelEvent::Configure(configure) => {
                self.handle_toplevel_configure(configure).location(loc!())?;
            },
        }
        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_popup_configure(&self, configure: &PopupConfigure) -> Result<()> {
        let surfaces = self.xdg_shell_state.popup_surfaces();
        surfaces
            .iter()
            .find(|surface| {
                let surface_id = WlSurfaceId::new(surface.wl_surface());
                debug!(
                    "inspecting surface {surface_id:?}, looking for surface {:?}",
                    configure.surface_id
                );
                surface_id == configure.surface_id
            })
            .map(|surface| {
                let surface_id = WlSurfaceId::new(surface.wl_surface());
                debug!("matched surface {surface_id:?}");
                surface.with_pending_state(|ref mut state| {
                    state.geometry = Rectangle::from_loc_and_size(
                        configure.position,
                        (configure.width, configure.height),
                    );
                });
                surface.send_configure().log_and_ignore(loc!());
            })
            .location(loc!())?;

        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_popup(&mut self, popup: PopupEvent) -> Result<()> {
        match &popup {
            PopupEvent::Configure(configure) => {
                self.handle_popup_configure(configure).location(loc!())?;
            },
        }
        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_output(&mut self, output: OutputInfo) -> Result<()> {
        let (local_output, _) = self.outputs.entry(output.id).or_insert_with_key(|id| {
            let new_output = Output::new(
                format!("{}_{}", id, output.name.unwrap_or("None".to_string())),
                PhysicalProperties {
                    size: output.physical_size.into(),
                    subpixel: output.subpixel.into(),
                    make: output.make,
                    model: output.model,
                },
            );
            let global_id = new_output.create_global::<Self>(&self.dh);
            (new_output, global_id)
        });

        let current_mode = local_output.current_mode().unwrap_or(OutputMode {
            size: (0, 0).into(),
            refresh: 0,
        });
        let received_mode = OutputMode {
            size: output.mode.dimensions.into(),
            refresh: output.mode.refresh_rate,
        };
        if current_mode != received_mode {
            local_output.delete_mode(current_mode);
        }

        local_output.change_current_state(
            Some(received_mode),
            Some(output.transform.into()),
            Some(Scale::Integer(output.scale_factor)),
            Some(output.location.into()),
        );

        if output.mode.preferred {
            local_output.set_preferred(received_mode);
        }

        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn handle_connect(&mut self) -> Result<()> {
        // TODO: sync client outputs
        self.serializer.set_other_end_connected(true);

        self.serializer
            .writer()
            .send(SendType::Object(Request::Capabilities(Capabilities {
                xwayland: self.xwayland_enabled,
            })));

        self.for_each_surface(|_, surface_data| {
            let surface_state = surface_data
                .data_map
                .get::<LockedSurfaceState>()
                .unwrap()
                .0
                .lock()
                .unwrap()
                .clone();

            self.serializer
                .writer()
                .send(SendType::Object(Request::Surface(SurfaceRequest {
                    client: surface_state.client,
                    surface: surface_state.id,
                    payload: SurfaceRequestPayload::Commit(surface_state),
                })));
        });

        Ok(())
    }

    #[allow(clippy::verbose_file_reads)]
    #[instrument(skip_all, level = "debug")]
    fn handle_data_event(&mut self, data_event: DataEvent) -> Result<()> {
        match data_event {
            DataEvent::SourceEvent(DataSourceEvent::DnDMimeTypeAcceptedByDestination(
                mime_type,
            )) => {
                if let Some(source) = &self.dnd_source {
                    source.target(mime_type);
                }
            },
            DataEvent::SourceEvent(DataSourceEvent::MimeTypeSendRequestedByDestination(
                source,
                mime,
            )) => {
                let (recv_fd, send_fd) = unistd::pipe2(OFlag::O_CLOEXEC).location(loc!())?; // TODO: handle error

                // SAFETY: we just opened the fds and they don't require any special
                // cleanup.
                let (recv_fd, send_fd) =
                    unsafe { (OwnedFd::from_raw_fd(recv_fd), OwnedFd::from_raw_fd(send_fd)) };
                let mut f = File::from(recv_fd);

                {
                    let writer = self.serializer.writer().into_inner();
                    // The data source application will write to the other end
                    // of read_pipe at its convenience and then close the file
                    // descriptor, so spawn off a thread to perform that read
                    // and send the data to the client whenever the read is
                    // completed. The thread will then terminate
                    thread::spawn(move || {
                        debug!("in receive read thread");
                        let mut buf = Vec::new();
                        let bytes_read = f.read_to_end(&mut buf);
                        debug!("read selection ({bytes_read:?} bytes): {buf:?}");
                        writer.send(SendType::Object(Request::Data(DataRequest::TransferData(
                            source,
                            DataToTransfer(buf),
                        ))))
                            // This should be infallible, writer is an
                            // InfallibleWriter, but we can't prove that to the
                            // compiler for thread lifetime reasons.
                            .unwrap();
                    });
                }

                match source {
                    DataSource::Selection => {
                        data_device::request_data_device_client_selection(
                            &self.seat, mime, send_fd,
                        )
                        .location(loc!())?;
                    },
                    DataSource::Primary => {
                        primary_selection::request_primary_client_selection(
                            &self.seat, mime, send_fd,
                        )
                        .location(loc!())?;
                    },
                    DataSource::DnD => {
                        // TODO: unwrap is wrong, need to check for none at the top
                        self.dnd_source
                            .as_ref()
                            .location(loc!())?
                            .send(mime, send_fd.as_fd());
                    },
                }
            },
            DataEvent::SourceEvent(DataSourceEvent::DnDActionSelected(action)) => {
                if let Some(source) = &self.dnd_source {
                    source.action(
                        action.try_into()
                                  // The error type is (). :(
                                  .map_err(|_| anyhow!("invalid dnd source action"))
                                  .location(loc!())?,
                    );
                }
            },
            DataEvent::SourceEvent(DataSourceEvent::DnDDropPerformed) => {
                if let Some(source) = &self.dnd_source {
                    source.dnd_drop_performed();
                }
            },
            DataEvent::SourceEvent(
                DataSourceEvent::DnDFinished | DataSourceEvent::DnDCancelled,
            ) => {
                if let Some(source) = self.dnd_source.take() {
                    if data_event == DataEvent::SourceEvent(DataSourceEvent::DnDFinished) {
                        source.dnd_finished();
                    }

                    let time = self.start_time.elapsed().as_millis() as u32;
                    let pointer = self.seat.get_pointer().location(loc!())?;

                    // unfocus window so we don't re-enter it while releasing buttons
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: (0.0, 0.0).into(),
                            serial: 0.into(), // unused
                            time,
                        },
                    );
                    let pressed_buttons: HashSet<u32> = self.pressed_buttons.drain().collect();
                    for button in pressed_buttons {
                        debug!("releasing button {}", button);
                        pointer.button(
                            self,
                            &ButtonEvent {
                                time,
                                button,
                                serial: SERIAL_COUNTER.next_serial(),
                                state: ButtonState::Released,
                            },
                        );
                    }
                }
            },
            // TODO: remove? after taking another pass at data device code.
            // DestinationEvent(DnDActionsOfferedBySource(_)) => {
            //     // handled by start_dnd
            // },
            DataEvent::DestinationEvent(DataDestinationEvent::DnDActionSelected(_action)) => {
                // TODO: remove? after taking another pass at data device code.
            },
            DataEvent::DestinationEvent(DataDestinationEvent::DnDEnter(drag_enter)) => {
                let Some(object_id) = self.object_map.get(&drag_enter.surface) else {
                    warn!(
                        "Ignoring DnDEnter for unknown surface {:?}.",
                        &drag_enter.surface
                    );
                    return Ok(());
                };
                let Ok(client) = self.dh.get_client(object_id.clone()) else {
                    warn!("Ignoring DnDEnter for unknown client of surface {object_id:?}.");
                    return Ok(());
                };
                let serial = self.serial_map.insert(drag_enter.serial);
                let pointer = self.seat.get_pointer().location(loc!())?;
                let Ok(surface) = client.object_from_protocol_id(&self.dh, object_id.protocol_id())
                else {
                    warn!("Ignoring DnDEnter for unknown client of surface {object_id:?}.");
                    return Ok(());
                };
                let grab = DndGrab::new(Some((surface, (0, 0).into())), 0, drag_enter.loc.into());
                pointer.set_grab(self, grab, serial, Focus::Keep);
                let drag_start_data = pointer.grab_start_data();
                debug!("DRAG GRAB: pointer.grab_start_data {:?}", drag_start_data);

                data_device::start_dnd(
                    &self.dh.clone(),
                    &self.seat.clone(),
                    self,
                    serial,
                    drag_start_data,
                    None,
                    SourceMetadata {
                        mime_types: drag_enter.mime_types,
                        dnd_action: drag_enter
                            .source_actions
                            .try_into()
                            // The error type is (). :(
                            .map_err(|_| anyhow!("invalid dnd source actions"))
                            .location(loc!())?,
                    },
                );
            },
            DataEvent::DestinationEvent(DataDestinationEvent::DnDLeave) => {
                let pointer = self.seat.get_pointer().location(loc!())?;
                debug!("drag leave");
                if let Some(_grab_start_data) = pointer.grab_start_data() {
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: (0.0, 0.0).into(),
                            serial: 0.into(), // unused
                            time: self.start_time.elapsed().as_millis() as u32,
                        },
                    );
                }
            },
            DataEvent::DestinationEvent(DataDestinationEvent::DnDMotion(drag_motion)) => {
                let pointer = self.seat.get_pointer().location(loc!())?;
                debug!("drag moved to {:?}", drag_motion);
                if let Some(grab_start_data) = pointer.grab_start_data() {
                    pointer.motion(
                        self,
                        Some((grab_start_data.focus.location(loc!())?.0, (0, 0).into())),
                        &MotionEvent {
                            location: drag_motion.into(),
                            serial: 0.into(), // unused
                            time: self.start_time.elapsed().as_millis() as u32,
                        },
                    );
                }
            },
            DataEvent::DestinationEvent(DataDestinationEvent::DnDDrop) => {
                let pointer = self.seat.get_pointer().location(loc!())?;
                debug!("drag dropped");
                let serial = SERIAL_COUNTER.next_serial();
                let time = self.start_time.elapsed().as_millis() as u32;
                pointer.unset_grab(self, serial, time);
                pointer.button(
                    self,
                    &ButtonEvent {
                        time,
                        button: 0,
                        serial,
                        state: ButtonState::Released,
                    },
                );
            },
            DataEvent::DestinationEvent(DataDestinationEvent::SelectionSet(source, metadata)) => {
                match source {
                    DataSource::Selection => data_device::set_data_device_selection(
                        &self.dh,
                        &self.seat,
                        metadata.mime_types,
                        (),
                    ),
                    DataSource::Primary => primary_selection::set_primary_selection(
                        &self.dh,
                        &self.seat,
                        metadata.mime_types,
                        (),
                    ),
                    DataSource::DnD => {},
                };
            },
            DataEvent::TransferData(source, data) => {
                let fd = match source {
                    DataSource::Selection => self.selection_pipe.take().location(loc!())?,
                    DataSource::Primary => self.primary_selection_pipe.take().location(loc!())?,
                    DataSource::DnD => self.dnd_pipe.take().location(loc!())?,
                };
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
    pub fn handle_event(&mut self, event: RecvType<Event>) {
        match event {
            RecvType::Object(Event::WprsClientConnect) => self.handle_connect(),
            RecvType::Object(Event::Toplevel(toplevel)) => self.handle_toplevel(toplevel),
            RecvType::Object(Event::Popup(popup)) => self.handle_popup(popup),
            RecvType::Object(Event::KeyboardEvent(event)) => self.handle_keyboard_event(event),
            RecvType::Object(Event::PointerFrame(events)) => self.handle_pointer_frame(events),
            RecvType::Object(Event::Output(output)) => self.handle_output(output),
            RecvType::Object(Event::Data(data_event)) => self.handle_data_event(data_event),
            RecvType::RawBuffer(_) => unreachable!(),
        }
        .log_and_ignore(loc!());
        // TODO: maybe send errors back to the client.
    }
}
