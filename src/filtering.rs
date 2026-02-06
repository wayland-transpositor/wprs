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

/// u8 AoS<>SoA conversion and filtering.
/// Ref:
/// * https://afrantzis.com/pixel-format-guide/wayland_drm.html
/// * https://stackoverflow.com/questions/44984724/whats-the-fastest-stride-3-gather-instruction-sequence.
/// * https://en.algorithmica.org/hpc/algorithms/prefix/.
use std::cmp;
use std::sync::Arc;

use itertools::izip;
use lagoon::ThreadPool;

use crate::buffer_pointer::BufferPointer;
use crate::prelude::*;
use crate::sharding_compression::CompressedShards;
use crate::sharding_compression::ShardingCompressor;
use crate::simd::SoaToAosPrevData;
use crate::simd::aos_to_soa_u8_32x4;
use crate::simd::soa_to_aos_u8_32x4;
use crate::vec4u8::Vec4u8;
use crate::vec4u8::Vec4u8s;

#[instrument(skip_all, level = "debug")]
fn vec4u8_aos_to_soa_avx2_parallel_compression(
    aos: BufferPointer<Vec4u8>,
    compressor: &mut ShardingCompressor,
) -> CompressedShards {
    if aos.is_empty() {
        return CompressedShards::default();
    }

    let len = aos.len();

    // aos_to_soa_u8_32x4 operates on blocks of 1024 bits aka 128 bytes aka 32
    // Vec4u8s.
    let n_blocks = len / 32; // number of 32x Vec4u8 blocks
    let lim = n_blocks * 32; // number of Vec4u8s to transpose in blocks
    let rem = len % 32; // remaining Vec4u8s to transpose individually
    let n_threads = 4;
    let blocks_per_thread = cmp::max(n_blocks / n_threads, 1);
    let thread_chunk_size = blocks_per_thread * 32;

    let compressor = Arc::new(compressor.begin());
    let compression_block_size = 128 * 1024;

    let (aos_to_lim, aos_remainder) = aos.split_at(lim);

    debug_span!("aos_to_soa_u8_32x4_loop").in_scope(|| {
        ThreadPool::global().scoped(|s| {
            for (thread_idx, aos) in aos_to_lim.chunks(thread_chunk_size).enumerate() {
                let compressor = compressor.clone();
                s.run(move || {
                    let mut idx = thread_idx * thread_chunk_size;
                    let (mut prev0, mut prev1, mut prev2, mut prev3) = (0, 0, 0, 0);
                    for aos in aos.chunks(compression_block_size) {
                        let soa_len = cmp::min(aos.len(), compression_block_size);
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
                            (prev0, prev1, prev2, prev3) = aos_to_soa_u8_32x4(
                                aos_chunk, soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, prev0,
                                prev1, prev2, prev3,
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

    // All the other clones of the Arc were inside the loops above.
    Arc::into_inner(compressor).unwrap().collect_shards()
}

#[instrument(skip_all, level = "debug")]
fn vec4u8_soa_to_aos_avx2_parallel(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    let len = soa.len();
    assert_eq!(len, aos.len());

    // soa_to_aos_u8_32x4 operates on blocks of 1024 bits aka 128 bytes aka 32
    // Vec4u8s.
    let n_blocks = len / 32; // number of 32x Vec4u8 blocks
    let lim = n_blocks * 32; // number of Vec4u8s to transpose in blocks
    let n_threads = 4;
    let blocks_per_thread = cmp::max(n_blocks / n_threads, 1);
    let thread_chunk_size = blocks_per_thread * 32;

    let mut prev = SoaToAosPrevData::default();

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
                        prev = soa_to_aos_u8_32x4(
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

fn vec4u8_aos_to_soa(
    aos: BufferPointer<Vec4u8>,
    compressor: &mut ShardingCompressor,
) -> CompressedShards {
    vec4u8_aos_to_soa_avx2_parallel_compression(aos, compressor)
}

fn vec4u8_soa_to_aos(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    vec4u8_soa_to_aos_avx2_parallel(soa, aos);
}

pub fn filter_and_compress(
    data: BufferPointer<u8>,
    compressor: &mut ShardingCompressor,
) -> CompressedShards {
    assert!(data.len().is_multiple_of(4)); // data is a buffer of argb or xrgb pixels.
    // SAFETY: Vec4u8 is a repr(C, packed) wrapper around [u8; 4].
    vec4u8_aos_to_soa(unsafe { data.cast::<Vec4u8>() }, compressor)
}

pub fn unfilter(data: &Vec4u8s, output_buf: &mut [u8]) {
    vec4u8_soa_to_aos(data, bytemuck::cast_slice_mut(output_buf));
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use fallible_iterator::IteratorExt;
    use proptest::prelude::*;

    use super::*;
    use crate::sharding_compression::CompressedShard;
    use crate::sharding_compression::ShardingDecompressor;
    use crate::simd::prefix_sum_test;
    use crate::simd::running_difference_test;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_prefix_sum() {
        let input: [u8; 32] = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let input_vec256 = crate::simd::into_vec256(input);
        let output = crate::simd::from_vec256(prefix_sum_test(input_vec256));

        let expected = [
            0, 1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 66, 78, 91, 105, 120, 136, 153, 171, 190, 210,
            231, 253, 20, 44, 69, 95, 122, 150, 179, 209, 240,
        ];
        assert_eq!(output, expected);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_running_difference() {
        let input: [u8; 32] = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let input_vec256 = crate::simd::into_vec256(input);
        let output = crate::simd::from_vec256(running_difference_test(input_vec256));

        let expected = [
            0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1,
        ];
        assert_eq!(output, expected);
    }

    fn test_vec(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 256) as u8).collect()
    }

    fn generate_aos(input: &[u8]) -> Vec<Vec4u8> {
        input
            .chunks(4)
            .map(|chunk| Vec4u8(chunk[0], chunk[1], chunk[2], chunk[3]))
            .collect()
    }

    fn test_roundtrip_impl(data: &[u8]) {
        assert!(data.len().is_multiple_of(4));

        let aos: Vec<Vec4u8> = generate_aos(data);
        let aos_ptr = aos.as_ptr();
        let aos_buf_ptr = unsafe { BufferPointer::new(&aos_ptr, aos.len()) };

        let mut compressor = ShardingCompressor::new(NonZeroUsize::new(16).unwrap(), 1).unwrap();
        let shards = vec4u8_aos_to_soa(aos_buf_ptr, &mut compressor);

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

        let mut expected_aos: Vec<Vec4u8> = vec![Vec4u8::new(); data.len() / 4];
        vec4u8_soa_to_aos(&soa.into(), &mut expected_aos);

        assert_eq!(aos, expected_aos);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_roundtrip() {
        for n in vec![
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
}
