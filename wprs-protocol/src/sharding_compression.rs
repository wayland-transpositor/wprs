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

use std::io::Read;
use std::io::Write;
use std::mem;
use std::num::NonZeroUsize;
use std::thread;

use anyhow::Error;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use divbuf::DivBufMut;
use divbuf::DivBufShared;
use fallible_iterator::FallibleIterator;
use wprs_common::utils;
use zstd::bulk;

use crate::arc_slice::ArcSlice;
use crate::prelude::*;

// TODO: benchmark this and pick a value based on that.
pub const MIN_SIZE_TO_COMPRESS: usize = 4096;

#[derive(Clone, Eq, PartialEq)]
pub struct CompressedShard {
    pub idx: u32,
    pub compression: u32, // TODO: this is terrible
    pub data: Vec<u8>,
}

impl CompressedShard {
    pub fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        debug!("writing idx: {}", self.idx);
        stream.write_all(&self.idx.to_le_bytes()).location(loc!())?;

        debug!("writing compression: {}", self.idx);
        stream
            .write_all(&self.compression.to_le_bytes())
            .location(loc!())?;

        let size = self.data.len() as u32;
        debug!("writing size: {}", size);
        stream.write_all(&size.to_le_bytes()).location(loc!())?;

        debug!("writing data");
        stream.write_all(&self.data).location(loc!())?;

        // Flush here instaed of after writing all the frames so that the client
        // can start decompressing the shards sooner.
        stream.flush().location(loc!())?;
        Ok(())
    }

    pub fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let mut buf: [u8; 12] = [0; 12];

        stream.read_exact(&mut buf[0..4])?;
        // from_le_bytes will fail if the slice is the wrong length, so this
        // (and the calls below) should never fail.
        let idx = u32::from_le_bytes(buf[0..4].try_into().location(loc!())?);
        debug!("read idx: {}", idx);

        stream.read_exact(&mut buf[4..8])?;
        let compression = u32::from_le_bytes(buf[4..8].try_into().location(loc!())?);
        debug!("read compression: {}", compression);

        stream.read_exact(&mut buf[8..12])?;
        let len = u32::from_le_bytes(buf[8..12].try_into().location(loc!())?);
        debug!("read len: {}", len);

        let mut data = vec![0; len.to_owned() as usize];
        // TODO: this fails on client disconnection
        stream.read_exact(&mut data)?;
        debug!("read data");

        Ok(Self {
            idx,
            compression,
            data,
        })
    }
}

fn spawn_compressor(
    compression_level: i32,
    input_rx: Receiver<(usize, ArcSlice<u8>)>,
    output_tx: Sender<CompressedShard>,
) -> Result<()> {
    let mut compressor = bulk::Compressor::new(compression_level).location(loc!())?;
    compressor.long_distance_matching(true).location(loc!())?;
    thread::spawn(move || {
        // The iterator (and, consequently, the thread) will terminate when all
        // the input senders (which are all in the ShardingCompressor) are
        // dropped.
        for (idx, input) in input_rx {
            let _span = debug_span!("compressor").entered();
            // We could pre-allocate a buffer at the end of the loop, while
            // waiting for the next input, and use compress_to_buffer, but that
            // doesn't result in a significant speedup here.
            //
            // This will allocate as much space as it needs, so compression
            // should never panic.
            let compression = if input.len() > MIN_SIZE_TO_COMPRESS {
                1
            } else {
                0
            };
            let data = if compression == 0 {
                input.as_ref().to_vec()
            } else {
                compressor.compress(&input).unwrap()
            };

            // This will be an error when the ShardingDecompressor is dropped,
            // but the for loop (and consequently this thread) will terminate at
            // the same time for the same reason.
            _ = output_tx.send(CompressedShard {
                idx: idx as u32,
                compression,
                data,
            });
        }
    });
    Ok(())
}

pub struct ShardingCompressor {
    compressor_input: Sender<(usize, ArcSlice<u8>)>,
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
    pub fn compress(
        &self,
        n_shards: NonZeroUsize,
        data: ArcSlice<u8>,
    ) -> impl Iterator<Item = CompressedShard> + '_ {
        let n_shards = n_shards.get();
        let size = data.len();
        let chunk_size = size / n_shards;
        debug!("chunk_size: {}", chunk_size);
        let chunks = data.chunks(chunk_size);
        let actual_n_shards = chunks.len();
        for (i, chunk) in chunks.enumerate() {
            self.compressor_input.send((i, chunk)).unwrap();
        }

        // Will only panic is the other end disconnected, which should never
        // happen.
        (0..actual_n_shards).map(|_| self.compressor_output.recv().unwrap())
    }
}

/// # Panics
/// If there is a bug and the decompression buffer wasn't resized to be large enough.
pub fn spawn_decompressor(
    input_rx: Receiver<(CompressedShard, DivBufMut)>,
    output_tx: Sender<()>,
) -> Result<()> {
    let mut decompressor = bulk::Decompressor::new().location(loc!())?;
    thread::spawn(move || {
        // The iterator (and, consequently, the thread) will terminate when all
        // the input senders (which are all in the ShardingDecompressor) are
        // dropped.
        for (input, mut output) in input_rx.iter() {
            let _span = debug_span!("decompressor").entered();
            if input.compression == 0 {
                // The last output block will be larger than the data.
                let output = &mut output[0..input.data.len()];
                output.copy_from_slice(&input.data);
            } else {
                // We made DivBufMut large enough, so this should never panic.
                decompressor
                    .decompress_to_buffer(&input.data, output.as_mut())
                    .unwrap();
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

    /// Panics
    /// If there are more compressed_shards than expected based on uncompressed_size
    /// and n_shards.
    #[instrument(skip_all, level = "debug")]
    fn decompress_impl(
        &mut self,
        n_shards: NonZeroUsize,
        uncompressed_size: usize,
        mut compressed_shards: impl FallibleIterator<Item = CompressedShard, Error = Error>,
    ) -> Result<()> {
        let n_shards = n_shards.get();

        // It would be nicer to use DivBufMut.chunks, but that is based on the
        // length of the buffer and resize is more expensive than it should be,
        // so do the chunking manually based on uncompressed_size.
        //
        // TODO(https://github.com/rust-lang/rust/issues/78485): use
        // DivShared.uninitialized to allocate the buffer on demand here. The
        // allocation is fast enough and it simplifies the code, but until the
        // read_buf rfc is implemented, uninitialized is technically undefined
        // behaviour, even though in practice it is likely fine and existing
        // software relies on uninitialized Vec<u8>s.
        let chunk_size = uncompressed_size / n_shards;
        let actual_n_shards = utils::n_chunks(uncompressed_size, chunk_size);
        let needed_buffer_size = actual_n_shards * chunk_size;
        debug!(
            "chunk_size: {}, actual_n_shards: {}, needed_buffer_size {}",
            chunk_size, actual_n_shards, needed_buffer_size
        );

        // resize should be a NOP if the buffer is large enough, but that seems
        // to not be the case for unknown reasons and it's actually quite
        // expensive. This is likely a bug somewhere. In the meantime, only
        // resize if the buffer really isn't large enough.
        if needed_buffer_size > self.buffer.len() {
            let _span = debug_span!("resize");
            debug!(
                "resizing buffer from {} to {}",
                self.buffer.len(),
                needed_buffer_size
            );
            self.buffer = DivBufShared::from(vec![0; needed_buffer_size]);
        }

        {
            // We're need mut_buf to split off blocks for each decompressor but
            // need it gone afterwards so that after the decompressors are done,
            // the only remaining reference to the data is self.buffer.
            let mut mut_buf = self
                .buffer
                .try_mut()
                .map_err(|s| anyhow!(s))
                .location(loc!())?;

            // Put the blocks in Options so we can take them out without changing the
            // indices; the index of the shard is used as the key for this array.
            let mut outs: Vec<Option<DivBufMut>> = (0..actual_n_shards)
                .map(|_| Some(mut_buf.split_to(chunk_size)))
                .collect();

            while let Some(shard) = compressed_shards.next()? {
                let out_block = outs.get_mut(shard.idx as usize).unwrap().take().unwrap();
                self.decompressor_input.send((shard, out_block)).unwrap();
            }
        }

        for _ in 0..actual_n_shards {
            // This should only panic if all the decompressor threads died, but
            // none of them should ever die.
            self.decompressor_output.recv().unwrap();
        }

        Ok(())
    }

    /// Panics
    /// If there are more compressed_shards than expected based on uncompressed_size
    /// and n_shards.
    #[instrument(skip_all, level = "debug")]
    pub fn decompress_with<F, T>(
        &mut self,
        n_shards: NonZeroUsize,
        uncompressed_size: usize,
        compressed_shards: impl FallibleIterator<Item = CompressedShard, Error = Error>,
        f: F,
    ) -> Result<T>
    where
        F: FnOnce(&[u8]) -> Result<T>,
    {
        self.decompress_impl(n_shards, uncompressed_size, compressed_shards)
            .location(loc!())?;

        // We dropped mut_buf and all the buffers in the decompressor threads
        // have been freed already, so this should never fail.
        let decompressed_data = self.buffer.try_const().unwrap().split_to(uncompressed_size);
        f(&decompressed_data)
    }

    /// Panics
    /// If there are more compressed_shards than expected based on uncompressed_size
    /// and n_shards.
    #[instrument(skip_all, level = "debug")]
    pub fn decompress_to_owned(
        &mut self,
        n_shards: NonZeroUsize,
        uncompressed_size: usize,
        compressed_shards: impl FallibleIterator<Item = CompressedShard, Error = Error>,
    ) -> Result<Vec<u8>> {
        self.decompress_impl(n_shards, uncompressed_size, compressed_shards)
            .location(loc!())?;

        let len = self.buffer.len();
        let buf = mem::replace(&mut self.buffer, DivBufShared::from(vec![0; len]));
        // We dropped mut_buf and all the buffers in the decompressor threads
        // have been freed already, so this should never fail.
        let mut vec: Vec<u8> = buf.try_into().unwrap();
        vec.truncate(uncompressed_size);
        Ok(vec)
    }
}
