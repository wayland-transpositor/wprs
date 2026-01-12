use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use ron::Options;
use ron::extensions::Extensions;
use serde::Deserialize;
use serde::Serialize;
use tracing::Level;
use tracing::metadata::ParseLevelError;

use crate::prelude::*;

fn fallback_config_parent_dir() -> Result<PathBuf> {
    Ok(Path::join(
        &home::home_dir().ok_or(anyhow!("unable to determine home dir"))?,
        ".config",
    ))
}

pub fn default_config_file_dir() -> PathBuf {
    Path::join(
        &env::var("XDG_CONFIG_HOME")
            .log(loc!())
            .ok()
            .map(Into::into)
            .or(fallback_config_parent_dir().log(loc!()).ok())
            .unwrap_or_else(|| "/etc".into()),
        "wprs",
    )
}

pub fn default_config_file(name: &str) -> PathBuf {
    Path::join(&default_config_file_dir(), format!("{name}.ron"))
}

pub fn default_wayland_display() -> String {
    "wprs-0".to_string()
}

fn socket_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(Into::into)
        .unwrap_or_else(|| Path::join(&env::temp_dir(), whoami::username()))
}

pub fn default_socket_path() -> PathBuf {
    Path::join(&socket_dir(), "wprs.sock")
}

pub fn default_control_socket_path(prefix: &str) -> PathBuf {
    Path::join(&socket_dir(), format!("{prefix}-ctrl.sock"))
}

pub fn maybe_read_ron_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        eprintln!("config file does not exist at {path:?}");
        return Ok(None);
    }

    let config_str = std::fs::read_to_string(path)
        .with_context(loc!(), || format!("unable to read config file {path:?}"))?;
    let config: T = Options::default()
        .with_default_extension(Extensions::IMPLICIT_SOME)
        .from_str(&config_str)
        .with_context(loc!(), || format!("error parsing config file {path:?}"))?;
    Ok(Some(config))
}

pub fn print_default_config_and_exit<T: Serialize + Default>() -> ! {
    println!(
        "{}",
        ron::ser::to_string_pretty::<T>(&Default::default(), ron::ser::PrettyConfig::default())
            .expect("default config must be serializable")
    );
    std::process::exit(0);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SerializableLevel(pub Level);

impl FromStr for SerializableLevel {
    type Err = ParseLevelError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Level::from_str(s)?))
    }
}

impl Serialize for SerializableLevel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for SerializableLevel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(Level::from_str(&s).map_err(serde::de::Error::custom)?))
    }
}

pub static LOG_PRIV_DATA: AtomicBool = AtomicBool::new(false);

pub fn set_log_priv_data(val: bool) {
    LOG_PRIV_DATA.store(val, Ordering::Relaxed);
}

pub fn get_log_priv_data() -> bool {
    LOG_PRIV_DATA.load(Ordering::Relaxed)
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
