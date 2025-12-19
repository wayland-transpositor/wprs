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

use crate::prelude::*;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::SendType;
use crate::protocols::wprs::wayland::DataEvent;
use crate::protocols::wprs::wayland::KeyboardEvent;
use crate::protocols::wprs::wayland::OutputEvent;
use crate::protocols::wprs::wayland::PointerEvent;
use crate::protocols::wprs::wayland::SurfaceEvent;
use crate::protocols::wprs::wayland::SurfaceRequest;
use crate::protocols::wprs::xdg_shell::PopupEvent;
use crate::protocols::wprs::xdg_shell::ToplevelEvent;

pub mod handshake;

/// Platform-neutral server core.
///
/// This module contains logic that should be testable on non-Linux platforms.
/// The Smithay adapter (Linux-only) is responsible for interacting with the
/// compositor and translating between Smithay events and protocol messages.
#[derive(Debug, Clone, Copy)]
pub struct Core {
    xwayland_enabled: bool,
}

impl Core {
    pub fn new(xwayland_enabled: bool) -> Self {
        Self { xwayland_enabled }
    }

    /// Builds the initial messages to send after a client connects.
    pub fn initial_messages(
        &self,
        surfaces: impl IntoIterator<Item = crate::protocols::wprs::wayland::SurfaceState>,
    ) -> Result<Vec<SendType<Request>>> {
        handshake::initial_messages(self.xwayland_enabled, surfaces)
    }

    /// Handles a protocol `Event` (originating from the client).
    ///
    /// `Event::WprsClientConnect` is intentionally not handled here because
    /// establishing the transport connection and producing the initial snapshot
    /// is adapter-specific.
    pub fn handle_event<B: Backend>(&self, backend: &mut B, event: Event) -> Result<()> {
        dispatch_event(backend, event)
    }
}

/// Platform-neutral backend interface for applying client events.
///
/// This is implemented by the Smithay-backed server on Linux, and can be
/// implemented by mocks for testing on non-Linux platforms.
pub trait Backend {
    fn on_toplevel_event(&mut self, event: ToplevelEvent) -> Result<()>;
    fn on_popup_event(&mut self, event: PopupEvent) -> Result<()>;
    fn on_keyboard_event(&mut self, event: KeyboardEvent) -> Result<()>;
    fn on_pointer_frame(&mut self, events: Vec<PointerEvent>) -> Result<()>;
    fn on_output_event(&mut self, event: OutputEvent) -> Result<()>;
    fn on_data_event(&mut self, event: DataEvent) -> Result<()>;
    fn on_surface_event(&mut self, event: SurfaceEvent) -> Result<()>;
}

/// Dispatches a protocol `Event` (originating from the client) into backend hooks.
///
/// `Event::WprsClientConnect` is intentionally not handled here because establishing
/// the transport connection and producing the initial snapshot is adapter-specific.
pub fn dispatch_event<B: Backend>(backend: &mut B, event: Event) -> Result<()> {
    match event {
        Event::WprsClientConnect => {
            bail!("WprsClientConnect must be handled by the transport adapter")
        },
        Event::Toplevel(event) => backend.on_toplevel_event(event).location(loc!())?,
        Event::Popup(event) => backend.on_popup_event(event).location(loc!())?,
        Event::KeyboardEvent(event) => backend.on_keyboard_event(event).location(loc!())?,
        Event::PointerFrame(events) => backend.on_pointer_frame(events).location(loc!())?,
        Event::Output(event) => backend.on_output_event(event).location(loc!())?,
        Event::Data(event) => backend.on_data_event(event).location(loc!())?,
        Event::Surface(event) => backend.on_surface_event(event).location(loc!())?,
    }
    Ok(())
}

/// Helper for creating a `SurfaceRequest` for a `SurfaceState` payload.
pub fn surface_request_from_state(
    state: crate::protocols::wprs::wayland::SurfaceState,
) -> SurfaceRequest {
    SurfaceRequest {
        client: state.client,
        surface: state.id,
        payload: crate::protocols::wprs::wayland::SurfaceRequestPayload::Commit(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::wprs::wayland::DataEvent;
    use crate::protocols::wprs::wayland::DataSourceEvent;
    use crate::protocols::wprs::wayland::ModifierState;
    use crate::protocols::wprs::wayland::Output;
    use crate::protocols::wprs::wayland::OutputEvent;
    use crate::protocols::wprs::wayland::OutputInfo;
    use crate::protocols::wprs::wayland::PointerEvent;
    use crate::protocols::wprs::wayland::PointerEventKind;
    use crate::protocols::wprs::wayland::Subpixel;
    use crate::protocols::wprs::wayland::SurfaceEvent;
    use crate::protocols::wprs::wayland::SurfaceEventPayload;
    use crate::protocols::wprs::wayland::Transform;
    use crate::protocols::wprs::wayland::WlSurfaceId;
    use crate::protocols::wprs::wayland::{AxisScroll, AxisSource, KeyboardEvent, Mode};
    use crate::protocols::wprs::xdg_shell::{ToplevelClose, ToplevelEvent};

    #[derive(Default)]
    struct MockBackend {
        toplevel_events: Vec<ToplevelEvent>,
        popup_events: usize,
        keyboard_events: Vec<KeyboardEvent>,
        pointer_frames: Vec<Vec<PointerEvent>>,
        output_events: Vec<OutputEvent>,
        data_events: Vec<DataEvent>,
        surface_events: Vec<SurfaceEvent>,
    }

    impl Backend for MockBackend {
        fn on_toplevel_event(&mut self, event: ToplevelEvent) -> Result<()> {
            self.toplevel_events.push(event);
            Ok(())
        }

        fn on_popup_event(&mut self, _event: PopupEvent) -> Result<()> {
            self.popup_events += 1;
            Ok(())
        }

        fn on_keyboard_event(&mut self, event: KeyboardEvent) -> Result<()> {
            self.keyboard_events.push(event);
            Ok(())
        }

        fn on_pointer_frame(&mut self, events: Vec<PointerEvent>) -> Result<()> {
            self.pointer_frames.push(events);
            Ok(())
        }

        fn on_output_event(&mut self, event: OutputEvent) -> Result<()> {
            self.output_events.push(event);
            Ok(())
        }

        fn on_data_event(&mut self, event: DataEvent) -> Result<()> {
            self.data_events.push(event);
            Ok(())
        }

        fn on_surface_event(&mut self, event: SurfaceEvent) -> Result<()> {
            self.surface_events.push(event);
            Ok(())
        }
    }

    fn dummy_output_info(id: u32) -> OutputInfo {
        OutputInfo {
            id,
            model: "model".to_string(),
            make: "make".to_string(),
            location: (0, 0).into(),
            physical_size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            transform: Transform::Normal,
            scale_factor: 1,
            mode: Mode {
                dimensions: (0, 0).into(),
                refresh_rate: 60_000,
                current: true,
                preferred: true,
            },
            name: None,
            description: None,
        }
    }

    #[test]
    fn dispatch_rejects_connect() {
        let mut backend = MockBackend::default();
        let err = dispatch_event(&mut backend, Event::WprsClientConnect).unwrap_err();
        assert!(err.to_string().contains("transport adapter"));
    }

    #[test]
    fn dispatch_routes_all_event_variants() {
        let mut backend = MockBackend::default();

        dispatch_event(
            &mut backend,
            Event::Toplevel(ToplevelEvent::Close(ToplevelClose {
                surface_id: WlSurfaceId(1),
            })),
        )
        .unwrap();

        dispatch_event(
            &mut backend,
            Event::KeyboardEvent(KeyboardEvent::Modifiers {
                modifier_state: ModifierState {
                    ctrl: false,
                    alt: false,
                    shift: false,
                    caps_lock: false,
                    logo: false,
                    num_lock: false,
                },
                layout_index: 0,
            }),
        )
        .unwrap();

        dispatch_event(
            &mut backend,
            Event::PointerFrame(vec![PointerEvent {
                surface_id: WlSurfaceId(2),
                position: (1.0, 2.0).into(),
                kind: PointerEventKind::Axis {
                    horizontal: AxisScroll {
                        absolute: 0.0,
                        discrete: 0,
                        stop: false,
                    },
                    vertical: AxisScroll {
                        absolute: 0.0,
                        discrete: 0,
                        stop: false,
                    },
                    source: Some(AxisSource::Finger),
                },
            }]),
        )
        .unwrap();

        dispatch_event(
            &mut backend,
            Event::Output(OutputEvent::New(dummy_output_info(7))),
        )
        .unwrap();

        dispatch_event(
            &mut backend,
            Event::Data(DataEvent::SourceEvent(DataSourceEvent::DnDCancelled)),
        )
        .unwrap();

        dispatch_event(
            &mut backend,
            Event::Surface(SurfaceEvent {
                surface_id: WlSurfaceId(3),
                payload: SurfaceEventPayload::OutputsChanged(vec![Output { id: 7 }]),
            }),
        )
        .unwrap();

        assert_eq!(backend.toplevel_events.len(), 1);
        assert_eq!(backend.keyboard_events.len(), 1);
        assert_eq!(backend.pointer_frames.len(), 1);
        assert_eq!(backend.output_events.len(), 1);
        assert_eq!(backend.data_events.len(), 1);
        assert_eq!(backend.surface_events.len(), 1);
    }
}
