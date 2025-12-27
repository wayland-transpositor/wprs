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

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprscConfig {
    /// Path to the local UNIX socket used for direct connections.
    pub socket: PathBuf,
    /// Optional endpoint override for TCP or UNIX socket connections.
    pub endpoint: Option<Endpoint>,
    /// Path to the local control socket.
    pub control_socket: PathBuf,
    /// Log file path for persisted logs.
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

#[derive(Debug, Clone)]
pub struct WprscArgs {
    pub print_default_config_and_exit: bool,

    pub config_file: Option<PathBuf>,

    pub socket: Option<PathBuf>,

    pub endpoint: Option<Endpoint>,

    pub control_socket: Option<PathBuf>,

    pub log_file: Option<PathBuf>,

    pub stderr_log_level: Option<SerializableLevel>,

    pub file_log_level: Option<SerializableLevel>,

    pub log_priv_data: Option<bool>,

    pub title_prefix: Option<String>,

    pub backend: Option<ClientBackend>,
}

fn wprsc_args() -> OptionParser<WprscArgs> {
    let print_default_config_and_exit = long("print-default-config-and-exit")
        .argument::<bool>("BOOL")
        .fallback(false);

    let config_file = long("config-file").argument::<PathBuf>("PATH").optional();
    let socket = long("socket").argument::<PathBuf>("PATH").optional();
    let endpoint = long("endpoint").argument::<Endpoint>("ENDPOINT").optional();
    let control_socket = long("control-socket")
        .argument::<PathBuf>("PATH")
        .optional();
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
    let title_prefix = long("title-prefix").argument::<String>("STRING").optional();
    let backend = long("backend")
        .argument::<ClientBackend>("BACKEND")
        .optional();
    construct!(WprscArgs {
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
}

impl WprscArgs {
    pub fn parse() -> Self {
        wprsc_args().run()
    }

    pub fn load_config(self) -> Result<WprscConfig> {
        if self.print_default_config_and_exit {
            config::print_default_config_and_exit::<WprscConfig>();
        }

        let config_file = self
            .config_file
            .clone()
            .unwrap_or_else(|| config::default_config_file("wprsc"));
        let mut cfg = WprscConfig::default();
        if let Some(from_file) =
            config::maybe_read_ron_file::<WprscConfig>(&config_file).location(loc!())?
        {
            cfg = from_file;
        }

        if let Some(socket) = self.socket {
            cfg.socket = socket;
        }
        if let Some(endpoint) = self.endpoint {
            cfg.endpoint = Some(endpoint);
        }
        if let Some(control_socket) = self.control_socket {
            cfg.control_socket = control_socket;
        }
        if let Some(log_file) = self.log_file {
            cfg.log_file = Some(log_file);
        }
        if let Some(level) = self.stderr_log_level {
            cfg.stderr_log_level = level;
        }
        if let Some(level) = self.file_log_level {
            cfg.file_log_level = level;
        }
        if let Some(val) = self.log_priv_data {
            cfg.log_priv_data = val;
        }
        if let Some(prefix) = self.title_prefix {
            cfg.title_prefix = prefix;
        }
        if let Some(backend) = self.backend {
            cfg.backend = backend;
        }
        Ok(cfg)
    }
}
