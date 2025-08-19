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

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use rkyv::rancor::Error as RancorError;
use wprs::serialization::wayland::DataToTransfer;

// benchmark to make sure we stay performant at serializing/deserializing large buffers.
// this means there should be zero copy, zero validation.
fn serialization_benchmark(c: &mut Criterion) {
    let i: u32 = 100 * 1024 * 1024;
    let buf = (0..i).map(|i| (i % 256) as u8).collect::<Vec<_>>();
    let message = DataToTransfer(buf);

    c.bench_function("serialize", |b| {
        b.iter(|| rkyv::to_bytes::<RancorError>(&message));
    });

    let archived_message = rkyv::to_bytes::<RancorError>(&message).unwrap();
    c.bench_function("deserialize", |b| {
        b.iter(|| rkyv::from_bytes::<DataToTransfer, RancorError>(&archived_message[..]));
    });
}

criterion_group!(benches, serialization_benchmark);
criterion_main!(benches);
