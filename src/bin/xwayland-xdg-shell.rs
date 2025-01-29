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

use std::ffi::OsString;
use std::path::PathBuf;

use bpaf::Parser;
use optional_struct::optional_struct;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::Interest;
use smithay::reexports::calloop::Mode;
use smithay::reexports::calloop::PostAction;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::socket::ListeningSocketSource;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::Connection;
use tracing::Level;
use wprs::args;
use wprs::args::Config;
use wprs::args::OptionalConfig;
use wprs::args::SerializableLevel;
use wprs::prelude::*;
use wprs::xwayland_xdg_shell::compositor::DecorationBehavior;
use wprs::xwayland_xdg_shell::compositor::XwaylandOptions;
use wprs::xwayland_xdg_shell::WprsState;
use wprs_common::utils;

#[optional_struct]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct XwaylandXdgShellConfig {
    // Skip serializing fields which aren't ever useful to put into a config
    // file.
    #[serde(skip_serializing)]
    print_default_config_and_exit: bool,
    #[serde(skip_serializing)]
    config_file: PathBuf,
    wayland_display: String,
    display: u32,
    // Optional fields don't get wrapped unless we specify it ourselves
    #[optional_wrap]
    log_file: Option<PathBuf>,
    stderr_log_level: SerializableLevel,
    file_log_level: SerializableLevel,
    log_priv_data: bool,
    xwayland_wayland_debug: bool,
    decoration_behavior: DecorationBehavior,
}

impl Default for XwaylandXdgShellConfig {
    fn default() -> Self {
        Self {
            print_default_config_and_exit: false,
            config_file: args::default_config_file("xwayland-xdg-shell"),
            wayland_display: "xwayland-xdg-shell-0".to_string(),
            display: 100,
            log_file: None,
            stderr_log_level: SerializableLevel(Level::INFO),
            file_log_level: SerializableLevel(Level::TRACE),
            log_priv_data: false,
            xwayland_wayland_debug: false,
            decoration_behavior: DecorationBehavior::Auto,
        }
    }
}

impl Config for XwaylandXdgShellConfig {
    fn config_file(&self) -> PathBuf {
        self.config_file.clone()
    }
}

fn display() -> impl Parser<Option<u32>> {
    bpaf::long("display").argument::<u32>("NUM").optional()
}

fn xwayland_wayland_debug() -> impl Parser<Option<bool>> {
    bpaf::long("xwayland-wayland-debug")
        .argument::<bool>("BOOL")
        .optional()
}

fn decoration_behavior() -> impl Parser<Option<DecorationBehavior>> {
    bpaf::long("decoration-behavior")
        .argument::<String>("Auto|AlwaysEnabled|AlwaysDisabled")
        .parse(|s| ron::from_str(&s))
        .optional()
}

impl OptionalConfig<XwaylandXdgShellConfig> for OptionalXwaylandXdgShellConfig {
    fn parse_args() -> Self {
        let print_default_config_and_exit = args::print_default_config_and_exit();
        let config_file = args::config_file();
        let wayland_display = args::wayland_display();
        let display = display();
        let log_file = args::log_file();
        let stderr_log_level = args::stderr_log_level();
        let file_log_level = args::file_log_level();
        let log_priv_data = args::log_priv_data();
        let xwayland_wayland_debug = xwayland_wayland_debug();
        let decoration_behavior = decoration_behavior();
        bpaf::construct!(Self {
            print_default_config_and_exit,
            config_file,
            wayland_display,
            display,
            log_file,
            stderr_log_level,
            file_log_level,
            log_priv_data,
            xwayland_wayland_debug,
            decoration_behavior,
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

fn init_wayland_listener(
    wayland_display: &str,
    mut display: Display<WprsState>,
    event_loop: &EventLoop<WprsState>,
) -> Result<OsString> {
    let listening_socket = ListeningSocketSource::with_name(wayland_display).location(loc!())?;
    let socket_name = listening_socket.socket_name().to_os_string();

    event_loop
        .handle()
        .insert_source(
            Generic::new(
                display.backend().poll_fd().try_clone_to_owned().unwrap(),
                Interest::READ,
                Mode::Level,
            ),
            move |_, _, state| {
                display.dispatch_clients(state).unwrap();
                Ok(PostAction::Continue)
            },
        )
        .location(loc!())?;

    Ok(socket_name)
}

#[allow(clippy::missing_panics_doc)]
pub fn main() -> Result<()> {
    let config = args::init_config::<XwaylandXdgShellConfig, OptionalXwaylandXdgShellConfig>();
    args::set_log_priv_data(config.log_priv_data);
    utils::configure_tracing(
        config.stderr_log_level.0,
        config.log_file,
        config.file_log_level.0,
    )
    .location(loc!())?;
    utils::exit_on_thread_panic();

    let mut event_loop = EventLoop::try_new().location(loc!())?;
    let display: Display<WprsState> = Display::new().location(loc!())?;

    let conn = Connection::connect_to_env().location(loc!())?;
    let (globals, event_queue) = registry_queue_init(&conn).location(loc!())?;

    let xwayland_options = XwaylandOptions {
        env: vec![(
            "WAYLAND_DEBUG",
            if config.xwayland_wayland_debug {
                "1"
            } else {
                "0"
            },
        )],
        display: Some(config.display),
    };

    let mut state = WprsState::new(
        display.handle(),
        &globals,
        event_queue.handle(),
        conn.clone(),
        event_loop.handle(),
        config.decoration_behavior,
        xwayland_options,
    )
    .location(loc!())?;

    init_wayland_listener(&config.wayland_display, display, &event_loop).location(loc!())?;

    let seat = &mut state.compositor_state.seat;
    // TODO: do this in WprsState::new;
    let _keyboard = seat
        .add_keyboard(Default::default(), 200, 200)
        .location(loc!())?;
    let _pointer = seat.add_pointer();

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .location(loc!())?;

    event_loop
        .run(None, &mut state, move |state| {
            state.dh.flush_clients().unwrap();
        })
        .context(loc!(), "Error starting event loop.")?;
    Ok(())
}
