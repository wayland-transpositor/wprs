use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use anyhow::ensure;
use calloop::EventLoop as CalloopEventLoop;
use calloop::channel::Event as CalloopChannelEvent;
use calloop::timer::TimeoutAction;
use calloop::timer::Timer;

use crate::buffer_pointer::BufferPointer;
use crate::filtering;
use crate::prelude::*;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::RecvType;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::SendType;
use crate::protocols::wprs::Serializer;
use crate::protocols::wprs::core::handshake;
use crate::protocols::wprs::wayland::BufferAssignment;
use crate::protocols::wprs::wayland::BufferData;
use crate::protocols::wprs::wayland::CompressedBufferData;
use crate::server::runtime::backend::BackendObservation;
use crate::server::runtime::backend::PollingBackend;
use crate::sharding_compression::ShardingCompressor;

struct State<B> {
    backend: B,
    serializer: Serializer<Request, Event>,
    compressor: ShardingCompressor,
}

fn send_initial_snapshot<B: PollingBackend>(state: &mut State<B>) -> Result<()> {
    let caps = state.backend.capabilities();
    state
        .serializer
        .writer()
        .send(SendType::Object(Request::Capabilities(caps)));

    let snapshot = state.backend.initial_snapshot().location(loc!())?;
    for surface in snapshot {
        for msg in handshake::surface_messages(surface.state).location(loc!())? {
            state.serializer.writer().send(msg);
        }
    }
    Ok(())
}

fn apply_observation<B: PollingBackend>(
    state: &mut State<B>,
    obs: BackendObservation,
) -> Result<()> {
    match obs {
        BackendObservation::SurfaceCommit { state: mut s, bgra } => {
            if let Some(bgra) = bgra {
                let Some(BufferAssignment::New(buf)) = s.buffer.as_mut() else {
                    bail!("SurfaceCommit with frame requires BufferAssignment::New")
                };

                let expected_len = buf.metadata.len();
                ensure!(
                    bgra.len() == expected_len,
                    "bgra size mismatch: expected {expected_len} bytes, got {}",
                    bgra.len()
                );

                let bgra_ptr = bgra.as_ptr();
                // SAFETY: `bgra_ptr` points to `bgra.len()` bytes for the duration of this call.
                let data = unsafe { BufferPointer::new(&bgra_ptr, bgra.len()) };
                let shards = filtering::filter_and_compress(data, &mut state.compressor);
                buf.data = BufferData::Compressed(CompressedBufferData(Arc::new(shards)));
            }

            for msg in handshake::surface_messages(s).location(loc!())? {
                state.serializer.writer().send(msg);
            }
        },
    }
    Ok(())
}

/// Runs a platform-neutral server loop.
///
/// - Waits for `Event::WprsClientConnect`.
/// - Sends capabilities + initial snapshot.
/// - Periodically polls the backend and sends commits.
/// - Forwards client events to the backend.
pub fn run<B: PollingBackend>(
    backend: B,
    serializer: Serializer<Request, Event>,
    tick_interval: Duration,
) -> Result<()> {
    // NOTE: This runner is polling-based and intended for capture-style backends.
    let mut event_loop = CalloopEventLoop::<State<B>>::try_new().location(loc!())?;

    let mut state = State {
        backend,
        serializer,
        compressor: ShardingCompressor::new(NonZeroUsize::new(16).unwrap(), 1).location(loc!())?,
    };

    let reader = state
        .serializer
        .reader()
        .ok_or_else(|| anyhow!("serializer reader already taken"))
        .location(loc!())?;

    event_loop
        .handle()
        .insert_source(reader, |event, _metadata, state| {
            if let CalloopChannelEvent::Msg(msg) = event {
                match msg {
                    RecvType::Object(Event::WprsClientConnect) => {
                        state.serializer.set_other_end_connected(true);
                        send_initial_snapshot(state).log_and_ignore(loc!());
                    },
                    RecvType::Object(other) => {
                        state
                            .backend
                            .handle_client_event(other)
                            .log_and_ignore(loc!());
                    },
                    RecvType::RawBuffer(_) => {
                        warn!("server received RawBuffer from client; ignoring")
                    },
                }
            }
        })
        .map_err(|e| anyhow!("insert_source(serializer reader) failed: {e:?}"))?;

    event_loop
        .handle()
        .insert_source(Timer::from_duration(tick_interval), move |_, _, state| {
            if !state.serializer.other_end_connected() {
                return TimeoutAction::ToDuration(tick_interval);
            }

            match state.backend.poll() {
                Ok(observations) => {
                    for obs in observations {
                        apply_observation(state, obs).log_and_ignore(loc!());
                    }
                },
                Err(err) => {
                    warn!("backend poll failed: {err:?}");
                },
            }

            TimeoutAction::ToDuration(tick_interval)
        })
        .map_err(|e| anyhow!("insert_source(timer) failed: {e:?}"))?;

    event_loop.run(None, &mut state, |_| {}).location(loc!())?;
    Ok(())
}
