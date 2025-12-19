use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use crate::server::runtime::backend::BackendObservation;
use crate::server::runtime::backend::PollingBackend;
use crate::server::runtime::backend::SurfaceSnapshot;
use crate::prelude::*;
use crate::protocols::wprs::Capabilities;
use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::wayland::Buffer;
use crate::protocols::wprs::wayland::BufferAssignment;
use crate::protocols::wprs::wayland::BufferData;
use crate::protocols::wprs::wayland::BufferFormat;
use crate::protocols::wprs::wayland::BufferMetadata;
use crate::protocols::wprs::wayland::Role;
use crate::protocols::wprs::wayland::SurfaceState;
use crate::protocols::wprs::wayland::WlSurfaceId;
use crate::protocols::wprs::xdg_shell::XdgToplevelId;
use crate::protocols::wprs::xdg_shell::XdgToplevelState;

pub mod patterns;

#[derive(Debug, Clone)]
pub struct MockOptions {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub windows: u32,
    pub title: String,
    pub socket_path: PathBuf,
}

impl MockOptions {
    pub fn parse(prefix: &str) -> Self {
        MockOptionsCli::parse().into_options(prefix)
    }

    pub fn defaults_with_socket_prefix(prefix: &str) -> Self {
        let socket_path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));

        Self {
            width: 512,
            height: 512,
            fps: 30,
            windows: 1,
            title: "wprs mock".to_string(),
            socket_path,
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "wprs mock backend", long_about = None)]
struct MockOptionsCli {
    /// Surface width (pixels)
    #[arg(long, default_value_t = 512)]
    width: u32,
    /// Surface height (pixels)
    #[arg(long, default_value_t = 512)]
    height: u32,
    /// Frames per second
    #[arg(long, default_value_t = 30)]
    fps: u32,
    /// Number of toplevel windows to simulate
    #[arg(long, default_value_t = 1)]
    windows: u32,
    /// Window title
    #[arg(long, default_value = "wprs mock")]
    title: String,
    /// Unix socket path to bind (default: a fresh temp path)
    #[arg(long)]
    socket: Option<PathBuf>,
}

impl MockOptionsCli {
    fn into_options(self, prefix: &str) -> MockOptions {
        let mut opts = MockOptions::defaults_with_socket_prefix(prefix);
        opts.width = self.width;
        opts.height = self.height;
        opts.fps = self.fps;
        opts.windows = self.windows;
        opts.title = self.title;
        if let Some(socket) = self.socket {
            opts.socket_path = socket;
        }
        opts
    }
}

#[derive(Debug, Clone)]
pub struct MockSurface {
    pub client: ClientId,
    pub id: WlSurfaceId,
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl MockSurface {
    pub fn toplevel(
        client: ClientId,
        id: WlSurfaceId,
        title: String,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            client,
            id,
            title,
            width,
            height,
        }
    }

    pub fn base_state(&self) -> SurfaceState {
        SurfaceState {
            client: self.client,
            id: self.id,
            buffer: Some(BufferAssignment::New(Buffer {
                metadata: BufferMetadata {
                    width: self.width as i32,
                    height: self.height as i32,
                    stride: (self.width * 4) as i32,
                    format: BufferFormat::Argb8888,
                },
                // Filled in by the core loop.
                data: BufferData::External,
            })),
            role: Some(Role::XdgToplevel(XdgToplevelState {
                id: XdgToplevelId(1),
                parent: None,
                title: Some(self.title.clone()),
                app_id: Some("wprs-mock".to_string()),
                decoration_mode: None,
                maximized: None,
                fullscreen: None,
            })),
            buffer_scale: 1,
            buffer_transform: None,
            opaque_region: None,
            input_region: None,
            z_ordered_children: Vec::new(),
            damage: None,
            output_ids: Vec::new(),
            viewport_state: None,
            xdg_surface_state: None,
        }
    }
}

pub struct MockBackend {
    surfaces: Vec<MockSurface>,
    fps: u32,
    frame: u64,
}

impl MockBackend {
    pub fn new(options: MockOptions) -> Self {
        let windows = options.windows.max(1);
        let mut surfaces = Vec::with_capacity(windows as usize);
        for idx in 0..windows {
            let title = if windows == 1 {
                options.title.clone()
            } else {
                format!("{} #{idx}", options.title)
            };
            surfaces.push(MockSurface::toplevel(
                ClientId(1),
                WlSurfaceId((idx + 1) as u64),
                title,
                options.width,
                options.height,
            ));
        }
        Self {
            surfaces,
            fps: options.fps,
            frame: 0,
        }
    }

    pub fn tick_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(1.0 / self.fps.max(1) as f64)
    }
}

impl PollingBackend for MockBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities { xwayland: false }
    }

    fn initial_snapshot(&mut self) -> Result<Vec<SurfaceSnapshot>> {
        Ok(self
            .surfaces
            .iter()
            .map(|surface| SurfaceSnapshot {
                state: surface.base_state(),
            })
            .collect())
    }

    fn poll(&mut self) -> Result<Vec<BackendObservation>> {
        self.frame = self.frame.wrapping_add(1);

        Ok(self
            .surfaces
            .iter()
            .enumerate()
            .map(|(idx, surface)| {
                let bgra = patterns::moving_gradient_bgra(
                    surface.width,
                    surface.height,
                    self.frame.wrapping_add(idx as u64 * 37),
                );
                BackendObservation::SurfaceCommit {
                    state: surface.base_state(),
                    bgra: Some(Arc::from(bgra.into_boxed_slice())),
                }
            })
            .collect())
    }

    fn handle_client_event(&mut self, _event: Event) -> Result<()> {
        // The mock backend ignores input for now.
        Ok(())
    }
}
