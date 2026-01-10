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

pub use std::arch::x86_64::_mm_extract_epi8;

pub use crate::simd::sse2_base::*;
pub use crate::simd::ssse3::_mm256_shuffle_epi8;

#[target_feature(enable = "sse4.1")]
#[inline]
pub fn _mm256_extract_epi8<const INDEX: i32>(a: __m256i) -> i32 {
    _mm256_extract_epi8!(a, INDEX)
}
