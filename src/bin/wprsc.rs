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

use wprs::client::ClientBackendConfig;
use wprs::client::build_client_backend;
use wprs::client::config::WprscArgs;
use wprs::config;
use wprs::prelude::*;
use wprs::protocols::wprs as protocol;
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

    let serializer: Serializer<protocol::Event, protocol::Request> = match &config.endpoint {
        Some(endpoint) => Serializer::new_client_endpoint(endpoint.clone())
            .with_context(loc!(), || {
                format!("Serializer unable to connect to endpoint {endpoint:?}.")
            })?,
        None => {
            fs::create_dir_all(config.socket.parent().location(loc!())?).location(loc!())?;
            Serializer::new_client(&config.socket).with_context(loc!(), || {
                format!(
                    "Serializer unable to connect to socket {:?}.",
                    &config.socket
                )
            })?
        },
    };

    let writer = serializer.writer();
    writer.send(protocol::SendType::Object(protocol::Event::WprsClientConnect));

    let backend = build_client_backend(
        config.backend,
        ClientBackendConfig {
            title_prefix: config.title_prefix.clone(),
            control_socket: config.control_socket.clone(),
        },
    )
    .location(loc!())?;

    info!("wprsc using backend: {}", backend.name());
    backend.run(serializer).location(loc!())
}
