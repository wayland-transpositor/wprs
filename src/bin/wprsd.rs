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
use std::time::Duration;

use clap::Parser;
use wprs::config;
use wprs::prelude::*;
use wprs::protocols::wprs::Event as ProtoEvent;
use wprs::protocols::wprs::Request as ProtoRequest;
use wprs::protocols::wprs::Serializer;
use wprs::server::backends;
use wprs::server::config::WprsdArgs;
use wprs::server::config::WprsdBackend;
use wprs::server::config::WprsdConfig;
use wprs::server::runtime::backend::ServerBackend;
use wprs::server::runtime::backend::TickMode;
use wprs::utils;

fn infer_backend(config: &WprsdConfig) -> WprsdBackend {
    if let Some(backend) = config.backend {
        return backend;
    }

    if cfg!(feature = "server") {
        return WprsdBackend::Wayland;
    }

    if cfg!(target_os = "macos") {
        return WprsdBackend::MacosFullscreen;
    }
    if cfg!(windows) {
        return WprsdBackend::WindowsFullscreen;
    }
    if cfg!(unix) {
        return WprsdBackend::X11Fullscreen;
    }

    WprsdBackend::WindowsFullscreen
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

fn make_server_serializer(config: &WprsdConfig) -> Result<Serializer<ProtoRequest, ProtoEvent>> {
    match &config.endpoint {
        Some(endpoint) => Serializer::new_server_endpoint(endpoint.clone()).location(loc!()),
        None => Serializer::new_server(&config.socket).location(loc!()),
    }
}

fn run_selected_backend(config: &WprsdConfig) -> Result<()> {
    let serializer = make_server_serializer(config).location(loc!())?;

    let backend_kind = infer_backend(config);
    let backend = build_backend(&backend_kind, config).location(loc!())?;

    let tick_interval = match backend.tick_mode() {
        TickMode::Polling => Some(Duration::from_secs_f64(
            1.0 / (config.framerate.max(1) as f64),
        )),
        TickMode::EventDriven => None,
    };

    backend.run(serializer, tick_interval).location(loc!())
}

fn build_backend(backend: &WprsdBackend, config: &WprsdConfig) -> Result<Box<dyn ServerBackend>> {
    match backend {
        WprsdBackend::X11Fullscreen => Ok(Box::new(
            backends::x11::X11FullscreenBackend::connect(config.x11_title.clone()).location(loc!())?,
        )),
        WprsdBackend::WindowsFullscreen => Ok(Box::new(backends::windows::WindowsFullscreenBackend::new())),
        WprsdBackend::MacosFullscreen => Ok(Box::new(backends::macos::MacosFullscreenBackend::new())),
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
                            xwayland_xdg_shell_wayland_debug: config.xwayland_xdg_shell_wayland_debug,
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
        }
    }
}
