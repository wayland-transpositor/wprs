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

use std::fs;
use std::fs::File;
use std::num::NonZeroUsize;
use std::path::Path;

use anyhow::Error;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
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

// TODO: there a bunch of expensive clones being done in the benchmarks, try to
// get rid of them. Until then, the runtime benchmarks are mostly useful for
// relative comparisons and less so for absolute comparisons. The compression
// ratio numbers are still useful absolutely.

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
    let decoder = Decoder::new(File::open(path).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
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
        b.iter(|| {
            let mut filtered_data_copy = filtered_data.clone();
            filtering::unfilter(&mut filtered_data_copy, &mut new_data);
        })
    });

    // assert_eq!(new_data, _orig_data);
}

fn compress_png(c: &mut Criterion, path: &Path) -> f64 {
    let data = read_png(path);

    let uncompressed_size = data.len();
    let mut compressed_size = 0;

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
            compressed_size = compressed_shards.iter().map(|shard| shard.data.len()).sum();
        })
    });

    let n_decompressors = NonZeroUsize::new(8).unwrap();
    let mut sharding_decompressor = ShardingDecompressor::new(n_decompressors).unwrap();

    let compressed_shards = compressed_shards
        .into_iter()
        .map(|shard| -> Result<CompressedShard, Error> { Ok(shard) });
    let compressed_shards = fallible_iterator::convert(compressed_shards);

    let mut compression_ratio = 0.0;
    let mut compression_rate = 0.0;
    let mut ret = 0.0;

    c.bench_function(&format!("decompress only: {}", path.display()), |b| {
        b.iter(|| {
            sharding_decompressor
                .decompress_with(
                    n_shards,
                    uncompressed_size,
                    compressed_shards.clone(),
                    |_decompressed_data| {
                        // assert_eq!(_decompressed_data, data_arcslice.as_ref());

                        compression_ratio = uncompressed_size as f64 / compressed_size as f64;
                        compression_rate = compressed_size as f64 / uncompressed_size as f64;
                        ret = compression_ratio;
                        Ok(())
                    },
                )
                .unwrap();
        })
    });
    println!("compression ratio: {compression_ratio:.1}");
    println!("compression rate: {:.1}%", compression_rate * 100.0);
    ret
}

fn filter_compress_png(c: &mut Criterion, path: &Path) -> f64 {
    let mut data = read_png(path);
    let _orig_data = data.clone();
    let data_ptr = &data.as_ptr();
    let buf_ptr = unsafe { BufferPointer::new(data_ptr, data.len()) };

    let uncompressed_size = data.len();
    let mut compressed_size = 0;

    let n_compressors = NonZeroUsize::new(16).unwrap();
    let n_shards = NonZeroUsize::new(32).unwrap();
    let compressor = ShardingCompressor::new(n_compressors, 1).unwrap();

    let mut compressed_shards = Vec::new();

    c.bench_function(&format!("filter and compress: {}", path.display()), |b| {
        b.iter(|| {
            let mut output_buf = Vec4u8s::with_total_size(data.len());
            filtering::filter(buf_ptr, &mut output_buf);
            let output_vec: Vec<u8> = output_buf.into();
            let output_arcslice = ArcSlice::new(output_vec);
            compressed_shards = compressor.compress(n_shards, output_arcslice).collect();
            compressed_size = compressed_shards.iter().map(|shard| shard.data.len()).sum();
        })
    });

    let n_decompressors = NonZeroUsize::new(8).unwrap();
    let mut sharding_decompressor = ShardingDecompressor::new(n_decompressors).unwrap();

    let compressed_shards = compressed_shards
        .into_iter()
        .map(|shard| -> Result<CompressedShard, Error> { Ok(shard) });
    let compressed_shards = fallible_iterator::convert(compressed_shards);

    let mut compression_ratio = 0.0;
    let mut compression_rate = 0.0;
    let mut ret = 0.0;

    c.bench_function(
        &format!("unfilter and decompress: {}", path.display()),
        |b| {
            b.iter(|| {
                sharding_decompressor
                    .decompress_with(
                        n_shards,
                        uncompressed_size,
                        compressed_shards.clone(),
                        |buf| {
                            let mut buf: Vec4u8s = buf.to_vec().into();
                            filtering::unfilter(&mut buf, &mut data);

                            // assert_eq!(data, _orig_data);

                            compression_ratio = uncompressed_size as f64 / compressed_size as f64;
                            compression_rate = compressed_size as f64 / uncompressed_size as f64;
                            ret = compression_ratio;
                            Ok(())
                        },
                    )
                    .unwrap();
            })
        },
    );
    println!("compression ratio: {compression_ratio:.1}");
    println!("compression rate: {:.1}%", compression_rate * 100.0);
    ret
}

fn mean(numbers: &[f64]) -> f64 {
    numbers.iter().sum::<f64>() / numbers.len() as f64
}

fn compression_benchmark(c: &mut Criterion) {
    // TODO: replace file path with in-memory image (or add an image that is safe to distribute with the codebase)
    // https://qoiformat.org/benchmark/
    let files = fs::read_dir("/home/rasputin/qoi_benchmark_images/screenshot_web/")
        .unwrap()
        .map(|dirent| dirent.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"));
    let mut compression_ratios = Vec::new();
    let mut filter_compression_ratios = Vec::new();
    for file in files {
        filter_png(c, &file);
        println!("");
        compression_ratios.push(compress_png(c, &file));
        println!("");
        filter_compression_ratios.push(filter_compress_png(c, &file));
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("");
    }
    let mean_compression_ratio = mean(&compression_ratios);
    let mean_filter_compression_ratio = mean(&filter_compression_ratios);
    println!("");
    println!("mean compression only ratio: {mean_compression_ratio:.1}");
    println!(
        "mean compression only rate: {:.1}%",
        1.0 / mean_compression_ratio * 100.0
    );
    println!("mean compression with filter ratio: {mean_filter_compression_ratio:.1}");
    println!(
        "mean compression with filter rate: {:.1}%",
        1.0 / mean_filter_compression_ratio * 100.0
    );
    println!("--------------------------------------------------------------------------------");
    println!("");
    println!("");
}

criterion_group!(benches, compression_benchmark);
criterion_main!(benches);
