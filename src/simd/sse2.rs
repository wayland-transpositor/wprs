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

use cfg_if::cfg_if;

use crate::buffer_pointer::KnownSizeBufferPointer;
use crate::vec4u8::Vec4u8;

#[allow(non_camel_case_types)]
#[repr(C, align(32))]
#[derive(Copy, Clone)]
pub struct __m256i {
    pub low: __m128i,
    pub high: __m128i,
}

pub use std::arch::x86_64::_mm_add_epi8;
pub use std::arch::x86_64::_mm_set1_epi8;
pub use std::arch::x86_64::_mm_setzero_si128;
pub use std::arch::x86_64::_mm_storeu_si128;

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_set_m128i(hi: __m128i, lo: __m128i) -> __m256i {
    // In SSE2, we simply wrap the two 128-bit values
    // into our custom 256-bit emulation struct.
    __m256i { low: lo, high: hi }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_castsi128_si256(a: __m128i) -> __m256i {
    // In SSE2, we wrap the 128-bit value into our 256-bit struct.
    // We set the high bits to zero to represent the 'undefined' state safely.
    __m256i {
        low: a,
        high: std::arch::x86_64::_mm_setzero_si128(),
    }
}

#[inline]
#[target_feature(enable = "sse2")]
pub fn _mm256_castsi256_si128(a: __m256i) -> __m128i {
    // In a native __m256i, the cast returns the lower 128 bits.
    a.low
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_storeu_si256(mem_addr: *mut __m256i, a: __m256i) {
    // 1. Cast the pointer to a byte-addressable pointer (u8)
    let base_ptr = mem_addr as *mut u8;

    unsafe {
        // 2. Store the low 128 bits at the base address
        std::arch::x86_64::_mm_storeu_si128(base_ptr as *mut __m128i, a.low);

        // 3. Store the high 128 bits 16 bytes (128 bits) offset from base
        std::arch::x86_64::_mm_storeu_si128(base_ptr.add(16) as *mut __m128i, a.high);
    }
}

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))] {
        #[target_feature(enable = "sse4.1")]
        #[inline]
        pub fn _mm_extract_epi8<const INDEX: i32>(a: __m128i) -> i32 {
            std::arch::x86_64::_mm_extract_epi8(a, INDEX)
        }

        #[target_feature(enable = "sse4.1")]
        #[inline]
        pub fn _mm_blend_epi32<const MASK: i32>(a: __m128i, b: __m128i) -> __m128i {
            // If target has SSE4.1, use the specialized blend instruction
            unsafe {std::arch::x86_64::_mm_blend_epi32(a, b, MASK)}
        }
    } else {
        // I am tagging this as unsafe because the sse4.1 variant is somehow tagged as unsafe
        // while the SSE2 fallback is not. I do not know why. So we get warnings of
        // unnecessary unsafe blocks in the callers.
        // TODO: revisit inside unsafe blocks or the external tagging of SSE4.1
        // functions fallbacks as unsafe
        #[target_feature(enable = "sse2")]
        #[inline]
        pub unsafe fn _mm_extract_epi8<const INDEX: i32>(a: __m128i) -> i32 {
            // TODO: revisit this when generic_const_exprs graduates from nightly
            // _mm_extract_epi16 is available in SSE2
            let word = match INDEX / 2 {
                0 => std::arch::x86_64::_mm_extract_epi16(a, 0),
                1 => std::arch::x86_64::_mm_extract_epi16(a, 1),
                2 => std::arch::x86_64::_mm_extract_epi16(a, 2),
                3 => std::arch::x86_64::_mm_extract_epi16(a, 3),
                4 => std::arch::x86_64::_mm_extract_epi16(a, 4),
                5 => std::arch::x86_64::_mm_extract_epi16(a, 5),
                6 => std::arch::x86_64::_mm_extract_epi16(a, 6),
                7 => std::arch::x86_64::_mm_extract_epi16(a, 7),
                _ => unreachable!(),
            };

            let is_high_byte = (INDEX % 2) != 0;
            if is_high_byte {
                (word >> 8) & 0xFF
            } else {
                word & 0xFF
            }
        }

        // I am tagging this as unsafe because the sse4.1 variant is somehow tagged as unsafe
        // while the SSE2 fallback is not. I do not know why. So we get warnings of
        // unnecessary unsafe blocks in the callers.
        // TODO: revisit inside unsafe blocks or the external tagging of SSE4.1
        // functions fallbacks as unsafe
        #[target_feature(enable = "sse2")]
        #[inline]
        pub unsafe fn _mm_blend_epi32<const MASK: i32>(a: __m128i, b: __m128i) -> __m128i {
            // Fallback for SSE2, SSE3, SSSE3 (Generic bitwise blend)
            // This is a bitwise selection: (b & mask) | (a & ~mask)
            // We create a 128-bit mask based on the 4-bit M constant
            let mask = std::arch::x86_64::_mm_set_epi32(
                if (MASK & 8) != 0 { -1 } else { 0 },
                if (MASK & 4) != 0 { -1 } else { 0 },
                if (MASK & 2) != 0 { -1 } else { 0 },
                if (MASK & 1) != 0 { -1 } else { 0 },
            );
            std::arch::x86_64::_mm_or_si128(std::arch::x86_64::_mm_and_si128(mask, b), std::arch::x86_64::_mm_andnot_si128(mask, a))
        }

    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_sub_epi8(a: __m256i, b: __m256i) -> __m256i {
    // We use _mm_sub_epi8 (SSE2) twice.
    __m256i {
        low: std::arch::x86_64::_mm_sub_epi8(a.low, b.low),
        high: std::arch::x86_64::_mm_sub_epi8(a.high, b.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_add_epi8(a: __m256i, b: __m256i) -> __m256i {
    // We use _mm_add_epi8 (SSE2) twice.
    __m256i {
        low: std::arch::x86_64::_mm_add_epi8(a.low, b.low),
        high: std::arch::x86_64::_mm_add_epi8(a.high, b.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_slli_si256<const SHIFT: i32>(a: __m256i) -> __m256i {
    // SAFETY: _mm_slli_si128 is an SSE2 intrinsic.
    // It shifts the 128-bit register left by SHIFT bytes.
    // Bits do not carry across the 128-bit boundary, perfectly
    // matching the behavior of the AVX2 256-bit version.
    __m256i {
        low: std::arch::x86_64::_mm_slli_si128(a.low, SHIFT),
        high: std::arch::x86_64::_mm_slli_si128(a.high, SHIFT),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_extracti128_si256<const HIGH: i32>(a: __m256i) -> __m128i {
    // Because HIGH must be a compile-time constant,
    // the compiler will optimize this branch away entirely.
    if HIGH == 0 { a.low } else { a.high }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_blend_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    // We only care about the lower 4 bits (0-15)
    // TODO: revisit this when generic_const_exprs graduates from nightly
    unsafe {
        let low = match MASK & 0xF {
            0 => _mm_blend_epi32::<0>(a.low, b.low),
            1 => _mm_blend_epi32::<1>(a.low, b.low),
            2 => _mm_blend_epi32::<2>(a.low, b.low),
            3 => _mm_blend_epi32::<3>(a.low, b.low),
            4 => _mm_blend_epi32::<4>(a.low, b.low),
            5 => _mm_blend_epi32::<5>(a.low, b.low),
            6 => _mm_blend_epi32::<6>(a.low, b.low),
            7 => _mm_blend_epi32::<7>(a.low, b.low),
            8 => _mm_blend_epi32::<8>(a.low, b.low),
            9 => _mm_blend_epi32::<9>(a.low, b.low),
            10 => _mm_blend_epi32::<10>(a.low, b.low),
            11 => _mm_blend_epi32::<11>(a.low, b.low),
            12 => _mm_blend_epi32::<12>(a.low, b.low),
            13 => _mm_blend_epi32::<13>(a.low, b.low),
            14 => _mm_blend_epi32::<14>(a.low, b.low),
            15 => _mm_blend_epi32::<15>(a.low, b.low),
            _ => unreachable!(),
        };

        // We only care about the lower 4 bits (0-15)
        let high = match (MASK >> 4) & 0xF {
            0 => _mm_blend_epi32::<0>(a.high, b.high),
            1 => _mm_blend_epi32::<1>(a.high, b.high),
            2 => _mm_blend_epi32::<2>(a.high, b.high),
            3 => _mm_blend_epi32::<3>(a.high, b.high),
            4 => _mm_blend_epi32::<4>(a.high, b.high),
            5 => _mm_blend_epi32::<5>(a.high, b.high),
            6 => _mm_blend_epi32::<6>(a.high, b.high),
            7 => _mm_blend_epi32::<7>(a.high, b.high),
            8 => _mm_blend_epi32::<8>(a.high, b.high),
            9 => _mm_blend_epi32::<9>(a.high, b.high),
            10 => _mm_blend_epi32::<10>(a.high, b.high),
            11 => _mm_blend_epi32::<11>(a.high, b.high),
            12 => _mm_blend_epi32::<12>(a.high, b.high),
            13 => _mm_blend_epi32::<13>(a.high, b.high),
            14 => _mm_blend_epi32::<14>(a.high, b.high),
            15 => _mm_blend_epi32::<15>(a.high, b.high),
            _ => unreachable!(),
        };

        __m256i { low, high }
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_extract_epi8<const INDEX: i32>(a: __m256i) -> i32 {
    // There are 32 bytes in a 256-bit register (0-31).
    // Indices 0-15 are in the 'low' 128-bit lane.
    // Indices 16-31 are in the 'high' 128-bit lane.
    // TODO: revisit this when generic_const_exprs graduates from nightly
    unsafe {
        match INDEX {
            // Lower Lane (0-15)
            0 => _mm_extract_epi8::<0>(a.low),
            1 => _mm_extract_epi8::<1>(a.low),
            2 => _mm_extract_epi8::<2>(a.low),
            3 => _mm_extract_epi8::<3>(a.low),
            4 => _mm_extract_epi8::<4>(a.low),
            5 => _mm_extract_epi8::<5>(a.low),
            6 => _mm_extract_epi8::<6>(a.low),
            7 => _mm_extract_epi8::<7>(a.low),
            8 => _mm_extract_epi8::<8>(a.low),
            9 => _mm_extract_epi8::<9>(a.low),
            10 => _mm_extract_epi8::<10>(a.low),
            11 => _mm_extract_epi8::<11>(a.low),
            12 => _mm_extract_epi8::<12>(a.low),
            13 => _mm_extract_epi8::<13>(a.low),
            14 => _mm_extract_epi8::<14>(a.low),
            15 => _mm_extract_epi8::<15>(a.low),
            // Upper Lane (16-31)
            16 => _mm_extract_epi8::<0>(a.high),
            17 => _mm_extract_epi8::<1>(a.high),
            18 => _mm_extract_epi8::<2>(a.high),
            19 => _mm_extract_epi8::<3>(a.high),
            20 => _mm_extract_epi8::<4>(a.high),
            21 => _mm_extract_epi8::<5>(a.high),
            22 => _mm_extract_epi8::<6>(a.high),
            23 => _mm_extract_epi8::<7>(a.high),
            24 => _mm_extract_epi8::<8>(a.high),
            25 => _mm_extract_epi8::<9>(a.high),
            26 => _mm_extract_epi8::<10>(a.high),
            27 => _mm_extract_epi8::<11>(a.high),
            28 => _mm_extract_epi8::<12>(a.high),
            29 => _mm_extract_epi8::<13>(a.high),
            30 => _mm_extract_epi8::<14>(a.high),
            31 => _mm_extract_epi8::<15>(a.high),
            _ => panic!("Index out of bounds for 256-bit extract"),
        }
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_inserti128_si256<const LANE: i32>(a: __m256i, b: __m128i) -> __m256i {
    // In SIMD, Lane 0 is the lower 128 bits, Lane 1 is the upper 128 bits.
    if LANE == 0 {
        __m256i {
            low: b,       // Replace low with new 128-bit value
            high: a.high, // Keep existing high
        }
    } else {
        __m256i {
            low: a.low, // Keep existing low
            high: b,    // Replace high with new 128-bit value
        }
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_set_epi8(
    e31: i8,
    e30: i8,
    e29: i8,
    e28: i8,
    e27: i8,
    e26: i8,
    e25: i8,
    e24: i8,
    e23: i8,
    e22: i8,
    e21: i8,
    e20: i8,
    e19: i8,
    e18: i8,
    e17: i8,
    e16: i8,
    e15: i8,
    e14: i8,
    e13: i8,
    e12: i8,
    e11: i8,
    e10: i8,
    e9: i8,
    e8: i8,
    e7: i8,
    e6: i8,
    e5: i8,
    e4: i8,
    e3: i8,
    e2: i8,
    e1: i8,
    e0: i8,
) -> __m256i {
    // Construct the low 128-bit part (e0 through e15)
    let low = std::arch::x86_64::_mm_set_epi8(
        e15, e14, e13, e12, e11, e10, e9, e8, e7, e6, e5, e4, e3, e2, e1, e0,
    );
    // Construct the high 128-bit part (e16 through e31)
    let high = std::arch::x86_64::_mm_set_epi8(
        e31, e30, e29, e28, e27, e26, e25, e24, e23, e22, e21, e20, e19, e18, e17, e16,
    );

    __m256i { low, high }
}

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))] {
        #[target_feature(enable = "ssse3")]
        #[inline]
        pub fn _mm256_shuffle_epi8(a: __m256i, b: __m256i) -> __m256i {
            // SSSE3 _mm_shuffle_epi8 operates on 128-bit registers.
            // We shuffle the 'low' part of 'a' using the 'low' part of 'b'.
            let low = std::arch::x86_64::_mm_shuffle_epi8(a.low, b.low);

            // We shuffle the 'high' part of 'a' using the 'high' part of 'b'.
            let high = std::arch::x86_64::_mm_shuffle_epi8(a.high, b.high);

            __m256i { low, high }
        }
    } else {
        /// Emulates SSSE3 _mm_shuffle_epi8 using non SIMD instructions
        #[inline]
        #[target_feature(enable = "sse2")]
        fn _mm_shuffle_epi8(a: __m128i, b: __m128i) -> __m128i {
            // 1. Cast to arrays to access individual bytes (SSE2 doesn't have a direct
            // variable byte-shuffler like PSHUFB).
            let src: [u8; 16] = unsafe {std::mem::transmute(a)};
            let mask: [u8; 16] = unsafe {std::mem::transmute(b)};
            let mut res = [0u8; 16];

            for i in 0..16 {
                // SSSE3/AVX2 Logic:
                // If bit 7 of the mask byte is set, the result is 0.
                // Otherwise, use the lower 4 bits as an index into the source lane.
                if (mask[i] & 0x80) == 0 {
                    let index = (mask[i] & 0x0F) as usize;
                    res[i] = src[index];
                } else {
                    res[i] = 0;
                }
            }

            unsafe {std::mem::transmute(res)}
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        pub fn _mm256_shuffle_epi8(a: __m256i, b: __m256i) -> __m256i {
            // We must shuffle 'low' and 'high' independently to match AVX2 behavior.
            __m256i {
                low: _mm_shuffle_epi8(a.low, b.low),
                high: _mm_shuffle_epi8(a.high, b.high),
            }
        }
    }
}

/**
 * NOTE: The following functions are not actual std::arch::x86_64 intrinsics.
 * They are wprs specific but we put them here because they have a specific
 * SSE counterpart
 */

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_shufps_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    // 1. Process the Low 128 bits
    let low = std::arch::x86_64::_mm_castps_si128(std::arch::x86_64::_mm_shuffle_ps(
        std::arch::x86_64::_mm_castsi128_ps(a.low),
        std::arch::x86_64::_mm_castsi128_ps(b.low),
        MASK,
    ));

    // 2. Process the High 128 bits (exactly the same logic)
    let high = std::arch::x86_64::_mm_castps_si128(std::arch::x86_64::_mm_shuffle_ps(
        std::arch::x86_64::_mm_castsi128_ps(a.high),
        std::arch::x86_64::_mm_castsi128_ps(b.high),
        MASK,
    ));

    __m256i { low, high }
}

/// Emulates a 256-bit aligned load using SSE2 instructions.
#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_loadu_si256_mem(src: &[u8; 32]) -> __m256i {
    unsafe {
        let ptr = src.as_ptr();

        // 1. Load the first 128 bits (indices 0, 1, 2, 3)
        // Cast the i32 pointer to an __m128i pointer for the intrinsic
        let low = std::arch::x86_64::_mm_loadu_si128(ptr.cast::<__m128i>());

        // 2. Load the second 128 bits (indices 4, 5, 6, 7)
        // We offset the pointer by 4 (since it's a *const i32, this is 16 bytes)
        let high = std::arch::x86_64::_mm_loadu_si128(ptr.add(16).cast::<__m128i>());

        __m256i { low, high }
    }
}

/// Emulates a 256-bit aligned store using SSE2 instructions.
#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_storeu_si256_mem(dst: &mut [u8; 32], val: __m256i) {
    // SAFETY: dst is 32 u8s (256 bits).
    // We store two 128-bit chunks sequentially.
    unsafe {
        let base_ptr = dst.as_mut_ptr();
        // 1. Store the low 128 bits into indices [0..16]
        std::arch::x86_64::_mm_storeu_si128(base_ptr.cast::<__m128i>(), val.low);

        // 2. Store the high 128 bits into indices [16..32]
        // We offset the pointer by 16 bytes.
        std::arch::x86_64::_mm_storeu_si128(base_ptr.add(16).cast::<__m128i>(), val.high);
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm_loadu_si128_vec4u8(src: &KnownSizeBufferPointer<Vec4u8, 4>) -> __m128i {
    // SAFETY: src is 4 Vec4u8s, which is 16 u8s, which is 128 bits, so it is
    // safe to read 128 bits from it.
    unsafe { std::arch::x86_64::_mm_loadu_si128(src.ptr().cast::<__m128i>()) }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_storeu_si256_vec4u8(dst: &mut [Vec4u8; 8], val: __m256i) {
    // SAFETY: dst is 8 Vec4u8s, which is 32 u8s, which is 256 bits, so it is
    // safe to write 256 bits to it.
    // val consists of two 128-bit registers = 256 bytes.
    unsafe {
        // Get a raw pointer to the start of the 32-byte buffer
        let base_ptr = dst.as_mut_ptr() as *mut Vec4u8;

        // Store the low 128 bits into the first 16 bytes (indices 0-15)
        std::arch::x86_64::_mm_storeu_si128(base_ptr.cast::<__m128i>(), val.low);

        // Store the high 128 bits into the next 16 bytes (indices 16-31)
        // .add(16) moves the pointer forward by 16 bytes
        std::arch::x86_64::_mm_storeu_si128(base_ptr.add(4).cast::<__m128i>(), val.high);
    }
}
