#![allow(unused_imports)]

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
use crate::vec4u8::Vec4u8;
use crate::vec4u8::Vec4u8s;

use cfg_if::cfg_if;

// The feature set is cumalative in x86_64. You cannot have avx2 without avx or ssse3, sse3, sse2
// target-cpu=x86-64-v2 has up to ssse3
// target-cpu=x86-64-v3 has also avx, avx2
// Sandy Bridge and Ivy Bridge is a special case that has avx but not avx2 so technically is v2
// with extra stuff (avx) but does not reach v3 due to avx2
// Compiling with native should unlock the usage of individual flags

// First the data - 128bit (SSE2)
// These needs to be also defined for the avx case so we cannot
// simply put them in the next switch
cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        use std::arch::x86_64::_mm_castps_si128;
        use std::arch::x86_64::_mm_castsi128_ps;
        use std::arch::x86_64::_mm_shuffle_ps;
        use std::arch::x86_64::_mm_loadu_si128;
        use std::arch::x86_64::_mm_storeu_si128;
        use std::arch::x86_64::_mm_sub_epi8;
        use std::arch::x86_64::_mm_add_epi8;
        use std::arch::x86_64::_mm_slli_si128;
        use std::arch::x86_64::_mm_set1_epi8;
        use std::arch::x86_64::_mm_setzero_si128;
        use std::arch::x86_64::_mm_set_epi8;

        cfg_if! {
            if #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))] {
                use std::arch::x86_64::_mm_extract_epi8;
                use std::arch::x86_64::_mm_blend_epi32;
            } else {
                use std::arch::x86_64::_mm_extract_epi16;
                use std::arch::x86_64::_mm_set_epi32;
                use std::arch::x86_64::_mm_or_si128;
                use std::arch::x86_64::_mm_and_si128;
                use std::arch::x86_64::_mm_andnot_si128;
            }
        }

        cfg_if! {
            if #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))] {
                use std::arch::x86_64::_mm_shuffle_epi8;
            }
        }

        use std::arch::x86_64::__m128i;

        #[allow(non_camel_case_types)]
        pub type wprs__m128i = __m128i;
    }
}

// Now the rest of data - 256bit (AVX), Generic
cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        // sse2 and 128bits intrinsics are also available
        use std::arch::x86_64::_mm256_castps_si256;
        use std::arch::x86_64::_mm256_castsi256_ps;
        use std::arch::x86_64::_mm256_shuffle_ps;
        use std::arch::x86_64::_mm256_loadu_si256;
        use std::arch::x86_64::_mm256_storeu_si256;
        use std::arch::x86_64::_mm256_castsi256_si128;
        use std::arch::x86_64::_mm256_set_m128i;
        use std::arch::x86_64::_mm256_castsi128_si256;
        use std::arch::x86_64::_mm256_insertf128_ps;

        cfg_if! {
            if #[cfg(all(target_arch = "x86_64", target_feature = "avx", not(target_feature = "avx2")))] {
                // AVX only not AVX2: Sandy Bridge and Ivy Bridge
                use std::arch::x86_64::_mm256_extractf128_si256;
                use std::arch::x86_64::_mm256_blend_ps;
            } else if #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))] {
                // AVX2 in addition of AVX
                use std::arch::x86_64::_mm256_sub_epi8;
                use std::arch::x86_64::_mm256_add_epi8;
                use std::arch::x86_64::_mm256_slli_si256;
                use std::arch::x86_64::_mm256_extracti128_si256;
                use std::arch::x86_64::_mm256_blend_epi32;
                use std::arch::x86_64::_mm256_extract_epi8;
                use std::arch::x86_64::_mm256_inserti128_si256;
                use std::arch::x86_64::_mm256_set_epi8;
                use std::arch::x86_64::_mm256_shuffle_epi8;
            }
        }

        use std::arch::x86_64::__m256i;

        #[allow(non_camel_case_types)]
        pub type wprs__m256i = __m256i;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        // SSE2 but not AVX
        #[allow(non_camel_case_types)]
        #[repr(C, align(32))]
        #[derive(Copy, Clone)]
        pub struct wprs__m256i {
            pub low: wprs__m128i,
            pub high: wprs__m128i,
        }
    } else {
        #[allow(non_camel_case_types)]
        #[repr(C, align(32))]
        #[derive(Copy, Clone)]
        pub struct wprs__m128i([i32; 4]);

        #[allow(non_camel_case_types)]
        #[repr(C, align(32))]
        #[derive(Copy, Clone)]
        pub struct wprs__m256i([i32; 8]);
    }
}

// We use this block when we have a triple implementation: AVX, SSE2 and Generic
cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_shufps_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            _mm256_castps_si256(_mm256_shuffle_ps(
                _mm256_castsi256_ps(a),
                _mm256_castsi256_ps(b),
                MASK,
            ))
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_loadu_si256_mem(src: &[u8; 32]) -> wprs__m256i {
            // SAFETY: src is which is 32 u8s, which is 256 bits, so it is safe to read
            // 256 bits from it.
            unsafe { _mm256_loadu_si256(src.as_ptr().cast::<wprs__m256i>()) }
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_storeu_si256_mem(dst: &mut [u8; 32], val: wprs__m256i) {
            // SAFETY: dst is 32 u8s, which is 256 bits, so it is safe to write 256 bits
            // to it.
            unsafe { _mm256_storeu_si256(dst.as_mut_ptr().cast::<wprs__m256i>(), val) }
        }

        // This is the same with the plain SSE2 but we want the VEX encoded variant
        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm_loadu_si128_vec4u8(src: &KnownSizeBufferPointer<Vec4u8, 4>) -> wprs__m128i {
            // SAFETY: src is 4 Vec4u8s, which is 16 u8s, which is 128 bits, so it is
            // safe to read 128 bits from it.
            unsafe { _mm_loadu_si128(src.ptr().cast::<wprs__m128i>()) }
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_storeu_si256_vec4u8(dst: &mut [Vec4u8; 8], val: wprs__m256i) {
            // SAFETY: dst is 8 Vec4u8s, which is 32 u8s, which is 256 bits, so it is
            // safe to write 256 bits to it.
            unsafe { _mm256_storeu_si256(dst.as_mut_ptr().cast::<wprs__m256i>(), val) }
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm_set1_epi8(a: i8) -> wprs__m128i {
            // This is the same with the plain SSE2 but we want the VEX encoded variant
            _mm_set1_epi8(a)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm_extract_epi8<const INDEX: i32>(a: wprs__m128i) -> i32 {
            // This is the same with the plain SSE4.1 but we want the VEX encoded variant
            _mm_extract_epi8(a, INDEX)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_set_m128i(hi: wprs__m128i, lo: wprs__m128i) -> wprs__m256i {
            _mm256_set_m128i(hi, lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm_setzero_si128() -> wprs__m128i {
            _mm_setzero_si128()
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_castsi128_si256(a: wprs__m128i) -> wprs__m256i {
            _mm256_castsi128_si256(a)
        }

        // This is the same with the plain SSE2 but we want the VEX encoded variant
        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm_add_epi8(a: wprs__m128i, b: wprs__m128i) -> wprs__m128i {
            _mm_add_epi8(a, b)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_castsi256_si128(a: wprs__m256i) -> wprs__m128i {
            _mm256_castsi256_si128(a)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn wprs_mm_storeu_si128(mem_addr: *mut wprs__m128i, a: wprs__m128i) {
            unsafe {
                _mm_storeu_si128(mem_addr, a)
            }
        }

        #[target_feature(enable = "avx")]
        #[inline]
        pub fn wprs_mm256_storeu_si256(mem_addr: *mut wprs__m256i, a: wprs__m256i) {
            unsafe {
                _mm256_storeu_si256(mem_addr, a)
            }
        }
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_shufps_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Process the Low 128 bits
            let low = _mm_castps_si128(_mm_shuffle_ps(
                _mm_castsi128_ps(a.low),
                _mm_castsi128_ps(b.low),
                MASK,
            ));

            // 2. Process the High 128 bits (exactly the same logic)
            let high = _mm_castps_si128(_mm_shuffle_ps(
                _mm_castsi128_ps(a.high),
                _mm_castsi128_ps(b.high),
                MASK,
            ));

            wprs__m256i {low, high}
        }

        /// Emulates a 256-bit aligned load using SSE2 instructions.
        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_loadu_si256_mem(src: &[u8; 32]) -> wprs__m256i {
            unsafe {
                let ptr = src.as_ptr();

                // 1. Load the first 128 bits (indices 0, 1, 2, 3)
                // Cast the i32 pointer to an __m128i pointer for the intrinsic
                let low = _mm_loadu_si128(ptr.cast::<__m128i>());

                // 2. Load the second 128 bits (indices 4, 5, 6, 7)
                // We offset the pointer by 4 (since it's a *const i32, this is 16 bytes)
                let high = _mm_loadu_si128(ptr.add(16).cast::<__m128i>());

                wprs__m256i {low, high}
            }
        }

        /// Emulates a 256-bit aligned store using SSE2 instructions.
        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_storeu_si256_mem(dst: &mut [u8; 32], val: wprs__m256i) {
            // SAFETY: dst is 32 u8s (256 bits).
            // We store two 128-bit chunks sequentially.
            unsafe {
                let base_ptr = dst.as_mut_ptr();
                // 1. Store the low 128 bits into indices [0..16]
                _mm_storeu_si128(base_ptr.cast::<__m128i>(), val.low);

                // 2. Store the high 128 bits into indices [16..32]
                // We offset the pointer by 16 bytes.
                _mm_storeu_si128(base_ptr.add(16).cast::<__m128i>(), val.high);
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm_loadu_si128_vec4u8(src: &KnownSizeBufferPointer<Vec4u8, 4>) -> wprs__m128i {
            // SAFETY: src is 4 Vec4u8s, which is 16 u8s, which is 128 bits, so it is
            // safe to read 128 bits from it.
            unsafe { _mm_loadu_si128(src.ptr().cast::<wprs__m128i>()) }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_storeu_si256_vec4u8(dst: &mut [Vec4u8; 8], val: wprs__m256i) {
            // SAFETY: dst is 8 Vec4u8s, which is 32 u8s, which is 256 bits, so it is
            // safe to write 256 bits to it.
            // val consists of two 128-bit registers = 256 bytes.
            unsafe {
                // Get a raw pointer to the start of the 32-byte buffer
                let base_ptr = dst.as_mut_ptr() as *mut Vec4u8;

                // Store the low 128 bits into the first 16 bytes (indices 0-15)
                _mm_storeu_si128(base_ptr.cast::<wprs__m128i>(), val.low);

                // Store the high 128 bits into the next 16 bytes (indices 16-31)
                // .add(16) moves the pointer forward by 16 bytes
                _mm_storeu_si128(base_ptr.add(4).cast::<wprs__m128i>(), val.high);
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm_set1_epi8(a: i8) -> wprs__m128i {
            _mm_set1_epi8(a)
        }

        cfg_if! {
            if #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))] {
                #[target_feature(enable = "sse4.1")]
                #[inline]
                fn wprs_mm_extract_epi8<const INDEX: i32>(a: wprs__m128i) -> i32 {
                    _mm_extract_epi8(a, INDEX)
                }
            } else {
                #[target_feature(enable = "sse2")]
                #[inline]
                fn wprs_mm_extract_epi8<const INDEX: i32>(a: wprs__m128i) -> i32 {
                    // TODO: revisit this when generic_const_exprs graduates from nightly
                    // _mm_extract_epi16 is available in SSE2
                    let word = match INDEX / 2 {
                        0 => _mm_extract_epi16(a, 0),
                        1 => _mm_extract_epi16(a, 1),
                        2 => _mm_extract_epi16(a, 2),
                        3 => _mm_extract_epi16(a, 3),
                        4 => _mm_extract_epi16(a, 4),
                        5 => _mm_extract_epi16(a, 5),
                        6 => _mm_extract_epi16(a, 6),
                        7 => _mm_extract_epi16(a, 7),
                        _ => unreachable!(),
                    };

                    let is_high_byte = (INDEX % 2) != 0;
                    if is_high_byte {
                        (word >> 8) & 0xFF
                    } else {
                        word & 0xFF
                    }
                }
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_set_m128i(hi: wprs__m128i, lo: wprs__m128i) -> wprs__m256i {
            // In SSE2, we simply wrap the two 128-bit values
            // into our custom 256-bit emulation struct.
            wprs__m256i {low: lo, high:hi}
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm_setzero_si128() -> wprs__m128i {
            _mm_setzero_si128()
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_castsi128_si256(a: wprs__m128i) -> wprs__m256i {
            // In SSE2, we wrap the 128-bit value into our 256-bit struct.
            // We set the high bits to zero to represent the 'undefined' state safely.
            wprs__m256i {
                low: a,
                high: _mm_setzero_si128(),
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm_add_epi8(a: wprs__m128i, b: wprs__m128i) -> wprs__m128i {
            _mm_add_epi8(a, b)
        }

        #[inline]
        #[target_feature(enable = "sse2")]
        fn wprs_mm256_castsi256_si128(a: wprs__m256i) -> wprs__m128i {
            // In a native __m256i, the cast returns the lower 128 bits.
            a.low
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        pub fn wprs_mm_storeu_si128(mem_addr: *mut wprs__m128i, a: wprs__m128i) {
            unsafe {
                _mm_storeu_si128(mem_addr, a)
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        pub fn wprs_mm256_storeu_si256(mem_addr: *mut wprs__m256i, a: wprs__m256i) {
            // 1. Cast the pointer to a byte-addressable pointer (u8)
            let base_ptr = mem_addr as *mut u8;

            unsafe {
                // 2. Store the low 128 bits at the base address
                _mm_storeu_si128(base_ptr as *mut __m128i, a.low);

                // 3. Store the high 128 bits 16 bytes (128 bits) offset from base
                _mm_storeu_si128(base_ptr.add(16) as *mut __m128i, a.high);
            }
        }
    } else {
        #[inline]
        fn wprs_mm256_shufps_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            let mut res = [i32; 8];

            // The mask is an 8-bit value: [index_b1, index_b0, index_a1, index_a0]
            // Each index is 2 bits (0-3).
            let m0 = (MASK & 0x3) as usize;          // Bits 0-1
            let m1 = ((MASK >> 2) & 0x3) as usize;   // Bits 2-3
            let m2 = ((MASK >> 4) & 0x3) as usize;   // Bits 4-5
            let m3 = ((MASK >> 6) & 0x3) as usize;   // Bits 6-7

            // Process Lower 128-bit Half (Indices 0-3)
            res[0] = a[m0];
            res[1] = a[m1];
            res[2] = b[m2];
            res[3] = b[m3];

            // Process Upper 128-bit Half (Indices 4-7)
            // AVX / AVX2 shuffles apply the same mask indices to the second half
            res[4] = a[4 + m0];
            res[5] = a[4 + m1];
            res[6] = b[4 + m2];
            res[7] = b[4 + m3];

            wprs__m256i(res)
        }

        #[inline]
        fn wprs_mm256_loadu_si256_mem(src: &[u8; 32]) -> wprs__m256i {
            // This will read exactly 32 bytes (256 bits) because
            // the generic type of the destination is [u8; 32].
            unsafe {
                wprs__m256i(src.as_ptr().cast::<wprs__m256i>())
            }
        }

        #[inline]
        fn wprs_mm256_storeu_si256_mem(dst: &mut [u8; 32], val: wprs__m256i) {
            // SAFETY:
            // 1. dst is a &mut [u8; 32], so we have exclusive access to 32 bytes.
            // 2. val is wprs__m256i which is [i32; 8], also exactly 32 bytes.
            unsafe {
                let ptr = dst.as_mut_ptr() as *mut wprs__m256i;
                std::ptr::write(ptr, val);
            }
        }

        #[inline]
        fn wprs_mm_loadu_si128_vec4u8(src: &KnownSizeBufferPointer<Vec4u8, 4>) -> wprs__m128i {
            unsafe {
                wprs__m128i(src.ptr().cast::<[u8; 16]>())
            }
        }

        #[inline]
        fn wprs_mm256_storeu_si256_vec4u8(dst: &mut [Vec4u8; 8], val: wprs__m256i) {
            // SAFETY:
            // 1. dst is [Vec4u8; 8]. Assuming Vec4u8 is 4 bytes, the total size is 32 bytes.
            // 2. val is wprs__m256i ([i32; 8]), which is also exactly 32 bytes.
            unsafe {
                let ptr = dst.as_mut_ptr() as *mut wprs__m256i;
                std::ptr::write(ptr, val);
            }
        }

        #[inline]
        fn wprs_mm_set1_epi8(a: i8) -> wprs__m128i {
            // 1. Cast the i8 to u8 to avoid sign extension issues during bitwise ops
            let b = a as u8 as u32;

            // 2. Pack the byte into all 4 positions of a 32-bit integer
            // Result: 0xBBBBBBBB where B is the byte 'a'
            let packed_i32 = b | (b << 8) | (b << 16) | (b << 24);

            // 3. Fill the internal array of the struct (4 * 32 bits = 128 bits)
            wprs__m128i([packed_i32 as i32; 4])
        }

        #[inline]
        fn wprs_mm_extract_epi8<const INDEX: i32>(a: wprs__m128i) -> i32 {
            // 1. Boundary check: 128-bit register has 16 bytes (0-15)
            // Using a constant assertion or a simple clamp/mask
            let idx = (INDEX as usize) & 0xF;

            // 2. Access the bytes.
            // We transmute the [i32; 4] to [u8; 16] to extract the specific byte.
            let bytes: [u8; 16] = unsafe { std::mem::transmute(a.0) };

            // 3. Extract and return as i32 (zero-extended)
            // The intrinsic returns the byte as a 32-bit integer.
            bytes[idx] as i32
        }

        #[inline]
        fn wprs_mm256_set_m128i(hi: wprs__m128i, lo: wprs__m128i) -> wprs__m256i {
            let mut result_array = [0i32; 8];

            // Copy 'lo' into the first half (indices 0, 1, 2, 3)
            result_array[0] = lo.0[0];
            result_array[1] = lo.0[1];
            result_array[2] = lo.0[2];
            result_array[3] = lo.0[3];

            // Copy 'hi' into the second half (indices 4, 5, 6, 7)
            result_array[4] = hi.0[0];
            result_array[5] = hi.0[1];
            result_array[6] = hi.0[2];
            result_array[7] = hi.0[3];

            wprs__m256i(result_array)
        }

        #[inline]
        fn wprs_mm_setzero_si128() -> wprs__m128i {
            // Initializes the [i32; 4] array with all zeros.
            // Modern compilers will optimize this into the most efficient
            // zeroing instruction for the target architecture.
            wprs__m128i([0; 4])
        }

        #[inline]
        fn wprs_mm256_castsi128_si256(a: wprs__m128i) -> wprs__m256i {
            // We take the four i32 elements from the 128-bit struct
            // and place them in the low lane (indices 0-3) of the 256-bit struct.
            // The high lane (indices 4-7) is set to zero (representing "undefined").
            wprs__m256i([a.0[0], a.0[1], a.0[2], a.0[3], 0, 0, 0, 0])
        }

        #[inline]
        fn wprs_mm_add_epi8(a: wprs__m128i, b: wprs__m128i) -> wprs__m128i {
            // 1. Transmute the i32 arrays into u8 arrays to access byte-level data
            // wprs__m128i is [i32; 4], which is 16 bytes total.
            let a_bytes: [u8; 16] = std::mem::transmute(a.0);
            let b_bytes: [u8; 16] = std::mem::transmute(b.0);
            let mut res_bytes = [0u8; 16];

            // 2. Perform vertical addition on each byte.
            // Rust's wrapping_add perfectly simulates the wraparound behavior of _mm_add_epi8.
            for i in 0..16 {
                res_bytes[i] = a_bytes[i].wrapping_add(b_bytes[i]);
            }

            // 3. Transmute back to the original struct format
            wprs__m128i(std::mem::transmute(res_bytes))
        }

        #[inline]
        fn wprs_mm256_castsi256_si128(a: wprs__m256i) -> wprs__m128i {
            // Extract the first 4 elements (0, 1, 2, 3) into the 128-bit struct
            // wprs__m128i([a.0[0], a.0[1], a.0[2], a.0[3]])
            // 1. Get a pointer to the start of the 256-bit array
            // 2. Cast it to a pointer to a 128-bit array (wprs__m128i)
            // 3. Read the value (Dereference)
            let ptr = &a as *const wprs__m256i as *const wprs__m128i;

            // Safety: wprs__m256i is 32 bytes and wprs__m128i is 16 bytes.
            // Reading the first 16 bytes from a 32-byte aligned source is safe.
            *ptr
        }

        #[inline]
        pub fn wprs_mm_storeu_si128(mem_addr: *mut wprs__m128i, a: wprs__m128i) {
            // We use copy_nonoverlapping, which is Rust's version of memcpy.
            // It is "unaligned-safe" by definition.
            // mem_addr: destination pointer
            // &a: source pointer (the local struct on the stack)
            // 1: number of wprs__m128i elements to copy
            std::ptr::copy_nonoverlapping(&a, mem_addr, 1);
        }

        #[inline]
        pub fn wprs_mm256_storeu_si256(mem_addr: *mut wprs__m256i, a: wprs__m256i) {
            // We use copy_nonoverlapping (effectively a memcpy).
            // This safely moves 32 bytes from the local stack variable 'a'
            // to the destination address, even if that address is not 32-byte aligned.
            std::ptr::copy_nonoverlapping(&a, mem_addr, 1);
        }
    }
}

// We use this block when we have a quadraple implementation: AVX2, AVX, SSE2 and Generic
// Sandy Bridge and Ivy Bridge have no AVX2
cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))] {
        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_sub_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            _mm256_sub_epi8(a, b)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_add_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            _mm256_add_epi8(a, b)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_slli_si256<const SHIFT: i32>(a: wprs__m256i) -> wprs__m256i {
            _mm256_slli_si256(a, SHIFT)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_extracti128_si256<const HIGH: i32>(a: wprs__m256i) -> wprs__m128i {
            _mm256_extracti128_si256(a, HIGH)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_blend_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            _mm256_blend_epi32(a, b, MASK)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_extract_epi8<const INDEX: i32>(a: wprs__m256i) -> i32 {
            _mm256_extract_epi8(a, INDEX)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_inserti128_si256<const LANE: i32>(a: wprs__m256i, b: wprs__m128i) -> wprs__m256i {
            _mm256_inserti128_si256(a, b, LANE)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_set_epi8(
            e31: i8, e30: i8, e29: i8, e28: i8, e27: i8, e26: i8, e25: i8, e24: i8,
            e23: i8, e22: i8, e21: i8, e20: i8, e19: i8, e18: i8, e17: i8, e16: i8,
            e15: i8, e14: i8, e13: i8, e12: i8, e11: i8, e10: i8, e9: i8, e8: i8,
            e7: i8, e6: i8, e5: i8, e4: i8, e3: i8, e2: i8, e1: i8, e0: i8) -> wprs__m256i {
            _mm256_set_epi8(
                e31, e30, e29, e28, e27, e26, e25, e24,
                e23, e22, e21, e20, e19, e18, e17, e16,
                e15, e14, e13, e12, e11, e10, e9, e8,
                e7, e6, e5, e4, e3, e2, e1, e0)
        }

        #[target_feature(enable = "avx2")]
        #[inline]
        fn wprs_mm256_shuffle_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            _mm256_shuffle_epi8(a, b)
        }
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_sub_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Extract the low 128-bit halves from the 256-bit registers.
            // _mm256_castsi256_si128 is a zero-cost instruction that just treats
            // the YMM register as an XMM register.
            let a_lo = _mm256_castsi256_si128(a);
            let b_lo = _mm256_castsi256_si128(b);

            // 2. Extract the high 128-bit halves.
            // _mm256_extractf128_si256 is an AVX instruction that pulls the
            // upper 128 bits into an XMM register.
            let a_hi = _mm256_extractf128_si256(a, 1);
            let b_hi = _mm256_extractf128_si256(b, 1);

            // 3. Perform 8-bit integer subtraction on the halves.
            // On AVX hardware, these will be emitted as VEX-encoded VPSUBB instructions.
            let res_lo = _mm_sub_epi8(a_lo, b_lo);
            let res_hi = _mm_sub_epi8(a_hi, b_hi);

            // 4. Recombine the two 128-bit results back into a single 256-bit register.
            wprs_mm256_set_m128i(res_hi, res_lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_add_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Extract the low 128-bit halves from the 256-bit registers.
            // _mm256_castsi256_si128 is a zero-cost instruction that just treats
            // the YMM register as an XMM register.
            let a_lo = _mm256_castsi256_si128(a);
            let b_lo = _mm256_castsi256_si128(b);

            // 2. Extract the high 128-bit halves.
            // _mm256_extractf128_si256 is an AVX instruction that pulls the
            // upper 128 bits into an XMM register.
            let a_hi = _mm256_extractf128_si256(a, 1);
            let b_hi = _mm256_extractf128_si256(b, 1);

            // 3. Perform 8-bit integer subtraction on the halves.
            // On AVX hardware, these will be emitted as VEX-encoded VPSUBB instructions.
            let res_lo = _mm_add_epi8(a_lo, b_lo);
            let res_hi = _mm_add_epi8(a_hi, b_hi);

            // 4. Recombine the two 128-bit results back into a single 256-bit register.
            wprs_mm256_set_m128i(res_hi, res_lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_slli_si256<const SHIFT: i32>(a: wprs__m256i) -> wprs__m256i {
            // 1. Split: Extract the 128-bit halves
            // Cast is zero-cost; it just treats the YMM as an XMM (low half)
            let lo = _mm256_castsi256_si128(a);
            // Extract the high 128 bits
            let hi = _mm256_extractf128_si256(a, 1);

            // 2. Shift: Apply 128-bit byte shift to each half
            // These will be compiled as VEX-encoded VPSLLDQ XMM instructions
            let res_lo = _mm_slli_si128(lo, SHIFT);
            let res_hi = _mm_slli_si128(hi, SHIFT);

            // 3. Merge: Combine back into a 256-bit register
            _mm256_set_m128i(res_hi, res_lo)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_extracti128_si256<const HIGH: i32>(a: wprs__m256i) -> wprs__m128i {
            _mm256_extractf128_si256(a, HIGH)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_blend_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Cast integer vectors to floating-point vectors (bit-preserving)
            let a_f = _mm256_castsi256_ps(a);
            let b_f = _mm256_castsi256_ps(b);

            // 2. Perform the blend using the AVX1 floating-point intrinsic.
            // We use a match to pipe the const generic MASK into the literal slot.
            let res_f = _mm256_blend_ps(a_f, b_f, MASK);

            // 3. Cast back to integer vector (bit-preserving)
            _mm256_castps_si256(res_f)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_extract_epi8<const INDEX: i32>(a: wprs__m256i) -> i32 {
            let v = if INDEX < 16 {
                // Extract from low 128-bit lane (XMM)
                _mm256_castsi256_si128(a)
            } else {
                // Extract high 128-bit lane, then extract byte
                _mm256_extractf128_si256::<1>(a)
            };

            // This is the same with the plain SSE4.1 but we want the VEX encoded variant
            // This should work with the two lines below
            // _mm_extract_epi8(low, INDEX)
            // _mm_extract_epi8(high, INDEX - 16)
            // TODO: revisit this when generic_const_exprs graduates from nightly
            match INDEX {
                // Lower Lane (0-15)
                0  => _mm_extract_epi8(v, 0),
                1  => _mm_extract_epi8(v, 1),
                2  => _mm_extract_epi8(v, 2),
                3  => _mm_extract_epi8(v, 3),
                4  => _mm_extract_epi8(v, 4),
                5  => _mm_extract_epi8(v, 5),
                6  => _mm_extract_epi8(v, 6),
                7  => _mm_extract_epi8(v, 7),
                8  => _mm_extract_epi8(v, 8),
                9  => _mm_extract_epi8(v, 9),
                10 => _mm_extract_epi8(v, 10),
                11 => _mm_extract_epi8(v, 11),
                12 => _mm_extract_epi8(v, 12),
                13 => _mm_extract_epi8(v, 13),
                14 => _mm_extract_epi8(v, 14),
                15 => _mm_extract_epi8(v, 15),
                // Upper Lane (16-31)
                16 => _mm_extract_epi8(v, 0),
                17 => _mm_extract_epi8(v, 1),
                18 => _mm_extract_epi8(v, 2),
                19 => _mm_extract_epi8(v, 3),
                20 => _mm_extract_epi8(v, 4),
                21 => _mm_extract_epi8(v, 5),
                22 => _mm_extract_epi8(v, 6),
                23 => _mm_extract_epi8(v, 7),
                24 => _mm_extract_epi8(v, 8),
                25 => _mm_extract_epi8(v, 9),
                26 => _mm_extract_epi8(v, 10),
                27 => _mm_extract_epi8(v, 11),
                28 => _mm_extract_epi8(v, 12),
                29 => _mm_extract_epi8(v, 13),
                30 => _mm_extract_epi8(v, 14),
                31 => _mm_extract_epi8(v, 15),
                _ => panic!("Index out of bounds for 256-bit extract"),
            }
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_inserti128_si256<const LANE: i32>(a: wprs__m256i, b: wprs__m128i) -> wprs__m256i {
            // Cast to __m256 (float), insert, cast back
            let a_f = _mm256_castsi256_ps(a);
            let b_f = _mm_castsi128_ps(b);
            let res_f = _mm256_insertf128_ps(a_f, b_f, LANE);
            _mm256_castps_si256(res_f)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_set_epi8(
            e31: i8, e30: i8, e29: i8, e28: i8, e27: i8, e26: i8, e25: i8, e24: i8,
            e23: i8, e22: i8, e21: i8, e20: i8, e19: i8, e18: i8, e17: i8, e16: i8,
            e15: i8, e14: i8, e13: i8, e12: i8, e11: i8, e10: i8, e9: i8, e8: i8,
            e7: i8, e6: i8, e5: i8, e4: i8, e3: i8, e2: i8, e1: i8, e0: i8) -> wprs__m256i {

            let low = _mm_set_epi8(e15, e14, e13, e12, e11, e10, e9, e8, e7, e6, e5, e4, e3, e2, e1, e0);
            let high = _mm_set_epi8(e31, e30, e29, e28, e27, e26, e25, e24, e23, e22, e21, e20, e19, e18, e17, e16);

            let res = _mm256_castsi128_si256(low);
            wprs_mm256_inserti128_si256::<1>(res, high)
        }

        #[target_feature(enable = "avx")]
        #[inline]
        fn wprs_mm256_shuffle_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Extract halves of data and mask
            let a_low = _mm256_castsi256_si128(a);
            let a_high = _mm256_extractf128_si256::<1>(a);
            let b_low = _mm256_castsi256_si128(b);
            let b_high = _mm256_extractf128_si256::<1>(b);

            // 2. Perform SSSE3 shuffle on each 128-bit lane
            let res_low = _mm_shuffle_epi8(a_low, b_low);
            let res_high = _mm_shuffle_epi8(a_high, b_high);

            // 3. Combine back into 256-bit
            let res = _mm256_castsi128_si256(res_low);
            wprs_mm256_inserti128_si256::<1>(res, res_high)
        }
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_sub_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // We use _mm_sub_epi8 (SSE2) twice.
            wprs__m256i {
                low: _mm_sub_epi8(a.low, b.low),
                high: _mm_sub_epi8(a.high, b.high)
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_add_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // We use _mm_add_epi8 (SSE2) twice.
            wprs__m256i {
                low: _mm_add_epi8(a.low, b.low),
                high: _mm_add_epi8(a.high, b.high)
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_slli_si256<const SHIFT: i32>(a: wprs__m256i) -> wprs__m256i {
            // SAFETY: _mm_slli_si128 is an SSE2 intrinsic.
            // It shifts the 128-bit register left by SHIFT bytes.
            // Bits do not carry across the 128-bit boundary, perfectly
            // matching the behavior of the AVX2 256-bit version.
            wprs__m256i {
                low: _mm_slli_si128(a.low, SHIFT),
                high: _mm_slli_si128(a.high, SHIFT)
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_extracti128_si256<const HIGH: i32>(a: wprs__m256i) -> wprs__m128i {
            // Because HIGH must be a compile-time constant,
            // the compiler will optimize this branch away entirely.
            if HIGH == 0 {
                a.low
            } else {
                a.high
            }
        }

        cfg_if! {
            if #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))] {
                #[target_feature(enable = "sse4.1")]
                #[inline]
                fn wprs_mm_blend_epi32<const MASK: i32>(a: wprs__m128i, b: wprs__m128i) -> wprs__m128i {
                    // If target has SSE4.1, use the specialized blend instruction
                    unsafe {_mm_blend_epi32(a, b, MASK)}
                }
            } else {
                #[target_feature(enable = "sse2")]
                #[inline]
                fn wprs_mm_blend_epi32<const MASK: i32>(a: wprs__m128i, b: wprs__m128i) -> wprs__m128i {
                    // Fallback for SSE2, SSE3, SSSE3 (Generic bitwise blend)
                    // This is a bitwise selection: (b & mask) | (a & ~mask)
                    // We create a 128-bit mask based on the 4-bit M constant
                    let mask = _mm_set_epi32(
                        if (MASK & 8) != 0 { -1 } else { 0 },
                        if (MASK & 4) != 0 { -1 } else { 0 },
                        if (MASK & 2) != 0 { -1 } else { 0 },
                        if (MASK & 1) != 0 { -1 } else { 0 },
                    );
                    _mm_or_si128(_mm_and_si128(mask, b), _mm_andnot_si128(mask, a))
                }
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_blend_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // We only care about the lower 4 bits (0-15)
            // TODO: revisit this when generic_const_exprs graduates from nightly
            let low = match MASK & 0xF {
                0  => wprs_mm_blend_epi32::< 0>(a.low, b.low),
                1  => wprs_mm_blend_epi32::< 1>(a.low, b.low),
                2  => wprs_mm_blend_epi32::< 2>(a.low, b.low),
                3  => wprs_mm_blend_epi32::< 3>(a.low, b.low),
                4  => wprs_mm_blend_epi32::< 4>(a.low, b.low),
                5  => wprs_mm_blend_epi32::< 5>(a.low, b.low),
                6  => wprs_mm_blend_epi32::< 6>(a.low, b.low),
                7  => wprs_mm_blend_epi32::< 7>(a.low, b.low),
                8  => wprs_mm_blend_epi32::< 8>(a.low, b.low),
                9  => wprs_mm_blend_epi32::< 9>(a.low, b.low),
                10 => wprs_mm_blend_epi32::<10>(a.low, b.low),
                11 => wprs_mm_blend_epi32::<11>(a.low, b.low),
                12 => wprs_mm_blend_epi32::<12>(a.low, b.low),
                13 => wprs_mm_blend_epi32::<13>(a.low, b.low),
                14 => wprs_mm_blend_epi32::<14>(a.low, b.low),
                15 => wprs_mm_blend_epi32::<15>(a.low, b.low),
                _ => unreachable!(),
            };

            // We only care about the lower 4 bits (0-15)
            let high = match (MASK >> 4) & 0xF {
                0  => wprs_mm_blend_epi32::< 0>(a.high, b.high),
                1  => wprs_mm_blend_epi32::< 1>(a.high, b.high),
                2  => wprs_mm_blend_epi32::< 2>(a.high, b.high),
                3  => wprs_mm_blend_epi32::< 3>(a.high, b.high),
                4  => wprs_mm_blend_epi32::< 4>(a.high, b.high),
                5  => wprs_mm_blend_epi32::< 5>(a.high, b.high),
                6  => wprs_mm_blend_epi32::< 6>(a.high, b.high),
                7  => wprs_mm_blend_epi32::< 7>(a.high, b.high),
                8  => wprs_mm_blend_epi32::< 8>(a.high, b.high),
                9  => wprs_mm_blend_epi32::< 9>(a.high, b.high),
                10 => wprs_mm_blend_epi32::<10>(a.high, b.high),
                11 => wprs_mm_blend_epi32::<11>(a.high, b.high),
                12 => wprs_mm_blend_epi32::<12>(a.high, b.high),
                13 => wprs_mm_blend_epi32::<13>(a.high, b.high),
                14 => wprs_mm_blend_epi32::<14>(a.high, b.high),
                15 => wprs_mm_blend_epi32::<15>(a.high, b.high),
                _ => unreachable!(),
            };

            wprs__m256i {
                low: low,
                high: high,
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_extract_epi8<const INDEX: i32>(a: wprs__m256i) -> i32 {
            // There are 32 bytes in a 256-bit register (0-31).
            // Indices 0-15 are in the 'low' 128-bit lane.
            // Indices 16-31 are in the 'high' 128-bit lane.
            // TODO: revisit this when generic_const_exprs graduates from nightly
            match INDEX {
                // Lower Lane (0-15)
                0  => wprs_mm_extract_epi8::< 0>(a.low),
                1  => wprs_mm_extract_epi8::< 1>(a.low),
                2  => wprs_mm_extract_epi8::< 2>(a.low),
                3  => wprs_mm_extract_epi8::< 3>(a.low),
                4  => wprs_mm_extract_epi8::< 4>(a.low),
                5  => wprs_mm_extract_epi8::< 5>(a.low),
                6  => wprs_mm_extract_epi8::< 6>(a.low),
                7  => wprs_mm_extract_epi8::< 7>(a.low),
                8  => wprs_mm_extract_epi8::< 8>(a.low),
                9  => wprs_mm_extract_epi8::< 9>(a.low),
                10 => wprs_mm_extract_epi8::<10>(a.low),
                11 => wprs_mm_extract_epi8::<11>(a.low),
                12 => wprs_mm_extract_epi8::<12>(a.low),
                13 => wprs_mm_extract_epi8::<13>(a.low),
                14 => wprs_mm_extract_epi8::<14>(a.low),
                15 => wprs_mm_extract_epi8::<15>(a.low),
                // Upper Lane (16-31)
                16 => wprs_mm_extract_epi8::< 0>(a.high),
                17 => wprs_mm_extract_epi8::< 1>(a.high),
                18 => wprs_mm_extract_epi8::< 2>(a.high),
                19 => wprs_mm_extract_epi8::< 3>(a.high),
                20 => wprs_mm_extract_epi8::< 4>(a.high),
                21 => wprs_mm_extract_epi8::< 5>(a.high),
                22 => wprs_mm_extract_epi8::< 6>(a.high),
                23 => wprs_mm_extract_epi8::< 7>(a.high),
                24 => wprs_mm_extract_epi8::< 8>(a.high),
                25 => wprs_mm_extract_epi8::< 9>(a.high),
                26 => wprs_mm_extract_epi8::<10>(a.high),
                27 => wprs_mm_extract_epi8::<11>(a.high),
                28 => wprs_mm_extract_epi8::<12>(a.high),
                29 => wprs_mm_extract_epi8::<13>(a.high),
                30 => wprs_mm_extract_epi8::<14>(a.high),
                31 => wprs_mm_extract_epi8::<15>(a.high),
                _ => panic!("Index out of bounds for 256-bit extract"),
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_inserti128_si256<const LANE: i32>(a: wprs__m256i, b: wprs__m128i) -> wprs__m256i {
            // In SIMD, Lane 0 is the lower 128 bits, Lane 1 is the upper 128 bits.
            if LANE == 0 {
                wprs__m256i {
                    low: b,        // Replace low with new 128-bit value
                    high: a.high,  // Keep existing high
                }
            } else {
                wprs__m256i {
                    low: a.low,    // Keep existing low
                    high: b,       // Replace high with new 128-bit value
                }
            }
        }

        #[target_feature(enable = "sse2")]
        #[inline]
        fn wprs_mm256_set_epi8(
            e31: i8, e30: i8, e29: i8, e28: i8, e27: i8, e26: i8, e25: i8, e24: i8,
            e23: i8, e22: i8, e21: i8, e20: i8, e19: i8, e18: i8, e17: i8, e16: i8,
            e15: i8, e14: i8, e13: i8, e12: i8, e11: i8, e10: i8, e9: i8, e8: i8,
            e7: i8, e6: i8, e5: i8, e4: i8, e3: i8, e2: i8, e1: i8, e0: i8,) -> wprs__m256i {
            // Construct the low 128-bit part (e0 through e15)
            let low = _mm_set_epi8(e15, e14, e13, e12, e11, e10, e9, e8, e7, e6, e5, e4, e3, e2, e1, e0);
            // Construct the high 128-bit part (e16 through e31)
            let high = _mm_set_epi8(e31, e30, e29, e28, e27, e26, e25, e24, e23, e22, e21, e20, e19, e18, e17, e16);

            wprs__m256i { low, high }
        }

        cfg_if! {
            if #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))] {
                #[target_feature(enable = "ssse3")]
                #[inline]
                fn wprs_mm256_shuffle_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
                    // SSSE3 _mm_shuffle_epi8 operates on 128-bit registers.
                    // We shuffle the 'low' part of 'a' using the 'low' part of 'b'.
                    let low = _mm_shuffle_epi8(a.low, b.low);

                    // We shuffle the 'high' part of 'a' using the 'high' part of 'b'.
                    let high = _mm_shuffle_epi8(a.high, b.high);

                    wprs__m256i { low, high }
                }
            } else {
                /// Emulates SSSE3 _mm_shuffle_epi8 using only SSE2 instructions
                #[inline]
                #[target_feature(enable = "sse2")]
                fn wprs_mm_shuffle_epi8(a: wprs__m128i, b: wprs__m128i) -> wprs__m128i {
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
                fn wprs_mm256_shuffle_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
                    // We must shuffle 'low' and 'high' independently to match AVX2 behavior.
                    wprs__m256i {
                        low: wprs_mm_shuffle_epi8(a.low, b.low),
                        high: wprs_mm_shuffle_epi8(a.high, b.high),
                    }
                }
            }
        }
    } else {
        #[inline]
        fn wprs_mm256_sub_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Treat the 32-byte structures as arrays of 32 bytes (u8).
            // This is safe because both are 256 bits (32 bytes) in total size.
            let a_bytes: [u8; 32] = unsafe { std::mem::transmute(a.0) };
            let b_bytes: [u8; 32] = unsafe { std::mem::transmute(b.0) };
            let mut res_bytes = [0u8; 32];

            // 2. Perform lane-wise subtraction.
            // We use wrapping_sub to emulate the hardware behavior where
            // 0 - 1 = 255.
            for i in 0..32 {
                res_bytes[i] = a_bytes[i].wrapping_sub(b_bytes[i]);
            }

            // 3. Convert the resulting bytes back into the i32-based struct.
            wprs__m256i(unsafe { std::mem::transmute(res_bytes) })
        }

        #[inline]
        fn wprs_mm256_add_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // 1. Treat the 32-byte structures as arrays of 32 bytes (u8).
            // This is safe because both are 256 bits (32 bytes) in total size.
            let a_bytes: [u8; 32] = unsafe { std::mem::transmute(a.0) };
            let b_bytes: [u8; 32] = unsafe { std::mem::transmute(b.0) };
            let mut res_bytes = [0u8; 32];

            // 2. Perform lane-wise subtraction.
            // We use wrapping_sub to emulate the hardware behavior where
            // 0 - 1 = 255.
            for i in 0..32 {
                res_bytes[i] = a_bytes[i].wrapping_add(b_bytes[i]);
            }

            // 3. Convert the resulting bytes back into the i32-based struct.
            wprs__m256i(unsafe { std::mem::transmute(res_bytes) })
        }

        #[inline]
        fn wprs_mm256_slli_si256<const SHIFT: i32>(a: wprs__m256i) -> wprs__m256i {
            // If shift is 16 or more, the result for a 128-bit lane is always zero.
            if SHIFT >= 16 {
                return wprs__m256i([0; 8]);
            }
            if SHIFT <= 0 {
                return a;
            }

            // We treat the 256-bit struct as two 128-bit (16-byte) lanes.
            // Lane 0: indices 0..4 (i32) or 0..16 (u8)
            // Lane 1: indices 4..8 (i32) or 16..32 (u8)
            let a_bytes: [u8; 32] = unsafe { std::mem::transmute(a.0) };
            let mut res_bytes = [0u8; 32];
            let s = SHIFT as usize;

            for i in 0..16 {
                // Shift within the first 128-bit lane
                if i + s < 16 {
                    res_bytes[i + s] = a_bytes[i];
                }
                // Shift within the second 128-bit lane
                if i + s < 16 {
                    res_bytes[i + s + 16] = a_bytes[i + 16];
                }
            }

            wprs__m256i(unsafe { std::mem::transmute(res_bytes) })
        }

        #[inline]
        fn wprs_mm256_extracti128_si256<const HIGH: i32>(a: wprs__m256i) -> wprs__m128i {
            // Since HIGH is a const, the compiler evaluates this branch at compile time.
            if HIGH == 0 {
                // Extract low 128 bits: indices 0, 1, 2, 3
                wprs__m128i([a.0[0], a.0[1], a.0[2], a.0[3]])
            } else {
                // Extract high 128 bits: indices 4, 5, 6, 7
                wprs__m128i([a.0[4], a.0[5], a.0[6], a.0[7]])
            }
        }

        #[inline]
        fn wprs_mm256_blend_epi32<const MASK: i32>(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            let mut result = [0i32; 8];

            // Each bit in the 8-bit MASK corresponds to one i32 lane.
            // 0 = take from 'a', 1 = take from 'b'.
            result[0] = if (MASK & (1 << 0)) != 0 { b.0[0] } else { a.0[0] };
            result[1] = if (MASK & (1 << 1)) != 0 { b.0[1] } else { a.0[1] };
            result[2] = if (MASK & (1 << 2)) != 0 { b.0[2] } else { a.0[2] };
            result[3] = if (MASK & (1 << 3)) != 0 { b.0[3] } else { a.0[3] };
            result[4] = if (MASK & (1 << 4)) != 0 { b.0[4] } else { a.0[4] };
            result[5] = if (MASK & (1 << 5)) != 0 { b.0[5] } else { a.0[5] };
            result[6] = if (MASK & (1 << 6)) != 0 { b.0[6] } else { a.0[6] };
            result[7] = if (MASK & (1 << 7)) != 0 { b.0[7] } else { a.0[7] };

            wprs__m256i(result)
        }

        #[inline]
        fn wprs_mm256_extract_epi8_alt<const INDEX: i32>(a: wprs__m256i) -> i32 {
            let bytes: [u8; 32] = unsafe { std::mem::transmute(a.0) };
            bytes[INDEX as usize] as i32
        }

        #[inline]
        fn wprs_mm256_inserti128_si256<const LANE: i32>(a: wprs__m256i, b: wprs__m128i) -> wprs__m256i {
            let mut res = a.0; // Start with a copy of the original 256-bit array

            if LANE == 0 {
                // Replace the lower 128 bits (indices 0, 1, 2, 3)
                res[0] = b.0[0];
                res[1] = b.0[1];
                res[2] = b.0[2];
                res[3] = b.0[3];
            } else {
                // Replace the upper 128 bits (indices 4, 5, 6, 7)
                res[4] = b.0[0];
                res[5] = b.0[1];
                res[6] = b.0[2];
                res[7] = b.0[3];
            }

            wprs__m256i(res)
        }

        #[inline]
        fn wprs_mm256_set_epi8(
            e31: i8, e30: i8, e29: i8, e28: i8, e27: i8, e26: i8, e25: i8, e24: i8,
            e23: i8, e22: i8, e21: i8, e20: i8, e19: i8, e18: i8, e17: i8, e16: i8,
            e15: i8, e14: i8, e13: i8, e12: i8, e11: i8, e10: i8, e9: i8, e8: i8,
            e7: i8, e6: i8, e5: i8, e4: i8, e3: i8, e2: i8, e1: i8, e0: i8,
        ) -> wprs__m256i {
            // Helper function to pack 4 bytes into one i32 in little-endian order
            // e.g., pack(e3, e2, e1, e0) puts e0 at bits 0-7
            #[inline(always)]
            fn pack(b3: i8, b2: i8, b1: i8, b0: i8) -> i32 {
                ((b3 as u32) << 24 | (b2 as u32) << 16 | (b1 as u32) << 8 | (b0 as u32)) as i32
            }

            wprs__m256i([
                pack(e3,  e2,  e1,  e0),
                pack(e7,  e6,  e5,  e4),
                pack(e11, e10, e9,  e8),
                pack(e15, e14, e13, e12),
                pack(e19, e18, e17, e16),
                pack(e23, e22, e21, e20),
                pack(e27, e26, e25, e24),
                pack(e31, e30, e29, e28),
            ])
        }

        #[inline]
        pub fn wprs_mm256_shuffle_epi8(a: wprs__m256i, b: wprs__m256i) -> wprs__m256i {
            // Treat the [i32; 8] as [u8; 32] via transmute
            let a_bytes: [u8; 32] = unsafe { std::mem::transmute(a.0) };
            let b_bytes: [u8; 32] = unsafe { std::mem::transmute(b.0) };
            let mut res_bytes = [0u8; 32];

            for i in 0..32 {
                let mask_byte = b_bytes[i];

                // If bit 7 is set, the result byte is 0
                if (mask_byte & 0x80) != 0 {
                    res_bytes[i] = 0;
                } else {
                    // "In-lane" behavior:
                    // Bytes 0-15 use index from low 128-bit lane
                    // Bytes 16-31 use index from high 128-bit lane
                    let lane_offset = (i / 16) * 16;
                    let index_within_lane = (mask_byte & 0x0F) as usize;

                    res_bytes[i] = a_bytes[lane_offset + index_within_lane];
                }
            }

            wprs__m256i(unsafe { std::mem::transmute(res_bytes) })
        }
    }
}

#[inline]
fn subtract_green(b: wprs__m256i, g: wprs__m256i, r: wprs__m256i) -> (wprs__m256i, wprs__m256i) {
    unsafe { (wprs_mm256_sub_epi8(b, g), wprs_mm256_sub_epi8(r, g)) }
}

#[inline]
fn add_green(b: wprs__m256i, g: wprs__m256i, r: wprs__m256i) -> (wprs__m256i, wprs__m256i) {
    unsafe { (wprs_mm256_add_epi8(b, g), wprs_mm256_add_epi8(r, g)) }
}

#[inline]
fn prefix_sum_32(mut block: wprs__m256i) -> wprs__m256i {
    unsafe {
        block = wprs_mm256_add_epi8(block, wprs_mm256_slli_si256::<1>(block));
        block = wprs_mm256_add_epi8(block, wprs_mm256_slli_si256::<2>(block));
        block = wprs_mm256_add_epi8(block, wprs_mm256_slli_si256::<4>(block));
        block = wprs_mm256_add_epi8(block, wprs_mm256_slli_si256::<8>(block));
    }
    block
}

#[inline]
fn accumulate_sum_16(
    mut block: wprs__m128i,
    prev_block: wprs__m128i,
) -> (wprs__m128i, wprs__m128i) {
    unsafe {
        let cur_sum = wprs_mm_set1_epi8(wprs_mm_extract_epi8::<15>(block) as i8);
        block = wprs_mm_add_epi8(prev_block, block);
        (block, wprs_mm_add_epi8(prev_block, cur_sum))
    }
}

#[inline]
fn accumulate_sum_32(block: wprs__m256i, prev_block: wprs__m128i) -> (wprs__m256i, wprs__m128i) {
    unsafe {
        let (block0, prev_block) =
            accumulate_sum_16(wprs_mm256_extracti128_si256::<0>(block), prev_block);
        let (block1, prev_block) =
            accumulate_sum_16(wprs_mm256_extracti128_si256::<1>(block), prev_block);
        (wprs_mm256_set_m128i(block1, block0), prev_block)
    }
}

#[inline]
fn prefix_sum(block: wprs__m256i, prev_block: wprs__m128i) -> (wprs__m256i, wprs__m128i) {
    accumulate_sum_32(prefix_sum_32(block), prev_block)
}

#[inline]
fn running_difference_32(mut block: wprs__m256i, prev: u8) -> (wprs__m256i, u8) {
    unsafe {
        let prev = wprs_mm256_set_epi8(
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, prev as i8,
        );
        let block15_16 = wprs_mm256_set_epi8(
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
            wprs_mm256_extract_epi8::<15>(block) as i8,
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
        let next = wprs_mm256_extract_epi8::<31>(block) as u8;

        block = wprs_mm256_sub_epi8(block, wprs_mm256_slli_si256::<1>(block));
        block = wprs_mm256_sub_epi8(block, block15_16);
        block = wprs_mm256_sub_epi8(block, prev);

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
    unsafe {
        let p0: wprs__m256i = wprs_mm256_set_epi8(
            15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13,
            9, 5, 1, 12, 8, 4, 0,
        );
        let p1: wprs__m256i = wprs_mm256_set_epi8(
            14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8,
            4, 0, 15, 11, 7, 3,
        );
        let p2: wprs__m256i = wprs_mm256_set_epi8(
            13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11,
            7, 3, 14, 10, 6, 2,
        );
        let p3: wprs__m256i = wprs_mm256_set_epi8(
            12, 8, 4, 0, 15, 11, 7, 3, 14, 10, 6, 2, 13, 9, 5, 1, 12, 8, 4, 0, 15, 11, 7, 3, 14,
            10, 6, 2, 13, 9, 5, 1,
        );

        let [i0, i1, i2, i3, i4, i5, i6, i7] = input.as_chunks::<4, 8>();

        // let input: *const u8 = input.ptr().cast();
        // print!("i0  ");
        // crate::utils::print_vec_char_256_hex(wprs_mm256_loadu_si256_mem(&*input.offset(0).cast::<[u8; 32]>()));
        // print!("i1  ");
        // crate::utils::print_vec_char_256_hex(wprs_mm256_loadu_si256_mem(&*input.offset(32).cast::<[u8; 32]>()));
        // print!("i2  ");
        // crate::utils::print_vec_char_256_hex(wprs_mm256_loadu_si256_mem(&*input.offset(64).cast::<[u8; 32]>()));
        // print!("i3  ");
        // crate::utils::print_vec_char_256_hex(wprs_mm256_loadu_si256_mem(&*input.offset(96).cast::<[u8; 32]>()));
        // print!("\n");

        // i0  1f 1e 1d 1c | 1b 1a 19 18 | 17 16 15 14 | 13 12 11 10 || 0f 0e 0d 0c | 0b 0a 09 08 | 07 06 05 04 | 03 02 01 00
        // i1  3f 3e 3d 3c | 3b 3a 39 38 | 37 36 35 34 | 33 32 31 30 || 2f 2e 2d 2c | 2b 2a 29 28 | 27 26 25 24 | 23 22 21 20
        // i2  5f 5e 5d 5c | 5b 5a 59 58 | 57 56 55 54 | 53 52 51 50 || 4f 4e 4d 4c | 4b 4a 49 48 | 47 46 45 44 | 43 42 41 40
        // i3  7f 7e 7d 7c | 7b 7a 79 78 | 77 76 75 74 | 73 72 71 70 || 6f 6e 6d 6c | 6b 6a 69 68 | 67 66 65 64 | 63 62 61 60

        let mut t0: wprs__m256i = wprs_mm256_castsi128_si256(wprs_mm_loadu_si128_vec4u8(&i0));
        let mut t1: wprs__m256i = wprs_mm256_castsi128_si256(wprs_mm_loadu_si128_vec4u8(&i1));
        let mut t2: wprs__m256i = wprs_mm256_castsi128_si256(wprs_mm_loadu_si128_vec4u8(&i2));
        let mut t3: wprs__m256i = wprs_mm256_castsi128_si256(wprs_mm_loadu_si128_vec4u8(&i3));

        t0 = wprs_mm256_inserti128_si256::<1>(t0, wprs_mm_loadu_si128_vec4u8(&i4));
        t1 = wprs_mm256_inserti128_si256::<1>(t1, wprs_mm_loadu_si128_vec4u8(&i5));
        t2 = wprs_mm256_inserti128_si256::<1>(t2, wprs_mm_loadu_si128_vec4u8(&i6));
        t3 = wprs_mm256_inserti128_si256::<1>(t3, wprs_mm_loadu_si128_vec4u8(&i7));

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

        t0 = wprs_mm256_shuffle_epi8(t0, p0);
        t1 = wprs_mm256_shuffle_epi8(t1, p1);
        t2 = wprs_mm256_shuffle_epi8(t2, p2);
        t3 = wprs_mm256_shuffle_epi8(t3, p3);

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

        let u0: wprs__m256i = wprs_mm256_blend_epi32::<0b10101010>(t0, t1);
        let u1: wprs__m256i = wprs_mm256_blend_epi32::<0b10101010>(t2, t3);
        let u2: wprs__m256i = wprs_mm256_blend_epi32::<0b01010101>(t0, t1);
        let u3: wprs__m256i = wprs_mm256_blend_epi32::<0b01010101>(t2, t3);

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

        t0 = wprs_mm256_blend_epi32::<0b11001100>(u0, u1);
        t1 = wprs_mm256_shufps_epi32::<0b00111001>(u2, u3);
        t2 = wprs_mm256_shufps_epi32::<0b01001110>(u0, u1);
        t3 = wprs_mm256_shufps_epi32::<0b10010011>(u2, u3);

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

        (t0, t2) = subtract_green(t0, t1, t2);

        #[allow(unused_assignments)]
        let (mut next0, mut next1, mut next2, mut next3) = (0, 0, 0, 0);
        (t0, next0) = running_difference_32(t0, prev0);
        (t1, next1) = running_difference_32(t1, prev1);
        (t2, next2) = running_difference_32(t2, prev2);
        (t3, next3) = running_difference_32(t3, prev3);

        wprs_mm256_storeu_si256_mem(out0, t0);
        wprs_mm256_storeu_si256_mem(out1, t1);
        wprs_mm256_storeu_si256_mem(out2, t2);
        wprs_mm256_storeu_si256_mem(out3, t3);

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
    mut prev0: wprs__m128i,
    mut prev1: wprs__m128i,
    mut prev2: wprs__m128i,
    mut prev3: wprs__m128i,
) -> (wprs__m128i, wprs__m128i, wprs__m128i, wprs__m128i) {
    unsafe {
        let p0 = wprs_mm256_set_epi8(
            7, 11, 15, 3, 6, 10, 14, 2, 5, 9, 13, 1, 4, 8, 12, 0, 7, 11, 15, 3, 6, 10, 14, 2, 5, 9,
            13, 1, 4, 8, 12, 0,
        );
        let p1 = wprs_mm256_set_epi8(
            3, 15, 11, 7, 2, 14, 10, 6, 1, 13, 9, 5, 0, 12, 8, 4, 3, 15, 11, 7, 2, 14, 10, 6, 1,
            13, 9, 5, 0, 12, 8, 4,
        );
        let p2 = wprs_mm256_set_epi8(
            15, 3, 7, 11, 14, 2, 6, 10, 13, 1, 5, 9, 12, 0, 4, 8, 15, 3, 7, 11, 14, 2, 6, 10, 13,
            1, 5, 9, 12, 0, 4, 8,
        );
        let p3 = wprs_mm256_set_epi8(
            11, 7, 3, 15, 10, 6, 2, 14, 9, 5, 1, 13, 8, 4, 0, 12, 11, 7, 3, 15, 10, 6, 2, 14, 9, 5,
            1, 13, 8, 4, 0, 12,
        );

        let mut t0 = wprs_mm256_loadu_si256_mem(input0);
        let mut t1 = wprs_mm256_loadu_si256_mem(input1);
        let mut t2 = wprs_mm256_loadu_si256_mem(input2);
        let mut t3 = wprs_mm256_loadu_si256_mem(input3);

        (t0, prev0) = prefix_sum(t0, prev0);
        (t1, prev1) = prefix_sum(t1, prev1);
        (t2, prev2) = prefix_sum(t2, prev2);
        (t3, prev3) = prefix_sum(t3, prev3);

        (t0, t2) = add_green(t0, t1, t2);

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

        let u0 = wprs_mm256_shufps_epi32::<0b01000100>(t0, t2);
        let u1 = wprs_mm256_shufps_epi32::<0b11101110>(t2, t0);
        let u2 = wprs_mm256_shufps_epi32::<0b00010001>(t3, t1);
        let u3 = wprs_mm256_shufps_epi32::<0b10111011>(t1, t3);

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

        t0 = wprs_mm256_blend_epi32::<0b01010101>(u2, u0);
        t1 = wprs_mm256_blend_epi32::<0b10101010>(u2, u0);
        t2 = wprs_mm256_blend_epi32::<0b01010101>(u3, u1);
        t3 = wprs_mm256_blend_epi32::<0b10101010>(u3, u1);

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

        t0 = wprs_mm256_shuffle_epi8(t0, p0);
        t1 = wprs_mm256_shuffle_epi8(t1, p1);
        t2 = wprs_mm256_shuffle_epi8(t2, p2);
        t3 = wprs_mm256_shuffle_epi8(t3, p3);

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

        wprs_mm256_storeu_si256_vec4u8(
            out.index_mut(0..8).try_into().unwrap(),
            wprs_mm256_set_m128i(
                wprs_mm256_castsi256_si128(t1),
                wprs_mm256_castsi256_si128(t0),
            ),
        );
        wprs_mm256_storeu_si256_vec4u8(
            out.index_mut(8..16).try_into().unwrap(),
            wprs_mm256_set_m128i(
                wprs_mm256_castsi256_si128(t3),
                wprs_mm256_castsi256_si128(t2),
            ),
        );

        wprs_mm256_storeu_si256_vec4u8(
            out.index_mut(16..24).try_into().unwrap(),
            wprs_mm256_set_m128i(
                wprs_mm256_extracti128_si256::<1>(t1),
                wprs_mm256_extracti128_si256::<1>(t0),
            ),
        );
        wprs_mm256_storeu_si256_vec4u8(
            out.index_mut(24..32).try_into().unwrap(),
            wprs_mm256_set_m128i(
                wprs_mm256_extracti128_si256::<1>(t3),
                wprs_mm256_extracti128_si256::<1>(t2),
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

    unsafe {
        let z: wprs__m128i = wprs_mm_setzero_si128();
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
            wprs_mm256_storeu_si256_mem(
                (&mut output[..]).try_into().unwrap(),
                prefix_sum(
                    wprs_mm256_loadu_si256_mem((&input[..]).try_into().unwrap()),
                    wprs_mm_setzero_si128(),
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
        unsafe {
            wprs_mm256_storeu_si256_mem(
                (&mut output[..]).try_into().unwrap(),
                running_difference_32(
                    wprs_mm256_loadu_si256_mem((&input[..]).try_into().unwrap()),
                    0,
                )
                .0,
            );
        }
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
