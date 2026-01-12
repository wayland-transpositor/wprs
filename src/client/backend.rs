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

use std::path::PathBuf;

use crate::client::config;
use crate::prelude::*;
use crate::protocols::wprs as proto;
use crate::protocols::wprs::Serializer;

#[derive(Debug, Clone)]
pub struct ClientBackendConfig {
    pub title_prefix: String,
    pub control_socket: PathBuf,
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
        },
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
                    },
                    Err(ConnectError::NoCompositor) => {
                        bail!("no Wayland compositor found")
                    },
                    Err(e) => return Err(anyhow!(e)),
                }
            }

            #[cfg(not(feature = "wayland-client"))]
            {
                let _ = config;
                bail!(
                    "no usable client backend available; rebuild with `--features wayland-client`"
                )
            }
        },
    }
}
