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

#[cfg(target_os = "linux")]
use std::ffi::OsString;

#[cfg(target_os = "linux")]
use calloop::RegistrationToken;
#[cfg(target_os = "linux")]
use calloop::signals::Signal;
#[cfg(target_os = "linux")]
use calloop::signals::Signals;
#[cfg(target_os = "linux")]
use smithay::reexports::calloop;
#[cfg(target_os = "linux")]
use smithay::reexports::calloop::EventLoop;
#[cfg(target_os = "linux")]
use smithay::reexports::calloop::Interest;
#[cfg(target_os = "linux")]
use smithay::reexports::calloop::Mode;
#[cfg(target_os = "linux")]
use smithay::reexports::calloop::PostAction;
#[cfg(target_os = "linux")]
use smithay::reexports::calloop::generic::Generic;
#[cfg(target_os = "linux")]
use smithay::reexports::wayland_server::Display;
#[cfg(target_os = "linux")]
use smithay::wayland::socket::ListeningSocketSource;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::client::Connection;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
#[cfg(target_os = "linux")]
use wprs::config;
#[cfg(target_os = "linux")]
use wprs::prelude::*;
#[cfg(target_os = "linux")]
use wprs::server::config::xwayland_xdg_shell::XwaylandXdgShellArgs;
#[cfg(target_os = "linux")]
use wprs::server::config::xwayland_xdg_shell::XwaylandXdgShellConfig;
#[cfg(target_os = "linux")]
use wprs::utils;
#[cfg(target_os = "linux")]
use wprs::xwayland_xdg_shell::WprsState;
#[cfg(target_os = "linux")]
use wprs::xwayland_xdg_shell::compositor::XwaylandOptions;

#[cfg(not(target_os = "linux"))]
compile_error!("xwayland-xdg-shell is only supported on Linux targets.");

#[cfg(target_os = "linux")]
fn init_wayland_listener(
    wayland_display: &str,
    mut display: Display<WprsState>,
    event_loop: &EventLoop<WprsState>,
    registration_tokens: &mut Vec<RegistrationToken>,
) -> Result<OsString> {
    let listening_socket = ListeningSocketSource::with_name(wayland_display).location(loc!())?;
    let socket_name = listening_socket.socket_name().to_os_string();

    let token = event_loop
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

    registration_tokens.push(token);

    Ok(socket_name)
}

#[cfg(target_os = "linux")]
#[allow(clippy::missing_panics_doc)]
fn main() -> Result<()> {
    let args = XwaylandXdgShellArgs::parse();
    let config: XwaylandXdgShellConfig = args.load_config().location(loc!())?;

    config::set_log_priv_data(config.log_priv_data);
    utils::configure_tracing(
        config.stderr_log_level.0,
        config.log_file.clone(),
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
            if config.xwayland_wayland_debug { "1" } else { "0" },
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

    let mut registration_tokens = vec![];

    let wayland_socket_name = init_wayland_listener(
        &config.wayland_display,
        display,
        &event_loop,
        &mut registration_tokens,
    )
    .location(loc!())?;
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .location(loc!())?;

    {
        let token = event_loop
            .handle()
            .insert_source(
                Signals::new([Signal::SIGINT, Signal::SIGTERM]).unwrap(),
                |event, _, state| {
                    for sig in event.signals() {
                        match sig {
                            Signal::SIGINT | Signal::SIGTERM => {
                                info!("received signal {sig:?}, exiting");
                                state.should_exit = true;
                            },
                            _ => {},
                        }
                    }
                },
            )
            .location(loc!())?;
        registration_tokens.push(token);
    }

    info!("xwayland-xdg-shell wayland socket: {wayland_socket_name:?}");
    event_loop.run(None, &mut state, |_| {}).location(loc!())
}
