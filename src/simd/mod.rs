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


cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))] {
        mod avx2;
        pub use crate::simd::avx2::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        mod avx;
        pub use crate::simd::avx::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))] {
        #[macro_use]
        mod sse2_base;
        mod ssse3;
        mod sse41;
        pub use crate::simd::sse41::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))] {
        #[macro_use]
        mod sse2_base;
        mod sse2;
        mod ssse3;
        pub use crate::simd::ssse3::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        #[macro_use]
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
