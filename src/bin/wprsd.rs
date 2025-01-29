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
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use bpaf::Parser;
use optional_struct::optional_struct;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use smithay::reexports::calloop::channel::Event;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::Interest;
use smithay::reexports::calloop::Mode;
use smithay::reexports::calloop::PostAction;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::socket::ListeningSocketSource;
use tracing::Level;
use wprs::args;
use wprs::args::Config;
use wprs::args::OptionalConfig;
use wprs::args::SerializableLevel;
use wprs::prelude::*;
use wprs::server::smithay_handlers::ClientState;
use wprs::server::WprsServerState;
use wprs_common::utils;
use wprs_protocol::serialization::Serializer;

#[optional_struct]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprsdConfig {
    // Skip serializing fields which aren't ever useful to put into a config
    // file.
    #[serde(skip_serializing)]
    print_default_config_and_exit: bool,
    #[serde(skip_serializing)]
    config_file: PathBuf,
    wayland_display: String,
    socket: PathBuf,
    framerate: u32,
    // Optional fields don't get wrapped unless we specify it ourselves
    #[optional_wrap]
    log_file: Option<PathBuf>,
    stderr_log_level: SerializableLevel,
    file_log_level: SerializableLevel,
    log_priv_data: bool,
    enable_xwayland: bool,
    xwayland_xdg_shell_path: String,
    xwayland_xdg_shell_wayland_debug: bool,
    xwayland_xdg_shell_args: Vec<String>,
    kde_server_side_decorations: bool,
}

impl Default for WprsdConfig {
    fn default() -> Self {
        Self {
            print_default_config_and_exit: false,
            config_file: args::default_config_file("wprsd"),
            wayland_display: "wprs-0".to_string(),
            socket: args::default_socket_path(),
            framerate: 60,
            log_file: None,
            stderr_log_level: SerializableLevel(Level::INFO),
            file_log_level: SerializableLevel(Level::TRACE),
            log_priv_data: false,
            enable_xwayland: true,
            xwayland_xdg_shell_path: "xwayland-xdg-shell".to_string(),
            xwayland_xdg_shell_wayland_debug: false,
            xwayland_xdg_shell_args: Vec::new(),
            kde_server_side_decorations: false,
        }
    }
}

impl Config for WprsdConfig {
    fn config_file(&self) -> PathBuf {
        self.config_file.clone()
    }
}

fn enable_xwayland() -> impl Parser<Option<bool>> {
    bpaf::long("enable-xwayland")
        .argument::<bool>("BOOL")
        .optional()
}

fn xwayland_xdg_shell_path() -> impl Parser<Option<String>> {
    bpaf::long("xwayland-xdg-shell-path")
        .argument::<String>("PATH")
        .optional()
}

fn xwayland_xdg_shell_wayland_debug() -> impl Parser<Option<bool>> {
    bpaf::long("xwayland-xdg-shell-wayland-debug")
        .argument::<bool>("BOOL")
        .optional()
}

fn xwayland_xdg_shell_args() -> impl Parser<Option<Vec<String>>> {
    bpaf::long("xwayland-xdg-shell-args")
        .argument::<String>("ARG1,ARG2,...,ARGN")
        .map(|s| s.split(',').map(str::to_string).collect::<Vec<_>>())
        .many()
        .map(|nested| nested.into_iter().flatten().collect())
        .optional()
}

fn kde_server_side_decorations() -> impl Parser<Option<bool>> {
    bpaf::long("kde-server-side-decorations")
        .argument::<bool>("BOOL")
        .help("Whether to prefer server-side decorations for applications which still use the org_kde_kwin_server_decoration_manager protocol. GTK is the only major user of that protocol and it ignores polite suggestions from the compositor about whether server-side or client-side decorations should be used, so have to configure this preference at wprsd startup. Once GTK moves to the xdg-decoration protocol, this can be removed and we can auto-detect the preference of the client compositor.")
        .optional()
}

impl OptionalConfig<WprsdConfig> for OptionalWprsdConfig {
    fn parse_args() -> Self {
        let print_default_config_and_exit = args::print_default_config_and_exit();
        let config_file = args::config_file();
        let wayland_display = args::wayland_display();
        let socket = args::socket();
        let framerate = args::framerate();
        let log_file = args::log_file();
        let stderr_log_level = args::stderr_log_level();
        let file_log_level = args::file_log_level();
        let log_priv_data = args::log_priv_data();
        let enable_xwayland = enable_xwayland();
        let xwayland_xdg_shell_path = xwayland_xdg_shell_path();
        let xwayland_xdg_shell_wayland_debug = xwayland_xdg_shell_wayland_debug();
        let xwayland_xdg_shell_args = xwayland_xdg_shell_args();
        let kde_server_side_decorations = kde_server_side_decorations();
        bpaf::construct!(Self {
            print_default_config_and_exit,
            config_file,
            wayland_display,
            socket,
            framerate,
            log_file,
            stderr_log_level,
            file_log_level,
            log_priv_data,
            enable_xwayland,
            xwayland_xdg_shell_path,
            xwayland_xdg_shell_wayland_debug,
            xwayland_xdg_shell_args,
            kde_server_side_decorations,
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
    mut display: Display<WprsServerState>,
    state: &mut WprsServerState,
    event_loop: &EventLoop<WprsServerState>,
) -> Result<()> {
    let listening_socket = ListeningSocketSource::with_name(wayland_display).location(loc!())?;
    let writer = state.serializer.writer().into_inner();
    let mut dh = display.handle();

    event_loop
        .handle()
        .insert_source(listening_socket, move |stream, _, _| {
            dh.insert_client(stream, Arc::new(ClientState::new(writer.clone())))
                .unwrap();
        })
        .location(loc!())?;

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

    Ok(())
}

fn start_xwayland_xdg_shell(
    wayland_display: &str,
    xwayland_xdg_shell_path: &str,
    xwayland_xdg_shell_wayland_debug: bool,
    xwayland_xdg_shell_args: &[String],
) {
    let mut child = Command::new(xwayland_xdg_shell_path)
        .env("WAYLAND_DISPLAY", wayland_display)
        .env(
            "WAYLAND_DEBUG",
            if xwayland_xdg_shell_wayland_debug {
                "1"
            } else {
                "0"
            },
        )
        .args(xwayland_xdg_shell_args)
        .spawn()
        .expect("failed executing xwayland-xdg-shell");

    std::thread::spawn(move || {
        child.wait().expect("failed waiting xwayland-xdg-shell");
    });
}

#[allow(clippy::missing_panics_doc)]
pub fn main() -> Result<()> {
    let config = args::init_config::<WprsdConfig, OptionalWprsdConfig>();
    args::set_log_priv_data(config.log_priv_data);
    utils::configure_tracing(
        config.stderr_log_level.0,
        config.log_file,
        config.file_log_level.0,
    )
    .location(loc!())?;
    utils::exit_on_thread_panic();

    fs::create_dir_all(config.socket.parent().location(loc!())?).location(loc!())?;
    let mut serializer = Serializer::new_server(&config.socket).location(loc!())?;
    let reader = serializer.reader().location(loc!())?;

    let mut event_loop = EventLoop::try_new().location(loc!())?;
    let display: Display<WprsServerState> = Display::new().location(loc!())?;

    let frame_interval = Duration::from_secs_f64(1.0 / (config.framerate as f64));

    let mut state = WprsServerState::new(
        display.handle(),
        event_loop.handle(),
        serializer,
        config.enable_xwayland,
        frame_interval,
        config.kde_server_side_decorations,
    );

    init_wayland_listener(&config.wayland_display, display, &mut state, &event_loop)
        .location(loc!())?;

    if config.enable_xwayland {
        start_xwayland_xdg_shell(
            &config.wayland_display,
            &config.xwayland_xdg_shell_path,
            config.xwayland_xdg_shell_wayland_debug,
            &config.xwayland_xdg_shell_args,
        );
    }

    // TODO: do this in WprsServerState::new;
    let _keyboard = state
        .seat
        .add_keyboard(Default::default(), 200, 200)
        .location(loc!())?;
    let _pointer = state.seat.add_pointer();

    event_loop
        .handle()
        .insert_source(reader, |event, _metadata, state| {
            match event {
                Event::Msg(msg) => state.handle_event(msg),
                Event::Closed => {
                    unreachable!("reader is an in-memory channel whose write end has the same lifetime as serializer: the lifetime of the program.")
                },
            }
        }).unwrap();

    event_loop
        .run(None, &mut state, move |state| {
            state.dh.flush_clients().unwrap();
        })
        .location(loc!())?;

    Ok(())
}
