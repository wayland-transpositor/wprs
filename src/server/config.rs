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
            xwayland_xdg_shell_path: "xwayland-xdg-shell".to_string(),
            xwayland_xdg_shell_wayland_debug: false,
            xwayland_xdg_shell_args: Vec::new(),
            kde_server_side_decorations: false,
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

    pub enable_xwayland: Option<bool>,

    pub xwayland_xdg_shell_path: Option<String>,

    pub xwayland_xdg_shell_wayland_debug: Option<bool>,

    pub xwayland_xdg_shell_args: Vec<String>,

    pub kde_server_side_decorations: Option<bool>,
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
    let log_priv_data = long("log-priv-data")
        .argument::<bool>("BOOL")
        .optional();
    let enable_xwayland = long("enable-xwayland")
        .argument::<bool>("BOOL")
        .optional();
    let xwayland_xdg_shell_path = long("xwayland-xdg-shell-path")
        .argument::<String>("PATH")
        .optional();
    let xwayland_xdg_shell_wayland_debug = long("xwayland-xdg-shell-wayland-debug")
        .argument::<bool>("BOOL")
        .optional();
    let xwayland_xdg_shell_args = long("xwayland-xdg-shell-args")
        .argument::<String>("ARG")
        .many();
    let kde_server_side_decorations = long("kde-server-side-decorations")
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
        enable_xwayland,
        xwayland_xdg_shell_path,
        xwayland_xdg_shell_wayland_debug,
        xwayland_xdg_shell_args,
        kde_server_side_decorations,
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
        if let Some(v) = self.xwayland_xdg_shell_path {
            cfg.xwayland_xdg_shell_path = v;
        }
        if let Some(v) = self.xwayland_xdg_shell_wayland_debug {
            cfg.xwayland_xdg_shell_wayland_debug = v;
        }
        if !self.xwayland_xdg_shell_args.is_empty() {
            let mut args = Vec::new();
            for value in self.xwayland_xdg_shell_args {
                for item in value.split(',') {
                    if !item.is_empty() {
                        args.push(item.to_string());
                    }
                }
            }
            if !args.is_empty() {
                cfg.xwayland_xdg_shell_args = args;
            }
        }
        if let Some(v) = self.kde_server_side_decorations {
            cfg.kde_server_side_decorations = v;
        }

        Ok(cfg)
    }
}

#[cfg(all(feature = "server", feature = "wayland-client", target_os = "linux"))]
pub mod xwayland_xdg_shell {
    use super::*;

    use bpaf::OptionParser;
    use bpaf::Parser;
    use bpaf::construct;
    use bpaf::long;

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

    #[derive(Debug, Clone)]
    pub struct XwaylandXdgShellArgs {
        pub print_default_config_and_exit: bool,

        pub config_file: Option<PathBuf>,

        pub wayland_display: Option<String>,

        pub display: Option<u32>,

        pub log_file: Option<PathBuf>,

        pub stderr_log_level: Option<SerializableLevel>,

        pub file_log_level: Option<SerializableLevel>,

        pub log_priv_data: Option<bool>,

        pub xwayland_wayland_debug: Option<bool>,

        pub decoration_behavior: Option<String>,
    }

    fn xwayland_xdg_shell_args() -> OptionParser<XwaylandXdgShellArgs> {
        let print_default_config_and_exit = long("print-default-config-and-exit")
            .argument::<bool>("BOOL")
            .fallback(false);
        let config_file = long("config-file").argument::<PathBuf>("PATH").optional();
        let wayland_display = long("wayland-display")
            .argument::<String>("NAME")
            .optional();
        let display = long("display").argument::<u32>("NUM").optional();
        let log_file = long("log-file").argument::<PathBuf>("PATH").optional();
        let stderr_log_level = long("stderr-log-level")
            .argument::<SerializableLevel>("LEVEL")
            .optional();
        let file_log_level = long("file-log-level")
            .argument::<SerializableLevel>("LEVEL")
            .optional();
        let log_priv_data = long("log-priv-data")
            .argument::<bool>("BOOL")
            .optional();
        let xwayland_wayland_debug = long("xwayland-wayland-debug")
            .argument::<bool>("BOOL")
            .optional();
        let decoration_behavior = long("decoration-behavior")
            .argument::<String>("Auto|AlwaysEnabled|AlwaysDisabled")
            .optional();

        construct!(XwaylandXdgShellArgs {
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
        .version(env!("CARGO_PKG_VERSION"))
    }

    impl XwaylandXdgShellArgs {
        pub fn parse() -> Self {
            xwayland_xdg_shell_args().run()
        }

        pub fn load_config(self) -> Result<XwaylandXdgShellConfig> {
            if self.print_default_config_and_exit {
                config::print_default_config_and_exit::<XwaylandXdgShellConfig>();
            }

            let config_file = self
                .config_file
                .clone()
                .unwrap_or_else(|| config::default_config_file("xwayland-xdg-shell"));
            let mut cfg = XwaylandXdgShellConfig::default();
            if let Some(from_file) =
                config::maybe_read_ron_file::<XwaylandXdgShellConfig>(&config_file)
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
