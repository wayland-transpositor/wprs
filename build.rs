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

use std::error::Error;

use merkle_hash::Algorithm;
use merkle_hash::MerkleTree;

fn main() -> Result<(), Box<dyn Error>> {
    let serialization_tree = MerkleTree::builder("./src/protocols/wprs")
        .algorithm(Algorithm::Blake3)
        .hash_names(false)
        .build()?;
    let serialization_hash = merkle_hash::bytes_to_hex(serialization_tree.root.item.hash);
    println!("cargo:rustc-env=SERIALIZATION_TREE_HASH={serialization_hash}");
    Ok(())
}
