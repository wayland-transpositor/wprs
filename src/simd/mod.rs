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

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", target_feature = "avx"))] {
        pub mod avx;
        pub use crate::simd::avx::*;
    } else if #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))] {
        pub mod sse2;
        pub use crate::simd::sse2::*;
    } else {
        compile_error!("x86_64 SIMD support is required.");
    }
}
