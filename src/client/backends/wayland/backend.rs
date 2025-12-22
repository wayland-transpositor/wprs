use calloop::EventLoop;
use calloop::channel::Event as CalloopChannelEvent;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::ConnectError;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;

use crate::client::backend::ClientBackend;
use crate::client::backend::ClientBackendConfig;
use crate::client::backends::wayland::ClientOptions;
use crate::client::backends::wayland::WprsClientState;
use crate::control_server;
use crate::prelude::*;
use crate::protocols::wprs as proto;
use crate::protocols::wprs::Serializer;

#[derive(Debug)]
pub struct WaylandClientBackend {
    config: ClientBackendConfig,
    conn: Connection,
}

impl WaylandClientBackend {
    pub fn new(config: ClientBackendConfig, conn: Connection) -> Self {
        Self { config, conn }
    }

    pub fn connect_to_env(config: ClientBackendConfig) -> Result<Self> {
        let conn = Connection::connect_to_env()
            .map_err(|e| anyhow!(e))
            .location(loc!())?;
        Ok(Self::new(config, conn))
    }

    pub fn try_connect_to_env(config: ClientBackendConfig) -> Result<Option<Self>> {
        match Connection::connect_to_env() {
            Ok(conn) => Ok(Some(Self::new(config, conn))),
            Err(ConnectError::NoCompositor) => Ok(None),
            Err(e) => Err(anyhow!(e)),
        }
    }
}

impl ClientBackend for WaylandClientBackend {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn run(self: Box<Self>, serializer: Serializer<proto::Event, proto::Request>) -> Result<()> {
        run_wayland(serializer, self.config, self.conn).location(loc!())
    }
}

fn run_wayland(
    mut serializer: Serializer<proto::Event, proto::Request>,
    config: ClientBackendConfig,
    conn: Connection,
) -> Result<()> {
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let reader = serializer.reader().location(loc!())?;

    let options = ClientOptions {
        title_prefix: config.title_prefix,
    };
    let mut state = WprsClientState::new(
        event_queue.handle(),
        globals,
        conn.clone(),
        serializer,
        options,
    )
    .location(loc!())?;

    {
        let capabilities = state.capabilities.clone();
        control_server::start(config.control_socket, move |input: &str| {
            Ok(match input {
                "caps" => serde_json::to_string(&capabilities.get())
                    .expect("a map with non-string keys was added to Capabilities"),
                _ => {
                    bail!("Unknown command: {input:?}")
                },
            })
        })
        .location(loc!())?;
    }

    let mut event_loop = EventLoop::try_new()?;
    event_loop
        .handle()
        .insert_source(reader, |event, _metadata, state: &mut WprsClientState| {
            if let CalloopChannelEvent::Msg(msg) = event {
                state.handle_request(msg)
            }
        })
        .map_err(|e| anyhow!("insert_source(serializer reader) failed: {e:?}"))?;

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .map_err(|e| anyhow!("insert_source(wayland) failed: {e}"))
        .location(loc!())?;

    event_loop.run(None, &mut state, |_| {}).location(loc!())
}
