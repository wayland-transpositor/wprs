// Copyright 2026 Google LLC
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

//! AoS↔SoA transposition and spatial/color filtering for pixel buffers.
//!
//! The pipeline transposes pixel data between array-of-structs (RGBA
//! interleaved) and struct-of-arrays (channels separated), applies
//! subtract_green (VP8L color decorrelation) and running_difference (delta
//! encoding) for improved entropy, then feeds the result to zstd.
//!
//! Architecture-specific backends:
//! - `x86`: permutation-based transpose (SSE2 through AVX2)
//! - `neon`: hardware structure load/store (vld4q/vst4q)
//!
//! Ref:
//! * <https://afrantzis.com/pixel-format-guide/wayland_drm.html>
//! * <https://en.algorithmica.org/hpc/algorithms/prefix/>
#[cfg(target_arch = "x86_64")]
#[path = "x86.rs"]
mod arch;

#[cfg(target_arch = "aarch64")]
#[path = "neon.rs"]
mod arch;

/// Bytes per compression shard fed to zstd. Balances parallelism granularity
/// against zstd context-window utilisation.
const COMPRESSION_BLOCK_SIZE: usize = 128 * 1024;

use std::cmp;
use std::sync::Arc;

use itertools::izip;
use lagoon::ThreadPool;

use crate::buffer_pointer::BufferPointer;
use crate::buffer_pointer::KnownSizeBufferPointer;
use crate::prelude::*;
use crate::sharding_compression::CompressedShards;
use crate::sharding_compression::ShardingCompressor;
use crate::vec4u8::Vec4u8;
use crate::vec4u8::Vec4u8s;

/// Architecture-specific 32-pixel block kernel.
pub(crate) trait FilterKernel {
    type ForwardCarry: Copy + Send;
    type InverseCarry: Copy + Send;

    fn zero_forward() -> (
        Self::ForwardCarry,
        Self::ForwardCarry,
        Self::ForwardCarry,
        Self::ForwardCarry,
    );

    fn zero_inverse() -> (
        Self::InverseCarry,
        Self::InverseCarry,
        Self::InverseCarry,
        Self::InverseCarry,
    );

    fn forward_block(
        input: KnownSizeBufferPointer<Vec4u8, 32>,
        out0: &mut [u8; 32],
        out1: &mut [u8; 32],
        out2: &mut [u8; 32],
        out3: &mut [u8; 32],
        prev: (
            Self::ForwardCarry,
            Self::ForwardCarry,
            Self::ForwardCarry,
            Self::ForwardCarry,
        ),
    ) -> (
        Self::ForwardCarry,
        Self::ForwardCarry,
        Self::ForwardCarry,
        Self::ForwardCarry,
    );

    fn inverse_block(
        in0: &[u8; 32],
        in1: &[u8; 32],
        in2: &[u8; 32],
        in3: &[u8; 32],
        out: &mut [Vec4u8; 32],
        prev: (
            Self::InverseCarry,
            Self::InverseCarry,
            Self::InverseCarry,
            Self::InverseCarry,
        ),
    ) -> (
        Self::InverseCarry,
        Self::InverseCarry,
        Self::InverseCarry,
        Self::InverseCarry,
    );
}

// --- shared parallel pipeline ---

#[instrument(skip_all, level = "debug")]
fn aos_to_soa_parallel_compression<K: FilterKernel>(
    aos: BufferPointer<Vec4u8>,
    compressor: &mut ShardingCompressor,
) -> CompressedShards {
    if aos.is_empty() {
        return CompressedShards::default();
    }

    let len = aos.len();
    let n_blocks = len / 32;
    let lim = n_blocks * 32;
    let rem = len % 32;
    let n_threads = 4;
    let blocks_per_thread = cmp::max(n_blocks / n_threads, 1);
    let thread_chunk_size = blocks_per_thread * 32;

    let compressor = Arc::new(compressor.begin());
    let (aos_to_lim, aos_remainder) = aos.split_at(lim);

    debug_span!("aos_to_soa_u8_32x4_loop").in_scope(|| {
        ThreadPool::global().scoped(|s| {
            for (thread_idx, aos) in aos_to_lim.chunks(thread_chunk_size).enumerate() {
                let compressor = compressor.clone();
                s.run(move || {
                    let mut idx = thread_idx * thread_chunk_size;
                    let mut prev = K::zero_forward();
                    for aos in aos.chunks(COMPRESSION_BLOCK_SIZE) {
                        let soa_len = cmp::min(aos.len(), COMPRESSION_BLOCK_SIZE);
                        let mut soa0 = vec![0; soa_len];
                        let mut soa1 = vec![0; soa_len];
                        let mut soa2 = vec![0; soa_len];
                        let mut soa3 = vec![0; soa_len];

                        for (aos_chunk, soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk) in izip!(
                            aos.array_chunks::<32>(),
                            soa0.as_chunks_mut::<32>().0,
                            soa1.as_chunks_mut::<32>().0,
                            soa2.as_chunks_mut::<32>().0,
                            soa3.as_chunks_mut::<32>().0,
                        ) {
                            prev = K::forward_block(
                                aos_chunk, soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, prev,
                            );
                        }

                        compressor.compress_shard(idx, soa0);
                        compressor.compress_shard(idx + len, soa1);
                        compressor.compress_shard(idx + 2 * len, soa2);
                        compressor.compress_shard(idx + 3 * len, soa3);

                        idx += aos.len();
                    }
                });
            }
        });
    });

    if rem > 0 {
        let mut rem0 = vec![0u8; rem];
        let mut rem1 = vec![0u8; rem];
        let mut rem2 = vec![0u8; rem];
        let mut rem3 = vec![0u8; rem];

        for (s, r0, r1, r2, r3) in izip!(
            aos_remainder.into_iter(),
            &mut rem0,
            &mut rem1,
            &mut rem2,
            &mut rem3
        ) {
            *r0 = s.0;
            *r1 = s.1;
            *r2 = s.2;
            *r3 = s.3;
        }

        compressor.compress_shard(len - rem, rem0);
        compressor.compress_shard(2 * len - rem, rem1);
        compressor.compress_shard(3 * len - rem, rem2);
        compressor.compress_shard(4 * len - rem, rem3);
    }

    Arc::into_inner(compressor).unwrap().collect_shards()
}

#[instrument(skip_all, level = "debug")]
fn soa_to_aos_parallel<K: FilterKernel>(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    let len = soa.len();
    assert_eq!(len, aos.len());

    let n_blocks = len / 32;
    let lim = n_blocks * 32;
    let n_threads = 4;
    let blocks_per_thread = cmp::max(n_blocks / n_threads, 1);
    let thread_chunk_size = blocks_per_thread * 32;

    let mut prev = K::zero_inverse();

    debug_span!("soa_to_aos_u8_32x4_loop").in_scope(|| {
        ThreadPool::global().scoped(|s| {
            for ((soa0, soa1, soa2, soa3), aos) in izip!(
                soa.chunks(thread_chunk_size),
                aos.chunks_mut(thread_chunk_size)
            ) {
                s.run(move || {
                    for (soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, aos_chunk) in izip!(
                        soa0.as_chunks::<32>().0,
                        soa1.as_chunks::<32>().0,
                        soa2.as_chunks::<32>().0,
                        soa3.as_chunks::<32>().0,
                        aos.as_chunks_mut::<32>().0,
                    ) {
                        prev = K::inverse_block(
                            soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, aos_chunk, prev,
                        );
                    }
                });
            }
        });
    });

    let (soa0, soa1, soa2, soa3) = soa.parts();
    for (a, s0, s1, s2, s3) in izip!(
        &mut aos[lim..len],
        &soa0[lim..len],
        &soa1[lim..len],
        &soa2[lim..len],
        &soa3[lim..len]
    ) {
        *a = Vec4u8(*s0, *s1, *s2, *s3);
    }
}

// --- public API ---

pub fn filter_and_compress(
    data: BufferPointer<u8>,
    compressor: &mut ShardingCompressor,
) -> CompressedShards {
    assert!(data.len().is_multiple_of(4));
    // SAFETY: Vec4u8 is a repr(C, packed) wrapper around [u8; 4].
    aos_to_soa_parallel_compression::<arch::Kernel>(unsafe { data.cast::<Vec4u8>() }, compressor)
}

pub fn unfilter(data: &Vec4u8s, output_buf: &mut [u8]) {
    soa_to_aos_parallel::<arch::Kernel>(data, bytemuck::cast_slice_mut(output_buf));
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use fallible_iterator::IteratorExt;
    use proptest::prelude::*;

    use super::*;
    use crate::sharding_compression::CompressedShard;
    use crate::sharding_compression::ShardingDecompressor;

    fn test_vec(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 256) as u8).collect()
    }

    fn test_roundtrip_impl(data: &[u8]) {
        assert!(data.len().is_multiple_of(4));

        let data_ptr = data.as_ptr();
        let buf_ptr = unsafe { BufferPointer::new(&data_ptr, data.len()) };

        let mut compressor = ShardingCompressor::new(NonZeroUsize::new(16).unwrap(), 1).unwrap();
        let shards = filter_and_compress(buf_ptr, &mut compressor);

        let mut decompressor = ShardingDecompressor::new(NonZeroUsize::new(8).unwrap()).unwrap();
        let indices = shards.indices();

        let soa = decompressor
            .decompress_to_owned(
                &indices,
                data.len(),
                shards
                    .shards
                    .into_iter()
                    .map(Ok::<CompressedShard, anyhow::Error>)
                    .transpose_into_fallible(),
            )
            .unwrap();

        let mut output = vec![0u8; data.len()];
        unfilter(&soa.into(), &mut output);

        assert_eq!(data, &output);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_roundtrip() {
        for n in [
            0,
            4,
            8,
            12,
            16,
            20,
            24,
            28,
            32,
            36,
            120,
            124,
            128,
            132,
            248,
            252,
            256,
            260,
            1016,
            1020,
            1024,
            1028,
            2040,
            2044,
            2048,
            2052,
            100,
            1920 * 1080,
            32768 * 4 + 4,
            1008 * 9513 * 4,
            1008 * 951 * 4,
        ] {
            test_roundtrip_impl(&test_vec(n));
        }
    }

    proptest! {
        #[test]
        #[cfg_attr(miri, ignore)]
        fn proptest_roundtrip(mut arr in proptest::collection::vec(0..u8::MAX, 0..1_000_000)) {
            arr.truncate((arr.len() / 4) * 4);
            assert!(arr.len() % 4 == 0);
            test_roundtrip_impl(&arr);
        }
    }

    /// Verify that the filtered SoA output matches known-good values.
    ///
    /// This test is architecture-independent: both x86 and aarch64 must
    /// produce identical compressed shards for the same input, since a
    /// buffer filtered on one architecture may be unfiltered on another.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_filtered_output_deterministic() {
        // 128 bytes = 32 pixels = exactly one SIMD block on both architectures.
        let data: Vec<u8> = (0..128u16).map(|i| ((i * 7 + 13) % 256) as u8).collect();
        let data_ptr = data.as_ptr();
        let buf_ptr = unsafe { BufferPointer::new(&data_ptr, data.len()) };

        let mut compressor = ShardingCompressor::new(NonZeroUsize::new(16).unwrap(), 1).unwrap();
        let shards = filter_and_compress(buf_ptr, &mut compressor);

        let mut decompressor = ShardingDecompressor::new(NonZeroUsize::new(8).unwrap()).unwrap();
        let indices = shards.indices();
        let soa = decompressor
            .decompress_to_owned(
                &indices,
                data.len(),
                shards
                    .shards
                    .into_iter()
                    .map(Ok::<CompressedShard, anyhow::Error>)
                    .transpose_into_fallible(),
            )
            .unwrap();

        // SoA layout: 4 channel arrays concatenated, each of length n_pixels.
        let n = 32;
        assert!(soa.len() >= 4 * n);
        let s0 = &soa[0..n];
        let s1 = &soa[n..2 * n];
        let s2 = &soa[2 * n..3 * n];
        let s3 = &soa[3 * n..4 * n];

        // Compute expected values via scalar reference implementation.
        // Forward: transpose → subtract_green → running_difference.
        let mut expected = [vec![0u8; n], vec![0u8; n], vec![0u8; n], vec![0u8; n]];
        for i in 0..n {
            expected[0][i] = data[i * 4]; // channel 0
            expected[1][i] = data[i * 4 + 1]; // channel 1 (green)
            expected[2][i] = data[i * 4 + 2]; // channel 2
            expected[3][i] = data[i * 4 + 3]; // channel 3
        }
        // subtract_green: ch0 -= ch1, ch2 -= ch1.
        let [ch0, ch1, ch2, ..] = &mut expected;
        for (b, (g, r)) in ch0.iter_mut().zip(ch1.iter().zip(ch2.iter_mut())) {
            *b = b.wrapping_sub(*g);
            *r = r.wrapping_sub(*g);
        }
        // running_difference per channel.
        for ch in &mut expected {
            let mut prev = 0u8;
            for v in ch.iter_mut() {
                let orig = *v;
                *v = orig.wrapping_sub(prev);
                prev = orig;
            }
        }

        assert_eq!(s0, &expected[0], "channel 0 mismatch");
        assert_eq!(s1, &expected[1], "channel 1 mismatch");
        assert_eq!(s2, &expected[2], "channel 2 mismatch");
        assert_eq!(s3, &expected[3], "channel 3 mismatch");
    }
}
