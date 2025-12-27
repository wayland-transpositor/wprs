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

use std::fs;
use wprs::config;
use wprs::prelude::*;
use wprs::protocols::wprs as protocol;
use wprs::protocols::wprs::Serializer;
use wprs::server::backends;
use wprs::server::backends::ServerBackend;
use wprs::server::config::WprsdArgs;
use wprs::server::config::WprsdBackend;
use wprs::server::config::WprsdConfig;
use wprs::utils;

fn infer_backend(config: &WprsdConfig) -> WprsdBackend {
    config.backend.unwrap_or(WprsdBackend::Wayland)
}

fn main() -> Result<()> {
    let args = WprsdArgs::parse();
    let config = args.load_config().location(loc!())?;

    config::set_log_priv_data(config.log_priv_data);
    utils::configure_tracing(
        config.stderr_log_level.0,
        config.log_file.clone(),
        config.file_log_level.0,
    )
    .location(loc!())?;
    utils::exit_on_thread_panic();

    if config.endpoint.is_none() {
        fs::create_dir_all(config.socket.parent().location(loc!())?).location(loc!())?;
    }

    run_selected_backend(&config).location(loc!())
}

fn make_server_serializer(
    config: &WprsdConfig,
) -> Result<Serializer<protocol::Request, protocol::Event>> {
    match &config.endpoint {
        Some(endpoint) => Serializer::new_server_endpoint(endpoint.clone()).location(loc!()),
        None => Serializer::new_server(&config.socket).location(loc!()),
    }
}

fn run_selected_backend(config: &WprsdConfig) -> Result<()> {
    let serializer = make_server_serializer(config).location(loc!())?;

    let backend_kind = infer_backend(config);
    let backend = build_backend(&backend_kind, config).location(loc!())?;
    backend.run(serializer).location(loc!())
}

fn build_backend(
    backend: &WprsdBackend,
    config: &WprsdConfig,
) -> Result<Box<dyn ServerBackend>> {
    match backend {
        WprsdBackend::Wayland => {
            #[cfg(feature = "server")]
            {
                Ok(Box::new(
                    backends::wayland::backend::WaylandSmithayBackend::new(
                        backends::wayland::backend::WaylandSmithayBackendConfig {
                            wayland_display: config.wayland_display.clone(),
                            framerate: config.framerate,
                            enable_xwayland: config.enable_xwayland,
                            xwayland_xdg_shell_path: config.xwayland_xdg_shell_path.clone(),
                            xwayland_xdg_shell_wayland_debug: config
                                .xwayland_xdg_shell_wayland_debug,
                            xwayland_xdg_shell_args: config.xwayland_xdg_shell_args.clone(),
                            kde_server_side_decorations: config.kde_server_side_decorations,
                        },
                    ),
                ))
            }
            #[cfg(not(feature = "server"))]
            {
                let _ = config;
                bail!("wayland backend requires building wprsd with `--features server`")
            }
        },
        other => {
            let _ = config;
            bail!("unsupported wprsd backend in this build: {other:?}")
        },
    }
}
