use std::sync::Arc;

use crate::prelude::*;
use crate::protocols::wprs::Capabilities;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::Serializer;
use crate::protocols::wprs::wayland::SurfaceState;

#[derive(Debug, Clone)]
pub struct SurfaceSnapshot {
    pub state: SurfaceState,
}

#[derive(Debug, Clone)]
pub enum BackendObservation {
    /// A surface commit, optionally carrying a full BGRA frame to be sent.
    ///
    /// If `bgra` is present, the core loop will compress it, emit a `RawBuffer`,
    /// and send a `Surface(Commit)` with buffer data externalized.
    SurfaceCommit {
        state: SurfaceState,
        bgra: Option<Arc<[u8]>>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TickMode {
    /// Backend is driven by a periodic timer.
    Polling,
    /// Backend runs its own event loop and does not require polling.
    EventDriven,
}

/// Polling-style backend.
///
/// These backends are driven by the shared `server::runtime::run_loop` and are
/// polled on a fixed interval.
pub trait PollingBackend {
    fn capabilities(&self) -> Capabilities;

    fn initial_snapshot(&mut self) -> Result<Vec<SurfaceSnapshot>>;

    fn poll(&mut self) -> Result<Vec<BackendObservation>>;

    fn handle_client_event(&mut self, event: Event) -> Result<()>;
}

/// Unified server backend interface.
///
/// - Polling/capture backends should implement `PollingBackend`.
/// - Event-driven backends (eg. a Wayland compositor) should implement `ServerBackend`
///   directly.
pub trait ServerBackend {
    fn tick_mode(&self) -> TickMode;

    fn run(
        self: Box<Self>,
        serializer: Serializer<Request, Event>,
        tick_interval: Option<std::time::Duration>,
    ) -> Result<()>;
}

impl<T: PollingBackend + 'static> ServerBackend for T {
    fn tick_mode(&self) -> TickMode {
        TickMode::Polling
    }

    fn run(
        self: Box<Self>,
        serializer: Serializer<Request, Event>,
        tick_interval: Option<std::time::Duration>,
    ) -> Result<()> {
        let tick_interval = tick_interval
            .ok_or_else(|| anyhow!("polling backend requires tick_interval"))
            .location(loc!())?;
        crate::server::runtime::run_loop::run(*self, serializer, tick_interval).location(loc!())
    }
}
