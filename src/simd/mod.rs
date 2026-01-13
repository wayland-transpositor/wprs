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

use cfg_if::cfg_if;

/**
 * We use these macros here to have the same body (implementation)
 * for SSE2 and SSE3 _mm256_* emulation while calling target specific
 * intrinsics without safety warnings.
 */
macro_rules! _mm256_shuffle_epi8 {
    ($a:expr, $b:expr) => {{
        // We must shuffle 'low' and 'high' independently to match AVX2 behavior.
        __m256i {
            low: _mm_shuffle_epi8($a.low, $b.low),
            high: _mm_shuffle_epi8($a.high, $b.high),
        }
    }};
}

/**
 * We use these macros here to have the same body (implementation)
 * for SSE2 and SSE4.1 _mm256_* emulation while calling target specific
 * intrinsics without safety warnings.
 */
macro_rules! _mm256_extract_epi8 {
    ($a:expr, $INDEX:expr) => {{
        match $INDEX {
            // Lower Lane (0-15)
            0 => _mm_extract_epi8::<0>($a.low),
            1 => _mm_extract_epi8::<1>($a.low),
            2 => _mm_extract_epi8::<2>($a.low),
            3 => _mm_extract_epi8::<3>($a.low),
            4 => _mm_extract_epi8::<4>($a.low),
            5 => _mm_extract_epi8::<5>($a.low),
            6 => _mm_extract_epi8::<6>($a.low),
            7 => _mm_extract_epi8::<7>($a.low),
            8 => _mm_extract_epi8::<8>($a.low),
            9 => _mm_extract_epi8::<9>($a.low),
            10 => _mm_extract_epi8::<10>($a.low),
            11 => _mm_extract_epi8::<11>($a.low),
            12 => _mm_extract_epi8::<12>($a.low),
            13 => _mm_extract_epi8::<13>($a.low),
            14 => _mm_extract_epi8::<14>($a.low),
            15 => _mm_extract_epi8::<15>($a.low),
            // Upper Lane (16-31)
            16 => _mm_extract_epi8::<0>($a.high),
            17 => _mm_extract_epi8::<1>($a.high),
            18 => _mm_extract_epi8::<2>($a.high),
            19 => _mm_extract_epi8::<3>($a.high),
            20 => _mm_extract_epi8::<4>($a.high),
            21 => _mm_extract_epi8::<5>($a.high),
            22 => _mm_extract_epi8::<6>($a.high),
            23 => _mm_extract_epi8::<7>($a.high),
            24 => _mm_extract_epi8::<8>($a.high),
            25 => _mm_extract_epi8::<9>($a.high),
            26 => _mm_extract_epi8::<10>($a.high),
            27 => _mm_extract_epi8::<11>($a.high),
            28 => _mm_extract_epi8::<12>($a.high),
            29 => _mm_extract_epi8::<13>($a.high),
            30 => _mm_extract_epi8::<14>($a.high),
            31 => _mm_extract_epi8::<15>($a.high),
            _ => panic!("Index out of bounds for 256-bit extract"),
        }
    }};
}

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))] {
        mod avx2;
        pub use crate::simd::avx2::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        mod avx;
        pub use crate::simd::avx::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))] {
        mod sse2_base;
        mod ssse3;
        mod sse41;
        pub use crate::simd::sse41::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))] {
        mod sse2_base;
        mod sse2;
        mod ssse3;
        pub use crate::simd::ssse3::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        mod sse2_base;
        mod sse2;
        pub use crate::simd::sse2::*;
    } else {
        compile_error!("x86_64 SIMD support is required.");
    }
}

#[allow(dead_code)]
pub fn print_vec_char_128_dec(x: __m128i) {
    let mut v = [0u8; 16];
    // SAFETY: dst is 16 * 8 = bytes
    unsafe {
        _mm_storeu_si128(v.as_mut_ptr().cast::<__m128i>(), x);
    }
    println!(
        "{:0>2} {:0>2} {:0>2} {:0>2} | {:0>2} {:0>2} {:0>2} {:0>2} | {:0>2} {:0>2} {:0>2} {:0>2} | {:0>2} {:0>2} {:0>2} {:0>2}",
        v[15],
        v[14],
        v[13],
        v[12],
        v[11],
        v[10],
        v[9],
        v[8],
        v[7],
        v[6],
        v[5],
        v[4],
        v[3],
        v[2],
        v[1],
        v[0]
    );
}

#[allow(dead_code)]
pub fn print_vec_char_256_hex(x: __m256i) {
    let mut v = [0u8; 32];
    // SAFETY: dst is 32 * 8 = bytes
    unsafe {
        _mm256_storeu_si256(v.as_mut_ptr().cast::<__m256i>(), x);
    }
    println!(
        "{:0>2x} {:0>2x} {:0>2x} {:0>2x} | {:0>2x} {:0>2x} {:0>2x} {:0>2x} | {:0>2x} {:0>2x} {:0>2x} {:0>2x} | {:0>2x} {:0>2x} {:0>2x} {:0>2x} || {:0>2x} {:0>2x} {:0>2x} {:0>2x} | {:0>2x} {:0>2x} {:0>2x} {:0>2x} | {:0>2x} {:0>2x} {:0>2x} {:0>2x} | {:0>2x} {:0>2x} {:0>2x} {:0>2x}",
        v[31],
        v[30],
        v[29],
        v[28],
        v[27],
        v[26],
        v[25],
        v[24],
        v[23],
        v[22],
        v[21],
        v[20],
        v[19],
        v[18],
        v[17],
        v[16],
        v[15],
        v[14],
        v[13],
        v[12],
        v[11],
        v[10],
        v[9],
        v[8],
        v[7],
        v[6],
        v[5],
        v[4],
        v[3],
        v[2],
        v[1],
        v[0]
    );
}
