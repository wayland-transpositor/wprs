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
use std::ops::IndexMut;
use std::sync::Arc;

use itertools::izip;
use lagoon::ThreadPool;

use crate::buffer_pointer::BufferPointer;
use crate::buffer_pointer::KnownSizeBufferPointer;
use crate::prelude::*;
use crate::sharding_compression::CompressedShards;
use crate::sharding_compression::ShardingCompressor;
use crate::simd::__m128i;
use crate::simd::__m256i;
use crate::simd::_mm_add_epi8;
use crate::simd::_mm_extract_epi8;
use crate::simd::_mm_loadu_si128;
use crate::simd::_mm_set1_epi8;
use crate::simd::_mm_setzero_si128;
use crate::simd::_mm256_add_epi8;
use crate::simd::_mm256_blend_epi32;
use crate::simd::_mm256_castps_si256;
use crate::simd::_mm256_castsi128_si256;
use crate::simd::_mm256_castsi256_ps;
use crate::simd::_mm256_castsi256_si128;
use crate::simd::_mm256_extract_epi8;
use crate::simd::_mm256_extracti128_si256;
use crate::simd::_mm256_inserti128_si256;
use crate::simd::_mm256_loadu_si256;
use crate::simd::_mm256_set_epi8;
use crate::simd::_mm256_set_m128i;
use crate::simd::_mm256_shuffle_epi8;
use crate::simd::_mm256_shuffle_ps;
use crate::simd::_mm256_slli_si256;
use crate::simd::_mm256_storeu_si256;
use crate::simd::_mm256_sub_epi8;
use crate::vec4u8::Vec4u8;
use crate::vec4u8::Vec4u8s;

#[inline]
fn _mm256_shufps_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        _mm256_castps_si256(_mm256_shuffle_ps::<MASK>(
            _mm256_castsi256_ps(a),
            _mm256_castsi256_ps(b),
        ))
    }
}

#[inline]
fn load_m128i_vec4u8(src: &KnownSizeBufferPointer<Vec4u8, 4>) -> __m128i {
    // SAFETY: src is 4 Vec4u8s, which is 16 u8s, which is 128 bits, so it is
    // safe to read 128 bits from it.
    unsafe { _mm_loadu_si128(src.ptr().cast::<__m128i>()) }
}

#[inline]
fn load_m256i(src: &[u8; 32]) -> __m256i {
    // SAFETY: src is which is 32 u8s, which is 256 bits, so it is safe to read
    // 256 bits from it.
    unsafe { _mm256_loadu_si256(src.as_ptr().cast::<__m256i>()) }
}

#[inline]
fn store_m256i(dst: &mut [u8; 32], val: __m256i) {
    // SAFETY: dst is 32 u8s, which is 256 bits, so it is safe to write 256 bits
    // to it.
    unsafe { _mm256_storeu_si256(dst.as_mut_ptr().cast::<__m256i>(), val) }
}

#[inline]
fn store_m256i_vec4u8(dst: &mut [Vec4u8; 8], val: __m256i) {
    // SAFETY: dst is 8 Vec4u8s, which is 32 u8s, which is 256 bits, so it is
    // safe to write 256 bits to it.
    unsafe { _mm256_storeu_si256(dst.as_mut_ptr().cast::<__m256i>(), val) }
}

#[inline]
fn subtract_green(b: __m256i, g: __m256i, r: __m256i) -> (__m256i, __m256i) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe { (_mm256_sub_epi8(b, g), _mm256_sub_epi8(r, g)) }
}

#[inline]
fn add_green(b: __m256i, g: __m256i, r: __m256i) -> (__m256i, __m256i) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe { (_mm256_add_epi8(b, g), _mm256_add_epi8(r, g)) }
}

#[inline]
fn prefix_sum_32(mut block: __m256i) -> __m256i {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        block = _mm256_add_epi8(block, _mm256_slli_si256::<1>(block));
        block = _mm256_add_epi8(block, _mm256_slli_si256::<2>(block));
        block = _mm256_add_epi8(block, _mm256_slli_si256::<4>(block));
        block = _mm256_add_epi8(block, _mm256_slli_si256::<8>(block));
    }
    block
}

#[inline]
fn accumulate_sum_16(mut block: __m128i, prev_block: __m128i) -> (__m128i, __m128i) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        let cur_sum = _mm_set1_epi8(_mm_extract_epi8::<15>(block) as i8);
        block = _mm_add_epi8(prev_block, block);
        (block, _mm_add_epi8(prev_block, cur_sum))
    }
}

#[inline]
fn accumulate_sum_32(block: __m256i, prev_block: __m128i) -> (__m256i, __m128i) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        let (block0, prev_block) =
            accumulate_sum_16(_mm256_extracti128_si256::<0>(block), prev_block);
        let (block1, prev_block) =
            accumulate_sum_16(_mm256_extracti128_si256::<1>(block), prev_block);
        (_mm256_set_m128i(block1, block0), prev_block)
    }
}

#[inline]
fn prefix_sum(block: __m256i, prev_block: __m128i) -> (__m256i, __m128i) {
    accumulate_sum_32(prefix_sum_32(block), prev_block)
}

#[inline]
fn running_difference_32(mut block: __m256i, prev: u8) -> (__m256i, u8) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        let prev = _mm256_set_epi8(
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, prev as i8,
        );
        let block15_16 = _mm256_set_epi8(
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            _mm256_extract_epi8::<15>(block) as i8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        );
        let next = _mm256_extract_epi8::<31>(block) as u8;

        block = _mm256_sub_epi8(block, _mm256_slli_si256::<1>(block));
        block = _mm256_sub_epi8(block, block15_16);
        block = _mm256_sub_epi8(block, prev);

        (block, next)
    }
}

#[inline]
fn aos_to_soa_u8_32x4(
    input: KnownSizeBufferPointer<Vec4u8, 32>,
    out0: &mut [u8; 32],
    out1: &mut [u8; 32],
    out2: &mut [u8; 32],
    out3: &mut [u8; 32],
    prev0: u8,
    prev1: u8,
    prev2: u8,
    prev3: u8,
) -> (u8, u8, u8, u8) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        let p0 = _mm256_set_epi8(
            15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13,
            9, 5, 1, 12, 8, 4, 0,
        );
        let p1 = _mm256_set_epi8(
            14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8,
            4, 0, 15, 11, 7, 3,
        );
        let p2 = _mm256_set_epi8(
            13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11,
            7, 3, 14, 10, 6, 2,
        );
        let p3 = _mm256_set_epi8(
            12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14,
            10, 6, 2, 13, 9, 5, 1,
        );

        let [i0, i1, i2, i3, i4, i5, i6, i7] = input.as_chunks::<4, 8>();

        // let input: *const u8 = input.ptr().cast();
        // print!("i0  ");
        // crate::simd::print_vec_char_256_hex(load_m256i(&*input.offset(0).cast::<[u8; 32]>()));
        // print!("i1  ");
        // crate::simd::print_vec_char_256_hex(load_m256i(&*input.offset(32).cast::<[u8; 32]>()));
        // print!("i2  ");
        // crate::simd::print_vec_char_256_hex(load_m256i(&*input.offset(64).cast::<[u8; 32]>()));
        // print!("i3  ");
        // crate::simd::print_vec_char_256_hex(load_m256i(&*input.offset(96).cast::<[u8; 32]>()));
        // print!("\n");

        // i0  1f 1e 1d 1c | 1b 1a 19 18 | 17 16 15 14 | 13 12 11 10 || 0f 0e 0d 0c | 0b 0a 09 08 | 07 06 05 04 | 03 02 01 00
        // i1  3f 3e 3d 3c | 3b 3a 39 38 | 37 36 35 34 | 33 32 31 30 || 2f 2e 2d 2c | 2b 2a 29 28 | 27 26 25 24 | 23 22 21 20
        // i2  5f 5e 5d 5c | 5b 5a 59 58 | 57 56 55 54 | 53 52 51 50 || 4f 4e 4d 4c | 4b 4a 49 48 | 47 46 45 44 | 43 42 41 40
        // i3  7f 7e 7d 7c | 7b 7a 79 78 | 77 76 75 74 | 73 72 71 70 || 6f 6e 6d 6c | 6b 6a 69 68 | 67 66 65 64 | 63 62 61 60

        let mut t0 = _mm256_castsi128_si256(load_m128i_vec4u8(&i0));
        let mut t1 = _mm256_castsi128_si256(load_m128i_vec4u8(&i1));
        let mut t2 = _mm256_castsi128_si256(load_m128i_vec4u8(&i2));
        let mut t3 = _mm256_castsi128_si256(load_m128i_vec4u8(&i3));

        t0 = _mm256_inserti128_si256::<1>(t0, load_m128i_vec4u8(&i4));
        t1 = _mm256_inserti128_si256::<1>(t1, load_m128i_vec4u8(&i5));
        t2 = _mm256_inserti128_si256::<1>(t2, load_m128i_vec4u8(&i6));
        t3 = _mm256_inserti128_si256::<1>(t3, load_m128i_vec4u8(&i7));

        // print!("t0  ");
        // crate::simd::print_vec_char_256_hex(t0);
        // print!("t1  ");
        // crate::simd::print_vec_char_256_hex(t1);
        // print!("t2  ");
        // crate::simd::print_vec_char_256_hex(t2);
        // print!("t3  ");
        // crate::simd::print_vec_char_256_hex(t3);
        // print!("\n");

        // t0  4f 4e 4d 4c | 4b 4a 49 48 | 47 46 45 44 | 43 42 41 40 || 0f 0e 0d 0c | 0b 0a 09 08 | 07 06 05 04 | 03 02 01 00
        // t1  5f 5e 5d 5c | 5b 5a 59 58 | 57 56 55 54 | 53 52 51 50 || 1f 1e 1d 1c | 1b 1a 19 18 | 17 16 15 14 | 13 12 11 10
        // t2  6f 6e 6d 6c | 6b 6a 69 68 | 67 66 65 64 | 63 62 61 60 || 2f 2e 2d 2c | 2b 2a 29 28 | 27 26 25 24 | 23 22 21 20
        // t3  7f 7e 7d 7c | 7b 7a 79 78 | 77 76 75 74 | 73 72 71 70 || 3f 3e 3d 3c | 3b 3a 39 38 | 37 36 35 34 | 33 32 31 30

        t0 = _mm256_shuffle_epi8(t0, p0);
        t1 = _mm256_shuffle_epi8(t1, p1);
        t2 = _mm256_shuffle_epi8(t2, p2);
        t3 = _mm256_shuffle_epi8(t3, p3);

        // print!("t0  ");
        // crate::simd::print_vec_char_256_hex(t0);
        // print!("t1  ");
        // crate::simd::print_vec_char_256_hex(t1);
        // print!("t2  ");
        // crate::simd::print_vec_char_256_hex(t2);
        // print!("t3  ");
        // crate::simd::print_vec_char_256_hex(t3);
        // print!("\n");

        // t0  4f 4b 47 43 | 4e 4a 46 42 | 4d 49 45 41 | 4c 48 44 40 || 0f 0b 07 03 | 0e 0a 06 02 | 0d 09 05 01 | 0c 08 04 00
        // t1  5e 5a 56 52 | 5d 59 55 51 | 5c 58 54 50 | 5f 5b 57 53 || 1e 1a 16 12 | 1d 19 15 11 | 1c 18 14 10 | 1f 1b 17 13
        // t2  6d 69 65 61 | 6c 68 64 60 | 6f 6b 67 63 | 6e 6a 66 62 || 2d 29 25 21 | 2c 28 24 20 | 2f 2b 27 23 | 2e 2a 26 22
        // t3  7c 78 74 70 | 7f 7b 77 73 | 7e 7a 76 72 | 7d 79 75 71 || 3c 38 34 30 | 3f 3b 37 33 | 3e 3a 36 32 | 3d 39 35 31

        let u0 = _mm256_blend_epi32::<0b10101010>(t0, t1);
        let u1 = _mm256_blend_epi32::<0b10101010>(t2, t3);
        let u2 = _mm256_blend_epi32::<0b01010101>(t0, t1);
        let u3 = _mm256_blend_epi32::<0b01010101>(t2, t3);

        // print!("u0  ");
        // crate::simd::print_vec_char_256_hex(u0);
        // print!("u1  ");
        // crate::simd::print_vec_char_256_hex(u1);
        // print!("u2  ");
        // crate::simd::print_vec_char_256_hex(u2);
        // print!("u3  ");
        // crate::simd::print_vec_char_256_hex(u3);
        // print!("\n");

        // u0  5e 5a 56 52 | 4e 4a 46 42 | 5c 58 54 50 | 4c 48 44 40 || 1e 1a 16 12 | 0e 0a 06 02 | 1c 18 14 10 | 0c 08 04 00
        // u1  7c 78 74 70 | 6c 68 64 60 | 7e 7a 76 72 | 6e 6a 66 62 || 3c 38 34 30 | 2c 28 24 20 | 3e 3a 36 32 | 2e 2a 26 22
        // u2  4f 4b 47 43 | 5d 59 55 51 | 4d 49 45 41 | 5f 5b 57 53 || 0f 0b 07 03 | 1d 19 15 11 | 0d 09 05 01 | 1f 1b 17 13
        // u3  6d 69 65 61 | 7f 7b 77 73 | 6f 6b 67 63 | 7d 79 75 71 || 2d 29 25 21 | 3f 3b 37 33 | 2f 2b 27 23 | 3d 39 35 31

        t0 = _mm256_blend_epi32::<0b11001100>(u0, u1);
        t1 = _mm256_shufps_epi32::<0b00111001>(u2, u3);
        t2 = _mm256_shufps_epi32::<0b01001110>(u0, u1);
        t3 = _mm256_shufps_epi32::<0b10010011>(u2, u3);

        // print!("t0  ");
        // crate::simd::print_vec_char_256_hex(t0);
        // print!("t1  ");
        // crate::simd::print_vec_char_256_hex(t1);
        // print!("t2  ");
        // crate::simd::print_vec_char_256_hex(t2);
        // print!("t3  ");
        // crate::simd::print_vec_char_256_hex(t3);
        // print!("\n");

        // t0  7c 78 74 70 | 6c 68 64 60 | 5c 58 54 50 | 4c 48 44 40 || 3c 38 34 30 | 2c 28 24 20 | 1c 18 14 10 | 0c 08 04 00
        // t1  7d 79 75 71 | 6d 69 65 61 | 5d 59 55 51 | 4d 49 45 41 || 3d 39 35 31 | 2d 29 25 21 | 1d 19 15 11 | 0d 09 05 01
        // t2  7e 7a 76 72 | 6e 6a 66 62 | 5e 5a 56 52 | 4e 4a 46 42 || 3e 3a 36 32 | 2e 2a 26 22 | 1e 1a 16 12 | 0e 0a 06 02
        // t3  7f 7b 77 73 | 6f 6b 67 63 | 5f 5b 57 53 | 4f 4b 47 43 || 3f 3b 37 33 | 2f 2b 27 23 | 1f 1b 17 13 | 0f 0b 07 03

        (t0, t2) = subtract_green(t0, t1, t2);

        #[allow(unused_assignments)]
        let (mut next0, mut next1, mut next2, mut next3) = (0, 0, 0, 0);
        (t0, next0) = running_difference_32(t0, prev0);
        (t1, next1) = running_difference_32(t1, prev1);
        (t2, next2) = running_difference_32(t2, prev2);
        (t3, next3) = running_difference_32(t3, prev3);

        store_m256i(out0, t0);
        store_m256i(out1, t1);
        store_m256i(out2, t2);
        store_m256i(out3, t3);

        (next0, next1, next2, next3)
    }
}

#[inline]
fn soa_to_aos_u8_32x4(
    input0: &[u8; 32],
    input1: &[u8; 32],
    input2: &[u8; 32],
    input3: &[u8; 32],
    out: &mut [Vec4u8; 32],
    mut prev0: __m128i,
    mut prev1: __m128i,
    mut prev2: __m128i,
    mut prev3: __m128i,
) -> (__m128i, __m128i, __m128i, __m128i) {
    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        let p0 = _mm256_set_epi8(
            7, 11, 15, 3, 6, 10, 14, 2, 5, 9, 13, 1, 4, 8, 12, 0, 7, 11, 15, 3, 6, 10, 14, 2, 5, 9,
            13, 1, 4, 8, 12, 0,
        );
        let p1 = _mm256_set_epi8(
            3, 15, 11, 7, 2, 14, 10, 6, 1, 13, 9, 5, 0, 12, 8, 4, 3, 15, 11, 7, 2, 14, 10, 6, 1,
            13, 9, 5, 0, 12, 8, 4,
        );
        let p2 = _mm256_set_epi8(
            15, 3, 7, 11, 14, 2, 6, 10, 13, 1, 5, 9, 12, 0, 4, 8, 15, 3, 7, 11, 14, 2, 6, 10, 13,
            1, 5, 9, 12, 0, 4, 8,
        );
        let p3 = _mm256_set_epi8(
            11, 7, 3, 15, 10, 6, 2, 14, 9, 5, 1, 13, 8, 4, 0, 12, 11, 7, 3, 15, 10, 6, 2, 14, 9, 5,
            1, 13, 8, 4, 0, 12,
        );

        let mut t0 = load_m256i(input0);
        let mut t1 = load_m256i(input1);
        let mut t2 = load_m256i(input2);
        let mut t3 = load_m256i(input3);

        (t0, prev0) = prefix_sum(t0, prev0);
        (t1, prev1) = prefix_sum(t1, prev1);
        (t2, prev2) = prefix_sum(t2, prev2);
        (t3, prev3) = prefix_sum(t3, prev3);

        (t0, t2) = add_green(t0, t1, t2);

        // print!("t0  ");
        // crate::simd::print_vec_char_256_hex(t0);
        // print!("t1  ");
        // crate::simd::print_vec_char_256_hex(t1);
        // print!("t2  ");
        // crate::simd::print_vec_char_256_hex(t2);
        // print!("t3  ");
        // crate::simd::print_vec_char_256_hex(t3);
        // print!("\n");

        // t0  7c 78 74 70 | 6c 68 64 60 | 5c 58 54 50 | 4c 48 44 40 || 3c 38 34 30 | 2c 28 24 20 | 1c 18 14 10 | 0c 08 04 00
        // t1  7d 79 75 71 | 6d 69 65 61 | 5d 59 55 51 | 4d 49 45 41 || 3d 39 35 31 | 2d 29 25 21 | 1d 19 15 11 | 0d 09 05 01
        // t2  7e 7a 76 72 | 6e 6a 66 62 | 5e 5a 56 52 | 4e 4a 46 42 || 3e 3a 36 32 | 2e 2a 26 22 | 1e 1a 16 12 | 0e 0a 06 02
        // t3  7f 7b 77 73 | 6f 6b 67 63 | 5f 5b 57 53 | 4f 4b 47 43 || 3f 3b 37 33 | 2f 2b 27 23 | 1f 1b 17 13 | 0f 0b 07 03

        let u0 = _mm256_shufps_epi32::<0b01000100>(t0, t2);
        let u1 = _mm256_shufps_epi32::<0b11101110>(t2, t0);
        let u2 = _mm256_shufps_epi32::<0b00010001>(t3, t1);
        let u3 = _mm256_shufps_epi32::<0b10111011>(t1, t3);

        // print!("u0  ");
        // crate::simd::print_vec_char_256_hex(u0);
        // print!("u1  ");
        // crate::simd::print_vec_char_256_hex(u1);
        // print!("u2  ");
        // crate::simd::print_vec_char_256_hex(u2);
        // print!("u3  ");
        // crate::simd::print_vec_char_256_hex(u3);
        // print!("\n");

        // u0  5e 5a 56 52 | 4e 4a 46 42 | 5c 58 54 50 | 4c 48 44 40 || 1e 1a 16 12 | 0e 0a 06 02 | 1c 18 14 10 | 0c 08 04 00
        // u1  7c 78 74 70 | 6c 68 64 60 | 7e 7a 76 72 | 6e 6a 66 62 || 3c 38 34 30 | 2c 28 24 20 | 3e 3a 36 32 | 2e 2a 26 22
        // u2  4d 49 45 41 | 5d 59 55 51 | 4f 4b 47 43 | 5f 5b 57 53 || 0d 09 05 01 | 1d 19 15 11 | 0f 0b 07 03 | 1f 1b 17 13
        // u3  6f 6b 67 63 | 7f 7b 77 73 | 6d 69 65 61 | 7d 79 75 71 || 2f 2b 27 23 | 3f 3b 37 33 | 2d 29 25 21 | 3d 39 35 31

        t0 = _mm256_blend_epi32::<0b01010101>(u2, u0);
        t1 = _mm256_blend_epi32::<0b10101010>(u2, u0);
        t2 = _mm256_blend_epi32::<0b01010101>(u3, u1);
        t3 = _mm256_blend_epi32::<0b10101010>(u3, u1);

        // print!("t0  ");
        // crate::simd::print_vec_char_256_hex(t0);
        // print!("t1  ");
        // crate::simd::print_vec_char_256_hex(t1);
        // print!("t2  ");
        // crate::simd::print_vec_char_256_hex(t2);
        // print!("t3  ");
        // crate::simd::print_vec_char_256_hex(t3);
        // print!("\n");

        // t0  4d 49 45 41 | 4e 4a 46 42 | 4f 4b 47 43 | 4c 48 44 40 || 0d 09 05 01 | 0e 0a 06 02 | 0f 0b 07 03 | 0c 08 04 00
        // t1  5e 5a 56 52 | 5d 59 55 51 | 5c 58 54 50 | 5f 5b 57 53 || 1e 1a 16 12 | 1d 19 15 11 | 1c 18 14 10 | 1f 1b 17 13
        // t2  6f 6b 67 63 | 6c 68 64 60 | 6d 69 65 61 | 6e 6a 66 62 || 2f 2b 27 23 | 2c 28 24 20 | 2d 29 25 21 | 2e 2a 26 22
        // t3  7c 78 74 70 | 7f 7b 77 73 | 7e 7a 76 72 | 7d 79 75 71 || 3c 38 34 30 | 3f 3b 37 33 | 3e 3a 36 32 | 3d 39 35 31

        t0 = _mm256_shuffle_epi8(t0, p0);
        t1 = _mm256_shuffle_epi8(t1, p1);
        t2 = _mm256_shuffle_epi8(t2, p2);
        t3 = _mm256_shuffle_epi8(t3, p3);

        // print!("t0  ");
        // crate::simd::print_vec_char_256_hex(t0);
        // print!("t1  ");
        // crate::simd::print_vec_char_256_hex(t1);
        // print!("t2  ");
        // crate::simd::print_vec_char_256_hex(t2);
        // print!("t3  ");
        // crate::simd::print_vec_char_256_hex(t3);
        // print!("\n");

        // t0  4f 4e 4d 4c | 4b 4a 49 48 | 47 46 45 44 | 43 42 41 40 || 0f 0e 0d 0c | 0b 0a 09 08 | 07 06 05 04 | 03 02 01 00
        // t1  5f 5e 5d 5c | 5b 5a 59 58 | 57 56 55 54 | 53 52 51 50 || 1f 1e 1d 1c | 1b 1a 19 18 | 17 16 15 14 | 13 12 11 10
        // t2  6f 6e 6d 6c | 6b 6a 69 68 | 67 66 65 64 | 63 62 61 60 || 2f 2e 2d 2c | 2b 2a 29 28 | 27 26 25 24 | 23 22 21 20
        // t3  7f 7e 7d 7c | 7b 7a 79 78 | 77 76 75 74 | 73 72 71 70 || 3f 3e 3d 3c | 3b 3a 39 38 | 37 36 35 34 | 33 32 31 30

        store_m256i_vec4u8(
            out.index_mut(0..8).try_into().unwrap(),
            _mm256_set_m128i(_mm256_castsi256_si128(t1), _mm256_castsi256_si128(t0)),
        );
        store_m256i_vec4u8(
            out.index_mut(8..16).try_into().unwrap(),
            _mm256_set_m128i(_mm256_castsi256_si128(t3), _mm256_castsi256_si128(t2)),
        );

        store_m256i_vec4u8(
            out.index_mut(16..24).try_into().unwrap(),
            _mm256_set_m128i(
                _mm256_extracti128_si256::<1>(t1),
                _mm256_extracti128_si256::<1>(t0),
            ),
        );
        store_m256i_vec4u8(
            out.index_mut(24..32).try_into().unwrap(),
            _mm256_set_m128i(
                _mm256_extracti128_si256::<1>(t3),
                _mm256_extracti128_si256::<1>(t2),
            ),
        );

        (prev0, prev1, prev2, prev3)
    }
}

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

    // SAFETY: simd/mod.rs exposes versions of this function for SSE2, SSSE3,
    // SSE4.1, AVX, and AVX2. Which version is used depends on compiler flags,
    // but there is no good way to plumb that through to avoid the unsafe.
    unsafe {
        let z = _mm_setzero_si128();
        let (mut prev0, mut prev1, mut prev2, mut prev3) = (z, z, z, z);

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
                            (prev0, prev1, prev2, prev3) = soa_to_aos_u8_32x4(
                                soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, aos_chunk, prev0,
                                prev1, prev2, prev3,
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
}

fn vec4u8_aos_to_soa(
    aos: BufferPointer<Vec4u8>,
    compressor: &mut ShardingCompressor,
) -> CompressedShards {
    vec4u8_aos_to_soa_avx2_parallel_compression(aos, compressor)
}

fn vec4u8_soa_to_aos(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    vec4u8_soa_to_aos_avx2_parallel(soa, aos)
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

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_prefix_sum() {
        let input = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];
        let mut output = [0; 32];
        unsafe {
            store_m256i(
                (&mut output[..]).try_into().unwrap(),
                prefix_sum(
                    load_m256i((&input[..]).try_into().unwrap()),
                    _mm_setzero_si128(),
                )
                .0,
            );
        }
        let expected = [
            0, 1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 66, 78, 91, 105, 120, 136, 153, 171, 190, 210,
            231, 253, 20, 44, 69, 95, 122, 150, 179, 209, 240,
        ];
        assert_eq!(output, expected);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_running_difference() {
        let input = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];
        let mut output = [0; 32];
        store_m256i(
            (&mut output[..]).try_into().unwrap(),
            running_difference_32(load_m256i((&input[..]).try_into().unwrap()), 0).0,
        );
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
