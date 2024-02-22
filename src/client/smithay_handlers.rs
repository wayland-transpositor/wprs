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

/// Handlers for events from smithay client toolkit.
use smithay::reexports::wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1;
use smithay::reexports::wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1;
use smithay_client_toolkit::compositor::CompositorHandler;
use smithay_client_toolkit::data_device_manager::data_device::DataDeviceHandler;
use smithay_client_toolkit::data_device_manager::data_offer::DataOfferHandler;
use smithay_client_toolkit::data_device_manager::data_offer::DragOffer;
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::data_device_manager::WritePipe;
use smithay_client_toolkit::output::OutputHandler;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::primary_selection::device::PrimarySelectionDeviceHandler;
use smithay_client_toolkit::primary_selection::selection::PrimarySelectionSourceHandler;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device::WlDataDevice;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_source::WlDataSource;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_output::Transform;
use smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput;
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_subcompositor::Event as WlSubcompositorEvent;
use smithay_client_toolkit::reexports::client::protocol::wl_subcompositor::WlSubcompositor;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::Event as WlSubsurfaceEvent;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::Dispatch;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::registry::ProvidesRegistryState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::registry_handlers;
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::seat::keyboard::KeyboardHandler;
use smithay_client_toolkit::seat::keyboard::Keymap;
use smithay_client_toolkit::seat::keyboard::Keysym;
use smithay_client_toolkit::seat::keyboard::Modifiers;
use smithay_client_toolkit::seat::keyboard::RepeatInfo;
use smithay_client_toolkit::seat::pointer::PointerEvent;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::seat::pointer::PointerHandler;
use smithay_client_toolkit::seat::pointer::ThemeSpec;
use smithay_client_toolkit::seat::Capability;
use smithay_client_toolkit::seat::SeatHandler;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::popup;
use smithay_client_toolkit::shell::xdg::window::Window;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use smithay_client_toolkit::shell::xdg::window::WindowHandler;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::shm::ShmHandler;
use tracing::Span;

use crate::args;
use crate::client::subsurface;
use crate::client::ObjectBimapExt;
use crate::client::Role;
use crate::client::SeatObject;
use crate::client::WprsClientState;
use crate::prelude::*;
use crate::serialization::wayland;
use crate::serialization::wayland::DataDestinationEvent;
use crate::serialization::wayland::DataEvent;
use crate::serialization::wayland::DataSource;
use crate::serialization::wayland::DataSourceEvent;
use crate::serialization::wayland::DragEnter;
use crate::serialization::wayland::KeyInner;
use crate::serialization::wayland::KeyState;
use crate::serialization::wayland::KeyboardEvent;
use crate::serialization::wayland::SourceMetadata;
use crate::serialization::xdg_shell::PopupConfigure;
use crate::serialization::xdg_shell::PopupEvent;
use crate::serialization::xdg_shell::ToplevelConfigure;
use crate::serialization::xdg_shell::ToplevelEvent;
use crate::serialization::Event;
use crate::serialization::SendType;

impl CompositorHandler for WprsClientState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
        // TODO: implement this
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: Transform,
    ) {
        // TODO: implement this
    }

    #[instrument(skip(self, _conn, qh, _time), level = "debug")]
    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _time: u32,
    ) {
        let Some((client_id, surface_id)) = self.object_bimap.get_wl_surface_id(&surface.id())
        else {
            // TODO: unwrap is wrong, can enter before surface exists. Currently
            // we're just returning in that case, but should we create a surface
            // instead?
            return;
        };
        let client = self.remote_display.client(&client_id);

        subsurface::commit_sync_children(surface_id, &mut client.surfaces).log_and_ignore(loc!());

        let Ok(surface) = client.surface(&surface_id) else {
            return;
        };

        surface.frame_callback_completed = true;
        match &surface.role {
            Some(Role::SubSurface(subsurface)) if subsurface.sync => {},
            _ => surface.attach_damage_frame_commit(qh).unwrap(),
        }
    }
}

impl OutputHandler for WprsClientState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: WlOutput) {
        let output_info = self.output_state().info(&output).unwrap();
        debug!("NEW OUTPUT {:?}", &output_info);
        self.serializer
            .writer()
            .send(SendType::Object(Event::Output(output_info.into())));
    }

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {
        // TODO
    }

    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {
        // TODO
    }
}

impl WindowHandler for WprsClientState {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}

    #[instrument(skip_all, level = "debug")]
    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let (client_id, surface_id) = self
            .object_bimap
            .get_wl_surface_id(&window.wl_surface().id())
            .expect("Object corresponding to client object id {key} not found.");

        let client = self.remote_display.client(&client_id);
        let surface = client.surface(&surface_id).unwrap();
        let toplevel = surface
            .role
            .as_mut()
            .unwrap()
            .as_xdg_toplevel_mut()
            .unwrap();

        if !toplevel.configured {
            toplevel.configured = true;
            surface
                .attach_damage_frame_commit(qh)
                .log_and_ignore(loc!());
        }

        self.serializer
            .writer()
            .send(SendType::Object(Event::Toplevel(ToplevelEvent::Configure(
                ToplevelConfigure::from_smithay(&surface_id, configure),
            ))));
    }
}

impl popup::PopupHandler for WprsClientState {
    #[instrument(skip_all, level = "debug")]
    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        popup: &popup::Popup,
        configure: popup::PopupConfigure,
    ) {
        let (client_id, surface_id) = self
            .object_bimap
            .get_wl_surface_id(&popup.wl_surface().id())
            .expect("Object corresponding to client object id {key} not found.");

        let client = self.remote_display.client(&client_id);
        let surface = client.surface(&surface_id).unwrap();
        let remote_popup = surface.role.as_mut().unwrap().as_xdg_popup_mut().unwrap();
        if !remote_popup.configured {
            remote_popup.configured = true;
            surface
                .attach_damage_frame_commit(qh)
                .log_and_ignore(loc!());
        }

        self.serializer
            .writer()
            .send(SendType::Object(Event::Popup(PopupEvent::Configure(
                PopupConfigure::from_smithay(&surface_id, configure),
            ))));
    }

    fn done(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _popup: &popup::Popup) {
        // TODO?
    }
}

impl SeatHandler for WprsClientState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        let seat_obj = if let Some(seat_obj) = self.seat_objects.iter_mut().find(|s| s.seat == seat)
        {
            seat_obj
        } else {
            // create the data device here for this seat
            let data_device_manager = &self.data_device_manager_state;
            let data_device = data_device_manager.get_data_device(qh, &seat);

            let primary_selection_device = self.primary_selection_manager_state.as_ref().map(
                |primary_selection_manager_state| {
                    primary_selection_manager_state.get_selection_device(qh, &seat)
                },
            );

            self.seat_objects.push(SeatObject {
                seat: seat.clone(),
                keyboard: None,
                pointer: None,
                data_device,
                primary_selection_device,
            });
            self.seat_objects.last_mut().unwrap()
        };

        if capability == Capability::Keyboard && seat_obj.keyboard.is_none() {
            debug!("set keyboard capability");
            let keyboard = self
                .seat_state
                .get_keyboard(qh, &seat, None)
                .expect("Failed to create keyboard");
            seat_obj.keyboard.replace(keyboard);
        }

        if capability == Capability::Pointer && seat_obj.pointer.is_none() {
            debug!("set pointer capability");
            let themed_pointer = self
                .seat_state
                .get_pointer_with_theme(
                    qh,
                    &seat,
                    self.shm_state.wl_shm(),
                    self.compositor_state.create_surface(qh),
                    ThemeSpec::default(),
                )
                .expect("Failed to create pointer");
            seat_obj.pointer.replace(themed_pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if let Some(seat_obj) = self.seat_objects.iter_mut().find(|s| s.seat == seat) {
            match capability {
                Capability::Keyboard => {
                    seat_obj.keyboard.take().map(|k| k.release());
                },
                Capability::Pointer => {
                    seat_obj.pointer.take();
                },
                _ => {},
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for WprsClientState {
    #[instrument(skip(self, _conn, _qh, _keyboard), level = "debug")]
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        surface: &WlSurface,
        serial: u32,
        raw: &[u32],
        keysyms: &[Keysym],
    ) {
        self.current_focus = Some(surface.clone());
        let Some((_, surface_id)) = self.object_bimap.get_wl_surface_id(&surface.id()) else {
            // TODO: unwrap is wrong, we can enter before surface exists.
            // Currently we're just returning in that case, but should we create
            // a surface instead?
            return;
        };

        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(
                KeyboardEvent::Enter {
                    serial,
                    surface_id,
                    keycodes: raw.into(),
                    keysyms: keysyms.iter().map(|k| k.raw()).collect(),
                },
            )));
    }

    #[instrument(skip(self, _conn, _qh, _keyboard), level = "debug")]
    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        surface: &WlSurface,
        serial: u32,
    ) {
        self.current_focus = None;
        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(
                KeyboardEvent::Leave { serial },
            )));
    }

    // INTENTIONALLY NOT LOGGING KEY EVENTS
    #[instrument(
        skip(self, _conn, _qh, _keyboard, event),
        fields(event = "<redacted>"),
        level = "debug"
    )]
    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.last_implicit_grab_serial = Some(serial);
        if args::get_log_priv_data() {
            Span::current().record("event", field::debug(&event));
        }
        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(KeyboardEvent::Key(
                KeyInner {
                    serial,
                    raw_code: event.raw_code,
                    state: KeyState::Pressed,
                },
            ))));
    }

    // INTENTIONALLY NOT LOGGING KEY EVENTS
    #[instrument(
        skip(self, _conn, _qh, _keyboard, event),
        fields(event = "<redacted>"),
        level = "debug"
    )]
    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        if args::get_log_priv_data() {
            Span::current().record("event", field::debug(&event));
        }
        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(KeyboardEvent::Key(
                KeyInner {
                    serial,
                    raw_code: event.raw_code,
                    state: KeyState::Released,
                },
            ))));
    }

    #[instrument(skip(self, _conn, _qh, _keyboard), level = "debug")]
    fn update_repeat_info(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        info: RepeatInfo,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(
                KeyboardEvent::RepeatInfo(info.into()),
            )));
    }

    #[instrument(skip(self, _conn, _qh, _keyboard, keymap), level = "debug")]
    fn update_keymap(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        keymap: Keymap<'_>,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(
                KeyboardEvent::Keymap(keymap.as_string()),
            )));
    }

    #[instrument(skip(self, _conn, _qh, _keyboard, _serial), level = "debug")]
    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        variant: u32,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::KeyboardEvent(
                KeyboardEvent::Modifiers {
                    modifier_state: modifiers.into(),
                    layout_index: variant,
                },
            )));
    }
}

impl PointerHandler for WprsClientState {
    #[instrument(skip(self, _conn, _qh, _pointer), level = "debug")]
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events.iter() {
            if self
                .object_bimap
                .get_by_right(&event.surface.id())
                .is_none()
            {
                // window was distroyed already, TODO handle consistently
                return;
            };

            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.last_enter_serial = serial;
                },
                PointerEventKind::Press { serial, .. } => {
                    self.last_mouse_down_serial = Some(serial);
                },
                _ => {},
            }
        }

        self.serializer
            .writer()
            .send(SendType::Object(Event::PointerFrame(
                events
                    .iter()
                    .map(|event| {
                        let (_, surface_id) = self
                            .object_bimap
                            .get_wl_surface_id(&event.surface.id())
                            .expect("Object corresponding to client object id {key} not found.");

                        wayland::PointerEvent::from_smithay(&surface_id, event)
                    })
                    .collect(),
            )));
    }
}

impl ShmHandler for WprsClientState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl DataDeviceHandler for WprsClientState {
    #[instrument(skip_all, level = "debug")]
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        _x: f64,
        _y: f64,
        _wl_surface: &WlSurface,
    ) {
        let data_device = &self
            .seat_objects
            .iter()
            .find(|seat| seat.data_device.inner() == data_device)
            .unwrap()
            .data_device
            .data();
        let drag_offer = data_device.drag_offer().unwrap();
        debug!(
            "data offer entered x: {:.2} y: {:.2}",
            drag_offer.x, drag_offer.y
        );
        let mime_types = drag_offer.with_mime_types(<[String]>::to_vec);
        if mime_types.contains(&"_wprs_marker".to_string()) {
            return;
        }
        self.dnd_offer = Some(drag_offer.clone());
        let (_, surface_id) = self
            .object_bimap
            .get_wl_surface_id(&drag_offer.surface.id())
            .expect("Object corresponding to client object id {key} not found.");
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::DestinationEvent(
                DataDestinationEvent::DnDEnter(DragEnter {
                    serial: drag_offer.serial,
                    surface: surface_id,
                    loc: (drag_offer.x, drag_offer.y).into(),
                    source_actions: drag_offer.source_actions.into(),
                    selected_action: drag_offer.selected_action.into(),
                    mime_types,
                }),
            ))));
    }

    #[instrument(skip_all, level = "debug")]
    fn leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _data_device: &WlDataDevice) {
        debug!("data offer left");
    }

    #[instrument(skip_all, level = "debug")]
    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
        let data_device = &self
            .seat_objects
            .iter()
            .find(|seat| seat.data_device.inner() == data_device)
            .unwrap()
            .data_device
            .data();
        let Some(drag_offer) = data_device.drag_offer() else {
            return;
        };
        debug!(
            "data offer motion x: {:.2} y: {:.2}",
            drag_offer.x, drag_offer.y
        );
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::DestinationEvent(
                DataDestinationEvent::DnDMotion((drag_offer.x, drag_offer.y).into()),
            ))));
    }

    #[instrument(skip_all, level = "debug")]
    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
    ) {
        let data_device = &self
            .seat_objects
            .iter()
            .find(|seat| seat.data_device.inner() == data_device)
            .unwrap()
            .data_device
            .data();
        let Some(offer) = data_device.selection_offer() else {
            return;
        };
        let mime_types = offer.with_mime_types(<[String]>::to_vec);
        if mime_types.contains(&"_wprs_marker".to_string()) {
            return;
        }
        self.selection_offer = Some(offer);
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::DestinationEvent(
                DataDestinationEvent::SelectionSet(
                    DataSource::Selection,
                    SourceMetadata::from_mime_types(mime_types),
                ),
            ))));
    }

    #[instrument(skip_all, level = "debug")]
    fn drop_performed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::DestinationEvent(
                DataDestinationEvent::DnDDrop,
            ))));
    }
}

impl DataOfferHandler for WprsClientState {
    #[instrument(skip(self, _conn, _qh, _offer), level = "debug")]
    fn source_actions(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        actions: DndAction,
    ) {
        // TODO
    }

    #[instrument(skip(self, _conn, _qh, _offer), level = "debug")]
    fn selected_action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        actions: DndAction,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::DestinationEvent(
                DataDestinationEvent::DnDActionSelected(actions.into()),
            ))));
    }
}

impl DataSourceHandler for WprsClientState {
    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn accept_mime(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: Option<String>,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::SourceEvent(
                DataSourceEvent::DnDMimeTypeAcceptedByDestination(mime),
            ))));
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        write_pipe: WritePipe,
    ) {
        match (source, &self.selection_source, &self.dnd_source) {
            (source, Some(selection_source), _) if source == selection_source.inner() => {
                self.selection_pipe = Some(write_pipe);
                self.serializer.writer().send(SendType::Object(Event::Data(
                    DataEvent::SourceEvent(DataSourceEvent::MimeTypeSendRequestedByDestination(
                        DataSource::Selection,
                        mime,
                    )),
                )));
            },
            (source, _, Some(dnd_source)) if source == dnd_source.inner() => {
                self.dnd_pipe = Some(write_pipe);
                self.serializer.writer().send(SendType::Object(Event::Data(
                    DataEvent::SourceEvent(DataSourceEvent::MimeTypeSendRequestedByDestination(
                        DataSource::DnD,
                        mime,
                    )),
                )));
            },
            _ => {
                warn!("request for unknown source");
            },
        }
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn cancelled(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, source: &WlDataSource) {
        match (source, &self.selection_source, &self.dnd_source) {
            (source, Some(selection_source), _) if source == selection_source.inner() => {
                self.selection_pipe = None;
                // self.serializer.writer().send(SendType::Object(Event::Data(DataSourceEvent::SelectionCancelled));
            },
            (source, _, Some(dnd_source)) if source == dnd_source.inner() => {
                self.dnd_source = None;
                self.dnd_pipe = None;
                self.serializer.writer().send(SendType::Object(Event::Data(
                    DataEvent::SourceEvent(DataSourceEvent::DnDCancelled),
                )));
            },
            _ => {
                warn!("cancellation for unknown source");
            },
        }
        source.destroy();
    }

    #[instrument(skip_all, level = "debug")]
    fn dnd_dropped(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _source: &WlDataSource) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::SourceEvent(
                DataSourceEvent::DnDDropPerformed,
            ))));
    }

    #[instrument(skip_all, level = "debug")]
    fn dnd_finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
    ) {
        self.dnd_source = None;
        self.dnd_pipe = None;
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::SourceEvent(
                DataSourceEvent::DnDFinished,
            ))));
    }

    #[instrument(skip(self, _conn, _qh, _source), level = "debug")]
    fn action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
        action: DndAction,
    ) {
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::SourceEvent(
                DataSourceEvent::DnDActionSelected(action.into()),
            ))));
    }
}

impl PrimarySelectionDeviceHandler for WprsClientState {
    #[instrument(skip_all, level = "debug")]
    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        primary_selection_device: &ZwpPrimarySelectionDeviceV1,
    ) {
        if self.primary_selection_manager_state.is_none() {
            return;
        }
        let primary_selection_device = &self
            .seat_objects
            .iter()
            .filter_map(|seat| seat.primary_selection_device.as_ref())
            .find(|device| device.inner() == primary_selection_device)
            .unwrap()
            .data();
        let Some(offer) = primary_selection_device.selection_offer() else {
            return;
        };
        let mime_types = offer.with_mime_types(<[String]>::to_vec);
        if mime_types.contains(&"_wprs_marker".to_string()) {
            return;
        }
        self.primary_selection_offer = Some(offer);
        self.serializer
            .writer()
            .send(SendType::Object(Event::Data(DataEvent::DestinationEvent(
                DataDestinationEvent::SelectionSet(
                    DataSource::Primary,
                    SourceMetadata::from_mime_types(mime_types),
                ),
            ))));
    }
}

impl PrimarySelectionSourceHandler for WprsClientState {
    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        write_pipe: WritePipe,
    ) {
        match &self.primary_selection_source {
            Some(primary_selection_source) if source == primary_selection_source.inner() => {
                self.primary_selection_pipe = Some(write_pipe);
                self.serializer.writer().send(SendType::Object(Event::Data(
                    DataEvent::SourceEvent(DataSourceEvent::MimeTypeSendRequestedByDestination(
                        DataSource::Primary,
                        mime,
                    )),
                )));
            },
            _ => {
                warn!("request for unknown source");
            },
        };
    }

    #[instrument(skip(self, _conn, _qh, _source), level = "debug")]
    fn cancelled(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &ZwpPrimarySelectionSourceV1,
    ) {
        self.primary_selection_pipe = None;
    }
}

smithay_client_toolkit::delegate_compositor!(WprsClientState);
smithay_client_toolkit::delegate_data_device!(WprsClientState);
smithay_client_toolkit::delegate_keyboard!(WprsClientState);
smithay_client_toolkit::delegate_output!(WprsClientState);
smithay_client_toolkit::delegate_pointer!(WprsClientState);
smithay_client_toolkit::delegate_registry!(WprsClientState);
smithay_client_toolkit::delegate_seat!(WprsClientState);
smithay_client_toolkit::delegate_shm!(WprsClientState);
smithay_client_toolkit::delegate_subcompositor!(WprsClientState);
smithay_client_toolkit::delegate_xdg_popup!(WprsClientState);
smithay_client_toolkit::delegate_xdg_shell!(WprsClientState);
smithay_client_toolkit::delegate_xdg_window!(WprsClientState);
smithay_client_toolkit::delegate_primary_selection!(WprsClientState);

impl ProvidesRegistryState for WprsClientState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}

pub(crate) struct SubCompositorData;

impl Dispatch<WlSubcompositor, SubCompositorData> for WprsClientState {
    fn event(
        _state: &mut Self,
        _subcompositor: &WlSubcompositor,
        _event: WlSubcompositorEvent,
        _data: &SubCompositorData,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        dbg!("SUBCOMPOSITOR DISPATCH");
    }
}

pub(crate) struct SubSurfaceData;

impl Dispatch<WlSubsurface, SubSurfaceData> for WprsClientState {
    fn event(
        _state: &mut Self,
        _subsurface: &WlSubsurface,
        _event: WlSubsurfaceEvent,
        _data: &SubSurfaceData,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        dbg!("SUBSURFACE DISPATCH");
    }
}
