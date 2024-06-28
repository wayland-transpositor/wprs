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

pub mod arc_slice;
pub mod args;
pub mod buffer_pointer;
pub mod channel_utils;
pub mod client;
pub mod client_utils;
pub mod compositor_utils;
pub mod constants;
pub mod control_server;
pub mod error_utils;
pub mod fallible_entry;
pub mod filtering;
pub mod prefix_sum;
pub mod prelude;
pub mod serialization;
pub mod server;
pub mod sharding_compression;
pub mod transpose;
pub mod utils;
pub mod vec4u8;
pub mod xwayland_xdg_shell;

#[cfg(feature = "tracy-allocator")]
pub mod tracy_allocator;
