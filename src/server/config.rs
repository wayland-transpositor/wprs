use std::path::PathBuf;

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use optional_struct::optional_struct;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use tracing::Level;

use crate::args;
use crate::args::Config;
use crate::args::OptionalConfig;
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

#[optional_struct]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprsdConfig {
    #[serde(skip_serializing, default)]
    print_default_config_and_exit: bool,
    #[serde(skip_serializing, default)]
    config_file: PathBuf,
    pub wayland_display: String,
    pub socket: PathBuf,
    #[optional_wrap]
    pub endpoint: Option<Endpoint>,
    #[optional_wrap]
    pub backend: Option<WprsdBackend>,
    pub framerate: u32,
    pub x11_title: String,
    #[optional_wrap]
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
            print_default_config_and_exit: false,
            config_file: config::default_config_file("wprsd"),
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

impl Config for WprsdConfig {
    fn config_file(&self) -> PathBuf {
        self.config_file.clone()
    }
}

impl OptionalConfig<WprsdConfig> for OptionalWprsdConfig {
    fn parse_args() -> Self {
        let print_default_config_and_exit = args::print_default_config_and_exit();
        let config_file = args::config_file();
        let wayland_display = args::wayland_display();
        let socket = args::socket();
        let endpoint = args::endpoint().map(|val| val.map(Some));
        let backend = long("backend")
            .argument::<WprsdBackend>("BACKEND")
            .optional()
            .map(|val| val.map(Some));
        let framerate = args::framerate();
        let x11_title = long("x11-title").argument::<String>("TITLE").optional();
        let log_file = args::log_file();
        let stderr_log_level = args::stderr_log_level();
        let file_log_level = args::file_log_level();
        let log_priv_data = args::log_priv_data();
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
            .many()
            .map(|values| {
                let mut args = Vec::new();
                for value in values {
                    for item in value.split(',') {
                        if !item.is_empty() {
                            args.push(item.to_string());
                        }
                    }
                }
                if args.is_empty() {
                    None
                } else {
                    Some(args)
                }
            });
        let kde_server_side_decorations = long("kde-server-side-decorations")
            .argument::<bool>("BOOL")
            .optional();

        construct!(Self {
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
        .run()
    }

    fn print_default_config_and_exit(&self) -> Option<bool> {
        self.print_default_config_and_exit
    }

    fn config_file(&self) -> Option<PathBuf> {
        self.config_file.clone()
    }
}

#[cfg(all(
    feature = "wayland-compositor",
    feature = "wayland-client",
    target_os = "linux"
))]
pub mod xwayland_xdg_shell {
    use super::*;

    use bpaf::Parser;
    use bpaf::construct;
    use bpaf::long;
    use optional_struct::optional_struct;

    use crate::xwayland_xdg_shell::compositor::DecorationBehavior;

    #[optional_struct]
    #[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
    pub struct XwaylandXdgShellConfig {
        #[serde(skip_serializing, default)]
        print_default_config_and_exit: bool,
        #[serde(skip_serializing, default)]
        config_file: PathBuf,
        pub wayland_display: String,
        pub display: u32,
        #[optional_wrap]
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
                print_default_config_and_exit: false,
                config_file: config::default_config_file("xwayland-xdg-shell"),
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

    impl OptionalConfig<XwaylandXdgShellConfig> for OptionalXwaylandXdgShellConfig {
        fn parse_args() -> Self {
            let print_default_config_and_exit = args::print_default_config_and_exit();
            let config_file = args::config_file();
            let wayland_display = args::wayland_display();
            let display = long("display").argument::<u32>("NUM").optional();
            let log_file = args::log_file();
            let stderr_log_level = args::stderr_log_level();
            let file_log_level = args::file_log_level();
            let log_priv_data = args::log_priv_data();
            let xwayland_wayland_debug = long("xwayland-wayland-debug")
                .argument::<bool>("BOOL")
                .optional();
            let decoration_behavior = long("decoration-behavior")
                .argument::<String>("Auto|AlwaysEnabled|AlwaysDisabled")
                .parse(|value| {
                    ron::from_str(&value).map_err(|_| {
                        format!(
                            "invalid --decoration-behavior {value:?} (expected: Auto|AlwaysEnabled|AlwaysDisabled)"
                        )
                    })
                })
                .optional();

            construct!(Self {
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
            .run()
        }

        fn print_default_config_and_exit(&self) -> Option<bool> {
            self.print_default_config_and_exit
        }

        fn config_file(&self) -> Option<PathBuf> {
            self.config_file.clone()
        }
    }
}
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
