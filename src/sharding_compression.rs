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

use std::collections::HashMap;
use std::convert::Into;
use std::fmt;
use std::io::Read;
use std::io::Write;
use std::mem;
use std::num::NonZeroUsize;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use divbuf::DivBufMut;
use divbuf::DivBufShared;
use fallible_iterator::FallibleIterator;
use fallible_iterator::IteratorExt;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;
use rkyv::rancor::Error as RancorError;
use rkyv::util::AlignedVec;
use zstd::bulk::Compressor;
use zstd::bulk::Decompressor;

use crate::arc_slice::ArcSlice;
use crate::prelude::*;
use crate::serialization::framing::Framed;

// TODO: benchmark this and pick a value based on that.
pub const MIN_SIZE_TO_COMPRESS: usize = 4096;

#[derive(Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct CompressedShard {
    pub idx: usize,
    pub uncompressed_size: usize,
    pub compression: bool,
    pub data: Vec<u8>,
}

impl CompressedShard {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.len() == 0
    }
}

impl fmt::Debug for CompressedShard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompressedShard")
            .field("idx", &self.idx)
            .field("compression", &self.compression)
            .field("data", &format_args!("Vec<u8>[{:?}]", &self.data.len()))
            .finish()
    }
}

impl Framed for CompressedShard {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        self.idx.framed_write(stream).location(loc!())?;
        self.uncompressed_size
            .framed_write(stream)
            .location(loc!())?;
        self.compression.framed_write(stream).location(loc!())?;
        self.data.framed_write(stream).location(loc!())?;
        Ok(())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let idx = usize::framed_read(stream).location(loc!())?;
        let uncompressed_size = usize::framed_read(stream).location(loc!())?;
        let compression = bool::framed_read(stream).location(loc!())?;
        // TODO: this fails on client disconnection
        let data = Vec::<u8>::framed_read(stream).location(loc!())?;
        Ok(Self {
            idx,
            uncompressed_size,
            compression,
            data,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Default, Archive, Deserialize, Serialize)]
pub struct CompressedShards {
    pub shards: Vec<CompressedShard>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CompressedShardsRef<'a> {
    pub shards: &'a CompressedShard,
}

impl CompressedShards {
    pub fn new(mut shards: Vec<CompressedShard>) -> Self {
        shards.sort_by_key(|k| k.idx);
        Self { shards }
    }

    pub fn len(&self) -> usize {
        self.shards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shards.is_empty()
    }

    pub fn indices(&self) -> Vec<usize> {
        self.shards.iter().map(|shard| shard.idx).collect()
    }

    pub fn uncompressed_size(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.uncompressed_size)
            .sum()
    }

    pub fn size(&self) -> usize {
        self.shards.iter().map(CompressedShard::len).sum()
    }

    #[instrument(skip_all, level = "debug")]
    pub fn streaming_framed_decompress_with<F, T, R: Read>(
        stream: &mut R,
        decompressor: &mut ShardingDecompressor,
        f: F,
    ) -> Result<T>
    where
        F: FnOnce(&[u8]) -> Result<T>,
    {
        let serialized_indices = AlignedVec::framed_read(stream).location(loc!())?;
        let indices =
            rkyv::from_bytes::<Vec<usize>, RancorError>(&serialized_indices).location(loc!())?;
        debug!("read indices: {:?}", indices);

        let uncompressed_size = usize::framed_read(stream).location(loc!())?;
        debug!("read uncompressed_size: {:?}", uncompressed_size);

        let shards = (0..indices.len())
            .map(|_| CompressedShard::framed_read(stream))
            .transpose_into_fallible();
        debug!("read data");

        decompressor.decompress_with(&indices, uncompressed_size, shards, f)
    }

    #[instrument(skip_all, level = "debug")]
    pub fn streaming_framed_decompress_to_owned<R: Read>(
        stream: &mut R,
        decompressor: &mut ShardingDecompressor,
    ) -> Result<Vec<u8>> {
        let serialized_indices = AlignedVec::framed_read(stream).location(loc!())?;
        let indices =
            rkyv::from_bytes::<Vec<usize>, RancorError>(&serialized_indices).location(loc!())?;
        debug!("read indices: {:?}", indices);

        let uncompressed_size = usize::framed_read(stream).location(loc!())?;
        debug!("read uncompressed_size: {:?}", uncompressed_size);

        let shards = (0..indices.len())
            .map(|_| CompressedShard::framed_read(stream))
            .transpose_into_fallible();
        debug!("read data");

        decompressor.decompress_to_owned(&indices, uncompressed_size, shards)
    }
}

impl Framed for CompressedShards {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        let indices = self.indices();
        rkyv::to_bytes::<RancorError>(&indices)
            .location(loc!())?
            .framed_write(stream)
            .location(loc!())?;

        self.uncompressed_size()
            .framed_write(stream)
            .location(loc!())?;

        for shard in self.shards.iter() {
            shard.framed_write(stream).location(loc!())?;
            // Flush here instaed of after writing all the frames so that the client
            // can start decompressing the shards sooner.
            stream.flush().location(loc!())?;
        }

        Ok(())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let serialized_indices = AlignedVec::framed_read(stream).location(loc!())?;
        let indices =
            rkyv::from_bytes::<Vec<usize>, RancorError>(&serialized_indices).location(loc!())?;
        let shards: Vec<CompressedShard> = (0..indices.len())
            .map(|_| CompressedShard::framed_read(stream))
            .transpose_into_fallible()
            .collect()
            .location(loc!())?;

        Ok(Self { shards })
    }
}

fn spawn_compressor(
    compression_level: i32,
    input_rx: Receiver<(usize, Box<dyn AsRef<[u8]> + Send + Sync + 'static>)>,
    output_tx: Sender<CompressedShard>,
) -> Result<()> {
    let mut compressor = Compressor::new(compression_level).location(loc!())?;
    compressor.long_distance_matching(true).location(loc!())?;
    thread::spawn(move || {
        // The iterator (and, consequently, the thread) will terminate when all
        // the input senders (which are all in the ShardingCompressor) are
        // dropped.
        for (idx, input) in input_rx {
            let input = (*input).as_ref();
            // We could pre-allocate a buffer at the end of the loop, while
            // waiting for the next input, and use compress_to_buffer, but that
            // doesn't result in a significant speedup here.
            //
            // This will allocate as much space as it needs, so compression
            // should never panic.
            let compression = input.len() > MIN_SIZE_TO_COMPRESS;
            let data = if compression {
                compressor.compress(input).unwrap()
            } else {
                input.to_vec()
            };

            // This will be an error when the ShardingDecompressor is dropped,
            // but the for loop (and consequently this thread) will terminate at
            // the same time for the same reason.
            _ = output_tx.send(CompressedShard {
                idx,
                uncompressed_size: input.len(),
                compression,
                data,
            });
        }
    });
    Ok(())
}

pub struct ShardingCompressor {
    compressor_input: Sender<(usize, Box<dyn AsRef<[u8]> + Send + Sync + 'static>)>,
    compressor_output: Receiver<CompressedShard>,
}

impl ShardingCompressor {
    pub fn new(n_compressors: NonZeroUsize, compression_level: i32) -> Result<Self> {
        // These channels will have at most n_shards items in them, but we only
        // know n_shards when compress is called, not now.
        let (compressor_input_tx, compressor_input_rx) = crossbeam_channel::unbounded();
        let (compressor_output_tx, compressor_output_rx) = crossbeam_channel::unbounded();
        for _ in 0..n_compressors.get() {
            spawn_compressor(
                compression_level,
                compressor_input_rx.clone(),
                compressor_output_tx.clone(),
            )
            .location(loc!())?;
        }

        Ok(Self {
            compressor_input: compressor_input_tx,
            compressor_output: compressor_output_rx,
        })
    }

    #[instrument(skip_all, level = "debug")]
    pub fn compress(&mut self, n_shards: NonZeroUsize, data: ArcSlice<u8>) -> CompressedShards {
        let n_shards = n_shards.get();
        let len = data.len();
        let chunk_size = len / n_shards;
        debug!("chunk_size: {:?}", chunk_size);
        let chunks = data.chunks(chunk_size);
        let accepting_compressor = self.begin();
        for (i, chunk) in chunks.enumerate() {
            accepting_compressor.compress_shard(i * chunk_size, chunk);
        }
        accepting_compressor.collect_shards()
    }

    // Intentionally &mut self to only allow a single session at a time. This
    // ensures a consistent set of input and output data.
    pub fn begin(&mut self) -> ShardingCompressorSession<'_> {
        ShardingCompressorSession {
            compressor: self,
            pending_shards: 0.into(),
        }
    }
}

pub struct ShardingCompressorSession<'a> {
    compressor: &'a mut ShardingCompressor,
    pending_shards: AtomicUsize,
}

impl ShardingCompressorSession<'_> {
    pub fn compress_shard(&self, idx: usize, data: impl AsRef<[u8]> + Send + Sync + 'static) {
        self.compressor
            .compressor_input
            .send((idx, Box::new(data)))
            .unwrap();
        self.pending_shards.fetch_add(1, Ordering::Relaxed);
    }

    pub fn iter_shards(&self) -> impl Iterator<Item = CompressedShard> + '_ {
        let n = self.pending_shards.swap(0, Ordering::AcqRel);
        // Will only panic is the other end disconnected, which should never
        // happen.
        (0..n).map(|_| self.compressor.compressor_output.recv().unwrap())
    }

    #[instrument(skip_all, level = "debug")]
    pub fn collect_shards(self) -> CompressedShards {
        CompressedShards::new(self.iter_shards().collect())
    }
}

/// # Panics
/// If there is a bug and the decompression buffer wasn't resized to be large enough.
pub fn spawn_decompressor(
    input_rx: Receiver<(CompressedShard, DivBufMut)>,
    output_tx: Sender<()>,
) -> Result<()> {
    let mut decompressor = Decompressor::new().location(loc!())?;
    thread::spawn(move || {
        // The iterator (and, consequently, the thread) will terminate when all
        // the input senders (which are all in the ShardingDecompressor) are
        // dropped.
        for (input, mut output) in input_rx.iter() {
            let _span = debug_span!("decompressor").entered();
            if input.compression {
                // We made DivBufMut large enough, so this should never panic.
                decompressor
                    .decompress_to_buffer(&input.data, output.as_mut())
                    .unwrap();
            } else {
                // The last output block will be larger than the data.
                let output = &mut output[0..input.data.len()];
                output.copy_from_slice(&input.data);
            }
            drop(output); // release our handle

            // This will be an error when the ShardingDecompressor is dropped,
            // but the for loop (and consequently this thread) will terminate at
            // the same time for the same reason.
            _ = output_tx.send(());
        }
    });
    Ok(())
}

pub struct ShardingDecompressor {
    decompressor_input: Sender<(CompressedShard, DivBufMut)>,
    decompressor_output: Receiver<()>,
    buffer: DivBufShared,
}

impl ShardingDecompressor {
    pub fn new(n_decompressors: NonZeroUsize) -> Result<Self> {
        // These channels will have at most n_shards items in them, but we only
        // know n_shards when decompress is called, not now.
        let (decompressor_input_tx, decompressor_input_rx) = crossbeam_channel::unbounded();
        let (decompressor_output_tx, decompressor_output_rx) = crossbeam_channel::unbounded();

        for _ in 0..n_decompressors.get() {
            spawn_decompressor(
                decompressor_input_rx.clone(),
                decompressor_output_tx.clone(),
            )
            .location(loc!())?;
        }

        Ok(Self {
            decompressor_input: decompressor_input_tx,
            decompressor_output: decompressor_output_rx,
            // Make the buffer larger than most frames we'll see to avoid the
            // performance hit of growing it later, but it will be grown if
            // necessary anyway. A 4k frame is ~33MB.
            buffer: DivBufShared::from(vec![0; 36_000_000]),
        })
    }

    /// IMPORTANT:
    /// * Indices must be sorted.
    /// * If indices != compressed_shards.collect().indices(), this function
    ///   will either panic or return corrupt data.
    /// * If indices.len() < compressed_shards.len(), this function
    ///   will hang forever.
    /// * If indices.len() > compressed_shards.len(), the decompressed data will
    ///   be incomplete or have chunks missing.
    #[instrument(skip_all, level = "debug")]
    fn decompress_impl<E: std::convert::From<anyhow::Error> + Send + Sync + 'static>(
        &mut self,
        indices: &[usize],
        uncompressed_size: usize,
        mut compressed_shards: impl FallibleIterator<Item = CompressedShard, Error = E>,
    ) -> std::result::Result<(), E>
    where
        Result<(), E>: anyhow::Context<(), E>,
    {
        // TODO(https://github.com/rust-lang/rust/issues/78485): use
        // DivShared.uninitialized to allocate the buffer on demand here. The
        // allocation is fast enough and it simplifies the code, but until the
        // read_buf rfc is implemented, uninitialized is technically undefined
        // behaviour, even though in practice it is likely fine and existing
        // software relies on uninitialized Vec<u8>s.

        // resize should be a NOP if the buffer is large enough, but that seems
        // to not be the case for unknown reasons and it's actually quite
        // expensive. This is likely a bug somewhere. In the meantime, only
        // resize if the buffer really isn't large enough.
        if uncompressed_size > self.buffer.len() {
            let _span = debug_span!("resize");
            debug!(
                "resizing buffer from {:?} to {:?}",
                self.buffer.len(),
                uncompressed_size
            );
            self.buffer = DivBufShared::from(vec![0; uncompressed_size]);
        }

        {
            // We need mut_buf to split off blocks for each decompressor but
            // need it gone afterwards so that after the decompressors are done,
            // the only remaining reference to the data is self.buffer.
            let mut mut_buf = self.buffer.try_mut().location(loc!())?;

            let mut prev_idx = 0;
            let mut output_divbufs: Vec<DivBufMut> = indices
                .iter()
            // The first index is 0, we don't need to split on it.
                .skip(1)
                .map(|idx| {
                    let buf = mut_buf.split_to(idx - prev_idx);
                    prev_idx = *idx;
                    buf
                })
                .collect();
            output_divbufs.push(mut_buf);

            let mut index_divbuf_map: HashMap<usize, DivBufMut> =
                indices.iter().cloned().zip(output_divbufs).collect();

            while let Some(shard) = compressed_shards.next()? {
                let divbuf = index_divbuf_map.remove(&shard.idx).unwrap();
                self.decompressor_input.send((shard, divbuf)).unwrap();
            }
        }

        Ok(())
    }

    #[instrument(skip_all, level = "debug")]
    fn iter_shards(&self, n: usize) -> impl Iterator<Item = ()> + '_ {
        // Will only panic is the other end disconnected, which should never
        // happen.
        (0..n).map(|_| self.decompressor_output.recv().unwrap())
    }

    #[instrument(skip_all, level = "debug")]
    fn collect_shards(&self, n: usize) {
        self.iter_shards(n).for_each(|_| {});
    }

    /// IMPORTANT: see note on decompress_impl.
    #[instrument(skip_all, level = "debug")]
    pub fn decompress_with<F, T, E: std::convert::From<anyhow::Error> + Send + Sync + 'static>(
        &mut self,
        indices: &[usize],
        uncompressed_size: usize,
        compressed_shards: impl FallibleIterator<Item = CompressedShard, Error = E>,
        f: F,
    ) -> Result<T>
    where
        Result<(), E>: anyhow::Context<(), E>,
        F: FnOnce(&[u8]) -> Result<T>,
    {
        if indices.is_empty() {
            bail!("Cannot call decompress_with on empty indices.");
        }

        self.decompress_impl(indices, uncompressed_size, compressed_shards)
            .location(loc!())?;
        self.collect_shards(indices.len());

        // We dropped mut_buf and all the buffers in the decompressor threads
        // have been freed already, so this should never fail.
        let decompressed_data = self.buffer.try_const().unwrap().split_to(uncompressed_size);
        f(&decompressed_data)
    }

    /// IMPORTANT: see note on decompress_impl.
    #[instrument(skip_all, level = "debug")]
    pub fn decompress_to_owned<E: std::convert::From<anyhow::Error> + Send + Sync + 'static>(
        &mut self,
        indices: &[usize],
        uncompressed_size: usize,
        compressed_shards: impl FallibleIterator<Item = CompressedShard, Error = E>,
    ) -> Result<Vec<u8>>
    where
        Result<(), E>: anyhow::Context<(), E>,
    {
        if indices.is_empty() {
            return Ok(vec![]);
        }

        self.decompress_impl(indices, uncompressed_size, compressed_shards)
            .location(loc!())?;
        self.collect_shards(indices.len());

        let len = self.buffer.len();
        let buf = mem::replace(&mut self.buffer, DivBufShared::from(vec![0; len]));
        // We dropped mut_buf and all the buffers in the decompressor threads
        // have been freed already, so this should never fail.
        let mut vec: Vec<u8> = buf.try_into().unwrap();
        vec.truncate(uncompressed_size);
        Ok(vec)
    }
}
