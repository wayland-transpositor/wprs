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
pub use std::arch::x86_64::_mm_extract_epi8;
pub use std::arch::x86_64::_mm_loadu_si128;
pub use std::arch::x86_64::_mm_set1_epi8;
pub use std::arch::x86_64::_mm_setzero_si128;
pub use std::arch::x86_64::_mm_storeu_si128;
pub use std::arch::x86_64::_mm256_add_epi8;
pub use std::arch::x86_64::_mm256_blend_epi32;
pub use std::arch::x86_64::_mm256_castps_si256;
pub use std::arch::x86_64::_mm256_castsi128_si256;
pub use std::arch::x86_64::_mm256_castsi256_ps;
pub use std::arch::x86_64::_mm256_castsi256_si128;
pub use std::arch::x86_64::_mm256_extract_epi8;
pub use std::arch::x86_64::_mm256_extracti128_si256;
pub use std::arch::x86_64::_mm256_inserti128_si256;
pub use std::arch::x86_64::_mm256_loadu_si256;
pub use std::arch::x86_64::_mm256_set_epi8;
pub use std::arch::x86_64::_mm256_set_m128i;
pub use std::arch::x86_64::_mm256_shuffle_epi8;
pub use std::arch::x86_64::_mm256_shuffle_ps;
pub use std::arch::x86_64::_mm256_slli_si256;
pub use std::arch::x86_64::_mm256_storeu_si256;
pub use std::arch::x86_64::_mm256_sub_epi8;
