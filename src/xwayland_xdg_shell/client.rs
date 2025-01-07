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

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::sync::Arc;

use enum_as_inner::EnumAsInner;
use smithay::backend::input::Axis;
use smithay::backend::input::AxisSource;
use smithay::backend::input::ButtonState;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Layout;
use smithay::input::keyboard::XkbContext;
use smithay::input::pointer::AxisFrame;
use smithay::input::pointer::ButtonEvent;
use smithay::input::pointer::MotionEvent;
use smithay::input::pointer::PointerTarget;
use smithay::reexports::wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1;
use smithay::reexports::wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Rectangle;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::compositor;
use smithay::wayland::compositor::SurfaceAttributes;
use smithay::wayland::selection::data_device;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::primary_selection;
use smithay::wayland::shm::BufferData;
use smithay::xwayland::X11Surface;
use smithay_client_toolkit::compositor::CompositorHandler;
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::compositor::Surface;
use smithay_client_toolkit::data_device_manager::data_device::DataDeviceHandler;
use smithay_client_toolkit::data_device_manager::data_offer::DataOfferHandler;
use smithay_client_toolkit::data_device_manager::data_offer::DragOffer;
use smithay_client_toolkit::data_device_manager::data_offer::SelectionOffer;
use smithay_client_toolkit::data_device_manager::data_source::CopyPasteSource;
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::data_device_manager::WritePipe;
use smithay_client_toolkit::output::OutputHandler;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::primary_selection::selection::PrimarySelectionSource;
use smithay_client_toolkit::primary_selection::PrimarySelectionManagerState;
use smithay_client_toolkit::primary_selection::device::PrimarySelectionDeviceHandler;
use smithay_client_toolkit::primary_selection::offer::PrimarySelectionOffer;
use smithay_client_toolkit::primary_selection::selection::PrimarySelectionSourceHandler;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device::WlDataDevice;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_source::WlDataSource;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_output::Transform;
use smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput;
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::AxisSource as WlPointerAxisSource;
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
use smithay_client_toolkit::reexports::csd_frame::CursorIcon;
use smithay_client_toolkit::reexports::csd_frame::DecorationsFrame;
use smithay_client_toolkit::reexports::csd_frame::WindowManagerCapabilities;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::Anchor;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::Gravity;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_surface::XdgSurface as SctkXdgSurface;
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
use smithay_client_toolkit::seat::pointer::ThemedPointer;
use smithay_client_toolkit::seat::Capability;
use smithay_client_toolkit::seat::SeatHandler;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::fallback_frame::FallbackFrame;
use smithay_client_toolkit::shell::xdg::popup::Popup;
use smithay_client_toolkit::shell::xdg::popup::PopupConfigure;
use smithay_client_toolkit::shell::xdg::popup::PopupHandler;
use smithay_client_toolkit::shell::xdg::window::Window;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use smithay_client_toolkit::shell::xdg::window::WindowDecorations;
use smithay_client_toolkit::shell::xdg::window::WindowHandler;
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::XdgSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::Buffer;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::shm::ShmHandler;
use smithay_client_toolkit::subcompositor::SubcompositorState;
use tracing::Span;

use crate::args;
use crate::buffer_pointer::BufferPointer;
use crate::client_utils::SeatObject;
use crate::prelude::*;
use crate::serialization;
use crate::serialization::geometry::Point;
use crate::serialization::wayland::BufferMetadata;
use crate::xwayland_xdg_shell::compositor::DecorationBehavior;
use crate::xwayland_xdg_shell::compositor::X11Parent;
use crate::xwayland_xdg_shell::compositor::X11ParentForPopup;
use crate::xwayland_xdg_shell::compositor::X11ParentForSubsurface;
use crate::xwayland_xdg_shell::decoration::handle_window_frame_pointer_event;
use crate::xwayland_xdg_shell::xsurface_from_client_surface;
use crate::xwayland_xdg_shell::WprsState;
use crate::xwayland_xdg_shell::XWaylandSurface;

const DEFAULT_WINDOW_SIZE: (i32, i32) = (512, 256);

#[derive(Debug)]
pub struct WprsClientState {
    pub qh: QueueHandle<WprsState>,
    pub(crate) conn: Connection,

    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub compositor_state: CompositorState,
    pub subcompositor_state: Arc<SubcompositorState>,
    pub shm_state: Shm,
    pub xdg_shell_state: XdgShell,

    pub(crate) data_device_manager_state: DataDeviceManagerState,
    pub(crate) primary_selection_manager_state: Option<PrimarySelectionManagerState>,

    pub exit: bool,
    pub pool: Option<SlotPool>,

    pub last_enter_serial: u32,
    pub(crate) last_implicit_grab_serial: u32,
    pub(crate) last_focused_window: Option<X11Parent>,

    pub(crate) seat_objects: Vec<SeatObject<ThemedPointer>>,
    pub(crate) cursor_icon: Option<CursorIcon>,
    pub(crate) selection_offer: Option<SelectionOffer>,
    pub(crate) selection_source: Option<CopyPasteSource>,
    pub(crate) primary_selection_offer: Option<PrimarySelectionOffer>,
    pub(crate) primary_selection_source: Option<PrimarySelectionSource>,
}

impl WprsClientState {
    pub fn new(globals: &GlobalList, qh: QueueHandle<WprsState>, conn: Connection) -> Result<Self> {
        let shm_state = Shm::bind(globals, &qh).context(loc!(), "wl_shm is not available")?;
        let pool =
            Some(SlotPool::new(3840 * 2160, &shm_state).context(loc!(), "failed to create pool")?);
        let compositor_state = CompositorState::bind(globals, &qh)
            .context(loc!(), "wl_compositor is not available")?;
        let subcompositor_state = Arc::new(
            SubcompositorState::bind(compositor_state.wl_compositor().clone(), globals, &qh)
                .context(loc!(), "wl_subcompositor is not available")?,
        );

        Ok(Self {
            qh: qh.clone(),
            conn,
            registry_state: RegistryState::new(globals),
            seat_state: SeatState::new(globals, &qh),
            output_state: OutputState::new(globals, &qh),
            compositor_state,
            subcompositor_state,
            shm_state,
            xdg_shell_state: XdgShell::bind(globals, &qh)
                .context(loc!(), "xdg shell is not available")?,
            data_device_manager_state: DataDeviceManagerState::bind(globals, &qh)
                .context(loc!(), "data device manager is not available")?,
            primary_selection_manager_state: PrimarySelectionManagerState::bind(globals, &qh)
                .context(loc!(), "primary selection manager is not available")
                .warn(loc!())
                .ok(),

            exit: false,
            pool,

            last_enter_serial: 0,
            last_implicit_grab_serial: 0,
            last_focused_window: None,

            seat_objects: Vec::new(),
            cursor_icon: None,
            selection_offer: None,
            selection_source: None,
            primary_selection_offer: None,
            primary_selection_source: None,
        })
    }
}

impl CompositorHandler for WprsState {
    #[instrument(skip(self, _conn, _qh, _new_factor), level = "debug")]
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _new_factor: i32,
    ) {
        self.sync_surface_outputs(surface);
    }

    #[instrument(skip(self, _conn, _qh, _new_transform), level = "debug")]
    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _new_transform: Transform,
    ) {
        self.sync_surface_outputs(surface);
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        time: u32,
    ) {
        if let Some(compositor_surface_id) = self.surface_bimap.get_by_right(&surface.id()) {
            let xwayland_surface = self.surfaces.get_mut(compositor_surface_id).unwrap();
            if let Some(Role::SubSurface(subsurface)) = &mut xwayland_surface.role {
                subsurface.pending_frame_callback = false;
            }
            if let Some(x11_surface) = &xwayland_surface.x11_surface {
                if let Some(wl_surface) = x11_surface.wl_surface() {
                    compositor::with_states(&wl_surface, |surface_data| {
                        for callback in surface_data
                            .cached_state
                            .get::<SurfaceAttributes>()
                            .current()
                            .frame_callbacks
                            .drain(..)
                        {
                            debug!(
                                "Sending callback for client surface {:?}, compositor surface {:?}: {:?}.",
                                surface.id(),
                                xwayland_surface.wl_surface().id(),
                                callback.id()
                            );
                            callback
                                .done(self.compositor_state.start_time.elapsed().as_millis()
                                    as u32);
                        }
                    });
                }
            }
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
        // handled by scale_factor_changed/transform_changed, which only process when the scaling actually changes.
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
        // handled by scale_factor_changed/transform_changed, which only process when the scaling actually changes.
    }
}

impl OutputHandler for WprsState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.client_state.output_state
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: WlOutput) {
        let output_info = self.output_state().info(&output).unwrap();
        self.compositor_state.new_output(output_info.into());
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: WlOutput) {
        let output_info = self.output_state().info(&output).unwrap();
        self.compositor_state.update_output(output_info.into());
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: WlOutput) {
        let output_info = self.output_state().info(&output).unwrap();
        self.compositor_state.destroy_output(output_info.into());
    }
}

impl WindowHandler for WprsState {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, window: &Window) {
        let compositor_surface_id = self
            .surface_bimap
            .get_by_right(&window.wl_surface().id())
            .unwrap();
        let xwayland_surface = self.surfaces.get_mut(compositor_surface_id).unwrap();
        let x11_surface = &xwayland_surface.x11_surface.as_ref().unwrap();
        x11_surface.close().log_and_ignore(loc!());
    }

    #[instrument(skip(self, _conn, _qh, _serial), level = "debug")]
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let Some(compositor_surface_id) =
            self.surface_bimap.get_by_right(&window.wl_surface().id())
        else {
            warn!("Received configure for already-destroyed window {window:?}.");
            return;
        };

        let xwayland_surface = self.surfaces.get_mut(compositor_surface_id).unwrap();

        let xdg_toplevel = match &mut xwayland_surface.role {
            Some(Role::XdgToplevel(xdg_toplevel)) => xdg_toplevel,
            _ => unreachable!(
                "expected role xdg_toplevel, found role {:?}",
                &xwayland_surface.role
            ),
        };
        let x11_surface = &xwayland_surface.x11_surface.as_ref().unwrap();

        x11_surface
            .set_maximized(configure.is_maximized())
            .log_and_ignore(loc!());
        x11_surface
            .set_fullscreen(configure.is_fullscreen())
            .log_and_ignore(loc!());

        xdg_toplevel
            .apply_decoration(
                x11_surface,
                Some(&configure),
                xwayland_surface
                    .buffer
                    .as_ref()
                    .map(|buffer| &buffer.metadata),
            )
            .log_and_ignore(loc!());

        // The code below commits the buffer we received but couldn't attach
        // because we hadn't received our initial commit. In the normal
        // configure case, the above code will have sent some X11 configure
        // events and the xwayland can commit in response if so desires.
        if xdg_toplevel.configured {
            return;
        }

        xdg_toplevel.configured = true;

        xwayland_surface.commit_buffer(&self.client_state.qh);
    }
}

impl PopupHandler for WprsState {
    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        popup: &Popup,
        configure: PopupConfigure,
    ) {
        let compositor_surface_id = self
            .surface_bimap
            .get_by_right(&popup.wl_surface().id())
            .unwrap();

        let xwayland_surface = self.surfaces.get_mut(compositor_surface_id).unwrap();

        let x11_surface = log_and_return!(xwayland_surface.get_x11_surface());
        let geo = if x11_surface.is_override_redirect() {
            None
        } else {
            Some(Rectangle::from_loc_and_size(
                configure.position,
                (configure.width, configure.height),
            ))
        };

        x11_surface.configure(geo).log_and_ignore(loc!());

        let xdg_popup = match &mut xwayland_surface.role {
            Some(Role::XdgPopup(xdg_popup)) => xdg_popup,
            _ => unreachable!(
                "expected role xdg_popup, found role {:?}",
                &xwayland_surface.role
            ),
        };

        // The code below commits the buffer we received but couldn't attach
        // because we hadn't received our initial commit. In the normal
        // configure case, the above code will have sent some X11 configure
        // events and the xwayland can commit in response if so desires.
        if xdg_popup.configured {
            return;
        }

        xdg_popup.configured = true;

        // TODO: maybe don't do this... most popups shouldn't need this and this
        // will be a problem for override_redirect windows when they try to
        // resize themselves.
        // xdg_popup
        //     .local_popup
        //     .xdg_surface()
        //     .set_window_geometry(0, 0, geo.size.w, geo.size.h);

        xwayland_surface.commit_buffer(&self.client_state.qh);
    }

    fn done(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _popup: &Popup) {
        // TODO?
    }
}

impl SeatHandler for WprsState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.client_state.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        let seat_obj = if let Some(seat_obj) = self
            .client_state
            .seat_objects
            .iter_mut()
            .find(|s| s.seat == seat)
        {
            seat_obj
        } else {
            // create the data device here for this seat
            let data_device_manager = &self.client_state.data_device_manager_state;
            let data_device = data_device_manager.get_data_device(qh, &seat);

            let primary_selection_device = self
                .client_state
                .primary_selection_manager_state
                .as_ref()
                .map(|x| x.get_selection_device(qh, &seat));

            self.client_state.seat_objects.push(SeatObject {
                seat: seat.clone(),
                keyboard: None,
                pointer: None,
                data_device,
                primary_selection_device,
            });
            self.client_state.seat_objects.last_mut().unwrap()
        };

        if capability == Capability::Keyboard && seat_obj.keyboard.is_none() {
            println!("Set keyboard capability");
            let keyboard = self
                .client_state
                .seat_state
                .get_keyboard(qh, &seat, None)
                .expect("Failed to create keyboard");
            seat_obj.keyboard.replace(keyboard);
        }

        if capability == Capability::Pointer && seat_obj.pointer.is_none() {
            println!("Set pointer capability");
            let themed_pointer = self
                .client_state
                .seat_state
                .get_pointer_with_theme(
                    qh,
                    &seat,
                    self.client_state.shm_state.wl_shm(),
                    self.client_state.compositor_state.create_surface(qh),
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
        if let Some(seat_obj) = self
            .client_state
            .seat_objects
            .iter_mut()
            .find(|s| s.seat == seat)
        {
            match capability {
                Capability::Keyboard => {
                    if let Some(k) = seat_obj.keyboard.take() {
                        k.release()
                    }
                },
                Capability::Pointer => {
                    if let Some(p) = seat_obj.pointer.take() {
                        p.pointer().release()
                    }
                },
                _ => {},
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for WprsState {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        surface: &WlSurface,
        serial: u32,
        raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        // see linux/input-event-codes.h for keycodes
        let modifier_keycodes = HashSet::from([
            /* KEY_LEFTCTRL */ 29, /* KEY_RIGHTCTRL */ 97, /* KEY_LEFTALT */ 56,
            /* KEY_RIGHTALT */ 100, /* KEY_LEFTMETA	*/ 125, /* KEY_RIGHTMETA */ 126,
            /* KEY_LEFTSHIFT */ 42, /* KEY_RIGHTSHIFT */ 54,
        ]);

        let keyboard = log_and_return!(self
            .compositor_state
            .seat
            .get_keyboard()
            .ok_or("seat has no keyboard"));

        // We simulate keycodes before focusing since that is what a normal wayland application would see.
        // Process modifier keys first so that they apply to other held keys.
        let mut delayed_keycodes = Vec::new();
        for keycode in raw {
            if modifier_keycodes.contains(keycode) {
                log_and_return!(self.set_key_state(
                    *keycode,
                    KeyState::Pressed,
                    SERIAL_COUNTER.next_serial(),
                ));
            } else {
                delayed_keycodes.push(keycode);
            }
        }
        for keycode in delayed_keycodes {
            log_and_return!(self.set_key_state(
                *keycode,
                KeyState::Pressed,
                SERIAL_COUNTER.next_serial()
            ));
        }
        let Some(xwayland_surface) =
            xsurface_from_client_surface(&self.surface_bimap, &mut self.surfaces, surface)
        else {
            // surface was already destroyed
            return;
        };
        let x11_surface = log_and_return!(xwayland_surface.get_x11_surface()).clone();
        let client = x11_surface.wl_surface().unwrap().client();
        x11_surface.set_activated(true).unwrap();
        let serial = self.compositor_state.serial_map.insert(serial);
        keyboard.set_focus(self, Some(x11_surface), serial);
        data_device::set_data_device_focus(
            &self.compositor_state.dh,
            &self.compositor_state.seat,
            client.clone(),
        );
        primary_selection::set_primary_focus(
            &self.compositor_state.dh,
            &self.compositor_state.seat,
            client,
        );
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        surface: &WlSurface,
        serial: u32,
    ) {
        let Some(xwayland_surface) =
            xsurface_from_client_surface(&self.surface_bimap, &mut self.surfaces, surface)
        else {
            // surface was already destroyed
            return;
        };
        let x11_surface = log_and_return!(xwayland_surface.get_x11_surface()).clone();
        x11_surface.set_activated(false).unwrap();
        let keyboard = log_and_return!(self
            .compositor_state
            .seat
            .get_keyboard()
            .ok_or("seat has no keyboard"));

        let serial = self.compositor_state.serial_map.insert(serial);
        keyboard.set_focus(self, None, serial);
        data_device::set_data_device_focus(
            &self.compositor_state.dh,
            &self.compositor_state.seat,
            None,
        );
        primary_selection::set_primary_focus(
            &self.compositor_state.dh,
            &self.compositor_state.seat,
            None,
        );

        for keycode in self.compositor_state.pressed_keys.clone() {
            log_and_return!(self.set_key_state(keycode, KeyState::Released, serial));
        }
    }

    // INTENTIONALLY NOT LOGGING KEY EVENTS
    #[instrument(
        skip(self, _conn, _qh, _keyboard, event),
        fields(event),
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
        if args::get_log_priv_data() {
            Span::current().record("event", field::debug(&event));
        }
        self.client_state.last_implicit_grab_serial = serial;
        let serial = self.compositor_state.serial_map.insert(serial);
        log_and_return!(self.set_key_state(event.raw_code, KeyState::Pressed, serial));
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
        let serial = self.compositor_state.serial_map.insert(serial);

        log_and_return!(self.set_key_state(event.raw_code, KeyState::Released, serial));
    }

    fn update_repeat_info(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        info: RepeatInfo,
    ) {
        let keyboard = log_and_return!(self
            .compositor_state
            .seat
            .get_keyboard()
            .ok_or("seat has no keyboard"));
        let (rate, delay) = match info {
            RepeatInfo::Repeat { rate, delay } => (rate.get(), delay),
            RepeatInfo::Disable => (0, 0),
        };
        keyboard.change_repeat_info(rate as i32, delay as i32);
    }

    fn update_keymap(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        keymap: Keymap<'_>,
    ) {
        let keyboard = log_and_return!(self
            .compositor_state
            .seat
            .get_keyboard()
            .ok_or("seat has no keyboard"));
        log_and_return!(keyboard.set_keymap_from_string(self, keymap.as_string()));
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        variant: u32,
    ) {
        let keyboard = log_and_return!(self
            .compositor_state
            .seat
            .get_keyboard()
            .ok_or("seat has no keyboard"));
        keyboard.with_xkb_state(self, |mut context: XkbContext| {
            context.set_layout(Layout(variant));
        });

        // see linux/input-event-codes.h for keycodes
        let mod_state = keyboard.modifier_state();
        for (new_modifier, current_modifier, keycode) in [
            (
                modifiers.caps_lock,
                mod_state.caps_lock,
                /* KEY_CAPSLOCK */ 58,
            ),
            (
                modifiers.num_lock,
                mod_state.num_lock,
                /* KEY_NUMLOCK */ 69,
            ),
        ] {
            if new_modifier != current_modifier {
                log_and_return!(self.set_key_state(
                    keycode,
                    KeyState::Pressed,
                    SERIAL_COUNTER.next_serial(),
                ));
                log_and_return!(self.set_key_state(
                    keycode,
                    KeyState::Released,
                    SERIAL_COUNTER.next_serial(),
                ));
            }
        }
    }
}

impl PointerHandler for WprsState {
    #[instrument(skip(self, conn, qh, pointer), level = "debug")]
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        let compositor_seat = self.compositor_state.seat.clone();
        let compositor_pointer = compositor_seat.get_pointer().unwrap();

        for event in events {
            let Some(xwayland_surface) = xsurface_from_client_surface(
                &self.surface_bimap,
                &mut self.surfaces,
                &event.surface,
            ) else {
                handle_window_frame_pointer_event(self, conn, qh, pointer, events)
                    .log_and_ignore(loc!());
                return;
            };
            let x11_surface = log_and_return!(xwayland_surface.get_x11_surface()).clone();

            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.client_state.last_enter_serial = serial;
                    // TODO: allow this to be a popup?
                    if let Some(Role::XdgToplevel(toplevel)) = &xwayland_surface.role {
                        let parent_id = self
                            .surface_bimap
                            .get_by_right(&event.surface.id())
                            .unwrap();
                        self.client_state.last_focused_window = Some(X11Parent {
                            surface_id: parent_id.clone(),
                            for_popup: Some(X11ParentForPopup {
                                surface_id: parent_id.clone(),
                                xdg_surface: toplevel.xdg_surface().clone(),
                                x11_offset: (
                                    toplevel.frame_offset.x + toplevel.x11_offset.x,
                                    toplevel.frame_offset.y + toplevel.x11_offset.y,
                                )
                                    .into(),
                                wl_offset: toplevel.frame_offset,
                            }),
                            for_subsurface: X11ParentForSubsurface {
                                surface: toplevel.wl_surface().clone(),
                                x11_offset: toplevel.x11_offset,
                            },
                        });
                    }
                    self.compositor_state
                        .xwm
                        .as_mut()
                        .unwrap()
                        .raise_window(&x11_surface)
                        .unwrap();
                    let serial = self.compositor_state.serial_map.insert(serial);
                    compositor_pointer.motion(
                        self,
                        Some((x11_surface, (0 as f64, 0 as f64).into())),
                        &MotionEvent {
                            location: event.position.into(),
                            serial,
                            time: 0, // unused
                        },
                    );
                },
                PointerEventKind::Leave { serial } => {
                    let serial = self.compositor_state.serial_map.insert(serial);
                    compositor_pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: event.position.into(),
                            serial,
                            time: 0, // unused
                        },
                    );
                },
                PointerEventKind::Motion { time } => {
                    compositor_pointer.motion(
                        self,
                        Some((x11_surface, (0 as f64, 0 as f64).into())),
                        &MotionEvent {
                            location: event.position.into(),
                            serial: 0.into(), // unused
                            time,
                        },
                    );
                },
                PointerEventKind::Press {
                    time,
                    button,
                    serial,
                } => {
                    let serial = self.compositor_state.serial_map.insert(serial);
                    compositor_pointer.button(
                        self,
                        &ButtonEvent {
                            time,
                            button,
                            serial,
                            state: ButtonState::Pressed,
                        },
                    );
                },
                PointerEventKind::Release {
                    time,
                    button,
                    serial,
                } => {
                    let serial = self.compositor_state.serial_map.insert(serial);
                    compositor_pointer.button(
                        self,
                        &ButtonEvent {
                            time,
                            button,
                            serial,
                            state: ButtonState::Released,
                        },
                    );
                },
                PointerEventKind::Axis {
                    time,
                    horizontal,
                    vertical,
                    source,
                } => x11_surface.axis(
                    &compositor_seat,
                    self,
                    AxisFrame::new(time)
                        .source(match source.unwrap() {
                            WlPointerAxisSource::Wheel => AxisSource::Wheel,
                            WlPointerAxisSource::Finger => AxisSource::Finger,
                            WlPointerAxisSource::Continuous => AxisSource::Continuous,
                            WlPointerAxisSource::WheelTilt => AxisSource::WheelTilt,
                            _ => unreachable!("got unknown AxisSource {:?}", source),
                        })
                        .value(Axis::Horizontal, horizontal.absolute)
                        .value(Axis::Vertical, vertical.absolute)
                        .v120(Axis::Horizontal, horizontal.discrete * 120)
                        .v120(Axis::Vertical, vertical.discrete * 120),
                ),
            }
        }
        compositor_pointer.frame(self);
    }
}

impl ShmHandler for WprsState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.client_state.shm_state
    }
}

#[derive(Debug)]
pub struct XWaylandBuffer {
    pub metadata: BufferMetadata,
    pub active_buffer: Buffer,
}

impl XWaylandBuffer {
    #[instrument(skip_all, level = "debug")]
    pub fn new(metadata: BufferMetadata, pool: &mut SlotPool) -> Result<Self> {
        let active_buffer = pool
            .create_buffer(
                metadata.width,
                metadata.height,
                metadata.stride,
                metadata.format.into(),
            )
            .location(loc!())?
            .0;

        Ok(Self {
            metadata,
            active_buffer,
        })
    }

    #[instrument(skip_all, level = "debug")]
    pub fn write_data(&mut self, data: BufferPointer<u8>, pool: &mut SlotPool) -> Result<()> {
        let canvas = match pool.canvas(&self.active_buffer) {
            Some(canvas) => canvas,
            None => {
                // This should be rare, but if the compositor has not
                // released the previous_button_state buffer, we need
                // double-buffering.
                debug!("previous buffer wasn't released, creating new buffer");
                self.active_buffer = pool
                    .create_buffer(
                        self.metadata.width,
                        self.metadata.height,
                        self.metadata.stride,
                        self.metadata.format.into(),
                    )
                    .location(loc!())?
                    .0;
                pool.canvas(&self.active_buffer).unwrap()
            },
        };
        data.copy_to_nonoverlapping(canvas);
        Ok(())
    }
}

impl XWaylandSurface {
    pub fn write_data(&mut self, data: BufferPointer<u8>, pool: &mut SlotPool) -> Result<()> {
        if let Some(buffer) = &mut self.buffer {
            buffer.write_data(data, pool).location(loc!())?;
        }
        Ok(())
    }

    #[instrument(skip(data, pool), level = "debug")]
    pub fn update_buffer(
        &mut self,
        metadata: &BufferData,
        data: BufferPointer<u8>,
        pool: &mut SlotPool,
    ) -> Result<()> {
        let metadata =
            serialization::wayland::BufferMetadata::from_buffer_data(metadata).location(loc!())?;
        let buffer = match &mut self.buffer {
            // Surface was previously committed.
            Some(buffer) => {
                // Only buffer data was updated, we can reuse the buffer.
                if buffer.metadata == metadata {
                    debug!(
                        "metadata matched, reusing buffer, {:?}, {:?}",
                        buffer.metadata, metadata
                    );
                    buffer
                } else {
                    // Buffer was resized or format changed, need to
                    // create a new one.
                    debug!(
                        "metadata didn't match, creating new buffer, {:?}, {:?}",
                        buffer.metadata, metadata
                    );
                    *buffer = XWaylandBuffer::new(metadata, pool).location(loc!())?;
                    buffer
                }
            },
            // First commit for surface with a buffer.
            None => {
                self.buffer = Some(XWaylandBuffer::new(metadata, pool).location(loc!())?);
                self.buffer.as_mut().unwrap()
            },
        };

        buffer.write_data(data, pool).location(loc!())?;

        Ok(())
    }

    pub fn commit(&self) {
        self.wl_surface().commit();
    }

    pub fn frame(&self, qh: &QueueHandle<WprsState>) {
        self.wl_surface().frame(qh, self.wl_surface().clone());
    }

    pub fn get_role(&self) -> Result<&Role> {
        self.role.as_ref().ok_or(anyhow!("Role was None."))
    }

    pub fn get_mut_role(&mut self) -> Result<&mut Role> {
        self.role.as_mut().ok_or(anyhow!("Role was None."))
    }

    pub fn xdg_toplevel(&mut self) -> Result<&mut XWaylandXdgToplevel> {
        self.get_mut_role()?
            .as_xdg_toplevel_mut()
            .ok_or(anyhow!("Role was not XdgToplevel."))
    }

    pub fn xdg_popup(&mut self) -> Result<&mut XWaylandXdgPopup> {
        self.get_mut_role()?
            .as_xdg_popup_mut()
            .ok_or(anyhow!("Role was not XdgPopup."))
    }
}

#[derive(Debug, EnumAsInner)]
pub enum Role {
    Cursor,
    XdgToplevel(XWaylandXdgToplevel),
    XdgPopup(XWaylandXdgPopup),
    SubSurface(XWaylandSubSurface),
}

#[derive(Debug)]
pub struct XWaylandXdgToplevel {
    pub local_window: Window,
    pub window_frame: FallbackFrame<WprsState>,
    pub frame_offset: Point<i32>,
    pub configured: bool,
    pub decoration_behavior: DecorationBehavior,
    pub x11_offset: Point<i32>,
}

impl XWaylandXdgToplevel {
    #[instrument(ret, level = "debug")]
    pub fn enable_decorations(
        &mut self,
        x11_surface: &X11Surface,
        configure: Option<&WindowConfigure>,
        buffer_metadata: Option<&BufferMetadata>,
    ) -> Result<(i32, i32)> {
        let default_window_size = (
            NonZeroU32::new(DEFAULT_WINDOW_SIZE.0 as u32),
            NonZeroU32::new(DEFAULT_WINDOW_SIZE.1 as u32),
        );
        let window_frame = &mut self.window_frame;
        window_frame.set_hidden(false);
        if let Some(configure) = configure {
            window_frame.update_state(configure.state);
            window_frame.update_wm_capabilities(configure.capabilities);
        }

        // configure.new_size has outer_dimensions, we want width and height to
        // be inner dimensions.
        let (width, height) = match (configure, buffer_metadata) {
            (
                Some(WindowConfigure {
                    new_size: (Some(width), Some(height)),
                    ..
                }),
                _,
            ) => window_frame.subtract_borders(*width, *height),
            (_, Some(buffer_metadata)) => (
                NonZeroU32::new(buffer_metadata.width as u32),
                NonZeroU32::new(buffer_metadata.height as u32),
            ),
            _ => {
                warn!("Unable to get size from either configure or buffer_metadata, using default size: {:?}", default_window_size);
                default_window_size
            },
        };

        // Clamp the size to at least one pixel.
        let width = width.unwrap_or(NonZeroU32::new(1).unwrap());
        let height = height.unwrap_or(NonZeroU32::new(1).unwrap());

        window_frame.resize(width, height);

        // Everything after this wants u32s or i32s.
        let width = width.get();
        let height = height.get();

        // X11's ConfigureNotify wants the outer coordinates but the inner
        // dimensions. And don't worry about border_width. /sigh
        x11_surface
            .configure(Rectangle::from_loc_and_size(
                (-self.x11_offset.x, -self.x11_offset.y),
                (width as i32, height as i32),
            ))
            .location(loc!())?;

        // Top-left corner of frame relative to the inner surface, so x and y
        // will always be negative. -x and -y are thus the coordinates of the
        // inner top-left corner (assuming the outer coordinates are 0).
        let (x, y) = window_frame.location();
        self.frame_offset = (-x, -y).into();

        let (outer_w, outer_h) = window_frame.add_borders(width, height);

        // set_window_geometry wants the "\"visisble bounds\" from the user's
        // perspective", but excluding things like drop shadows, so outer
        // coordinates and dimensions. *However*, the frame subsurfaces are
        // placed at negative offsets from the main buffer and x and y are in
        // surface-local coordinates, so passing them in directly is what we
        // must do.
        self.local_window
            .xdg_surface()
            .set_window_geometry(x, y, outer_w as i32, outer_h as i32);

        Ok((width as i32, height as i32))
    }

    #[instrument(skip_all, level = "debug")]
    pub fn disable_decoration(
        &mut self,
        x11_surface: &X11Surface,
        configure: Option<&WindowConfigure>,
        buffer_metadata: Option<&BufferMetadata>,
    ) -> Result<(i32, i32)> {
        let default_window_size = DEFAULT_WINDOW_SIZE;
        let window_frame = &mut self.window_frame;
        window_frame.set_hidden(true);
        self.frame_offset = (0, 0).into();

        let (width, height) = match (configure, buffer_metadata) {
            (
                Some(WindowConfigure {
                    new_size: (Some(width), Some(height)),
                    ..
                }),
                _,
            ) => (width.get() as i32, height.get() as i32),
            (_, Some(buffer_metadata)) => (buffer_metadata.width, buffer_metadata.height),
            _ => {
                warn!("Unable to get size from either configure or buffer_metadata, using default size: {:?}", default_window_size);
                default_window_size
            },
        };

        x11_surface
            .configure(Rectangle::from_loc_and_size(
                (-self.x11_offset.x, -self.x11_offset.y),
                (width, height),
            ))
            .location(loc!())?;

        self.local_window
            .xdg_surface()
            .set_window_geometry(0, 0, width, height);

        Ok((width, height))
    }

    pub fn apply_decoration(
        &mut self,
        x11_surface: &X11Surface,
        configure: Option<&WindowConfigure>,
        buffer_metadata: Option<&BufferMetadata>,
    ) -> Result<(i32, i32)> {
        match self.decoration_behavior {
            DecorationBehavior::Auto => {
                if !x11_surface.is_decorated() {
                    self.enable_decorations(x11_surface, configure, buffer_metadata)
                } else {
                    self.disable_decoration(x11_surface, configure, buffer_metadata)
                }
            },
            DecorationBehavior::AlwaysEnabled => {
                self.enable_decorations(x11_surface, configure, buffer_metadata)
            },
            DecorationBehavior::AlwaysDisabled => {
                self.disable_decoration(x11_surface, configure, buffer_metadata)
            },
        }
    }

    pub fn set_role(
        surface: &mut XWaylandSurface,
        x11_offset: Point<i32>,
        xdg_shell_state: &XdgShell,
        shm_state: &Shm,
        subcompositor_state: Arc<SubcompositorState>,
        qh: &QueueHandle<WprsState>,
        decoration_behavior: DecorationBehavior,
    ) -> Result<()> {
        let local_surface = surface.local_surface.take().location(loc!())?;
        let local_window =
            xdg_shell_state.create_window(local_surface, WindowDecorations::ServerDefault, qh);

        let x11_surface = surface.get_x11_surface().location(loc!())?;
        local_window.set_title(x11_surface.title());

        if let Some(max_size) = x11_surface.max_size() {
            local_window.set_max_size(Some((max_size.w as u32, max_size.h as u32)));
        }

        if let Some(min_size) = x11_surface.min_size() {
            local_window.set_min_size(Some((min_size.w as u32, min_size.h as u32)));
        }

        // TODO: decorations

        local_window.commit();

        let window_frame =
            FallbackFrame::new(&local_window, shm_state, subcompositor_state, qh.clone())
                .map_err(|e| anyhow!("failed to create client side decorations frame: {e:?}."))
                .location(loc!())?;

        let new_toplevel = Self {
            local_window,
            window_frame,
            frame_offset: (0, 0).into(),
            configured: false,
            decoration_behavior,
            x11_offset,
        };
        surface.role = Some(Role::XdgToplevel(new_toplevel));
        Ok(())
    }
}

impl WaylandSurface for XWaylandXdgToplevel {
    fn wl_surface(&self) -> &WlSurface {
        self.local_window.wl_surface()
    }
}

impl XdgSurface for XWaylandXdgToplevel {
    fn xdg_surface(&self) -> &SctkXdgSurface {
        self.local_window.xdg_surface()
    }
}

#[derive(Debug)]
pub struct SubSurface {
    pub subsurface: WlSubsurface,
    pub surface: Surface,
}

impl WaylandSurface for SubSurface {
    fn wl_surface(&self) -> &WlSurface {
        self.surface.wl_surface()
    }
}

impl Drop for SubSurface {
    fn drop(&mut self) {
        self.subsurface.destroy();
    }
}

#[derive(Debug)]
pub struct XWaylandSubSurface {
    pub local_subsurface: SubSurface,
    pub parent_surface: WlSurface,
    pub offset: Point<i32>,
    pub frame: Option<FallbackFrame<WprsState>>,
    pub move_active: bool,
    pub move_pointer_location: (f64, f64),
    pub pending_frame_callback: bool,
    pub buffer_attached: bool,
}

impl XWaylandSubSurface {
    pub(crate) fn set_role(
        surface: &mut XWaylandSurface,
        parent: X11ParentForSubsurface,
        shm_state: &Shm,
        subcompositor_state: Arc<SubcompositorState>,
        qh: &QueueHandle<WprsState>,
    ) -> Result<()> {
        let local_surface = surface.local_surface.take().unwrap();
        let subsurface = subcompositor_state
            .subsurface_from_surface(local_surface.wl_surface(), qh)
            .unwrap();

        let local_subsurface = SubSurface {
            subsurface,
            surface: local_surface,
        };
        local_subsurface.subsurface.set_desync();

        let x11_surface = surface.get_x11_surface().location(loc!())?;
        let geometry = x11_surface.geometry();

        // is_decorated means that the surface is already decorated and does NOT want our decorations.
        let frame = if !x11_surface.is_decorated() && !x11_surface.is_override_redirect() {
            let mut frame = FallbackFrame::new(
                &local_subsurface,
                shm_state,
                subcompositor_state,
                qh.clone(),
            )
            .map_err(|e| anyhow!("failed to create client side decorations frame: {e:?}."))
            .location(loc!())?;

            // not an xdg-shell window, so we can't fullscreen/maximize/etc.
            frame.update_wm_capabilities(WindowManagerCapabilities::empty());

            //we want width and height to be inner dimensions.
            let width = NonZeroU32::new(geometry.size.w.max(1) as u32).unwrap();
            let height = NonZeroU32::new(geometry.size.h.max(1) as u32).unwrap();
            frame.resize(width, height);

            Some(frame)
        } else {
            None
        };

        x11_surface.configure(None).location(loc!())?;

        // TODO: it seems like we should probably also include frame offset of our window decorations somehwere
        let new_subsurface = Self {
            local_subsurface,
            parent_surface: parent.surface,
            offset: parent.x11_offset,
            frame,
            move_active: false,
            move_pointer_location: (0 as f64, 0 as f64),
            pending_frame_callback: false,
            buffer_attached: false,
        };
        surface.role = Some(Role::SubSurface(new_subsurface));

        // the initial commit must include both the subsurface position and the buffer
        // to prevent desyncs
        if let Some(Role::SubSurface(subsurface)) = &mut surface.role {
            subsurface.move_without_commit(geometry.loc.x, geometry.loc.y, qh);
        }

        Ok(())
    }

    pub(crate) fn move_without_commit(&mut self, x: i32, y: i32, qh: &QueueHandle<WprsState>) {
        if !self.pending_frame_callback {
            let local_wl_surface = self.wl_surface();

            self.local_subsurface
                .subsurface
                .set_position(x + self.offset.x, y + self.offset.y);
            local_wl_surface.frame(qh, local_wl_surface.clone());
            self.parent_surface.commit();

            self.pending_frame_callback = true;
        }
    }

    pub(crate) fn move_(&mut self, x: i32, y: i32, qh: &QueueHandle<WprsState>) {
        if !self.pending_frame_callback {
            self.move_without_commit(x, y, qh);
            self.wl_surface().commit();
        }
    }
}

impl WaylandSurface for XWaylandSubSurface {
    fn wl_surface(&self) -> &WlSurface {
        self.local_subsurface.wl_surface()
    }
}

#[derive(Debug)]
pub struct XWaylandXdgPopup {
    pub local_popup: Popup,
    pub parent: ObjectId,
    pub configured: bool,
}

impl XWaylandXdgPopup {
    pub(crate) fn set_role(
        surface: &mut XWaylandSurface,
        parent: &X11ParentForPopup,
        xdg_shell_state: &XdgShell,
        qh: &QueueHandle<WprsState>,
    ) -> Result<()> {
        let x11_surface = &surface.get_x11_surface().location(loc!())?;
        // TODO: move into function
        let positioner = XdgPositioner::new(xdg_shell_state).unwrap();
        let geometry = x11_surface.geometry();
        positioner.set_size(geometry.size.w, geometry.size.h);
        positioner.set_anchor_rect(
            geometry.loc.x + parent.x11_offset.x,
            geometry.loc.y + parent.x11_offset.y,
            1,
            1,
        );
        positioner.set_anchor(Anchor::TopLeft);
        positioner.set_gravity(Gravity::BottomRight);

        let configure_rect = if x11_surface.is_override_redirect() {
            None
        } else {
            Some(Rectangle::from_loc_and_size(
                (
                    geometry.loc.x + parent.wl_offset.x,
                    geometry.loc.y + parent.wl_offset.x,
                ),
                (geometry.size.w, geometry.size.h),
            ))
        };

        x11_surface.configure(configure_rect).location(loc!())?;

        // TODO: send this data over from server
        // positioner.set_constraint_adjustment(popup_state.positioner.constraint_adjustment);
        // positioner.set_offset(
        //     popup_state.positioner.offset.x,
        //     popup_state.positioner.offset.y,
        // );
        // if popup_state.positioner.reactive {
        //     positioner.set_reactive();
        // }
        // if let Some(parent_size) = &popup_state.positioner.parent_size {
        //     positioner.set_parent_size(parent_size.w, parent_size.h);
        // };
        // if let Some(parent_configure) = popup_state.positioner.parent_configure {
        //     positioner.set_parent_configure(parent_configure);
        // };

        let local_popup = Popup::from_surface(
            Some(&parent.xdg_surface),
            &positioner,
            qh,
            surface.local_surface.take().unwrap(),
            xdg_shell_state,
        )
        .unwrap();

        // TODO: finish with popup grabs.
        // if popup_state.grab_requested {
        //     local_popup
        //         .xdg_popup()
        //         .grab(&seat_state.seats().next().unwrap(), 0); // TODO: serial
        // }

        let new_popup = Self {
            local_popup,
            parent: parent.surface_id.clone(),
            configured: false,
        };
        surface.role = Some(Role::XdgPopup(new_popup));
        Ok(())
    }
}

impl WaylandSurface for XWaylandXdgPopup {
    fn wl_surface(&self) -> &WlSurface {
        self.local_popup.wl_surface()
    }
}

impl XdgSurface for XWaylandXdgPopup {
    fn xdg_surface(&self) -> &SctkXdgSurface {
        self.local_popup.xdg_surface()
    }
}

impl DataDeviceHandler for WprsState {
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
            .client_state
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

        // TODO: revisit xwayland drag-and-drop.
        // // accept the first mime type we support
        // if let Some(m) = data_device
        //     .drag_mime_types()
        //     .iter()
        //     .find(|m| SUPPORTED_MIME_TYPES.contains(&m.as_str()))
        // {
        //     drag_offer.accept_mime_type(0, Some(m.clone()));
        // }

        // // accept the action now just in case
        // drag_offer.set_actions(DndAction::Copy, DndAction::Copy);
    }

    fn leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _data_device: &WlDataDevice) {
        debug!("data offer left");
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
        // TODO: revisit xwayland drag-and-drop.
        // let DragOffer { x, y, time, .. } = data_device.drag_offer().unwrap();
        // dbg!((time, x, y));
    }

    #[instrument(skip_all, level = "debug")]
    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
    ) {
        let data_device = &self
            .client_state
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
        if mime_types.contains(&"_xwayland_xdg_shell_marker".to_string()) {
            return;
        }
        self.client_state.selection_offer = Some(offer);
        if let Some(xwm) = &mut self.compositor_state.xwm {
            xwm.new_selection(SelectionTarget::Clipboard, Some(mime_types))
                .log_and_ignore(loc!());
        }
        // TODO: do we need this?
        // data_device::set_data_device_selection(&self.compositor_state.dh,
        //                                        &self.compositor_state.seat,
        //                                        mime_types,
        //                                        ())
    }

    fn drop_performed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &WlDataDevice,
    ) {
        // TODO: revisit xwayland drag-and-drop.
        // if let Some(offer) = data_device.drag_offer() {
        //     println!("Dropped: {offer:?}");
        //     self.dnd_offers.push((offer, String::new(), None));
        //     let cur_offer = self.dnd_offers.last_mut().unwrap();
        //     let mime_type = match data_device
        //         .drag_mime_types()
        //         .iter()
        //         .find(|m| SUPPORTED_MIME_TYPES.contains(&m.as_str()))
        //         .cloned()
        //     {
        //         Some(mime) => mime,
        //         None => return,
        //     };
        //     dbg!(&mime_type);
        //     self.accept_counter += 1;
        //     cur_offer.0.accept_mime_type(self.accept_counter, Some(mime_type.clone()));
        //     cur_offer.0.set_actions(DndAction::Copy, DndAction::Copy);
        //     if let Ok(read_pipe) = cur_offer.0.receive(mime_type.clone()) {
        //         let offer_clone = cur_offer.0.clone();
        //         match self.loop_handle.insert_source(read_pipe, move |_, f, state| {
        //             let (offer, mut contents, token) = state
        //                 .dnd_offers
        //                 .iter()
        //                 .position(|o| &o.0.inner() == &offer_clone.inner())
        //                 .map(|p| state.dnd_offers.remove(p))
        //                 .unwrap();

        //             f.read_to_string(&mut contents).unwrap();
        //             println!("TEXT FROM drop: {contents}");
        //             state.loop_handle.remove(token.unwrap());

        //             offer.finish();
        //         }) {
        //             Ok(token) => {
        //                 cur_offer.2.replace(token);
        //             }
        //             Err(err) => {
        //                 eprintln!("{:?}", err);
        //             }
        //         }
        //     }
        // }
    }
}

impl DataOfferHandler for WprsState {
    fn source_actions(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        actions: DndAction,
    ) {
        debug!("Source actions: {actions:?}");
        // TODO: revisit xwayland drag-and-drop.
        // offer.set_actions(DndAction::Copy, DndAction::Copy);
    }

    fn selected_action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        actions: DndAction,
    ) {
        debug!("Selected action: {actions:?}");
        // TODO ?
    }
}

impl DataSourceHandler for WprsState {
    fn accept_mime(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
        mime: Option<String>,
    ) {
        debug!("Source mime type: {mime:?} was accepted");
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
        // TODO: handle multiple sources
        if let Some(xwm) = &mut self.compositor_state.xwm {
            xwm.send_selection(
                SelectionTarget::Clipboard,
                mime,
                write_pipe.into(),
                self.event_loop_handle.clone(),
            )
            .log_and_ignore(loc!());
        }
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn cancelled(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, source: &WlDataSource) {
        // TODO: revisit xwayland drag-and-drop.
    }

    #[instrument(skip_all, level = "debug")]
    fn dnd_dropped(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _source: &WlDataSource) {
        // TODO: revisit xwayland drag-and-drop.
    }

    #[instrument(skip_all, level = "debug")]
    fn dnd_finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
    ) {
        // TODO: revisit xwayland drag-and-drop.
    }

    #[instrument(skip_all, level = "debug")]
    fn action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
        _action: DndAction,
    ) {
        // TODO: revisit xwayland drag-and-drop.
    }
}

impl PrimarySelectionDeviceHandler for WprsState {
    #[instrument(skip_all, level = "debug")]
    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        primary_selection_device: &ZwpPrimarySelectionDeviceV1,
    ) {
        if self.client_state.primary_selection_manager_state.is_none() {
            return;
        }
        let primary_selection_device = &self
            .client_state
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
        if mime_types.contains(&"_xwayland_xdg_shell_marker".to_string()) {
            return;
        }
        self.client_state.primary_selection_offer = Some(offer);
        if let Some(xwm) = &mut self.compositor_state.xwm {
            xwm.new_selection(SelectionTarget::Primary, Some(mime_types))
                .log_and_ignore(loc!());
        }
    }
}

impl PrimarySelectionSourceHandler for WprsState {
    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        write_pipe: WritePipe,
    ) {
        if let Some(xwm) = &mut self.compositor_state.xwm {
            xwm.send_selection(
                SelectionTarget::Primary,
                mime,
                write_pipe.into(),
                self.event_loop_handle.clone(),
            )
            .log_and_ignore(loc!());
        }
    }

    #[instrument(skip(self, _conn, _qh), level = "debug")]
    fn cancelled(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
    ) {
    }
}

smithay_client_toolkit::delegate_compositor!(WprsState);
smithay_client_toolkit::delegate_data_device!(WprsState);
smithay_client_toolkit::delegate_keyboard!(WprsState);
smithay_client_toolkit::delegate_output!(WprsState);
smithay_client_toolkit::delegate_pointer!(WprsState);
smithay_client_toolkit::delegate_registry!(WprsState);
smithay_client_toolkit::delegate_seat!(WprsState);
smithay_client_toolkit::delegate_shm!(WprsState);
smithay_client_toolkit::delegate_subcompositor!(WprsState);
smithay_client_toolkit::delegate_xdg_popup!(WprsState);
smithay_client_toolkit::delegate_xdg_shell!(WprsState);
smithay_client_toolkit::delegate_xdg_window!(WprsState);
smithay_client_toolkit::delegate_primary_selection!(WprsState);

impl ProvidesRegistryState for WprsState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.client_state.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}

struct SubCompositorData;

impl Dispatch<WlSubcompositor, SubCompositorData> for WprsState {
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

struct SubSurfaceData;

impl Dispatch<WlSubsurface, SubSurfaceData> for WprsState {
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
