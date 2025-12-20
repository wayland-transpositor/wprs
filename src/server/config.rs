use std::path::PathBuf;

use clap::Parser;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum XwaylandMode {
    /// Spawns the external `xwayland-xdg-shell` helper process.
    SpawnProxy,
    /// Runs the Xwayland proxy inline inside `wprsd`.
    InlineProxy,
    /// Does not spawn any helper; expects external management.
    External,
}

impl Default for XwaylandMode {
    fn default() -> Self {
        if cfg!(all(feature = "wayland", target_os = "linux")) {
            Self::InlineProxy
        } else {
            Self::SpawnProxy
        }
    }
}

impl std::str::FromStr for XwaylandMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "spawn-proxy" => Ok(Self::SpawnProxy),
            "inline-proxy" => Ok(Self::InlineProxy),
            "external" => Ok(Self::External),
            other => bail!(
                "invalid xwayland mode {other:?} (expected: inline-proxy|spawn-proxy|external)"
            ),
        }
    }
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
    pub wayland_display: String,
    pub socket: PathBuf,
    pub endpoint: Option<Endpoint>,
    pub backend: Option<WprsdBackend>,
    pub framerate: u32,
    pub x11_title: String,
    pub log_file: Option<PathBuf>,
    pub stderr_log_level: SerializableLevel,
    pub file_log_level: SerializableLevel,
    pub log_priv_data: bool,
    pub enable_xwayland: bool,
    pub xwayland_mode: XwaylandMode,
    pub xwayland_xdg_shell_path: String,
    pub xwayland_xdg_shell_wayland_debug: bool,
    pub xwayland_xdg_shell_args: Vec<String>,
    pub kde_server_side_decorations: bool,
}

impl Default for WprsdConfig {
    fn default() -> Self {
        Self {
            wayland_display: config::default_wayland_display(),
            socket: config::default_socket_path(),
            endpoint: None,
            backend: None,
            framerate: 60,
            x11_title: "wprs x11".to_string(),
            log_file: None,
            stderr_log_level: SerializableLevel(Level::INFO),
            file_log_level: SerializableLevel(Level::TRACE),
            log_priv_data: false,
            enable_xwayland: true,
            xwayland_mode: XwaylandMode::default(),
            xwayland_xdg_shell_path: "xwayland-xdg-shell".to_string(),
            xwayland_xdg_shell_wayland_debug: false,
            xwayland_xdg_shell_args: Vec::new(),
            kde_server_side_decorations: false,
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(name = "wprsd")]
pub struct WprsdArgs {
    #[arg(long, value_name = "BOOL", default_value_t = false, action = clap::ArgAction::Set)]
    pub print_default_config_and_exit: bool,

    #[arg(long, value_name = "PATH")]
    pub config_file: Option<PathBuf>,

    #[arg(long, value_name = "NAME")]
    pub wayland_display: Option<String>,

    #[arg(long, value_name = "PATH")]
    pub socket: Option<PathBuf>,

    #[arg(long, value_name = "ENDPOINT")]
    pub endpoint: Option<Endpoint>,

    #[arg(long, value_name = "BACKEND")]
    pub backend: Option<WprsdBackend>,

    #[arg(long, value_name = "FPS")]
    pub framerate: Option<u32>,

    #[arg(long, value_name = "TITLE")]
    pub x11_title: Option<String>,

    #[arg(long, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    #[arg(long, value_name = "LEVEL")]
    pub stderr_log_level: Option<SerializableLevel>,

    #[arg(long, value_name = "LEVEL")]
    pub file_log_level: Option<SerializableLevel>,

    #[arg(long, value_name = "BOOL")]
    pub log_priv_data: Option<bool>,

    #[arg(long, value_name = "BOOL")]
    pub enable_xwayland: Option<bool>,

    #[arg(long, value_name = "MODE")]
    pub xwayland_mode: Option<XwaylandMode>,

    #[arg(long, value_name = "PATH")]
    pub xwayland_xdg_shell_path: Option<String>,

    #[arg(long, value_name = "BOOL")]
    pub xwayland_xdg_shell_wayland_debug: Option<bool>,

    #[arg(long, value_name = "ARG", value_delimiter = ',')]
    pub xwayland_xdg_shell_args: Vec<String>,

    #[arg(long, value_name = "BOOL")]
    pub kde_server_side_decorations: Option<bool>,
}

impl WprsdArgs {
    pub fn load_config(self) -> Result<WprsdConfig> {
        if self.print_default_config_and_exit {
            config::print_default_config_and_exit::<WprsdConfig>();
        }

        let config_file = self
            .config_file
            .clone()
            .unwrap_or_else(|| config::default_config_file("wprsd"));
        let mut cfg = WprsdConfig::default();
        if let Some(from_file) = config::maybe_read_ron_file::<WprsdConfig>(&config_file)
            .location(loc!())?
        {
            cfg = from_file;
        }

        if let Some(v) = self.wayland_display {
            cfg.wayland_display = v;
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
        if let Some(v) = self.enable_xwayland {
            cfg.enable_xwayland = v;
        }
        if let Some(v) = self.xwayland_mode {
            cfg.xwayland_mode = v;
        }
        if let Some(v) = self.xwayland_xdg_shell_path {
            cfg.xwayland_xdg_shell_path = v;
        }
        if let Some(v) = self.xwayland_xdg_shell_wayland_debug {
            cfg.xwayland_xdg_shell_wayland_debug = v;
        }
        if !self.xwayland_xdg_shell_args.is_empty() {
            cfg.xwayland_xdg_shell_args = self.xwayland_xdg_shell_args;
        }
        if let Some(v) = self.kde_server_side_decorations {
            cfg.kde_server_side_decorations = v;
        }

        Ok(cfg)
    }
}

#[cfg(all(feature = "wayland", feature = "wayland-client", target_os = "linux"))]
pub mod xwayland_xdg_shell {
    use super::*;

    use crate::xwayland_xdg_shell::compositor::DecorationBehavior;

    #[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
    pub struct XwaylandXdgShellConfig {
        pub wayland_display: String,
        pub display: u32,
        pub log_file: Option<PathBuf>,
        pub stderr_log_level: SerializableLevel,
        pub file_log_level: SerializableLevel,
        pub log_priv_data: bool,
        pub xwayland_wayland_debug: bool,
        pub decoration_behavior: DecorationBehavior,
    }

    impl Default for XwaylandXdgShellConfig {
        fn default() -> Self {
            Self {
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

    #[derive(Parser, Debug, Clone)]
    #[command(name = "xwayland-xdg-shell")]
    pub struct XwaylandXdgShellArgs {
        #[arg(long, value_name = "BOOL", default_value_t = false, action = clap::ArgAction::Set)]
        pub print_default_config_and_exit: bool,

        #[arg(long, value_name = "PATH")]
        pub config_file: Option<PathBuf>,

        #[arg(long, value_name = "NAME")]
        pub wayland_display: Option<String>,

        #[arg(long, value_name = "NUM")]
        pub display: Option<u32>,

        #[arg(long, value_name = "PATH")]
        pub log_file: Option<PathBuf>,

        #[arg(long, value_name = "LEVEL")]
        pub stderr_log_level: Option<SerializableLevel>,

        #[arg(long, value_name = "LEVEL")]
        pub file_log_level: Option<SerializableLevel>,

        #[arg(long, value_name = "BOOL")]
        pub log_priv_data: Option<bool>,

        #[arg(long, value_name = "BOOL")]
        pub xwayland_wayland_debug: Option<bool>,

        #[arg(long, value_name = "Auto|AlwaysEnabled|AlwaysDisabled")]
        pub decoration_behavior: Option<String>,
    }

    impl XwaylandXdgShellArgs {
        pub fn load_config(self) -> Result<XwaylandXdgShellConfig> {
            if self.print_default_config_and_exit {
                config::print_default_config_and_exit::<XwaylandXdgShellConfig>();
            }

            let config_file = self
                .config_file
                .clone()
                .unwrap_or_else(|| config::default_config_file("xwayland-xdg-shell"));
            let mut cfg = XwaylandXdgShellConfig::default();
            if let Some(from_file) = config::maybe_read_ron_file::<XwaylandXdgShellConfig>(&config_file)
                .location(loc!())?
            {
                cfg = from_file;
            }

            if let Some(v) = self.wayland_display {
                cfg.wayland_display = v;
            }
            if let Some(v) = self.display {
                cfg.display = v;
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
            if let Some(v) = self.xwayland_wayland_debug {
                cfg.xwayland_wayland_debug = v;
            }
            if let Some(v) = self.decoration_behavior {
                cfg.decoration_behavior = ron::from_str(&v).with_context(loc!(), || {
                    format!(
                        "invalid --decoration-behavior {v:?} (expected: Auto|AlwaysEnabled|AlwaysDisabled)"
                    )
                })?;
            }

            Ok(cfg)
        }
    }
}
