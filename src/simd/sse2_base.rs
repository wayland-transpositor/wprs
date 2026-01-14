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

pub use std::arch::x86_64::__m128;
pub use std::arch::x86_64::__m128i;
pub use std::arch::x86_64::_mm_add_epi8;
use std::arch::x86_64::_mm_and_si128;
use std::arch::x86_64::_mm_andnot_si128;
use std::arch::x86_64::_mm_castps_si128;
use std::arch::x86_64::_mm_castsi128_ps;
pub use std::arch::x86_64::_mm_loadu_si128;
use std::arch::x86_64::_mm_or_si128;
use std::arch::x86_64::_mm_set_epi8;
use std::arch::x86_64::_mm_set_epi32;
pub use std::arch::x86_64::_mm_set1_epi8;
pub use std::arch::x86_64::_mm_setzero_si128;
use std::arch::x86_64::_mm_shuffle_ps;
use std::arch::x86_64::_mm_slli_si128;
pub use std::arch::x86_64::_mm_storeu_si128;
use std::arch::x86_64::_mm_sub_epi8;

#[allow(non_camel_case_types)]
#[repr(C, align(32))]
#[derive(Copy, Clone)]
pub struct __m256 {
    pub low: __m128,
    pub high: __m128,
}

#[allow(non_camel_case_types)]
#[repr(C, align(32))]
#[derive(Copy, Clone)]
pub struct __m256i {
    pub low: __m128i,
    pub high: __m128i,
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_set_m128i(high: __m128i, low: __m128i) -> __m256i {
    // In SSE2, we simply wrap the two 128-bit values
    // into our custom 256-bit emulation struct.
    __m256i { low, high }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_castsi128_si256(a: __m128i) -> __m256i {
    // In SSE2, we wrap the 128-bit value into our 256-bit struct.
    // We set the high bits to zero to represent the 'undefined' state safely.
    __m256i {
        low: a,
        high: _mm_setzero_si128(),
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
pub unsafe fn _mm256_loadu_si256(src: *const __m256i) -> __m256i {
    // SAFETY: src is pointer to __m256i, so it is safe to read
    // 256 bits from it in two rounds of 128bit each.
    unsafe {
        __m256i {
            low: _mm_loadu_si128(&(*src).low),
            high: _mm_loadu_si128(&(*src).high),
        }
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub unsafe fn _mm256_storeu_si256(dst: *mut __m256i, a: __m256i) {
    // SAFETY: dst is pointer to __m256i, so it is safe to read
    // 256 bits from it in two rounds of 128bit each.
    unsafe {
        _mm_storeu_si128(&mut (*dst).low, a.low);
        _mm_storeu_si128(&mut (*dst).high, a.high);
    }
}

#[target_feature(enable = "sse2")]
#[inline]
fn _mm_blend_epi32<const MASK: i32>(a: __m128i, b: __m128i) -> __m128i {
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

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_blend_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    // We only care about the lower 4 bits (0-15)
    // TODO: revisit this when generic_const_exprs graduates from nightly
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

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_sub_epi8(a: __m256i, b: __m256i) -> __m256i {
    __m256i {
        low: _mm_sub_epi8(a.low, b.low),
        high: _mm_sub_epi8(a.high, b.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_add_epi8(a: __m256i, b: __m256i) -> __m256i {
    __m256i {
        low: _mm_add_epi8(a.low, b.low),
        high: _mm_add_epi8(a.high, b.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_slli_si256<const SHIFT: i32>(a: __m256i) -> __m256i {
    // It shifts the 128-bit register left by SHIFT bytes.
    // Bits do not carry across the 128-bit boundary, perfectly
    // matching the behavior of the AVX2 256-bit version.
    __m256i {
        low: _mm_slli_si128::<SHIFT>(a.low),
        high: _mm_slli_si128::<SHIFT>(a.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_extracti128_si256<const LANE: i32>(a: __m256i) -> __m128i {
    // Because LANE must be a compile-time constant,
    // the compiler will optimize this branch away entirely.
    match LANE {
        0 => a.low,
        1 => a.high,
        _ => panic!("LANE can be 0 or 1"),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_inserti128_si256<const LANE: i32>(a: __m256i, b: __m128i) -> __m256i {
    // In SIMD, Lane 0 is the lower 128 bits, Lane 1 is the upper 128 bits.
    match LANE {
        0 => __m256i {
            low: b,       // Replace low with new 128-bit value
            high: a.high, // Keep existing high
        },
        1 => __m256i {
            low: a.low, // Keep existing low
            high: b,    // Replace high with new 128-bit value
        },
        _ => panic!("LANE can be 0 or 1"),
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
    e09: i8,
    e08: i8,
    e07: i8,
    e06: i8,
    e05: i8,
    e04: i8,
    e03: i8,
    e02: i8,
    e01: i8,
    e00: i8,
) -> __m256i {
    // Construct the low 128-bit part (e0 through e15)
    let low = _mm_set_epi8(
        e15, e14, e13, e12, e11, e10, e09, e08, e07, e06, e05, e04, e03, e02, e01, e00,
    );
    // Construct the high 128-bit part (e16 through e31)
    let high = _mm_set_epi8(
        e31, e30, e29, e28, e27, e26, e25, e24, e23, e22, e21, e20, e19, e18, e17, e16,
    );

    __m256i { low, high }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_castsi256_ps(a: __m256i) -> __m256 {
    __m256 {
        // Cast the lower 128 bits
        low: _mm_castsi128_ps(a.low),
        // Cast the upper 128 bits
        high: _mm_castsi128_ps(a.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_castps_si256(a: __m256) -> __m256i {
    __m256i {
        // Cast lower 128 bits float -> int
        low: _mm_castps_si128(a.low),
        // Cast upper 128 bits float -> int
        high: _mm_castps_si128(a.high),
    }
}

#[target_feature(enable = "sse2")]
#[inline]
pub fn _mm256_shuffle_ps<const MASK: i32>(a: __m256, b: __m256) -> __m256 {
    __m256 {
        // Low lane shuffle: uses a.low and b.low
        low: _mm_shuffle_ps::<MASK>(a.low, b.low),
        // High lane shuffle: uses a.high and b.high
        high: _mm_shuffle_ps::<MASK>(a.high, b.high),
    }
}
