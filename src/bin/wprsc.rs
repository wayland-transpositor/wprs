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
use std::path::PathBuf;

use bpaf::Parser;
use futures_util::future::try_join;
use futures_util::stream;
use futures_util::StreamExt;
use optional_struct::optional_struct;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use smithay::reexports::calloop::channel::Event;
use smithay::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::ConnectError;
use smithay_client_toolkit::reexports::client::Connection;
use tracing::Level;
use wprs::args;
use wprs::args::Config;
use wprs::args::OptionalConfig;
use wprs::args::SerializableLevel;
use wprs::client::ClientOptions;
use wprs::client::WprsClientState;
use wprs::control_server;
use wprs::dbus::NotificationSignals;
use wprs::dbus::NotificationsProxy;
use wprs::prelude::*;
use wprs::serialization;
use wprs::serialization::Serializer;
use wprs::utils;

#[optional_struct]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprscConfig {
    // Skip serializing fields which aren't ever useful to put into a config
    // file.
    #[serde(skip_serializing)]
    print_default_config_and_exit: bool,
    #[serde(skip_serializing)]
    config_file: PathBuf,
    pub socket: PathBuf,
    pub control_socket: PathBuf,
    // Optional fields don't get wrapped unless we specify it ourselves
    #[optional_wrap]
    pub log_file: Option<PathBuf>,
    pub stderr_log_level: SerializableLevel,
    pub file_log_level: SerializableLevel,
    pub log_priv_data: bool,
    pub title_prefix: String,
}

impl Default for WprscConfig {
    fn default() -> Self {
        Self {
            print_default_config_and_exit: false,
            config_file: args::default_config_file("wprsc"),
            socket: args::default_socket_path(),
            control_socket: args::default_control_socket_path("wprsc"),
            log_file: None,
            stderr_log_level: SerializableLevel(Level::INFO),
            file_log_level: SerializableLevel(Level::TRACE),
            log_priv_data: false,
            title_prefix: String::new(),
        }
    }
}

impl Config for WprscConfig {
    fn config_file(&self) -> PathBuf {
        self.config_file.clone()
    }
}

impl OptionalConfig<WprscConfig> for OptionalWprscConfig {
    fn parse_args() -> Self {
        let print_default_config_and_exit = args::print_default_config_and_exit();
        let config_file = args::config_file();
        let socket = args::socket();
        let control_socket = args::control_socket();
        let log_file = args::log_file();
        let stderr_log_level = args::stderr_log_level();
        let file_log_level = args::file_log_level();
        let log_priv_data = args::log_priv_data();
        let title_prefix = args::title_prefix();
        bpaf::construct!(Self {
            print_default_config_and_exit,
            config_file,
            socket,
            control_socket,
            log_file,
            stderr_log_level,
            file_log_level,
            log_priv_data,
            title_prefix,
        })
        .to_options()
        .run()
    }

    fn print_default_config_and_exit(&self) -> Option<bool> {
        self.print_default_config_and_exit
    }

    fn config_file(&self) -> Option<PathBuf> {
        self.config_file.clone()
    }
}

fn main() -> Result<()> {
    let config = args::init_config::<WprscConfig, OptionalWprscConfig>();
    args::set_log_priv_data(config.log_priv_data);
    utils::configure_tracing(
        config.stderr_log_level.0,
        config.log_file,
        config.file_log_level.0,
    )
    .location(loc!())?;
    utils::exit_on_thread_panic();

    let conn = Connection::connect_to_env().map_err(|e| match e {
        // give a more helpful/actionable message, since people who aren't familiar with wayland will run into this
        ConnectError::NoCompositor => {
            anyhow!("{e}, make sure you're running wprs from a wayland desktop environment")
        },
        _ => anyhow!(e),
    })?;

    let (globals, event_queue) = registry_queue_init(&conn)?;

    fs::create_dir_all(config.socket.parent().location(loc!())?).location(loc!())?;
    let mut serializer = Serializer::new_client(&config.socket).with_context(loc!(), || {
        format!(
            "Serializer unable to connect to socket {:?}.",
            &config.socket
        )
    })?;
    let reader = serializer.reader().location(loc!())?;
    let writer = serializer.writer();
    writer.send(serialization::SendType::Object(
        serialization::Event::WprsClientConnect,
    ));

    let options = ClientOptions {
        title_prefix: config.title_prefix,
    };

    let (exec, sched) = calloop::futures::executor().context(
        loc!(),
        "failed to get calloop async executor for notifications",
    )?;

    let dbus_connection = async_io::block_on(zbus::Connection::session())
        .context(loc!(), "failed to block on notification proxy connect")?;

    let notification_proxy = async_io::block_on(NotificationsProxy::new(&dbus_connection))
        .context(loc!(), "failed to create notification proxy")?;

    let mut state = WprsClientState::new(
        event_queue.handle(),
        globals,
        conn.clone(),
        serializer,
        sched.clone(),
        notification_proxy.clone(),
        options,
    )
    .location(loc!())?;

    let (sender, receiver) = calloop::channel::channel();

    {
        let notification_proxy = notification_proxy.clone();
        sched.schedule(async move {
            let (notification_close_stream, action_stream) = match try_join(
                notification_proxy.receive_notification_closed(),
                notification_proxy.receive_action_invoked(),
            )
            .await
            {
                Ok(result) => result,
                Err(err) => {
                    error!("failed to get signal streams: {:?}", err);
                    return;
                },
            };

            let mut signal_stream = stream::select(
                notification_close_stream.map(NotificationSignals::try_from),
                action_stream.map(NotificationSignals::try_from),
            );

            while let Some(signal) = signal_stream.next().await {
                match signal {
                    Ok(signal) => {
                        if let Err(err) = sender.send(signal) {
                            error!("failed to send signal to channel: {:?}", err);
                        };
                    },
                    Err(err) => {
                        error!("failed to get signal: {:?}", err);
                    },
                }
            }
        })?;
    }

    let mut event_loop = EventLoop::try_new()?;

    let handle = event_loop.handle();

    handle
        .insert_source(receiver, |event, _metadata, state: &mut WprsClientState| {
            if let Event::Msg(signal) = event {
                state.handle_notification_signal(signal)
            }
        })
        .unwrap();

    handle
        .insert_source(exec, |_event, _metadata, _state: &mut WprsClientState| {})
        .unwrap();

    handle.insert_source(
        reader,
        |event, _metadata, state: &mut WprsClientState| {
            match event {
                Event::Msg(msg) => state.handle_request(msg),
                Event::Closed => {
                    unreachable!("serialization::client_loop terminates the process when the server disconnects.");
                },
            }
        },
    ).unwrap();

    {
        let capabilities = state.capabilities.clone();
        control_server::start(config.control_socket, move |input: &str| {
            Ok(match input {
                // TODO: make the input use json when we have more commands
                "caps" => serde_json::to_string(&capabilities.get())
                    .expect("a map with non-string keys was added to Capabilities"),
                _ => {
                    bail!("Unknown command: {input:?}")
                },
            })
        })
        .location(loc!())?;
    }

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .location(loc!())?;

    event_loop.run(None, &mut state, |_| {}).location(loc!())?;

    Ok(())
}
