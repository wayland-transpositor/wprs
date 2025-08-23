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

use std::env;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::num::NonZeroUsize;
use std::path::Path;

use anyhow::Error;
use criterion::BatchSize;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use fallible_iterator::IteratorExt;
use png::BitDepth;
use png::ColorType;
use png::Decoder;
use wprs::arc_slice::ArcSlice;
use wprs::buffer_pointer::BufferPointer;
use wprs::filtering;
use wprs::sharding_compression::CompressedShard;
use wprs::sharding_compression::ShardingCompressor;
use wprs::sharding_compression::ShardingDecompressor;
use wprs::vec4u8::Vec4u8s;

fn reorder_channels(data: &mut [u8]) {
    for pixel in data.chunks_mut(4) {
        let r = pixel[0];
        let g = pixel[1];
        let b = pixel[2];
        let a = pixel[3];

        // https://afrantzis.com/pixel-format-guide/wayland_drm.html
        pixel[0] = b;
        pixel[1] = g;
        pixel[2] = r;
        pixel[3] = a;
    }
}

fn read_png(path: &Path) -> Vec<u8> {
    println!("reading png {}", path.display());
    let decoder = Decoder::new(BufReader::new(File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    println!("INFO {info:?}");
    assert_eq!(info.color_type, ColorType::Rgba);
    assert_eq!(info.bit_depth, BitDepth::Eight);
    let mut data = buf[..info.buffer_size()].to_vec();
    reorder_channels(&mut data);
    data
}

#[allow(clippy::redundant_clone)]
fn filter_png(c: &mut Criterion, path: &Path) {
    let data = read_png(path);
    let _orig_data = data.clone();
    // SAFETY: ptr was created from an owned vec, so it is non-null, aligned,
    // and valid for reads of data.len() elements.
    let data_ptr = &data.as_ptr();
    let buf_ptr = unsafe { BufferPointer::new(data_ptr, data.len()) };

    let mut filtered_data = Vec4u8s::with_total_size(data.len());

    c.bench_function(&format!("filter only: {}", path.display()), |b| {
        b.iter(|| {
            filtering::filter(buf_ptr, &mut filtered_data);
        })
    });

    let mut new_data = vec![0; data.len()];

    c.bench_function(&format!("unfilter only: {}", path.display()), |b| {
        b.iter_batched(
            || filtered_data.clone(),
            |mut filtered_data| {
                filtering::unfilter(&mut filtered_data, &mut new_data);
            },
            BatchSize::SmallInput,
        )
    });

    // assert_eq!(new_data, _orig_data);
}

fn compress_png(c: &mut Criterion, path: &Path) {
    let data = read_png(path);

    let uncompressed_size = data.len();

    let n_compressors = NonZeroUsize::new(16).unwrap();
    let n_shards = NonZeroUsize::new(32).unwrap();
    let compressor = ShardingCompressor::new(n_compressors, 1).unwrap();

    let data_arcslice = ArcSlice::new(data);
    let mut compressed_shards = Vec::new();

    c.bench_function(&format!("compress only: {}", path.display()), |b| {
        b.iter(|| {
            compressed_shards = compressor
                .compress(n_shards, data_arcslice.clone())
                .collect();
        })
    });
    let compressed_size: usize = compressed_shards.iter().map(|shard| shard.data.len()).sum();

    let n_decompressors = NonZeroUsize::new(8).unwrap();
    let mut sharding_decompressor = ShardingDecompressor::new(n_decompressors).unwrap();

    let compressed_shards = compressed_shards
        .into_iter()
        .map(Ok::<CompressedShard, Error>)
        .transpose_into_fallible();

    c.bench_function(&format!("decompress only: {}", path.display()), |b| {
        b.iter_batched(
            || compressed_shards.clone(),
            |compressed_shards| {
                let _decompressed_data = sharding_decompressor
                    .decompress_to_owned(n_shards, uncompressed_size, compressed_shards)
                    .unwrap();
                // assert_eq!(_decompressed_data, data_arcslice.as_ref());
            },
            BatchSize::SmallInput,
        )
    });
    let compression_ratio = uncompressed_size as f64 / compressed_size as f64;
    let compression_rate = compressed_size as f64 / uncompressed_size as f64;
    println!("compression ratio (higher is better): {compression_ratio:.1}");
    println!(
        "compression rate (lower is better): {:.1}%",
        compression_rate * 100.0
    );
}

fn filter_compress_png(c: &mut Criterion, path: &Path) {
    let mut data = read_png(path);
    let _orig_data = data.clone();
    let data_ptr = &data.as_ptr();
    let buf_ptr = unsafe { BufferPointer::new(data_ptr, data.len()) };

    let uncompressed_size = data.len();

    let n_compressors = NonZeroUsize::new(16).unwrap();
    let n_shards = NonZeroUsize::new(32).unwrap();
    let compressor = ShardingCompressor::new(n_compressors, 1).unwrap();

    let mut compressed_shards = Vec::new();

    c.bench_function(&format!("filter and compress: {}", path.display()), |b| {
        b.iter_batched(
            || Vec4u8s::with_total_size(data.len()),
            |mut output_buf| {
                filtering::filter(buf_ptr, &mut output_buf);
                let output_vec: Vec<u8> = output_buf.into();
                let output_arcslice = ArcSlice::new(output_vec);
                compressed_shards = compressor.compress(n_shards, output_arcslice).collect();
            },
            BatchSize::SmallInput,
        )
    });

    let compressed_size: usize = compressed_shards.iter().map(|shard| shard.data.len()).sum();

    let n_decompressors = NonZeroUsize::new(8).unwrap();
    let mut sharding_decompressor = ShardingDecompressor::new(n_decompressors).unwrap();

    let compressed_shards = compressed_shards
        .into_iter()
        .map(Ok::<CompressedShard, Error>)
        .transpose_into_fallible();

    c.bench_function(
        &format!("unfilter and decompress: {}", path.display()),
        |b| {
            b.iter_batched(
                || compressed_shards.clone(),
                |compressed_shards| {
                    let mut buf: Vec4u8s = sharding_decompressor
                        .decompress_to_owned(n_shards, uncompressed_size, compressed_shards)
                        .unwrap()
                        .into();

                    filtering::unfilter(&mut buf, &mut data);
                    // assert_eq!(data, _orig_data);
                },
                BatchSize::SmallInput,
            )
        },
    );

    let compression_ratio = uncompressed_size as f64 / compressed_size as f64;
    let compression_rate = compressed_size as f64 / uncompressed_size as f64;
    println!("compression ratio (higher is better): {compression_ratio:.1}");
    println!(
        "compression rate (lower is better): {:.1}%",
        compression_rate * 100.0
    );
}

fn compression_benchmark(c: &mut Criterion) {
    wprs::utils::exit_on_thread_panic();

    let image_dir: String =
        env::var("WPRS_BENCH_IMAGE_DIR").expect("WPRS_BENCH_IMAGE_DIR env var must be set.");
    if image_dir.is_empty() {
        panic!("WPRS_BENCH_IMAGE_DIR env var must be non-empty.")
    }

    let files = fs::read_dir(image_dir)
        .unwrap()
        .map(|dirent| dirent.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"));
    for file in files {
        filter_png(c, &file);
        println!("");
        compress_png(c, &file);
        println!("");
        filter_compress_png(c, &file);
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("");
    }
}

criterion_group!(benches, compression_benchmark);
criterion_main!(benches);
