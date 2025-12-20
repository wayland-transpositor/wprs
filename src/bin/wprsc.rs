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
use wprs::client::ClientBackendConfig;
use wprs::client::build_client_backend;
use wprs::client::config::WprscArgs;
use wprs::config;
use wprs::prelude::*;
use wprs::protocols::wprs as proto;
use wprs::protocols::wprs::Serializer;
use wprs::utils;

fn main() -> Result<()> {
    let args = WprscArgs::parse();
    let config = args.load_config().location(loc!())?;

    config::set_log_priv_data(config.log_priv_data);
    utils::configure_tracing(
        config.stderr_log_level.0,
        config.log_file.clone(),
        config.file_log_level.0,
    )
    .location(loc!())?;
    utils::exit_on_thread_panic();

    if config.forward_only {
        let endpoint = config
            .endpoint
            .clone()
            .ok_or_else(|| anyhow!("--forward-only requires --endpoint=ssh://..."))
            .location(loc!())?;

        let (local_endpoint, guard) = proto::setup_client_transport(endpoint).location(loc!())?;
        let _guard = guard
            .ok_or_else(|| anyhow!("--forward-only requires an ssh:// endpoint"))
            .location(loc!())?;

        println!("{local_endpoint}");
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    let serializer_options = proto::SerializerClientOptions {
        auto_reconnect: config.auto_reconnect,
        on_connect: vec![proto::SendType::Object(proto::Event::WprsClientConnect)],
    };

    let serializer: Serializer<proto::Event, proto::Request> = match &config.endpoint {
        Some(endpoint) => {
            Serializer::new_client_endpoint_with_options(endpoint.clone(), serializer_options)
                .with_context(loc!(), || {
                    format!("Serializer failed to initialize for endpoint {endpoint:?}.")
                })?
        }
        None => {
            fs::create_dir_all(config.socket.parent().location(loc!())?).location(loc!())?;
            Serializer::new_client_with_options(&config.socket, serializer_options)
                .with_context(loc!(), || {
                    format!("Serializer failed to initialize for socket {:?}.", &config.socket)
                })?
        }
    };

    let backend = build_client_backend(
        config.backend,
        ClientBackendConfig {
            title_prefix: config.title_prefix.clone(),
            control_socket: config.control_socket.clone(),
            keyboard_mode: config.keyboard_mode,
            xkb_keymap_file: config.xkb_keymap_file.clone(),
        },
    )
    .location(loc!())?;

    info!("wprsc using backend: {}", backend.name());
    backend.run(serializer).location(loc!())
}
