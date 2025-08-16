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

/// u8 prefix sum functions, based on
/// https://en.algorithmica.org/hpc/algorithms/prefix/.
use std::arch::x86_64::__m128i;
use std::arch::x86_64::__m256i;
use std::arch::x86_64::_MM_HINT_T0;
use std::arch::x86_64::_mm_add_epi8;
use std::arch::x86_64::_mm_loadu_si128;
use std::arch::x86_64::_mm_prefetch;
use std::arch::x86_64::_mm_set1_epi8;
use std::arch::x86_64::_mm_setzero_si128;
use std::arch::x86_64::_mm_storeu_si128;
use std::arch::x86_64::_mm256_add_epi8;
use std::arch::x86_64::_mm256_loadu_si256;
use std::arch::x86_64::_mm256_slli_si256;
use std::arch::x86_64::_mm256_storeu_si256;

use crate::utils::AssertN;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[inline]
fn prefix_32(block: &mut [u8; 32]) {
    let block: *mut u8 = block.as_mut_ptr().cast();
    // SAFETY: block is an &mut [u8; 32] and so is valid for reads of 32 bytes.
    let mut x: __m256i = unsafe { _mm256_loadu_si256(block.cast::<__m256i>()) };
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 1));
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 2));
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 4));
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 8));
    // SAFETY: block is an &mut [u8; 32] and so is valid for writes of 32 bytes.
    unsafe { _mm256_storeu_si256(block.cast::<__m256i>(), x) };
}

use std::ops::IndexMut;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
#[inline]
fn accumulate_16(block: &mut [u8; 16], prev_block: __m128i) -> __m128i {
    let cur_sum = _mm_set1_epi8(block[15] as i8);
    let block_ptr: *mut u8 = block.as_mut_ptr().cast();
    // SAFETY: block is an &mut [u8; 16] and so is valid for reads of 16 bytes.
    let mut cur_block: __m128i = unsafe { _mm_loadu_si128(block_ptr.cast::<__m128i>()) };
    cur_block = _mm_add_epi8(prev_block, cur_block);
    // SAFETY: block is an &mut [u8; 32] and so is valid for writes of 16 bytes.
    unsafe {
        _mm_storeu_si128(block_ptr.cast::<__m128i>(), cur_block);
    }
    _mm_add_epi8(prev_block, cur_sum)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
#[inline]
fn accumulate_32(block: &mut [u8; 32], mut prev_block: __m128i) -> __m128i {
    prev_block = accumulate_16(block.index_mut(0..16).try_into().unwrap(), prev_block);
    accumulate_16(block.index_mut(16..32).try_into().unwrap(), prev_block)
}

/// Computes the prefix sum of `arr` in-place using SIMD instructions. BS bytes
/// will be processed at a time; small sizes will cause pipeline stalls and
/// large sizes will cause cache misses. `arr.len()` and `BS` must be non-zero,
/// multiples of 32, and `arr.len()` must be >= `BS`.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2,sse2")]
#[inline]
fn prefix_sum_avx2<const BS: usize>(arr: &mut [u8]) {
    let _ = AssertN::<BS>::NE_0;
    let _ = AssertN::<BS>::MULTIPLE_OF_32;
    let chunks_per_block = BS / 32;
    let len = arr.len();
    assert!(len >= BS);
    assert!(len.is_multiple_of(32));

    // We could replace many of the functions below wich unchecked verions, but
    // benchmarking shows performance to be a wash.

    let mut s: __m128i = _mm_setzero_si128();
    // arr.len() is a multiple of 32, so there won't be a remainder.
    let chunks = arr.as_chunks_mut::<32>().0;

    // prefix 0 .. BS
    for chunk in chunks.index_mut(..chunks_per_block) {
        prefix_32(chunk);
    }

    if chunks.len() > chunks_per_block && chunks.len() <= 2 * chunks_per_block {
        // prefix     BS .. len
        // accumulate 0 .. len - BS
        for i in chunks_per_block..chunks.len() {
            let chunk = chunks.index_mut(i);
            prefix_32(chunk);
            s = accumulate_32(chunks.index_mut(i - chunks_per_block), s);
        }
    } else if chunks.len() > 2 * chunks_per_block {
        // prefix     BS .. len - 2*BS
        // accumulate 0 .. len - 3*BS
        for i in chunks_per_block..chunks.len() - 2 * chunks_per_block {
            let chunk = chunks.index_mut(i);
            prefix_32(chunk);
            // SAFETY: this loop ends at arr[len - 2*BS], so the current chunk pointer
            // plus BS is still within arr.
            unsafe { _mm_prefetch(chunk.as_ptr().add(BS).cast(), _MM_HINT_T0) };
            s = accumulate_32(chunks.index_mut(i - chunks_per_block), s);
        }

        // prefix     len - 2*BS .. len
        // accumulate len - 3*BS .. len - BS
        for i in chunks.len() - 2 * chunks_per_block..chunks.len() {
            let chunk = chunks.index_mut(i);
            prefix_32(chunk);
            s = accumulate_32(chunks.index_mut(i - chunks_per_block), s);
        }
    }

    // accumulate len - BS .. len
    for chunk in chunks.index_mut(chunks.len() - chunks_per_block..) {
        s = accumulate_32(chunk, s);
    }
}

#[inline(always)]
pub fn prefix_sum_scalar(a: &mut [u8], prior_sum: u8) {
    a[0] = a[0].wrapping_add(prior_sum);
    for i in 1..a.len() {
        a[i] = a[i].wrapping_add(a[i - 1]);
    }
}

/// Computes the prefix sum of `arr` in-place.
///
/// BS bytes will be processed at a time; small sizes will cause pipeline
/// stalls and large sizes will cause cache misses. `BS` must be non-zero
/// and a multiple of 32.
///
/// # Safety
/// * requires AVX2 and SSE2
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2,sse2")]
#[inline]
pub fn prefix_sum_bs<const BS: usize>(arr: &mut [u8]) {
    let _ = AssertN::<BS>::NE_0;
    let _ = AssertN::<BS>::MULTIPLE_OF_32;
    let lim = (arr.len() / BS) * BS;

    let prior_sum = if lim > 0 {
        prefix_sum_avx2::<BS>(&mut arr[..lim]);
        arr[lim - 1]
    } else {
        0
    };

    if lim != arr.len() {
        prefix_sum_scalar(&mut arr[lim..], prior_sum);
    }
}

/// Computes the prefix sum of `arr` in-place. Will use SIMD intrinsics if AVX2
/// is available. *Significantly* (~4.5x) slower without AVX2.
#[inline(always)]
pub fn prefix_sum(arr: &mut [u8]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("sse2") {
            // A block size of 2048 seems to perform well based on benchmarks.
            // SAFETY: checked for avx2 and sse2 support.
            return unsafe { prefix_sum_bs::<2048>(arr) };
        }
    }

    prefix_sum_scalar(arr, 0)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_prefix_scalar() {
        let mut input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let expected = vec![
            0, 1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 66, 78, 91, 105, 120, 136,
        ];

        prefix_sum_scalar(&mut input, 0);

        assert_eq!(input, expected);
    }

    #[test]
    fn test_prefix_sum() {
        let i: u32 = 4096;
        let mut arr = (0..i).map(|_| 1u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_smaller_than_block_size() {
        let i: u32 = 10;
        let mut arr = (0..i).map(|_| 1u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_smaller_than_2_times_block_size() {
        let i: u32 = 3072;
        let mut arr = (0..i).map(|_| 1u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_size_power_of_two() {
        let i: u32 = 256;
        let mut arr = (0..i).map(|_| 1u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_size_odd() {
        let i: u32 = 1001;
        let mut arr = (0..i).map(|_| 1u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    proptest! {
        #[test]
        #[cfg_attr(miri, ignore)]
        fn proptest_prefix_sum(mut arr in proptest::collection::vec(0..u8::MAX, 0..1_000_000)) {
            let mut expected_arr = arr.clone();

            prefix_sum(&mut arr);
            prefix_sum_scalar(&mut expected_arr, 0);

            prop_assert_eq!(arr, expected_arr);
        }
    }
}
