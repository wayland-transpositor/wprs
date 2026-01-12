use crate::server::runtime::backend::BackendObservation;
use crate::server::runtime::backend::PollingBackend;
use crate::server::runtime::backend::SurfaceSnapshot;
use crate::prelude::*;
use crate::protocols::wprs::Capabilities;
use crate::protocols::wprs::Event;

#[derive(Debug, Default)]
pub struct MacosWindowBackend;

impl MacosWindowBackend {
    pub fn new() -> Self {
        Self
    }
}

impl PollingBackend for MacosWindowBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities { xwayland: false }
    }

    fn initial_snapshot(&mut self) -> Result<Vec<SurfaceSnapshot>> {
        bail!("macOS per-window capture backend is not implemented yet")
    }

    fn poll(&mut self) -> Result<Vec<BackendObservation>> {
        bail!("macOS per-window capture backend is not implemented yet")
    }

    fn handle_client_event(&mut self, _event: Event) -> Result<()> {
        Ok(())
    }
}
