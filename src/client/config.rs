use std::path::PathBuf;

use clap::Parser;
use clap::ValueEnum;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use tracing::Level;

use crate::config;
use crate::config::SerializableLevel;
use crate::prelude::*;
use crate::protocols::wprs::Endpoint;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum ClientBackend {
    Auto,
    Wayland,
    WinitWgpu,
}

impl Default for ClientBackend {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum KeyboardMode {
    Keymap,
    Evdev,
}

impl Default for KeyboardMode {
    fn default() -> Self {
        Self::Keymap
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WprscConfig {
    pub socket: PathBuf,
    pub endpoint: Option<Endpoint>,
    pub control_socket: PathBuf,
    pub log_file: Option<PathBuf>,
    pub stderr_log_level: SerializableLevel,
    pub file_log_level: SerializableLevel,
    pub log_priv_data: bool,
    pub title_prefix: String,

    pub backend: ClientBackend,

    pub keyboard_mode: KeyboardMode,
    pub xkb_keymap_file: Option<PathBuf>,

    #[serde(skip_serializing, default)]
    pub forward_only: bool,
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

            keyboard_mode: KeyboardMode::default(),
            xkb_keymap_file: None,

            forward_only: false,
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(name = "wprsc")]
pub struct WprscArgs {
    #[arg(long, value_name = "BOOL", default_value_t = false, action = clap::ArgAction::Set)]
    pub print_default_config_and_exit: bool,

    #[arg(long, value_name = "PATH")]
    pub config_file: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    pub socket: Option<PathBuf>,

    #[arg(long, value_name = "ENDPOINT")]
    pub endpoint: Option<Endpoint>,

    #[arg(long, value_name = "PATH")]
    pub control_socket: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    #[arg(long, value_name = "LEVEL")]
    pub stderr_log_level: Option<SerializableLevel>,

    #[arg(long, value_name = "LEVEL")]
    pub file_log_level: Option<SerializableLevel>,

    #[arg(long, value_name = "BOOL")]
    pub log_priv_data: Option<bool>,

    #[arg(long, value_name = "STRING")]
    pub title_prefix: Option<String>,

    #[arg(long, value_name = "BACKEND")]
    pub backend: Option<ClientBackend>,

    #[arg(long, value_name = "MODE")]
    pub keyboard_mode: Option<KeyboardMode>,

    #[arg(long, value_name = "PATH")]
    pub xkb_keymap_file: Option<PathBuf>,

    #[arg(long, value_name = "BOOL", default_value_t = false, action = clap::ArgAction::Set)]
    pub forward_only: bool,
}

impl WprscArgs {
    pub fn load_config(self) -> Result<WprscConfig> {
        if self.print_default_config_and_exit {
            config::print_default_config_and_exit::<WprscConfig>();
        }

        let config_file = self
            .config_file
            .clone()
            .unwrap_or_else(|| config::default_config_file("wprsc"));
        let mut cfg = WprscConfig::default();
        if let Some(from_file) = config::maybe_read_ron_file::<WprscConfig>(&config_file)
            .location(loc!())?
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
        if let Some(mode) = self.keyboard_mode {
            cfg.keyboard_mode = mode;
        }
        if let Some(path) = self.xkb_keymap_file {
            cfg.xkb_keymap_file = Some(path);
        }
        if self.forward_only {
            cfg.forward_only = true;
        }

        Ok(cfg)
    }
}
