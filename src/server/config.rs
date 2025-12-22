use std::path::PathBuf;

use bpaf::OptionParser;
use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use tracing::Level;

use crate::config;
use crate::config::SerializableLevel;
use crate::prelude::*;
use crate::protocols::wprs::Endpoint;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WprsdBackend {
    Wayland,
    X11Fullscreen,
    WindowsFullscreen,
    MacosFullscreen,
}

impl Default for WprsdBackend {
    fn default() -> Self {
        Self::Wayland
    }
}

impl std::str::FromStr for WprsdBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wayland" => Ok(Self::Wayland),
            "x11-fullscreen" => Ok(Self::X11Fullscreen),
            "windows-fullscreen" => Ok(Self::WindowsFullscreen),
            "macos-fullscreen" => Ok(Self::MacosFullscreen),
            other => bail!(
                "invalid backend {other:?} (expected: wayland|x11-fullscreen|windows-fullscreen|macos-fullscreen)"
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprsdConfig {
    pub socket: PathBuf,
    pub endpoint: Option<Endpoint>,
    pub backend: Option<WprsdBackend>,
    pub framerate: u32,
    pub x11_title: String,
    pub log_file: Option<PathBuf>,
    pub stderr_log_level: SerializableLevel,
    pub file_log_level: SerializableLevel,
    pub log_priv_data: bool,

    #[serde(default)]
    pub wayland: WprsdWaylandConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprsdWaylandConfig {
    pub display: String,
    pub kde_server_side_decorations: bool,
    #[serde(default)]
    pub xwayland: Option<XwaylandConfig>,
}

impl Default for WprsdWaylandConfig {
    fn default() -> Self {
        Self {
            display: config::default_wayland_display(),
            kde_server_side_decorations: false,
            // Preserve the historical behavior of enabling XWayland by default.
            xwayland: Some(XwaylandConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct XwaylandConfig {
    pub display: Option<u32>,
    pub wayland_debug: bool,
}

impl Default for XwaylandConfig {
    fn default() -> Self {
        Self {
            display: Some(100),
            wayland_debug: false,
        }
    }
}

impl Default for WprsdConfig {
    fn default() -> Self {
        Self {
            socket: config::default_socket_path(),
            endpoint: None,
            backend: None,
            framerate: 60,
            x11_title: "wprs x11".to_string(),
            log_file: None,
            stderr_log_level: SerializableLevel(Level::INFO),
            file_log_level: SerializableLevel(Level::TRACE),
            log_priv_data: false,
            wayland: WprsdWaylandConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WprsdArgs {
    pub print_default_config_and_exit: bool,
    pub config_file: Option<PathBuf>,

    pub wayland_display: Option<String>,
    pub socket: Option<PathBuf>,
    pub endpoint: Option<Endpoint>,
    pub backend: Option<WprsdBackend>,
    pub framerate: Option<u32>,
    pub x11_title: Option<String>,
    pub log_file: Option<PathBuf>,
    pub stderr_log_level: Option<SerializableLevel>,
    pub file_log_level: Option<SerializableLevel>,
    pub log_priv_data: Option<bool>,

    pub kde_server_side_decorations: Option<bool>,
    pub xwayland_display: Option<u32>,
    pub xwayland_wayland_debug: Option<bool>,
}

fn wprsd_args() -> OptionParser<WprsdArgs> {
    let print_default_config_and_exit = long("print-default-config-and-exit")
        .argument::<bool>("BOOL")
        .fallback(false);
    let config_file = long("config-file").argument::<PathBuf>("PATH").optional();
    let wayland_display = long("wayland-display")
        .argument::<String>("NAME")
        .optional();
    let socket = long("socket").argument::<PathBuf>("PATH").optional();
    let endpoint = long("endpoint").argument::<Endpoint>("ENDPOINT").optional();
    let backend = long("backend")
        .argument::<WprsdBackend>("BACKEND")
        .optional();
    let framerate = long("framerate").argument::<u32>("FPS").optional();
    let x11_title = long("x11-title").argument::<String>("TITLE").optional();
    let log_file = long("log-file").argument::<PathBuf>("PATH").optional();
    let stderr_log_level = long("stderr-log-level")
        .argument::<SerializableLevel>("LEVEL")
        .optional();
    let file_log_level = long("file-log-level")
        .argument::<SerializableLevel>("LEVEL")
        .optional();
    let log_priv_data = long("log-priv-data").argument::<bool>("BOOL").optional();

    let kde_server_side_decorations = long("kde-server-side-decorations")
        .argument::<bool>("BOOL")
        .optional();
    let xwayland_display = long("xwayland-display").argument::<u32>("NUM").optional();
    let xwayland_wayland_debug = long("xwayland-wayland-debug")
        .argument::<bool>("BOOL")
        .optional();

    construct!(WprsdArgs {
        print_default_config_and_exit,
        config_file,
        wayland_display,
        socket,
        endpoint,
        backend,
        framerate,
        x11_title,
        log_file,
        stderr_log_level,
        file_log_level,
        log_priv_data,
        kde_server_side_decorations,
        xwayland_display,
        xwayland_wayland_debug,
    })
    .to_options()
    .version(env!("CARGO_PKG_VERSION"))
}

impl WprsdArgs {
    pub fn parse() -> Self {
        wprsd_args().run()
    }

    pub fn load_config(self) -> Result<WprsdConfig> {
        if self.print_default_config_and_exit {
            config::print_default_config_and_exit::<WprsdConfig>();
        }

        let config_file = self
            .config_file
            .clone()
            .unwrap_or_else(|| config::default_config_file("wprsd"));
        let mut cfg = WprsdConfig::default();
        if let Some(from_file) =
            config::maybe_read_ron_file::<WprsdConfig>(&config_file).location(loc!())?
        {
            cfg = from_file;
        }

        if let Some(v) = self.wayland_display {
            cfg.wayland.display = v;
        }
        if let Some(v) = self.socket {
            cfg.socket = v;
        }
        if let Some(v) = self.endpoint {
            cfg.endpoint = Some(v);
        }
        if let Some(v) = self.backend {
            cfg.backend = Some(v);
        }
        if let Some(v) = self.framerate {
            cfg.framerate = v;
        }
        if let Some(v) = self.x11_title {
            cfg.x11_title = v;
        }
        if let Some(v) = self.log_file {
            cfg.log_file = Some(v);
        }
        if let Some(v) = self.stderr_log_level {
            cfg.stderr_log_level = v;
        }
        if let Some(v) = self.file_log_level {
            cfg.file_log_level = v;
        }
        if let Some(v) = self.log_priv_data {
            cfg.log_priv_data = v;
        }

        if let Some(v) = self.kde_server_side_decorations {
            cfg.wayland.kde_server_side_decorations = v;
        }
        if let Some(v) = self.xwayland_display {
            cfg.wayland
                .xwayland
                .get_or_insert_with(XwaylandConfig::default)
                .display = Some(v);
        }
        if let Some(v) = self.xwayland_wayland_debug {
            cfg.wayland
                .xwayland
                .get_or_insert_with(XwaylandConfig::default)
                .wayland_debug = v;
        }

        Ok(cfg)
    }
}

