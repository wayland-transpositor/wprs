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
use std::arch::x86_64::_mm_castsi128_ps;
pub use std::arch::x86_64::_mm_extract_epi8;
pub use std::arch::x86_64::_mm_loadu_si128;
pub use std::arch::x86_64::_mm_set1_epi8;
pub use std::arch::x86_64::_mm_setzero_si128;
use std::arch::x86_64::_mm_shuffle_epi8;
use std::arch::x86_64::_mm_slli_si128;
pub use std::arch::x86_64::_mm_storeu_si128;
use std::arch::x86_64::_mm_sub_epi8;
use std::arch::x86_64::_mm256_blend_ps;
pub use std::arch::x86_64::_mm256_castps_si256;
pub use std::arch::x86_64::_mm256_castsi128_si256;
pub use std::arch::x86_64::_mm256_castsi256_ps;
pub use std::arch::x86_64::_mm256_castsi256_si128;
use std::arch::x86_64::_mm256_extractf128_si256;
use std::arch::x86_64::_mm256_insertf128_ps;
pub use std::arch::x86_64::_mm256_loadu_si256;
pub use std::arch::x86_64::_mm256_set_epi8;
pub use std::arch::x86_64::_mm256_set_m128i;
pub use std::arch::x86_64::_mm256_shuffle_ps;
pub use std::arch::x86_64::_mm256_storeu_si256;

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_sub_epi8(a: __m256i, b: __m256i) -> __m256i {
    // 1. Extract the low 128-bit halves from the 256-bit registers.
    // _mm256_castsi256_si128 is a zero-cost instruction that just treats
    // the YMM register as an XMM register.
    let a_lo = _mm256_castsi256_si128(a);
    let b_lo = _mm256_castsi256_si128(b);

    // 2. Extract the high 128-bit halves.
    // _mm256_extractf128_si256 is an AVX instruction that pulls the
    // upper 128 bits into an XMM register.
    let a_hi = _mm256_extractf128_si256::<1>(a);
    let b_hi = _mm256_extractf128_si256::<1>(b);

    // 3. Perform 8-bit integer subtraction on the halves.
    // On AVX hardware, these will be emitted as VEX-encoded VPSUBB instructions.
    let res_lo = _mm_sub_epi8(a_lo, b_lo);
    let res_hi = _mm_sub_epi8(a_hi, b_hi);

    // 4. Recombine the two 128-bit results back into a single 256-bit register.
    _mm256_set_m128i(res_hi, res_lo)
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_add_epi8(a: __m256i, b: __m256i) -> __m256i {
    // 1. Extract the low 128-bit halves from the 256-bit registers.
    // _mm256_castsi256_si128 is a zero-cost instruction that just treats
    // the YMM register as an XMM register.
    let a_lo = _mm256_castsi256_si128(a);
    let b_lo = _mm256_castsi256_si128(b);

    // 2. Extract the high 128-bit halves.
    // _mm256_extractf128_si256 is an AVX instruction that pulls the
    // upper 128 bits into an XMM register.
    let a_hi = _mm256_extractf128_si256::<1>(a);
    let b_hi = _mm256_extractf128_si256::<1>(b);

    // 3. Perform 8-bit integer subtraction on the halves.
    // On AVX hardware, these will be emitted as VEX-encoded VPSUBB instructions.
    let res_lo = _mm_add_epi8(a_lo, b_lo);
    let res_hi = _mm_add_epi8(a_hi, b_hi);

    // 4. Recombine the two 128-bit results back into a single 256-bit register.
    _mm256_set_m128i(res_hi, res_lo)
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_slli_si256<const SHIFT: i32>(a: __m256i) -> __m256i {
    // 1. Split: Extract the 128-bit halves
    // Cast is zero-cost; it just treats the YMM as an XMM (low half)
    let lo = _mm256_castsi256_si128(a);
    // Extract the high 128 bits
    let hi = _mm256_extractf128_si256::<1>(a);

    // 2. Shift: Apply 128-bit byte shift to each half
    // These will be compiled as VEX-encoded VPSLLDQ XMM instructions
    let res_lo = _mm_slli_si128::<SHIFT>(lo);
    let res_hi = _mm_slli_si128::<SHIFT>(hi);

    // 3. Merge: Combine back into a 256-bit register
    _mm256_set_m128i(res_hi, res_lo)
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_extracti128_si256<const HIGH: i32>(a: __m256i) -> __m128i {
    _mm256_extractf128_si256::<HIGH>(a)
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_blend_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    // 1. Cast integer vectors to floating-point vectors (bit-preserving)
    let a_f = _mm256_castsi256_ps(a);
    let b_f = _mm256_castsi256_ps(b);

    // 2. Perform the blend using the AVX1 floating-point intrinsic.
    // We use a match to pipe the const generic MASK into the literal slot.
    let res_f = _mm256_blend_ps::<MASK>(a_f, b_f);

    // 3. Cast back to integer vector (bit-preserving)
    _mm256_castps_si256(res_f)
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_extract_epi8<const INDEX: i32>(a: __m256i) -> i32 {
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
        0 => _mm_extract_epi8::<0>(v),
        1 => _mm_extract_epi8::<1>(v),
        2 => _mm_extract_epi8::<2>(v),
        3 => _mm_extract_epi8::<3>(v),
        4 => _mm_extract_epi8::<4>(v),
        5 => _mm_extract_epi8::<5>(v),
        6 => _mm_extract_epi8::<6>(v),
        7 => _mm_extract_epi8::<7>(v),
        8 => _mm_extract_epi8::<8>(v),
        9 => _mm_extract_epi8::<9>(v),
        10 => _mm_extract_epi8::<10>(v),
        11 => _mm_extract_epi8::<11>(v),
        12 => _mm_extract_epi8::<12>(v),
        13 => _mm_extract_epi8::<13>(v),
        14 => _mm_extract_epi8::<14>(v),
        15 => _mm_extract_epi8::<15>(v),
        // Upper Lane (16-31)
        16 => _mm_extract_epi8::<0>(v),
        17 => _mm_extract_epi8::<1>(v),
        18 => _mm_extract_epi8::<2>(v),
        19 => _mm_extract_epi8::<3>(v),
        20 => _mm_extract_epi8::<4>(v),
        21 => _mm_extract_epi8::<5>(v),
        22 => _mm_extract_epi8::<6>(v),
        23 => _mm_extract_epi8::<7>(v),
        24 => _mm_extract_epi8::<8>(v),
        25 => _mm_extract_epi8::<9>(v),
        26 =>  _mm_extract_epi8::<10>(v),
        27 =>  _mm_extract_epi8::<11>(v),
        28 =>  _mm_extract_epi8::<12>(v),
        29 =>  _mm_extract_epi8::<13>(v),
        30 =>  _mm_extract_epi8::<14>(v),
        31 =>  _mm_extract_epi8::<15>(v),
        _ => panic!("Index out of bounds for 256-bit extract"),
    }
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_inserti128_si256<const LANE: i32>(a: __m256i, b: __m128i) -> __m256i {
    // Cast to __m256 (float), insert, cast back
    let a_f = _mm256_castsi256_ps(a);
    let b_f = _mm_castsi128_ps(b);
    let res_f = _mm256_insertf128_ps::<LANE>(a_f, b_f);
    _mm256_castps_si256(res_f)
}

#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_shuffle_epi8(a: __m256i, b: __m256i) -> __m256i {
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
    _mm256_inserti128_si256::<1>(res, res_high)
}

/**
 * NOTE: The following functions are not actual std::arch::x86_64 intrinsics.
 * They are wprs specific but we put them here because they have a specific
 * SSE counterpart
 */
#[target_feature(enable = "avx")]
#[inline]
pub fn _mm256_shufps_epi32<const MASK: i32>(a: __m256i, b: __m256i) -> __m256i {
    _mm256_castps_si256(_mm256_shuffle_ps::<MASK>(
        _mm256_castsi256_ps(a),
        _mm256_castsi256_ps(b),
    ))
}
