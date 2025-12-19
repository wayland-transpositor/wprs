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

/// Tuple structs to work around tuple's lack of repr(c) support.
///
/// rkyv's struct mode uses repr(c) to ensure layout compatibility between rust
/// versions, but rust tuples don't support repr(c).
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Result;

use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct Tuple2<T1, T2>(pub T1, pub T2);

impl<T1, T2> Debug for ArchivedTuple2<T1, T2>
where
    T1: Debug + Archive,
    <T1 as Archive>::Archived: Debug,
    T2: Debug + Archive,
    <T2 as Archive>::Archived: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_struct("Tuple2")
            .field("0", &self.0)
            .field("1", &self.1)
            .finish()
    }
}

impl<T1, T2> From<(T1, T2)> for Tuple2<T1, T2> {
    fn from((x1, x2): (T1, T2)) -> Self {
        Self(x1, x2)
    }
}

impl<T1, T2> From<Tuple2<T1, T2>> for (T1, T2) {
    fn from(Tuple2(x1, x2): Tuple2<T1, T2>) -> Self {
        (x1, x2)
    }
}
