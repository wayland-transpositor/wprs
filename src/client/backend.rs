use std::path::PathBuf;

use crate::client::config;
use crate::prelude::*;
use crate::protocols::wprs as proto;
use crate::protocols::wprs::Serializer;

#[derive(Debug, Clone)]
pub struct ClientBackendConfig {
    pub title_prefix: String,
    pub control_socket: PathBuf,
    pub keyboard_mode: config::KeyboardMode,
    pub xkb_keymap_file: Option<PathBuf>,
}

pub trait ClientBackend {
    fn name(&self) -> &'static str;

    fn run(self: Box<Self>, serializer: Serializer<proto::Event, proto::Request>) -> Result<()>;
}

pub fn build_client_backend(
    requested: config::ClientBackend,
    config: ClientBackendConfig,
) -> Result<Box<dyn ClientBackend>> {
    match requested {
        config::ClientBackend::Wayland => {
            #[cfg(feature = "wayland-client")]
            {
                Ok(Box::new(
                    crate::client::backends::wayland::WaylandClientBackend::connect_to_env(config)
                        .location(loc!())?,
                ))
            }

            #[cfg(not(feature = "wayland-client"))]
            {
                let _ = config;
                bail!(
                    "Wayland backend requested but not compiled in. Rebuild with `--features wayland-client`."
                )
            }
        }
        config::ClientBackend::WinitWgpu => {
            #[cfg(feature = "winit-wgpu-client")]
            {
                Ok(Box::new(
                    crate::client::backends::winit_wgpu::WinitWgpuClientBackend::new(config),
                ))
            }

            #[cfg(not(feature = "winit-wgpu-client"))]
            {
                let _ = config;
                bail!(
                    "winit-wgpu backend requested but not compiled in. Rebuild with `--features winit-wgpu-client`."
                )
            }
        }
        config::ClientBackend::Auto => {
            #[cfg(feature = "wayland-client")]
            {
                use smithay_client_toolkit::reexports::client::ConnectError;
                use smithay_client_toolkit::reexports::client::Connection;

                match Connection::connect_to_env() {
                    Ok(conn) => {
                        return Ok(Box::new(
                            crate::client::backends::wayland::WaylandClientBackend::new(
                                config, conn,
                            ),
                        ));
                    }
                    Err(ConnectError::NoCompositor) => {
                        // No compositor; fall back below.
                    }
                    Err(e) => return Err(anyhow!(e)),
                }
            }

            #[cfg(feature = "winit-wgpu-client")]
            {
                Ok(Box::new(
                    crate::client::backends::winit_wgpu::WinitWgpuClientBackend::new(config),
                ))
            }

            #[cfg(not(feature = "winit-wgpu-client"))]
            {
                let _ = config;
                bail!(
                    "No usable client backend available. Enable `wayland-client` for the Wayland backend and/or `winit-wgpu-client` for the cross-platform backend."
                )
            }
        }
    }
}
