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
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use wprs::prefix_sum;

fn prefix_sum_benchmark(c: &mut Criterion) {
    let i: u32 = 100 * 1024 * 1024;
    let mut arr = (0..i).map(|i| (i % 256) as u8).collect::<Vec<_>>();

    c.bench_function("prefix-sum-scalar", |b| {
        b.iter(|| prefix_sum::prefix_sum_scalar(black_box(&mut arr), 0))
    });
    c.bench_function("prefix-sum-32", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<32>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-64", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<64>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-128", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<128>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-256", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<256>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-512", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<512>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-1024", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<1024>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-2048", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<2048>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-4096", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<4096>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-8192", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<8192>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-16384", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<16384>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-32768", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<32768>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-65536", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<65536>(black_box(&mut arr)) })
    });
    c.bench_function("prefix-sum-131072", |b| {
        b.iter(|| unsafe { prefix_sum::prefix_sum_bs::<131072>(black_box(&mut arr)) })
    });
}

criterion_group!(benches, prefix_sum_benchmark);
criterion_main!(benches);
