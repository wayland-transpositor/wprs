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

use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io;
use std::os::unix::net::UnixListener;
use std::panic;
use std::path::Path;
use std::process;
use std::sync::Mutex;
use std::thread::ScopedJoinHandle;

use nix::sys::stat;
use nix::sys::stat::Mode;
use smithay::utils::SERIAL_COUNTER;
use smithay::utils::Serial;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

use crate::prelude::*;

pub fn configure_tracing<P: AsRef<Path>>(
    stderr_log_level: Level,
    path: Option<P>,
    file_log_level: Level,
) -> Result<()> {
    let mut layers = Vec::new();

    let layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr.with_max_level(stderr_log_level))
        // TODO(https://github.com/tokio-rs/tracing/pull/2655): uncomment
        // .with_binary_name(true, None)
        // .with_process_id(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE);

    if let Some(path) = path {
        let log_file = File::create(path).location(loc!())?;
        let log_file_writer = Mutex::new(log_file).with_max_level(file_log_level);
        let layer = layer.map_writer(|w| w.and(log_file_writer));
        layers.push(layer.boxed());
    } else {
        layers.push(layer.boxed());
    };

    #[cfg(feature = "tracy")]
    {
        layers
            .push(tracing_tracy::TracyLayer::new(tracing_tracy::DefaultConfig::default()).boxed());
    }

    tracing_subscriber::registry().with(layers).init();
    Ok(())
}

pub fn exit_on_thread_panic() {
    let orig_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let backtrace = Backtrace::capture();
        error!("panic!:\n{panic_info}\n{backtrace}");
        orig_hook(panic_info);
        process::exit(1);
    }));
}

pub fn join_unwrap<T>(handle: ScopedJoinHandle<T>) -> T {
    match handle.join() {
        Ok(t) => t,
        Err(e) => panic::resume_unwind(e),
    }
}

#[derive(Debug)]
pub struct SerialMap {
    map: HashMap<u32, u32>,
    last_serial: u32,
}

impl SerialMap {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity(2000),
            last_serial: 0,
        }
    }

    const PRUNE_THRESHOLD: usize = 2000;
    const PRUNE_AGE: u32 = 1000;

    #[instrument(skip(self), level = "debug")]
    fn prune(&mut self) {
        if self.map.len() > Self::PRUNE_THRESHOLD {
            self.map
                .retain(|&k, _| k > self.last_serial.saturating_sub(Self::PRUNE_AGE));
        }
    }

    pub fn insert(&mut self, client_serial: u32) -> Serial {
        self.last_serial = SERIAL_COUNTER.next_serial().into();
        _ = self.map.insert(self.last_serial, client_serial).is_none();
        self.prune();
        self.last_serial.into()
    }

    pub fn remove(&mut self, server_serial: Serial) -> Option<u32> {
        self.map.remove(&server_serial.into())
    }
}

impl Default for SerialMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the number of chunks that will result from splitting a collection
/// of size len into chunks of chunk_size.
///
/// # Panics
/// If chunk_size = 0.
pub fn n_chunks(len: usize, chunk_size: usize) -> usize {
    assert!(chunk_size != 0);
    if len == 0 {
        0
    } else {
        let n = len / chunk_size;
        let rem = len % chunk_size;
        if rem > 0 { n + 1 } else { n }
    }
}

pub fn bind_user_socket<P: AsRef<Path>>(sock_path: P) -> Result<UnixListener> {
    if sock_path.as_ref().try_exists().location(loc!())? {
        fs::remove_file(&sock_path).location(loc!())?;
    }

    let old_umask = stat::umask(Mode::S_IXUSR | Mode::S_IRWXG | Mode::S_IRWXO);
    let listener = UnixListener::bind(sock_path).location(loc!())?;
    stat::umask(old_umask);

    Ok(listener)
}

// https://github.com/nvzqz/static-assertions/issues/21
// https://stackoverflow.com/questions/72582671/const-generics-how-to-ensure-that-usize-const-is-0
pub struct AssertN<const N: usize>;

impl<const N: usize> AssertN<N> {
    pub const NE_0: () = assert!(N != 0);
    pub const MULTIPLE_OF_32: () = assert!(N.is_multiple_of(32));
}

pub struct AssertN3<const N1: usize, const N2: usize, const N3: usize>;

impl<const N1: usize, const N2: usize, const N3: usize> AssertN3<N1, N2, N3> {
    pub const N1_X_N2_EQ_N3: () = assert!(N1.checked_mul(N2).unwrap() == N3);
}
