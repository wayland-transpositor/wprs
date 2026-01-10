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

use std::arch::x86_64::_mm_shuffle_epi8;

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", not(target_feature = "sse4.1")))] {
        pub use crate::simd::sse2::_mm_extract_epi8;
        pub use crate::simd::sse2::_mm256_extract_epi8;
    }
}
pub use crate::simd::sse2_base::*;

#[target_feature(enable = "ssse3")]
#[inline]
pub fn _mm256_shuffle_epi8(a: __m256i, b: __m256i) -> __m256i {
    _mm256_shuffle_epi8!(a, b)
}
