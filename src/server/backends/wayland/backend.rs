use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::Interest;
use smithay::reexports::calloop::Mode;
use smithay::reexports::calloop::PostAction;
use smithay::reexports::calloop::channel::Event as CalloopEvent;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::socket::ListeningSocketSource;

use crate::prelude::*;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::Serializer;
use crate::server::backends::ServerBackend;
use crate::server::backends::wayland::smithay_handlers::ClientState;

use super::WprsServerState;

#[derive(Debug, Clone)]
pub struct WaylandSmithayBackendConfig {
    pub wayland_display: String,
    pub framerate: u32,
    pub enable_xwayland: bool,
    pub xwayland_xdg_shell_path: String,
    pub xwayland_xdg_shell_wayland_debug: bool,
    pub xwayland_xdg_shell_args: Vec<String>,
    pub kde_server_side_decorations: bool,
}

#[derive(Debug)]
pub struct WaylandSmithayBackend {
    config: WaylandSmithayBackendConfig,
}

impl WaylandSmithayBackend {
    pub fn new(config: WaylandSmithayBackendConfig) -> Self {
        Self { config }
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

impl ServerBackend for WaylandSmithayBackend {
    fn run(self: Box<Self>, mut serializer: Serializer<Request, Event>) -> Result<()> {
        let config = self.config;

        let reader = serializer
            .reader()
            .ok_or_else(|| anyhow!("serializer reader already taken"))
            .location(loc!())?;

        let mut event_loop = EventLoop::try_new().location(loc!())?;
        let display: Display<WprsServerState> = Display::new().location(loc!())?;

        let frame_interval = Duration::from_secs_f64(1.0 / (config.framerate.max(1) as f64));
        let dh = display.handle();

        let mut state = WprsServerState::new(
            &dh,
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

        let _keyboard = state
            .seat
            .add_keyboard(Default::default(), 200, 200)
            .location(loc!())?;
        let _pointer = state.seat.add_pointer();

        event_loop
            .handle()
            .insert_source(reader, |event, _metadata, state| {
                if let CalloopEvent::Msg(msg) = event {
                    state.handle_event(msg);
                }
            })
            .map_err(|e| anyhow!("insert_source(serializer reader) failed: {e:?}"))?;

        event_loop
            .run(None, &mut state, move |state| {
                state.dh.flush_clients().unwrap();
            })
            .location(loc!())?;

        Ok(())
    }
}
