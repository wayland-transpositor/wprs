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
use std::arch::x86_64::_mm256_add_epi8;
use std::arch::x86_64::_mm256_loadu_si256;
use std::arch::x86_64::_mm256_slli_si256;
use std::arch::x86_64::_mm256_storeu_si256;
use std::arch::x86_64::_mm_add_epi8;
use std::arch::x86_64::_mm_loadu_si128;
use std::arch::x86_64::_mm_prefetch;
use std::arch::x86_64::_mm_set1_epi8;
use std::arch::x86_64::_mm_setzero_si128;
use std::arch::x86_64::_mm_storeu_si128;
use std::arch::x86_64::_MM_HINT_T0;

// SAFETY:
// * avx2 must be available.
// * `block` must be valid for reads and writes of 32 bytes.
#[allow(unsafe_op_in_unsafe_fn)]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn prefix_32(block: *mut u8) {
    let mut x: __m256i = _mm256_loadu_si256(block.cast::<__m256i>());
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 1));
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 2));
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 4));
    x = _mm256_add_epi8(x, _mm256_slli_si256(x, 8));
    _mm256_storeu_si256(block.cast::<__m256i>(), x);
}

// SAFETY:
// * sse2 must be available.
// * `block` must be valid for reads and writes of 16 bytes.
#[allow(unsafe_op_in_unsafe_fn)]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn accumulate_16<const BS: usize>(block: *mut u8, prev_block: __m128i) -> __m128i {
    let cur_sum = _mm_set1_epi8(block.add(15).read() as i8);
    let mut cur_block: __m128i = _mm_loadu_si128(block.cast::<__m128i>());
    cur_block = _mm_add_epi8(prev_block, cur_block);
    _mm_storeu_si128(block.cast::<__m128i>(), cur_block);
    _mm_add_epi8(prev_block, cur_sum)
}

/// Computes the prefix sum of `arr` in-place using SIMD instructions. BS bytes
/// will be processed at a time; small sizes will cause pipeline stalls and
/// large sizes will cause cache misses. `arr.len()` and `BS` must be non-zero,
/// multiples of 32, and `arr.len()` must be >= `BS`.
///
/// SAFETY:
/// * avx2 and sse2 must be available.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn prefix_sum_avx2<const BS: usize>(arr: &mut [u8]) {
    let len = arr.len();
    let arr = arr.as_mut_ptr();
    assert!(len > 0);
    assert!(BS > 0);
    assert!(len >= BS);
    assert!(len % 32 == 0);
    assert!(BS % 32 == 0);

    // SAFETY:
    // * avx2 and sse2 are available by precondition.
    // * a.len() is a multiple of 32, so reading blocks of 32 from a at
    // offset i*32 will be valid when i < n / 32. Since BS < n, reading
    // from offset i*32 will also be valid when i < BS / 32.
    unsafe {
        let mut s: __m128i = _mm_setzero_si128();

        for i in (0..BS).step_by(32) {
            prefix_32(arr.add(i));
        }

        for i in (BS..len).step_by(32) {
            prefix_32(arr.add(i));
            _mm_prefetch(arr.add(i + BS).cast(), _MM_HINT_T0);
            s = accumulate_16::<BS>(arr.add(i - BS), s);
            s = accumulate_16::<BS>(arr.add(i - BS + 16), s);
        }

        for i in ((len - BS)..len).step_by(32) {
            _mm_prefetch(arr.add(i + BS).cast(), _MM_HINT_T0);
            s = accumulate_16::<BS>(arr.add(i), s);
            s = accumulate_16::<BS>(arr.add(i + 16), s);
        }
    }
}

#[inline(always)]
pub fn prefix_sum_scalar(a: &mut [u8], prior_sum: u8) {
    let len = a.len();
    a[0] = a[0].wrapping_add(prior_sum);
    for i in 1..len {
        a[i] = a[i].wrapping_add(a[i - 1]);
    }
}

/// Computes the prefix sum of `arr` in-place. BS bytes will be processed at a
/// time; small sizes will cause pipeline stalls and large sizes will cause
/// cache misses. `BS` must be non-zero and a multiple of 32.
///
/// # Safety
/// * avx2 and sse2 must be available.
///
/// # Panics
/// If BS is 0 or not a multiple of 32.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
#[target_feature(enable = "sse2")]
#[inline]
pub unsafe fn prefix_sum_bs<const BS: usize>(arr: &mut [u8]) {
    assert!(BS > 0);
    assert!(BS % 32 == 0);
    let len = arr.len();
    let lim = (len / BS) * BS;

    let prior_sum = if lim > 0 {
        // SAFETY: avx2 and sse2 are available by precondition.
        unsafe {
            prefix_sum_avx2::<BS>(&mut arr[0..lim]);
        }
        arr[lim - 1]
    } else {
        0
    };

    if lim != len {
        prefix_sum_scalar(&mut arr[lim..len], prior_sum);
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
        let i: u32 = 1000;
        let mut arr = (0..i).map(|i| (i % 256) as u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_smaller_than_block_size() {
        let i: u32 = 10;
        let mut arr = (0..i).map(|i| (i % 256) as u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_size_power_of_two() {
        let i: u32 = 256;
        let mut arr = (0..i).map(|i| (i % 256) as u8).collect::<Vec<_>>();
        let mut expected_arr = arr.clone();

        prefix_sum(&mut arr);
        prefix_sum_scalar(&mut expected_arr, 0);

        assert_eq!(arr, expected_arr);
    }

    #[test]
    fn test_prefix_sum_input_size_odd() {
        let i: u32 = 1001;
        let mut arr = (0..i).map(|i| (i % 256) as u8).collect::<Vec<_>>();
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
