use bimap::BiMap;
use smithay_client_toolkit::reexports::client::backend::ObjectId as SctkObjectId;

use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::ObjectId;

pub(crate) type ObjectBimap = BiMap<(ClientId, ObjectId), SctkObjectId>;

pub mod backend;
mod sctk;

pub mod server_handlers;
pub mod smithay_handlers;
mod subsurface;
mod xdg_shell;

pub use backend::WaylandClientBackend;
pub use sctk::*;
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
