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

/// u8 AoS<>SoA conversion, based on
/// https://stackoverflow.com/questions/44984724/whats-the-fastest-stride-3-gather-instruction-sequence.
use std::arch::x86_64::__m128i;
use std::arch::x86_64::__m256i;
use std::arch::x86_64::_mm_loadu_si128;
use std::arch::x86_64::_mm_storeu_si128;
use std::arch::x86_64::_mm256_blend_epi32;
use std::arch::x86_64::_mm256_castps_si256;
use std::arch::x86_64::_mm256_castsi128_si256;
use std::arch::x86_64::_mm256_castsi256_ps;
use std::arch::x86_64::_mm256_castsi256_si128;
use std::arch::x86_64::_mm256_extracti128_si256;
use std::arch::x86_64::_mm256_inserti128_si256;
use std::arch::x86_64::_mm256_loadu_si256;
use std::arch::x86_64::_mm256_set_epi8;
use std::arch::x86_64::_mm256_shuffle_epi8;
use std::arch::x86_64::_mm256_shuffle_ps;
use std::arch::x86_64::_mm256_storeu_si256;
use std::cmp;

use itertools::izip;
use lagoon::ThreadPool;

use crate::buffer_pointer::BufferPointer;
use crate::prelude::*;
use crate::vec4u8::Vec4u8;
use crate::vec4u8::Vec4u8s;

// SAFETY:
// * avx2 must be available.
#[allow(unsafe_op_in_unsafe_fn)]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn _mm256_shufps_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    _mm256_castps_si256(_mm256_shuffle_ps(
        _mm256_castsi256_ps(a),
        _mm256_castsi256_ps(b),
        MASK,
    ))
}

// SAFETY:
// * avx2 and sse2 must be available.
// * `input` must be valid for reads of 128 bytes.
// * Each `out` must be valid for writes of 32 bytes.
#[allow(unsafe_op_in_unsafe_fn)]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn aos_to_soa_u8_32x4(
    input: BufferPointer<Vec4u8>,
    out0: &mut [u8],
    out1: &mut [u8],
    out2: &mut [u8],
    out3: &mut [u8],
) {
    let p0: __m256i = _mm256_set_epi8(
        15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5,
        1, 12, 8, 4, 0,
    );
    let p1: __m256i = _mm256_set_epi8(
        14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4,
        0, 15, 11, 7, 3,
    );
    let p2: __m256i = _mm256_set_epi8(
        13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7,
        3, 14, 10, 6, 2,
    );
    let p3: __m256i = _mm256_set_epi8(
        12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6,
        2, 13, 9, 5, 1,
    );

    let input: *const u8 = input.ptr().cast();
    let out0: *mut u8 = out0.as_mut_ptr();
    let out1: *mut u8 = out1.as_mut_ptr();
    let out2: *mut u8 = out2.as_mut_ptr();
    let out3: *mut u8 = out3.as_mut_ptr();

    // print!("i0  ");
    // crate::utils::print_vec_char_256_hex(_mm256_loadu_si256(input.offset(0).cast::<__m256i>()));
    // print!("i1  ");
    // crate::utils::print_vec_char_256_hex(_mm256_loadu_si256(input.offset(32).cast::<__m256i>()));
    // print!("i2  ");
    // crate::utils::print_vec_char_256_hex(_mm256_loadu_si256(input.offset(64).cast::<__m256i>()));
    // print!("i3  ");
    // crate::utils::print_vec_char_256_hex(_mm256_loadu_si256(input.offset(96).cast::<__m256i>()));
    // print!("\n");

    // i0  1f 1e 1d 1c | 1b 1a 19 18 | 17 16 15 14 | 13 12 11 10 || 0f 0e 0d 0c | 0b 0a 09 08 | 07 06 05 04 | 03 02 01 00
    // i1  3f 3e 3d 3c | 3b 3a 39 38 | 37 36 35 34 | 33 32 31 30 || 2f 2e 2d 2c | 2b 2a 29 28 | 27 26 25 24 | 23 22 21 20
    // i2  5f 5e 5d 5c | 5b 5a 59 58 | 57 56 55 54 | 53 52 51 50 || 4f 4e 4d 4c | 4b 4a 49 48 | 47 46 45 44 | 43 42 41 40
    // i3  7f 7e 7d 7c | 7b 7a 79 78 | 77 76 75 74 | 73 72 71 70 || 6f 6e 6d 6c | 6b 6a 69 68 | 67 66 65 64 | 63 62 61 60

    let mut t0: __m256i =
        _mm256_castsi128_si256(_mm_loadu_si128(input.offset(0).cast::<__m128i>()));
    let mut t1: __m256i =
        _mm256_castsi128_si256(_mm_loadu_si128(input.offset(16).cast::<__m128i>()));
    let mut t2: __m256i =
        _mm256_castsi128_si256(_mm_loadu_si128(input.offset(32).cast::<__m128i>()));
    let mut t3: __m256i =
        _mm256_castsi128_si256(_mm_loadu_si128(input.offset(48).cast::<__m128i>()));

    t0 = _mm256_inserti128_si256(t0, _mm_loadu_si128(input.offset(64).cast::<__m128i>()), 1);
    t1 = _mm256_inserti128_si256(t1, _mm_loadu_si128(input.offset(80).cast::<__m128i>()), 1);
    t2 = _mm256_inserti128_si256(t2, _mm_loadu_si128(input.offset(96).cast::<__m128i>()), 1);
    t3 = _mm256_inserti128_si256(t3, _mm_loadu_si128(input.offset(112).cast::<__m128i>()), 1);

    // print!("t0  ");
    // crate::utils::print_vec_char_256_hex(t0);
    // print!("t1  ");
    // crate::utils::print_vec_char_256_hex(t1);
    // print!("t2  ");
    // crate::utils::print_vec_char_256_hex(t2);
    // print!("t3  ");
    // crate::utils::print_vec_char_256_hex(t3);
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
    // crate::utils::print_vec_char_256_hex(t0);
    // print!("t1  ");
    // crate::utils::print_vec_char_256_hex(t1);
    // print!("t2  ");
    // crate::utils::print_vec_char_256_hex(t2);
    // print!("t3  ");
    // crate::utils::print_vec_char_256_hex(t3);
    // print!("\n");

    // t0  4f 4b 47 43 | 4e 4a 46 42 | 4d 49 45 41 | 4c 48 44 40 || 0f 0b 07 03 | 0e 0a 06 02 | 0d 09 05 01 | 0c 08 04 00
    // t1  5e 5a 56 52 | 5d 59 55 51 | 5c 58 54 50 | 5f 5b 57 53 || 1e 1a 16 12 | 1d 19 15 11 | 1c 18 14 10 | 1f 1b 17 13
    // t2  6d 69 65 61 | 6c 68 64 60 | 6f 6b 67 63 | 6e 6a 66 62 || 2d 29 25 21 | 2c 28 24 20 | 2f 2b 27 23 | 2e 2a 26 22
    // t3  7c 78 74 70 | 7f 7b 77 73 | 7e 7a 76 72 | 7d 79 75 71 || 3c 38 34 30 | 3f 3b 37 33 | 3e 3a 36 32 | 3d 39 35 31

    let u0: __m256i = _mm256_blend_epi32(t0, t1, 0b10101010);
    let u1: __m256i = _mm256_blend_epi32(t2, t3, 0b10101010);
    let u2: __m256i = _mm256_blend_epi32(t0, t1, 0b01010101);
    let u3: __m256i = _mm256_blend_epi32(t2, t3, 0b01010101);

    // print!("u0  ");
    // crate::utils::print_vec_char_256_hex(u0);
    // print!("u1  ");
    // crate::utils::print_vec_char_256_hex(u1);
    // print!("u2  ");
    // crate::utils::print_vec_char_256_hex(u2);
    // print!("u3  ");
    // crate::utils::print_vec_char_256_hex(u3);
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
    // crate::utils::print_vec_char_256_hex(t0);
    // print!("t1  ");
    // crate::utils::print_vec_char_256_hex(t1);
    // print!("t2  ");
    // crate::utils::print_vec_char_256_hex(t2);
    // print!("t3  ");
    // crate::utils::print_vec_char_256_hex(t3);
    // print!("\n");

    // t0  7c 78 74 70 | 6c 68 64 60 | 5c 58 54 50 | 4c 48 44 40 || 3c 38 34 30 | 2c 28 24 20 | 1c 18 14 10 | 0c 08 04 00
    // t1  7d 79 75 71 | 6d 69 65 61 | 5d 59 55 51 | 4d 49 45 41 || 3d 39 35 31 | 2d 29 25 21 | 1d 19 15 11 | 0d 09 05 01
    // t2  7e 7a 76 72 | 6e 6a 66 62 | 5e 5a 56 52 | 4e 4a 46 42 || 3e 3a 36 32 | 2e 2a 26 22 | 1e 1a 16 12 | 0e 0a 06 02
    // t3  7f 7b 77 73 | 6f 6b 67 63 | 5f 5b 57 53 | 4f 4b 47 43 || 3f 3b 37 33 | 2f 2b 27 23 | 1f 1b 17 13 | 0f 0b 07 03

    _mm256_storeu_si256(out0.cast::<__m256i>(), t0);
    _mm256_storeu_si256(out1.cast::<__m256i>(), t1);
    _mm256_storeu_si256(out2.cast::<__m256i>(), t2);
    _mm256_storeu_si256(out3.cast::<__m256i>(), t3);
}

// SAFETY:
// * avx2 and sse2 must be available.
// * Each `input` must be valid for reads of 32 bytes.
// * `out` must be valid for writes of 128 bytes.
#[allow(unsafe_op_in_unsafe_fn)]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn soa_to_aos_u8_32x4(
    input0: &[u8],
    input1: &[u8],
    input2: &[u8],
    input3: &[u8],
    out: &mut [Vec4u8],
) {
    let p0 = _mm256_set_epi8(
        7, 11, 15, 3, 6, 10, 14, 2, 5, 9, 13, 1, 4, 8, 12, 0, 7, 11, 15, 3, 6, 10, 14, 2, 5, 9, 13,
        1, 4, 8, 12, 0,
    );
    let p1 = _mm256_set_epi8(
        3, 15, 11, 7, 2, 14, 10, 6, 1, 13, 9, 5, 0, 12, 8, 4, 3, 15, 11, 7, 2, 14, 10, 6, 1, 13, 9,
        5, 0, 12, 8, 4,
    );
    let p2 = _mm256_set_epi8(
        15, 3, 7, 11, 14, 2, 6, 10, 13, 1, 5, 9, 12, 0, 4, 8, 15, 3, 7, 11, 14, 2, 6, 10, 13, 1, 5,
        9, 12, 0, 4, 8,
    );
    let p3 = _mm256_set_epi8(
        11, 7, 3, 15, 10, 6, 2, 14, 9, 5, 1, 13, 8, 4, 0, 12, 11, 7, 3, 15, 10, 6, 2, 14, 9, 5, 1,
        13, 8, 4, 0, 12,
    );

    let input0: *const u8 = input0.as_ptr();
    let input1: *const u8 = input1.as_ptr();
    let input2: *const u8 = input2.as_ptr();
    let input3: *const u8 = input3.as_ptr();
    let out: *mut u8 = out.as_mut_ptr().cast();

    let mut t0: __m256i = _mm256_loadu_si256(input0.cast::<__m256i>());
    let mut t1: __m256i = _mm256_loadu_si256(input1.cast::<__m256i>());
    let mut t2: __m256i = _mm256_loadu_si256(input2.cast::<__m256i>());
    let mut t3: __m256i = _mm256_loadu_si256(input3.cast::<__m256i>());

    // print!("t0  ");
    // crate::utils::print_vec_char_256_hex(t0);
    // print!("t1  ");
    // crate::utils::print_vec_char_256_hex(t1);
    // print!("t2  ");
    // crate::utils::print_vec_char_256_hex(t2);
    // print!("t3  ");
    // crate::utils::print_vec_char_256_hex(t3);
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
    // crate::utils::print_vec_char_256_hex(u0);
    // print!("u1  ");
    // crate::utils::print_vec_char_256_hex(u1);
    // print!("u2  ");
    // crate::utils::print_vec_char_256_hex(u2);
    // print!("u3  ");
    // crate::utils::print_vec_char_256_hex(u3);
    // print!("\n");

    // u0  5e 5a 56 52 | 4e 4a 46 42 | 5c 58 54 50 | 4c 48 44 40 || 1e 1a 16 12 | 0e 0a 06 02 | 1c 18 14 10 | 0c 08 04 00
    // u1  7c 78 74 70 | 6c 68 64 60 | 7e 7a 76 72 | 6e 6a 66 62 || 3c 38 34 30 | 2c 28 24 20 | 3e 3a 36 32 | 2e 2a 26 22
    // u2  4d 49 45 41 | 5d 59 55 51 | 4f 4b 47 43 | 5f 5b 57 53 || 0d 09 05 01 | 1d 19 15 11 | 0f 0b 07 03 | 1f 1b 17 13
    // u3  6f 6b 67 63 | 7f 7b 77 73 | 6d 69 65 61 | 7d 79 75 71 || 2f 2b 27 23 | 3f 3b 37 33 | 2d 29 25 21 | 3d 39 35 31

    t0 = _mm256_blend_epi32(u2, u0, 0b01010101);
    t1 = _mm256_blend_epi32(u2, u0, 0b10101010);
    t2 = _mm256_blend_epi32(u3, u1, 0b01010101);
    t3 = _mm256_blend_epi32(u3, u1, 0b10101010);

    // print!("t0  ");
    // crate::utils::print_vec_char_256_hex(t0);
    // print!("t1  ");
    // crate::utils::print_vec_char_256_hex(t1);
    // print!("t2  ");
    // crate::utils::print_vec_char_256_hex(t2);
    // print!("t3  ");
    // crate::utils::print_vec_char_256_hex(t3);
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
    // crate::utils::print_vec_char_256_hex(t0);
    // print!("t1  ");
    // crate::utils::print_vec_char_256_hex(t1);
    // print!("t2  ");
    // crate::utils::print_vec_char_256_hex(t2);
    // print!("t3  ");
    // crate::utils::print_vec_char_256_hex(t3);
    // print!("\n");

    // t0  4f 4e 4d 4c | 4b 4a 49 48 | 47 46 45 44 | 43 42 41 40 || 0f 0e 0d 0c | 0b 0a 09 08 | 07 06 05 04 | 03 02 01 00
    // t1  5f 5e 5d 5c | 5b 5a 59 58 | 57 56 55 54 | 53 52 51 50 || 1f 1e 1d 1c | 1b 1a 19 18 | 17 16 15 14 | 13 12 11 10
    // t2  6f 6e 6d 6c | 6b 6a 69 68 | 67 66 65 64 | 63 62 61 60 || 2f 2e 2d 2c | 2b 2a 29 28 | 27 26 25 24 | 23 22 21 20
    // t3  7f 7e 7d 7c | 7b 7a 79 78 | 77 76 75 74 | 73 72 71 70 || 3f 3e 3d 3c | 3b 3a 39 38 | 37 36 35 34 | 33 32 31 30

    _mm_storeu_si128(out.offset(0).cast::<__m128i>(), _mm256_castsi256_si128(t0));
    _mm_storeu_si128(out.offset(16).cast::<__m128i>(), _mm256_castsi256_si128(t1));
    _mm_storeu_si128(out.offset(32).cast::<__m128i>(), _mm256_castsi256_si128(t2));
    _mm_storeu_si128(out.offset(48).cast::<__m128i>(), _mm256_castsi256_si128(t3));

    _mm_storeu_si128(
        out.offset(64).cast::<__m128i>(),
        _mm256_extracti128_si256(t0, 1),
    );
    _mm_storeu_si128(
        out.offset(80).cast::<__m128i>(),
        _mm256_extracti128_si256(t1, 1),
    );
    _mm_storeu_si128(
        out.offset(96).cast::<__m128i>(),
        _mm256_extracti128_si256(t2, 1),
    );
    _mm_storeu_si128(
        out.offset(112).cast::<__m128i>(),
        _mm256_extracti128_si256(t3, 1),
    );
}

// SAFETY:
// * avx2 and sse2 must be available.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[target_feature(enable = "sse2")]
#[instrument(skip_all, level = "debug")]
pub unsafe fn vec4u8_aos_to_soa_avx2_parallel(aos: BufferPointer<Vec4u8>, soa: &mut Vec4u8s) {
    let len = aos.len();
    assert_eq!(len, soa.len());

    // aos_to_soa_u8_32x4 operates on blocks of 1024 bits aka 128 bytes aka 32
    // Vec4u8s.
    let n_blocks = len / 32; // number of 32x Vec4u8 blocks
    let lim = n_blocks * 32; // number of Vec4u8s to transpose in blocks
    let rem = len % 32; // remaining Vec4u8s to transpose individually
    let n_threads = 4;
    let blocks_per_thread = cmp::max(n_blocks / n_threads, 1);
    let thread_chunk_size = blocks_per_thread * 32;
    // debug!("lim {lim:?}, rem {rem:?}, thread_chunk_size {thread_chunk_size:?}");

    let mut rem0 = [0u8; 31];
    let mut rem1 = [0u8; 31];
    let mut rem2 = [0u8; 31];
    let mut rem3 = [0u8; 31];

    let (aos_to_lim, aos_remainder) = aos.split_at(lim);

    debug_span!("aos_to_soa_u8_32x4_loop").in_scope(|| {
        ThreadPool::global().scoped(|s| {
            for (aos, (soa0, soa1, soa2, soa3)) in izip!(
                aos_to_lim.chunks(thread_chunk_size),
                soa.chunks_mut(thread_chunk_size)
            ) {
                s.run(move || {
                    for (aos_chunk, soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk) in izip!(
                        aos.chunks_exact(32).0,
                        soa0.chunks_exact_mut(32),
                        soa1.chunks_exact_mut(32),
                        soa2.chunks_exact_mut(32),
                        soa3.chunks_exact_mut(32)
                    ) {
                        unsafe {
                            // SAFETY:
                            // * aos_chunk is 32 Vec4u8s, which is 32*4 = 128 bytes.
                            // * soa chunks are 32 bytes.
                            aos_to_soa_u8_32x4(
                                aos_chunk, soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk,
                            );
                        }
                    }
                });
            }
        });
    });

    // Using this style of loop for the whole thing (with this function
    // signature, etc.) lets the compiler to a pretty good job at
    // auto-vectorization, but aos_to_soa_u8_32x4 is still faster (e.g.,
    // 400-500us vs 700-800us). We got here by implementing aos_to_soa_u8_32x4
    // though, and it shouldn't be a large maintenance burden, so we might as
    // well keep it. We can use this version for non-AVX2 platforms and hope the
    // compiler does a reasonable job with whatever SIMD instructions are
    // available.
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

    let (soa0, soa1, soa2, soa3) = soa.parts_mut();
    soa0[lim..len].copy_from_slice(&rem0[0..rem]);
    soa1[lim..len].copy_from_slice(&rem1[0..rem]);
    soa2[lim..len].copy_from_slice(&rem2[0..rem]);
    soa3[lim..len].copy_from_slice(&rem3[0..rem]);
}

// TODO: multithread this
#[instrument(skip_all, level = "debug")]
pub fn vec4u8_aos_to_soa_scalar(aos: BufferPointer<Vec4u8>, soa: &mut Vec4u8s) {
    let (soa0, soa1, soa2, soa3) = soa.parts_mut();
    for (s, r0, r1, r2, r3) in izip!(aos.into_iter(), soa0, soa1, soa2, soa3) {
        *r0 = s.0;
        *r1 = s.1;
        *r2 = s.2;
        *r3 = s.3;
    }
}

#[instrument(skip_all, level = "debug")]
pub fn vec4u8_aos_to_soa(aos: BufferPointer<Vec4u8>, soa: &mut Vec4u8s) {
    soa.resize(aos.len());

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("sse2") {
            // SAFETY: checked for avx2 and sse2 support.
            return unsafe { vec4u8_aos_to_soa_avx2_parallel(aos, soa) };
        }
    }

    vec4u8_aos_to_soa_scalar(aos, soa)
}

// SAFETY:
// * avx2 and sse2 must be available.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[target_feature(enable = "sse2")]
#[instrument(skip_all, level = "debug")]
pub unsafe fn vec4u8_soa_to_aos_avx2_parallel(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    let len = soa.len();
    assert_eq!(len, aos.len());

    // soa_to_aos_u8_32x4 operates on blocks of 1024 bits aka 128 bytes aka 32
    // Vec4u8s.
    let n_blocks = len / 32; // number of 32x Vec4u8 blocks
    let lim = n_blocks * 32; // number of Vec4u8s to transpose in blocks
    let n_threads = 4;
    let blocks_per_thread = cmp::max(n_blocks / n_threads, 1);
    let thread_chunk_size = blocks_per_thread * 32;

    debug_span!("soa_to_aos_u8_32x4_loop").in_scope(|| {
        ThreadPool::global().scoped(|s| {
            for ((soa0, soa1, soa2, soa3), aos) in izip!(
                soa.chunks(thread_chunk_size),
                aos.chunks_mut(thread_chunk_size)
            ) {
                s.run(move || {
                    for (soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, aos_chunk) in izip!(
                        soa0.chunks_exact(32),
                        soa1.chunks_exact(32),
                        soa2.chunks_exact(32),
                        soa3.chunks_exact(32),
                        aos.chunks_exact_mut(32)
                    ) {
                        unsafe {
                            // SAFETY:
                            // * soa chunks are 32 bytes.
                            // * aos_chunk is 32 Vec4u8s, which is 32*4 = 128 bytes.
                            soa_to_aos_u8_32x4(
                                soa0_chunk, soa1_chunk, soa2_chunk, soa3_chunk, aos_chunk,
                            );
                        }
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

// TODO: multithread this
pub fn vec4u8_soa_to_aos_scalar(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    let (soa0, soa1, soa2, soa3) = soa.parts();
    for (a, s0, s1, s2, s3) in izip!(aos, soa0, soa1, soa2, soa3) {
        *a = Vec4u8(*s0, *s1, *s2, *s3);
    }
}

pub fn vec4u8_soa_to_aos(soa: &Vec4u8s, aos: &mut [Vec4u8]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("sse2") {
            // SAFETY: checked for avx2 and sse2 support.
            return unsafe { vec4u8_soa_to_aos_avx2_parallel(soa, aos) };
        }
    }

    vec4u8_soa_to_aos_scalar(soa, aos)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn test_vec(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 256) as u8).collect()
    }

    fn generate_aos(input: &[u8]) -> Vec<Vec4u8> {
        input
            .chunks(4)
            .map(|chunk| Vec4u8(chunk[0], chunk[1], chunk[2], chunk[3]))
            .collect()
    }

    fn generate_soa(input: &[u8]) -> Vec4u8s {
        let mut output = Vec4u8s::with_total_size(input.len());
        input
            .chunks(4)
            .zip(output.iter_mut())
            .for_each(|(chunk, parts)| {
                *parts.0 = chunk[0];
                *parts.1 = chunk[1];
                *parts.2 = chunk[2];
                *parts.3 = chunk[3];
            });
        output
    }

    fn test_vec4u8_aos_to_soa_impl(data: &[u8]) {
        assert!(data.len() % 4 == 0);

        let aos: Vec<Vec4u8> = generate_aos(data);
        let aos_ptr = aos.as_ptr();
        let aos_buf_ptr = unsafe { BufferPointer::new(&aos_ptr, aos.len()) };

        let mut soa_avx2 = Vec4u8s::with_total_size(data.len());
        unsafe { vec4u8_aos_to_soa_avx2_parallel(aos_buf_ptr, &mut soa_avx2) };

        let mut soa_scalar = Vec4u8s::with_total_size(data.len());
        vec4u8_aos_to_soa_scalar(aos_buf_ptr, &mut soa_scalar);

        let expected_soa = generate_soa(data);

        assert_eq!(soa_avx2, expected_soa);
        assert_eq!(soa_scalar, expected_soa);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_vec4u8_aos_to_soa() {
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
        ] {
            test_vec4u8_aos_to_soa_impl(&test_vec(n));
        }
    }

    fn test_vec4u8_soa_to_aos_impl(data: &[u8]) {
        assert!(data.len() % 4 == 0);

        let soa = generate_soa(data);

        let mut aos_avx2: Vec<Vec4u8> = vec![Vec4u8::new(); data.len() / 4];
        unsafe { vec4u8_soa_to_aos_avx2_parallel(&soa, &mut aos_avx2) };

        let mut aos_scalar: Vec<Vec4u8> = vec![Vec4u8::new(); data.len() / 4];
        vec4u8_soa_to_aos_scalar(&soa, &mut aos_scalar);

        let expected_aos = generate_aos(data);

        assert_eq!(aos_avx2, expected_aos);
        assert_eq!(aos_scalar, expected_aos);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_vec4u8_soa_to_aos() {
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
        ] {
            test_vec4u8_soa_to_aos_impl(&test_vec(n));
        }
    }

    fn test_roundtrip_impl(data: &[u8]) {
        assert!(data.len() % 4 == 0);

        let aos: Vec<Vec4u8> = generate_aos(data);
        let aos_ptr = aos.as_ptr();
        let aos_buf_ptr = unsafe { BufferPointer::new(&aos_ptr, aos.len()) };

        let mut soa = Vec4u8s::with_total_size(data.len());
        vec4u8_aos_to_soa(aos_buf_ptr, &mut soa);

        let mut expected_aos: Vec<Vec4u8> = vec![Vec4u8::new(); data.len() / 4];
        vec4u8_soa_to_aos(&soa, &mut expected_aos);

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
        ] {
            test_roundtrip_impl(&test_vec(n));
        }
    }

    proptest! {
        #[test]
        #[cfg_attr(miri, ignore)]
        fn proptest_vec4u8_aos_to_soa(mut arr in proptest::collection::vec(0..u8::MAX, 0..1_000_000)) {
            arr.truncate((arr.len() / 4) * 4);
            assert!(arr.len() % 4 == 0);
            test_vec4u8_aos_to_soa_impl(&arr);
        }

        #[test]
        #[cfg_attr(miri, ignore)]
        fn proptest_vec4u8_soa_to_aos(mut arr in proptest::collection::vec(0..u8::MAX, 0..1_000_000)) {
            arr.truncate((arr.len() / 4) * 4);
            assert!(arr.len() % 4 == 0);
            test_vec4u8_soa_to_aos_impl(&arr);
        }
    }
}
