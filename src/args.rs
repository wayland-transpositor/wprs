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

use std::fmt::Debug;
use std::path::PathBuf;
use std::str::FromStr;

use bpaf::Parser;
use optional_struct::Applicable;
use serde::Deserialize;
use serde::Serialize;

use crate::config;
use crate::config::SerializableLevel;
use crate::prelude::*;
use crate::protocols::wprs::Endpoint;

pub trait Config: Debug + Default + Serialize {
    fn config_file(&self) -> PathBuf;

    fn print_default_and_exit() -> ! {
        config::print_default_config_and_exit::<Self>()
    }
}

pub trait OptionalConfig<Conf: Config>:
    Debug + Default + Applicable<Base = Conf> + for<'a> Deserialize<'a>
{
    fn parse_args() -> Self;
    fn print_default_config_and_exit(&self) -> Option<bool>;
    fn config_file(&self) -> Option<PathBuf>;

    fn read_from_file(&self) -> Result<Option<Self>> {
        let config_file = self
            .config_file()
            .unwrap_or_else(|| Conf::default().config_file());
        config::maybe_read_ron_file::<Self>(&config_file)
    }
}

pub fn init_config<Conf: Config, OptConf: OptionalConfig<Conf>>() -> Result<Conf> {
    let mut config = Conf::default();
    let args = OptConf::parse_args();

    if let Some(true) = args.print_default_config_and_exit() {
        Conf::print_default_and_exit();
    }

    if let Some(config_from_file) = args.read_from_file().location(loc!())? {
        config_from_file.apply_to(&mut config);
    }
    args.apply_to(&mut config);

    Ok(config)
}

pub fn print_default_config_and_exit() -> impl Parser<Option<bool>> {
    bpaf::long("print-default-config-and-exit")
        .argument::<bool>("BOOL")
        .help("Print a configuration file with default values to stdout.")
        .optional()
}

pub fn config_file() -> impl Parser<Option<PathBuf>> {
    bpaf::long("config-file")
        .argument::<PathBuf>("PATH")
        .help("Path to the config file to use.")
        .optional()
}

pub fn wayland_display() -> impl Parser<Option<String>> {
    bpaf::long("wayland-display")
        .argument::<String>("NAME")
        .optional()
}

pub fn socket() -> impl Parser<Option<PathBuf>> {
    bpaf::long("socket").argument::<PathBuf>("PATH").optional()
}

pub fn endpoint() -> impl Parser<Option<Endpoint>> {
    bpaf::long("endpoint")
        .argument::<Endpoint>("ENDPOINT")
        .optional()
}

pub fn control_socket() -> impl Parser<Option<PathBuf>> {
    bpaf::long("control-socket")
        .argument::<PathBuf>("PATH")
        .optional()
}

pub fn log_file() -> impl Parser<Option<Option<PathBuf>>> {
    bpaf::long("log-file")
        .argument::<PathBuf>("PATH")
        .optional()
        .map(|log_file| log_file.map(Some))
}

pub fn stderr_log_level() -> impl Parser<Option<SerializableLevel>> {
    bpaf::long("stderr-log-level")
        .argument::<String>("LEVEL")
        .parse(|s| FromStr::from_str(&s))
        .optional()
}

pub fn file_log_level() -> impl Parser<Option<SerializableLevel>> {
    bpaf::long("file-log-level")
        .argument::<String>("LEVEL")
        .parse(|s| FromStr::from_str(&s))
        .optional()
}

pub fn log_priv_data() -> impl Parser<Option<bool>> {
    bpaf::long("log-priv-data")
        .argument::<bool>("BOOL")
        .optional()
}

pub fn title_prefix() -> impl Parser<Option<String>> {
    bpaf::long("title-prefix")
        .argument::<String>("STRING")
        .help("Prefix window titles with a string.")
        .optional()
}

pub fn framerate() -> impl Parser<Option<u32>> {
    bpaf::long("framerate").argument::<u32>("FPS").optional()
}
