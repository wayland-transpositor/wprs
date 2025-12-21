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

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::net::Shutdown;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::num::NonZeroUsize;
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::Scope;
use std::thread::ScopedJoinHandle;
use std::time::Duration;
use calloop::channel;
use calloop::channel::Channel;
use crossbeam_channel::Receiver;
use crossbeam_channel::RecvTimeoutError;
use crossbeam_channel::Sender;
use nix::sys::socket;
use nix::sys::socket::sockopt::RcvBuf;
use nix::sys::socket::sockopt::SndBuf;
use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;
use rkyv::api::high::HighDeserializer;
use rkyv::api::high::HighSerializer;
use rkyv::api::high::HighValidator;
use rkyv::bytecheck;
use rkyv::rancor::Error as RancorError;
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;

#[cfg(feature = "server")]
use smithay::reexports::wayland_server::Client;
#[cfg(feature = "server")]
use smithay::reexports::wayland_server::backend;
use sysctl::Ctl;
use sysctl::Sysctl;

use crate::arc_slice::ArcSlice;
use crate::prelude::*;
use crate::sharding_compression::CompressedShards;
use crate::sharding_compression::ShardingCompressor;
use crate::sharding_compression::ShardingDecompressor;
use crate::utils;
use crate::utils::channel::DiscardingSender;
use crate::utils::channel::InfallibleSender;

#[derive(Debug, Clone, Eq, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Endpoint {
    /// Unix domain socket path.
    ///
    /// Preferred for local connections on Unix platforms.
    Unix { path: PathBuf },

    /// TCP address.
    ///
    /// Allowed, but non-loopback addresses should be treated as unsafe unless you
    /// add authentication + encryption at a higher layer.
    Tcp { addr: SocketAddr },
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Unix { path } => write!(f, "unix://{}", path.display()),
            Endpoint::Tcp { addr } => write!(f, "tcp://{addr}"),
        }
    }
}

impl Endpoint {
    pub fn warn_if_non_loopback(&self, kind: &str) {
        if let Endpoint::Tcp { addr } = self {
            if !addr.ip().is_loopback() {
                warn!(
                    "{kind} is bound to {addr:?} (non-loopback). This is not recommended without authentication/encryption. Prefer localhost (127.0.0.1/::1)."
                );
            }
        }
    }
}


impl std::str::FromStr for Endpoint {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Note: check the URI form first, otherwise `strip_prefix("tcp:")` would
        // accept `tcp://...` and leave a leading `//`.
        if let Some(rest) = s.strip_prefix("tcp://") {
            let addr: SocketAddr = rest
                .parse()
                .map_err(|e| anyhow!("invalid tcp endpoint {rest:?}: {e}"))?;
            return Ok(Self::Tcp { addr });
        }

        if let Some(rest) = s.strip_prefix("tcp:") {
            let rest = rest.strip_prefix("//").unwrap_or(rest);
            let addr: SocketAddr = rest
                .parse()
                .map_err(|e| anyhow!("invalid tcp endpoint {rest:?}: {e}"))?;
            return Ok(Self::Tcp { addr });
        }

        if let Some(rest) = s.strip_prefix("unix:") {
            return Ok(Self::Unix {
                path: PathBuf::from(rest),
            });
        }

        if let Some(rest) = s.strip_prefix("unix://") {
            #[cfg(unix)]
            {
                let path = rest.strip_prefix('/').unwrap_or(rest);
                // Preserve absolute paths for the common `unix:///abs/path` form.
                let path = if rest.starts_with('/') {
                    PathBuf::from(rest)
                } else {
                    PathBuf::from(path)
                };
                return Ok(Self::Unix { path });
            }

            #[cfg(not(unix))]
            {
                let _ = rest;
                bail!("unix endpoint is not supported on this platform")
            }
        }

        #[cfg(unix)]
        return Ok(Self::Unix {
            path: PathBuf::from(s),
        });

        #[cfg(not(unix))]
        bail!("invalid endpoint {s:?} (expected: tcp:IP:PORT)")
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parse_tcp_uri_form() {
        let ep = Endpoint::from_str("tcp://127.0.0.1:1234").unwrap();
        assert_eq!(
            ep,
            Endpoint::Tcp {
                addr: SocketAddr::from(([127, 0, 0, 1], 1234))
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn parse_unix_uri_form() {
        let ep = Endpoint::from_str("unix:///tmp/wprs.sock").unwrap();
        assert_eq!(
            ep,
            Endpoint::Unix {
                path: PathBuf::from("/tmp/wprs.sock")
            }
        );
    }

    #[test]
    fn endpoint_display_tcp_uri() {
        let ep = Endpoint::Tcp {
            addr: SocketAddr::from(([127, 0, 0, 1], 1234)),
        };
        assert_eq!(ep.to_string(), "tcp://127.0.0.1:1234");
    }
}

pub mod framing;
pub mod geometry;
pub mod tuple;
pub mod wayland;
pub mod xdg_shell;

use framing::Framed;

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct ClientId(pub u64);

impl ClientId {
    #[cfg(feature = "server")]
    pub fn new(client: &Client) -> Self {
        Self(hash(&client.id()))
    }
}

#[cfg(feature = "server")]
impl From<backend::ClientId> for ClientId {
    fn from(client_id: backend::ClientId) -> Self {
        (&client_id).into()
    }
}

#[cfg(feature = "server")]
impl From<&backend::ClientId> for ClientId {
    fn from(client_id: &backend::ClientId) -> Self {
        Self(hash(client_id))
    }
}

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub enum ObjectId {
    WlSurface(wayland::WlSurfaceId),
    XdgSurface(xdg_shell::XdgSurfaceId),
    XdgToplevel(xdg_shell::XdgToplevelId),
    XdgPopup(xdg_shell::XdgPopupId),
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize, serde_derive::Serialize)]
pub struct Capabilities {
    pub xwayland: bool,
}

// TODO: https://github.com/rust-lang/rfcs/pull/2593 - simplify all the enums.

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub enum Request {
    Surface(wayland::SurfaceRequest),
    CursorImage(wayland::CursorImage),
    Toplevel(xdg_shell::ToplevelRequest),
    Popup(xdg_shell::PopupRequest),
    Data(wayland::DataRequest),
    ClientDisconnected(ClientId),
    Capabilities(Capabilities),
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub enum Event {
    WprsClientConnect,
    Output(wayland::OutputEvent),
    PointerFrame(Vec<wayland::PointerEvent>),
    KeyboardEvent(wayland::KeyboardEvent),
    Toplevel(xdg_shell::ToplevelEvent),
    Popup(xdg_shell::PopupEvent),
    Data(wayland::DataEvent),
    Surface(wayland::SurfaceEvent),
}

// TODO: test that object ids with same value from different clients hash
// differently.
pub fn hash<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

const CHANNEL_SIZE: usize = 1024;

pub trait Serializable:
    Debug
    + Send
    + Archive
    + for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, RancorError>>
    + 'static
{
}

impl<T> Serializable for T where
    T: Debug
        + Send
        + Archive
        + for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, RancorError>>
        + 'static
{
}

fn socket_buffer_limits() -> Result<(usize, usize)> {
    const DEFAULT_SOCKET_BUFFER: usize = 4 * 1024 * 1024;

    let rmem_max = match Ctl::new("net.core.rmem_max").and_then(|c| c.value_string()) {
        Ok(v) => v,
        Err(_) => {
            debug!("sysctl net.core.rmem_max not available; using default");
            return Ok((DEFAULT_SOCKET_BUFFER, DEFAULT_SOCKET_BUFFER));
        },
    };
    let wmem_max = match Ctl::new("net.core.wmem_max").and_then(|c| c.value_string()) {
        Ok(v) => v,
        Err(_) => {
            debug!("sysctl net.core.wmem_max not available; using default");
            return Ok((DEFAULT_SOCKET_BUFFER, DEFAULT_SOCKET_BUFFER));
        },
    };

    let rmem_max: usize = rmem_max.parse().unwrap_or(DEFAULT_SOCKET_BUFFER);
    let wmem_max: usize = wmem_max.parse().unwrap_or(DEFAULT_SOCKET_BUFFER);
    Ok((rmem_max, wmem_max))
}

#[cfg(unix)]
fn enlarge_socket_buffer<F: AsFd>(fd: &F) {
    let (rmem_max, wmem_max) = warn_and_return!(socket_buffer_limits());

    socket::setsockopt(fd, RcvBuf, &rmem_max).warn_and_ignore(loc!());
    socket::setsockopt(fd, SndBuf, &wmem_max).warn_and_ignore(loc!());
}

#[cfg(not(unix))]
fn enlarge_socket_buffer<T>(_fd: &T) {}

trait CloneableStream: Read + Write + Send + 'static {
    fn clone_stream(&self) -> std::io::Result<Self>
    where
        Self: Sized;

    fn shutdown_both(&self) -> std::io::Result<()>;
}

#[cfg(unix)]
impl CloneableStream for UnixStream {
    fn clone_stream(&self) -> std::io::Result<Self> {
        UnixStream::try_clone(self)
    }

    fn shutdown_both(&self) -> std::io::Result<()> {
        self.shutdown(Shutdown::Both)
    }
}

impl CloneableStream for TcpStream {
    fn clone_stream(&self) -> std::io::Result<Self> {
        TcpStream::try_clone(self)
    }

    fn shutdown_both(&self) -> std::io::Result<()> {
        self.shutdown(Shutdown::Both)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Version(String);

impl Version {
    fn new() -> Self {
        Self(env!("SERIALIZATION_TREE_HASH").to_string())
    }

    fn compare_and_warn(&self, other: &Self) {
        if self != other {
            warn!(
                "Self version is {:?}, while other version is {:?}. These versions may be incompatible; if you experience bugs (especially hanging or crashes), restart the server.",
                self, other
            );
        }
    }
}

impl Framed for Version {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        self.0.framed_write(stream)
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        Ok(Self(String::framed_read(stream).location(loc!())?))
    }
}

// TODO: figure out how to shorten the T::Archived bound. This may require
// https://github.com/rust-lang/rust/issues/52662.

pub enum SendType<ST>
where
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    Object(ST),
    RawBuffer(Arc<CompressedShards>),
}

impl<ST> fmt::Debug for SendType<ST>
where
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Object(obj) => write!(f, "Object({obj:?})"),
            Self::RawBuffer(shards) => {
                write!(f, "RawBuffer([{:?}])", shards.uncompressed_size())
            },
        }
    }
}

pub enum RecvType<RT>
where
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    Object(RT),
    RawBuffer(Vec<u8>),
}

impl<RT> fmt::Debug for RecvType<RT>
where
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Object(obj) => write!(f, "Object({obj:?})"),
            Self::RawBuffer(vec) => write!(f, "RawBuffer([{:?}])", vec.len()),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum MessageType {
    Object,
    RawBuffer,
}

impl Framed for MessageType {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        let val: u8 = (*self).into();
        val.framed_write(stream)
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        Self::try_from(u8::framed_read(stream).location(loc!())?).location(loc!())
    }
}

fn read_loop<R, RT>(mut stream: R, output_channel: channel::SyncSender<RecvType<RT>>) -> Result<()>
where
    R: Read,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    // TODO: try tuning this based on the number of cpus the machine has.
    let mut decompressor =
        ShardingDecompressor::new(NonZeroUsize::new(8).unwrap()).location(loc!())?;

    Version::new().compare_and_warn(&Version::framed_read(&mut stream).location(loc!())?);

    loop {
        let message_type = MessageType::framed_read(&mut stream).location(loc!())?;
        debug!("read message_type: {:?}", message_type);

        // read_exact blocks waiting for data, so start the span afterward.
        let _span = debug_span!("serializer_read_loop").entered();

        match message_type {
            MessageType::Object => {
                CompressedShards::streaming_framed_decompress_with(
                    &mut stream,
                    &mut decompressor,
                    |buf| {
                        let obj = RecvType::Object(
                            debug_span!("deserialize")
                                .in_scope(|| rkyv::from_bytes(buf))
                                .location(loc!())?,
                        );
                        debug!("read obj: {obj:?}");
                        output_channel.send(obj)
                        // The error type is not Send + Sync, which anyhow requires.
                            .map_err(|e| anyhow!("{e}"))
                            .location(loc!())?;
                        Ok(())
                    },
                )
                .location(loc!())?;
            },
            MessageType::RawBuffer => {
                let obj = RecvType::RawBuffer(
                    CompressedShards::streaming_framed_decompress_to_owned(
                        &mut stream,
                        &mut decompressor,
                    )
                    .location(loc!())?,
                );
                debug!("read obj: {obj:?}");
                output_channel.send(obj)
                // The error type is not Send + Sync, which anyhow requires.
                    .map_err(|e| anyhow!("{e}"))
                    .location(loc!())?;
            },
        }
    }
}

fn write_loop<W, ST>(
    stream: W,
    input_channel: Receiver<SendType<ST>>,
    other_end_connected: Arc<AtomicBool>,
) -> Result<()>
where
    W: Write,
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    let (_, wmem_max) = socket_buffer_limits().location(loc!())?;
    let mut stream = BufWriter::with_capacity(
        wmem_max, // match the socket's buffer size
        stream,
    );

    // This compressor is only used for objects, not raw buffers, so it doesn't
    // need a lot of threads,
    let mut compressor =
        ShardingCompressor::new(NonZeroUsize::new(1).unwrap(), 1).location(loc!())?;

    Version::new().framed_write(&mut stream).location(loc!())?;
    stream.flush().location(loc!())?;

    loop {
        let obj = match input_channel.recv_timeout(Duration::from_secs(1)) {
            Ok(obj) => obj,
            Err(RecvTimeoutError::Timeout) => {
                if !other_end_connected.load(Ordering::Acquire) {
                    break;
                } else {
                    continue;
                }
            },
            Err(RecvTimeoutError::Disconnected) => {
                break;
            },
        };
        debug!("sending obj: {:?}", obj);

        // recv blocks while waiting for data, so start the span afterward.
        let span = debug_span!(
            "serializer_write_loop",
            uncompressed_size = field::Empty,
            compressed_size = field::Empty,
            compression_ratio = field::Empty
        )
        .entered();
        let (compressed_shards, message_type): (Arc<CompressedShards>, MessageType) = match obj {
            SendType::Object(obj) => {
                let serialized_data = ArcSlice::new(
                    debug_span!("serialize")
                        .in_scope(|| rkyv::to_bytes::<RancorError>(&obj))
                        .location(loc!())?,
                );

                let shards = compressor.compress(NonZeroUsize::new(1).unwrap(), serialized_data);
                (Arc::new(shards), MessageType::Object)
            },
            SendType::RawBuffer(compressed_shards) => (compressed_shards, MessageType::RawBuffer),
        };

        message_type.framed_write(&mut stream).location(loc!())?;
        compressed_shards
            .framed_write(&mut stream)
            .location(loc!())?;
        stream.flush().location(loc!())?;

        // metrics
        {
            let uncompressed_size = compressed_shards.uncompressed_size();
            let compressed_size = compressed_shards.size();
            let compression_ratio = uncompressed_size as f64 / compressed_size as f64;
            span.record("uncompressed_size", field::debug(uncompressed_size));
            span.record("compressed_size", compressed_size);
            span.record("compression_ratio", compression_ratio);

            #[cfg(feature = "tracy")]
            if let Some(tracy_client) = tracy_client::Client::running() {
                tracy_client.plot(
                    tracy_client::plot_name!("compressed_size"),
                    compressed_size as f64,
                );
                tracy_client.plot(
                    tracy_client::plot_name!("compression_ratio"),
                    compression_ratio,
                );
                if compression_ratio > 1.0 {
                    tracy_client.plot(
                        tracy_client::plot_name!("filtered_compression_ratio"),
                        compression_ratio,
                    );
                }
            }
        }
    }
    Ok(())
}

fn spawn_rw_loops<'scope, ST, RT, S>(
    scope: &'scope Scope<'scope, '_>,
    stream: S,
    read_channel_tx: channel::SyncSender<RecvType<RT>>,
    write_channel_rx: Receiver<SendType<ST>>,
    other_end_connected: Arc<AtomicBool>,
) -> Result<(
    ScopedJoinHandle<'scope, Result<()>>,
    ScopedJoinHandle<'scope, Result<()>>,
)>
where
    S: CloneableStream,
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    let read_stream = stream.clone_stream().location(loc!())?;
    let read_thread = scope.spawn(move || read_loop(read_stream, read_channel_tx));

    let write_stream = stream.clone_stream().location(loc!())?;
    let write_thread =
        scope.spawn(move || write_loop(write_stream, write_channel_rx, other_end_connected));

    Ok((read_thread, write_thread))
}

fn accept_loop_inner<ST, RT, S>(
    stream: S,
    read_channel_tx: channel::SyncSender<RecvType<RT>>,
    write_channel_rx: Receiver<SendType<ST>>,
    other_end_connected: Arc<AtomicBool>,
) where
    S: CloneableStream,
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    thread::scope(|scope| {
        let (read_thread, write_thread) = spawn_rw_loops(
            scope,
            stream,
            read_channel_tx,
            write_channel_rx,
            other_end_connected.clone(),
        )
        .unwrap();
        let read_thread_result = utils::join_unwrap(read_thread);
        debug!("read thread joined: {read_thread_result:?}");
        other_end_connected.store(false, Ordering::Relaxed);
        let write_thread_result = utils::join_unwrap(write_thread);
        debug!("write thread joined: {write_thread_result:?}");
    });
}

#[cfg(unix)]
fn accept_loop_unix<ST, RT>(
    listener: UnixListener,
    read_channel_tx: channel::SyncSender<RecvType<RT>>,
    write_channel_rx: Receiver<SendType<ST>>,
    other_end_connected: Arc<AtomicBool>,
) where
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    thread::scope(|_scope| {
        loop {
            debug!("waiting for client connection");
            let (stream, _) = listener.accept().unwrap();
            info!("wprs client connected");
            accept_loop_inner(
                stream.try_clone().unwrap(),
                read_channel_tx.clone(),
                write_channel_rx.clone(),
                other_end_connected.clone(),
            );

            // The usual reason for the read/write threads terminating will be the
            // client disconnect and closing the socket, but they may have terminated
            // because the client sent bad data. In case that was the issue, shut down
            // the stream to disconnect the client.
            stream.shutdown_both().unwrap();
        }
    });
}

fn accept_loop_tcp<ST, RT>(
    listener: TcpListener,
    read_channel_tx: channel::SyncSender<RecvType<RT>>,
    write_channel_rx: Receiver<SendType<ST>>,
    other_end_connected: Arc<AtomicBool>,
) where
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    thread::scope(|_scope| {
        loop {
            debug!("waiting for client connection");
            let (stream, peer) = listener.accept().unwrap();
            info!("wprs client connected from {peer:?}");
            accept_loop_inner(
                stream.try_clone().unwrap(),
                read_channel_tx.clone(),
                write_channel_rx.clone(),
                other_end_connected.clone(),
            );
            stream.shutdown_both().unwrap();
        }
    });
}

fn client_loop<ST, RT, S>(
    stream: S,
    read_channel_tx: channel::SyncSender<RecvType<RT>>,
    write_channel_rx: Receiver<SendType<ST>>,
    other_end_connected: Arc<AtomicBool>,
) -> Result<()>
where
    S: CloneableStream,
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    thread::scope(|scope| {
        let (read_thread, _) = spawn_rw_loops(
            scope,
            stream,
            read_channel_tx,
            write_channel_rx,
            other_end_connected,
        )
        .location(loc!())?;

        // TODO: consider actually look at the error and not printing the reason
        // if was actually just a disconnection and not some other error.
        let result = utils::join_unwrap(read_thread);
        debug!("read thread joined: {:?}", result);
        eprintln!("server disconnected: {result:?}");
        process::exit(1);
    })
}

// TODO: can we create a separate thread to handle serialization/deserialization
// for each client? In principle, each client's stream is independent, but what
// about things like setting the cursor? Rather, which client do we associate
// that with? Any client?
pub struct Serializer<ST, RT>
where
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    read_handle: Option<Channel<RecvType<RT>>>,
    write_handle: DiscardingSender<Sender<SendType<ST>>>,
    other_end_connected: Arc<AtomicBool>,
}

impl<ST, RT> Serializer<ST, RT>
where
    ST: Serializable,
    ST::Archived: Deserialize<ST, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
    RT: Serializable,
    RT::Archived: Deserialize<RT, HighDeserializer<RancorError>>
        + for<'a> bytecheck::CheckBytes<HighValidator<'a, RancorError>>,
{
    pub fn new_server_endpoint(endpoint: Endpoint) -> Result<Self> {
        endpoint.warn_if_non_loopback("wprs server endpoint");

        match endpoint {
            Endpoint::Unix { path } => Self::new_server(&path),
            Endpoint::Tcp { addr } => Self::new_server_tcp(addr),
        }
    }

    pub fn new_client_endpoint(endpoint: Endpoint) -> Result<Self> {
        endpoint.warn_if_non_loopback("wprs client endpoint");

        match endpoint {
            Endpoint::Unix { path } => Self::new_client(&path),
            Endpoint::Tcp { addr } => Self::new_client_tcp(addr),
        }
    }

    pub fn new_server<P: AsRef<Path>>(sock_path: P) -> Result<Self> {
        #[cfg(not(unix))]
        {
            let _ = sock_path;
            bail!("unix socket server is not supported on this platform")
        }

        #[cfg(unix)]
        {
            let listener = utils::bind_user_socket(sock_path).location(loc!())?;
            enlarge_socket_buffer(&listener);

            let (reader_tx, reader_rx): (channel::SyncSender<RecvType<RT>>, Channel<RecvType<RT>>) =
                channel::sync_channel(CHANNEL_SIZE);
            let (writer_tx, writer_rx): (Sender<SendType<ST>>, Receiver<SendType<ST>>) =
                crossbeam_channel::unbounded();
            let other_end_connected = Arc::new(AtomicBool::new(false));

            {
                let other_end_connected = other_end_connected.clone();
                thread::spawn(move || {
                    accept_loop_unix(listener, reader_tx, writer_rx, other_end_connected)
                });
            }

            let writer_tx = DiscardingSender {
                sender: writer_tx,
                actually_send: other_end_connected.clone(),
            };

            Ok(Self {
                read_handle: Some(reader_rx),
                write_handle: writer_tx,
                other_end_connected,
            })
        }
    }

    pub fn new_client<P: AsRef<Path>>(sock_path: P) -> Result<Self> {
        #[cfg(not(unix))]
        {
            let _ = sock_path;
            bail!("unix socket client is not supported on this platform")
        }

        #[cfg(unix)]
        {
            let stream = UnixStream::connect(sock_path).location(loc!())?;
            enlarge_socket_buffer(&stream);

            let (reader_tx, reader_rx): (channel::SyncSender<RecvType<RT>>, Channel<RecvType<RT>>) =
                channel::sync_channel(CHANNEL_SIZE);
            let (writer_tx, writer_rx): (Sender<SendType<ST>>, Receiver<SendType<ST>>) =
                crossbeam_channel::unbounded();
            let other_end_connected = Arc::new(AtomicBool::new(true));

            {
                let other_end_connected = other_end_connected.clone();
                thread::spawn(move || {
                    client_loop(stream, reader_tx, writer_rx, other_end_connected)
                });
            }

            let writer_tx = DiscardingSender {
                sender: writer_tx,
                actually_send: other_end_connected.clone(),
            };

            Ok(Self {
                read_handle: Some(reader_rx),
                write_handle: writer_tx,
                other_end_connected,
            })
        }
    }

    pub fn new_server_tcp(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).location(loc!())?;
        #[cfg(unix)]
        enlarge_socket_buffer(&listener);

        let (reader_tx, reader_rx): (channel::SyncSender<RecvType<RT>>, Channel<RecvType<RT>>) =
            channel::sync_channel(CHANNEL_SIZE);
        let (writer_tx, writer_rx): (Sender<SendType<ST>>, Receiver<SendType<ST>>) =
            crossbeam_channel::unbounded();
        let other_end_connected = Arc::new(AtomicBool::new(false));

        {
            let other_end_connected = other_end_connected.clone();
            thread::spawn(move || {
                accept_loop_tcp(listener, reader_tx, writer_rx, other_end_connected)
            });
        }

        let writer_tx = DiscardingSender {
            sender: writer_tx,
            actually_send: other_end_connected.clone(),
        };

        Ok(Self {
            read_handle: Some(reader_rx),
            write_handle: writer_tx,
            other_end_connected,
        })
    }

    pub fn new_client_tcp(addr: SocketAddr) -> Result<Self> {
        let stream = TcpStream::connect(addr).location(loc!())?;
        let _ = stream.set_nodelay(true);
        #[cfg(unix)]
        enlarge_socket_buffer(&stream);

        let (reader_tx, reader_rx): (channel::SyncSender<RecvType<RT>>, Channel<RecvType<RT>>) =
            channel::sync_channel(CHANNEL_SIZE);
        let (writer_tx, writer_rx): (Sender<SendType<ST>>, Receiver<SendType<ST>>) =
            crossbeam_channel::unbounded();
        let other_end_connected = Arc::new(AtomicBool::new(true));

        {
            let other_end_connected = other_end_connected.clone();
            thread::spawn(move || client_loop(stream, reader_tx, writer_rx, other_end_connected));
        }

        let writer_tx = DiscardingSender {
            sender: writer_tx,
            actually_send: other_end_connected.clone(),
        };

        Ok(Self {
            read_handle: Some(reader_rx),
            write_handle: writer_tx,
            other_end_connected,
        })
    }

    // TODO: https://github.com/rust-lang/rfcs/issues/1215 - Ideally this would
    // return an &mut, but we can't afford to tie up the entire serializer for,
    // well, ever. Change this to return an &mut once rust supports partial
    // borrowing of struct fields.
    // TODO: rename to receiver.
    pub fn reader(&mut self) -> Option<Channel<RecvType<RT>>> {
        self.read_handle.take()
    }

    // TODO: rename to writer.
    pub fn writer(&self) -> InfallibleSender<'_, DiscardingSender<Sender<SendType<ST>>>> {
        InfallibleSender::new(self.write_handle.clone(), self)
    }

    pub fn other_end_connected(&mut self) -> bool {
        self.other_end_connected.load(Ordering::Acquire)
    }

    pub fn set_other_end_connected(&mut self, state: bool) {
        self.other_end_connected.store(state, Ordering::Relaxed);
    }
}
