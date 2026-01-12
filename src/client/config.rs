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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientBackend {
    Auto,
    Wayland,
}

impl Default for ClientBackend {
    fn default() -> Self {
        Self::Auto
    }
}

impl std::str::FromStr for ClientBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "wayland" => Ok(Self::Wayland),
            other => bail!("invalid backend {other:?} (expected: auto|wayland)"),
        }
    }
}

#[optional_struct]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprscConfig {
    #[serde(skip_serializing, default)]
    print_default_config_and_exit: bool,
    #[serde(skip_serializing, default)]
    config_file: PathBuf,
    /// Path to the local UNIX socket used for direct connections.
    pub socket: PathBuf,
    /// Optional endpoint override for TCP or UNIX socket connections.
    #[optional_wrap]
    pub endpoint: Option<Endpoint>,
    /// Path to the local control socket.
    pub control_socket: PathBuf,
    /// Log file path for persisted logs.
    #[optional_wrap]
    pub log_file: Option<PathBuf>,
    /// Log level for stderr output.
    pub stderr_log_level: SerializableLevel,
    /// Log level for file output.
    pub file_log_level: SerializableLevel,
    /// Whether to include private data in logs.
    pub log_priv_data: bool,
    /// Prefix added to window titles.
    pub title_prefix: String,

    /// Client backend selection.
    pub backend: ClientBackend,

}

impl Default for WprscConfig {
    fn default() -> Self {
        Self {
            print_default_config_and_exit: false,
            config_file: config::default_config_file("wprsc"),
            socket: config::default_socket_path(),
            endpoint: None,
            control_socket: config::default_control_socket_path("wprsc"),
            log_file: None,
            stderr_log_level: SerializableLevel(Level::INFO),
            file_log_level: SerializableLevel(Level::TRACE),
            log_priv_data: false,
            title_prefix: String::new(),

            backend: ClientBackend::default(),
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
        let endpoint = args::endpoint().map(|val| val.map(Some));
        let control_socket = args::control_socket();
        let log_file = args::log_file();
        let stderr_log_level = args::stderr_log_level();
        let file_log_level = args::file_log_level();
        let log_priv_data = args::log_priv_data();
        let title_prefix = args::title_prefix();
        let backend = long("backend")
            .argument::<ClientBackend>("BACKEND")
            .optional();

        construct!(Self {
            print_default_config_and_exit,
            config_file,
            socket,
            endpoint,
            control_socket,
            log_file,
            stderr_log_level,
            file_log_level,
            log_priv_data,
            title_prefix,
            backend,
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
