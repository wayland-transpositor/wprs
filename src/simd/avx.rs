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

pub use std::arch::x86_64::__m128i;
pub use std::arch::x86_64::__m256i;
pub use std::arch::x86_64::_mm_add_epi8;
pub use std::arch::x86_64::_mm_set1_epi8;
pub use std::arch::x86_64::_mm_setzero_si128;
pub use std::arch::x86_64::_mm_storeu_si128;
pub use std::arch::x86_64::_mm256_castsi128_si256;
pub use std::arch::x86_64::_mm256_castsi256_si128;
pub use std::arch::x86_64::_mm256_set_m128i;
pub use std::arch::x86_64::_mm256_storeu_si256;

use cfg_if::cfg_if;

use crate::buffer_pointer::KnownSizeBufferPointer;
use crate::vec4u8::Vec4u8;

/*
 * We must enfonce that a parameter is an immediate constant in order
 * to fit inside the assembly instruction. The Rust way to do this
 * is with a Const Generic Parameter hence this pattern.
 */
#[target_feature(enable = "avx")]
#[inline]
pub fn _mm_extract_epi8<const INDEX: i32>(a: __m128i) -> i32 {
    // This is the same with the plain SSE4.1 but we want the VEX encoded variant
    std::arch::x86_64::_mm_extract_epi8(a, INDEX)
}

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))] {
        pub use std::arch::x86_64::_mm256_sub_epi8;
        pub use std::arch::x86_64::_mm256_add_epi8;
        pub use std::arch::x86_64::_mm256_set_epi8;
        pub use std::arch::x86_64::_mm256_shuffle_epi8;

        /* We must enfonce that a parameter is an immediate constant in order
         to fit inside the assembly instruction. The Rust way to do this
         is with a Const Generic Parameter */
        #[target_feature(enable = "avx2")]
        #[inline]
        pub fn _mm256_slli_si256<const SHIFT: i32>(a: __m256i) -> __m256i {
            std::arch::x86_64::_mm256_slli_si256(a, SHIFT)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        pub fn _mm256_extracti128_si256<const HIGH: i32>(a: __m256i) -> __m128i {
            std::arch::x86_64::_mm256_extracti128_si256(a, HIGH)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        pub fn _mm256_blend_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
            std::arch::x86_64::_mm256_blend_epi32(a, b, MASK)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        pub fn _mm256_extract_epi8<const INDEX: i32>(a: __m256i) -> i32 {
            std::arch::x86_64::_mm256_extract_epi8(a, INDEX)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        pub fn _mm256_inserti128_si256<const LANE: i32>(a: __m256i, b: __m128i) -> __m256i {
            std::arch::x86_64::_mm256_inserti128_si256(a, b, LANE)
        }
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_sub_epi8(a: __m256i, b: __m256i) -> __m256i {
            // 1. Extract the low 128-bit halves from the 256-bit registers.
            // _mm256_castsi256_si128 is a zero-cost instruction that just treats
            // the YMM register as an XMM register.
            let a_lo = std::arch::x86_64::_mm256_castsi256_si128(a);
            let b_lo = std::arch::x86_64::_mm256_castsi256_si128(b);

            // 2. Extract the high 128-bit halves.
            // _mm256_extractf128_si256 is an AVX instruction that pulls the
            // upper 128 bits into an XMM register.
            let a_hi = std::arch::x86_64::_mm256_extractf128_si256(a, 1);
            let b_hi = std::arch::x86_64::_mm256_extractf128_si256(b, 1);

            // 3. Perform 8-bit integer subtraction on the halves.
            // On AVX hardware, these will be emitted as VEX-encoded VPSUBB instructions.
            let res_lo = std::arch::x86_64::_mm_sub_epi8(a_lo, b_lo);
            let res_hi = std::arch::x86_64::_mm_sub_epi8(a_hi, b_hi);

            // 4. Recombine the two 128-bit results back into a single 256-bit register.
            std::arch::x86_64::_mm256_set_m128i(res_hi, res_lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_add_epi8(a: __m256i, b: __m256i) -> __m256i {
            // 1. Extract the low 128-bit halves from the 256-bit registers.
            // _mm256_castsi256_si128 is a zero-cost instruction that just treats
            // the YMM register as an XMM register.
            let a_lo = std::arch::x86_64::_mm256_castsi256_si128(a);
            let b_lo = std::arch::x86_64::_mm256_castsi256_si128(b);

            // 2. Extract the high 128-bit halves.
            // _mm256_extractf128_si256 is an AVX instruction that pulls the
            // upper 128 bits into an XMM register.
            let a_hi = std::arch::x86_64::_mm256_extractf128_si256(a, 1);
            let b_hi = std::arch::x86_64::_mm256_extractf128_si256(b, 1);

            // 3. Perform 8-bit integer subtraction on the halves.
            // On AVX hardware, these will be emitted as VEX-encoded VPSUBB instructions.
            let res_lo = std::arch::x86_64::_mm_add_epi8(a_lo, b_lo);
            let res_hi = std::arch::x86_64::_mm_add_epi8(a_hi, b_hi);

            // 4. Recombine the two 128-bit results back into a single 256-bit register.
            std::arch::x86_64::_mm256_set_m128i(res_hi, res_lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_slli_si256<const SHIFT: i32>(a: __m256i) -> __m256i {
            // 1. Split: Extract the 128-bit halves
            // Cast is zero-cost; it just treats the YMM as an XMM (low half)
            let lo = std::arch::x86_64::_mm256_castsi256_si128(a);
            // Extract the high 128 bits
            let hi = std::arch::x86_64::_mm256_extractf128_si256(a, 1);

            // 2. Shift: Apply 128-bit byte shift to each half
            // These will be compiled as VEX-encoded VPSLLDQ XMM instructions
            let res_lo = std::arch::x86_64::_mm_slli_si128(lo, SHIFT);
            let res_hi = std::arch::x86_64::_mm_slli_si128(hi, SHIFT);

            // 3. Merge: Combine back into a 256-bit register
            std::arch::x86_64::_mm256_set_m128i(res_hi, res_lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_extracti128_si256<const HIGH: i32>(a: __m256i) -> __m128i {
            std::arch::x86_64::_mm256_extractf128_si256(a, HIGH)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_blend_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
            // 1. Cast integer vectors to floating-point vectors (bit-preserving)
            let a_f = std::arch::x86_64::_mm256_castsi256_ps(a);
            let b_f = std::arch::x86_64::_mm256_castsi256_ps(b);

            // 2. Perform the blend using the AVX1 floating-point intrinsic.
            // We use a match to pipe the const generic MASK into the literal slot.
            let res_f = std::arch::x86_64::_mm256_blend_ps(a_f, b_f, MASK);

            // 3. Cast back to integer vector (bit-preserving)
            std::arch::x86_64::_mm256_castps_si256(res_f)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_extract_epi8<const INDEX: i32>(a: __m256i) -> i32 {
            let v = if INDEX < 16 {
                // Extract from low 128-bit lane (XMM)
                std::arch::x86_64::_mm256_castsi256_si128(a)
            } else {
                // Extract high 128-bit lane, then extract byte
                std::arch::x86_64::_mm256_extractf128_si256(a, 1)
            };

            // This is the same with the plain SSE4.1 but we want the VEX encoded variant
            // This should work with the two lines below
            // _mm_extract_epi8(low, INDEX)
            // _mm_extract_epi8(high, INDEX - 16)
            // TODO: revisit this when generic_const_exprs graduates from nightly
            match INDEX {
                // Lower Lane (0-15)
                0  => std::arch::x86_64::_mm_extract_epi8(v, 0),
                1  => std::arch::x86_64::_mm_extract_epi8(v, 1),
                2  => std::arch::x86_64::_mm_extract_epi8(v, 2),
                3  => std::arch::x86_64::_mm_extract_epi8(v, 3),
                4  => std::arch::x86_64::_mm_extract_epi8(v, 4),
                5  => std::arch::x86_64::_mm_extract_epi8(v, 5),
                6  => std::arch::x86_64::_mm_extract_epi8(v, 6),
                7  => std::arch::x86_64::_mm_extract_epi8(v, 7),
                8  => std::arch::x86_64::_mm_extract_epi8(v, 8),
                9  => std::arch::x86_64::_mm_extract_epi8(v, 9),
                10 => std::arch::x86_64::_mm_extract_epi8(v, 10),
                11 => std::arch::x86_64::_mm_extract_epi8(v, 11),
                12 => std::arch::x86_64::_mm_extract_epi8(v, 12),
                13 => std::arch::x86_64::_mm_extract_epi8(v, 13),
                14 => std::arch::x86_64::_mm_extract_epi8(v, 14),
                15 => std::arch::x86_64::_mm_extract_epi8(v, 15),
                // Upper Lane (16-31)
                16 => std::arch::x86_64::_mm_extract_epi8(v, 0),
                17 => std::arch::x86_64::_mm_extract_epi8(v, 1),
                18 => std::arch::x86_64::_mm_extract_epi8(v, 2),
                19 => std::arch::x86_64::_mm_extract_epi8(v, 3),
                20 => std::arch::x86_64::_mm_extract_epi8(v, 4),
                21 => std::arch::x86_64::_mm_extract_epi8(v, 5),
                22 => std::arch::x86_64::_mm_extract_epi8(v, 6),
                23 => std::arch::x86_64::_mm_extract_epi8(v, 7),
                24 => std::arch::x86_64::_mm_extract_epi8(v, 8),
                25 => std::arch::x86_64::_mm_extract_epi8(v, 9),
                26 => std::arch::x86_64::_mm_extract_epi8(v, 10),
                27 => std::arch::x86_64::_mm_extract_epi8(v, 11),
                28 => std::arch::x86_64::_mm_extract_epi8(v, 12),
                29 => std::arch::x86_64::_mm_extract_epi8(v, 13),
                30 => std::arch::x86_64::_mm_extract_epi8(v, 14),
                31 => std::arch::x86_64::_mm_extract_epi8(v, 15),
                _ => panic!("Index out of bounds for 256-bit extract"),
            }
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_inserti128_si256<const LANE: i32>(a: __m256i, b: __m128i) -> __m256i {
            // Cast to __m256 (float), insert, cast back
            let a_f = std::arch::x86_64::_mm256_castsi256_ps(a);
            let b_f = std::arch::x86_64::_mm_castsi128_ps(b);
            let res_f = std::arch::x86_64::_mm256_insertf128_ps(a_f, b_f, LANE);
            std::arch::x86_64::_mm256_castps_si256(res_f)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_set_epi8(
            e31: i8, e30: i8, e29: i8, e28: i8, e27: i8, e26: i8, e25: i8, e24: i8,
            e23: i8, e22: i8, e21: i8, e20: i8, e19: i8, e18: i8, e17: i8, e16: i8,
            e15: i8, e14: i8, e13: i8, e12: i8, e11: i8, e10: i8, e9: i8, e8: i8,
            e7: i8, e6: i8, e5: i8, e4: i8, e3: i8, e2: i8, e1: i8, e0: i8) -> __m256i {

            let low = std::arch::x86_64::_mm_set_epi8(e15, e14, e13, e12, e11, e10, e9, e8, e7, e6, e5, e4, e3, e2, e1, e0);
            let high = std::arch::x86_64::_mm_set_epi8(e31, e30, e29, e28, e27, e26, e25, e24, e23, e22, e21, e20, e19, e18, e17, e16);

            let res = std::arch::x86_64::_mm256_castsi128_si256(low);
            _mm256_inserti128_si256::<1>(res, high)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn _mm256_shuffle_epi8(a: __m256i, b: __m256i) -> __m256i {
            // 1. Extract halves of data and mask
            let a_low = std::arch::x86_64::_mm256_castsi256_si128(a);
            let a_high = std::arch::x86_64::_mm256_extractf128_si256(a, 1);
            let b_low = std::arch::x86_64::_mm256_castsi256_si128(b);
            let b_high = std::arch::x86_64::_mm256_extractf128_si256(b, 1);

            // 2. Perform SSSE3 shuffle on each 128-bit lane
            let res_low = std::arch::x86_64::_mm_shuffle_epi8(a_low, b_low);
            let res_high = std::arch::x86_64::_mm_shuffle_epi8(a_high, b_high);

            // 3. Combine back into 256-bit
            let res = std::arch::x86_64::_mm256_castsi128_si256(res_low);
            _mm256_inserti128_si256::<1>(res, res_high)
        }
    }
}

/**
 * NOTE: The following functions are not actual std::arch::x86_64 intrinsics.
 * They are wprs specific but we put them here because they have a specific
 * SSE counterpart
 */
#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_shufps_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    std::arch::x86_64::_mm256_castps_si256(std::arch::x86_64::_mm256_shuffle_ps(
        std::arch::x86_64::_mm256_castsi256_ps(a),
        std::arch::x86_64::_mm256_castsi256_ps(b),
        MASK,
    ))
}
#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_loadu_si256_mem(src: &[u8; 32]) -> __m256i {
    // SAFETY: src is which is 32 u8s, which is 256 bits, so it is safe to read
    // 256 bits from it.
    unsafe { std::arch::x86_64::_mm256_loadu_si256(src.as_ptr().cast::<__m256i>()) }
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_storeu_si256_mem(dst: &mut [u8; 32], val: __m256i) {
    // SAFETY: dst is 32 u8s, which is 256 bits, so it is safe to write 256 bits
    // to it.
    unsafe { std::arch::x86_64::_mm256_storeu_si256(dst.as_mut_ptr().cast::<__m256i>(), val) }
}

// This is the same with the plain SSE2 but we want the VEX encoded variant
#[target_feature(enable = "avx")]
#[inline]
pub fn _mm_loadu_si128_vec4u8(src: &KnownSizeBufferPointer<Vec4u8, 4>) -> __m128i {
    // SAFETY: src is 4 Vec4u8s, which is 16 u8s, which is 128 bits, so it is
    // safe to read 128 bits from it.
    unsafe { std::arch::x86_64::_mm_loadu_si128(src.ptr().cast::<__m128i>()) }
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_storeu_si256_vec4u8(dst: &mut [Vec4u8; 8], val: __m256i) {
    // SAFETY: dst is 8 Vec4u8s, which is 32 u8s, which is 256 bits, so it is
    // safe to write 256 bits to it.
    unsafe { std::arch::x86_64::_mm256_storeu_si256(dst.as_mut_ptr().cast::<__m256i>(), val) }
}
