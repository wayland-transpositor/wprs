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

use crate::buffer_pointer::BufferPointer;
use crate::prefix_sum;
use crate::prelude::*;
use crate::transpose;
use crate::vec4u8::Vec4u8;
use crate::vec4u8::Vec4u8s;

// TODO: benchmarks, enable avx2 for auto-vectorization:
// https://doc.rust-lang.org/beta/core/arch/index.html#examples

#[instrument(skip_all, level = "debug")]
pub fn filter(data: BufferPointer<u8>, output_buf: &mut Vec4u8s) {
    assert!(data.len().is_multiple_of(4)); // data is a buffer of argb or xrgb pixels.
    // SAFETY: Vec4u8 is a repr(C, packed) wrapper around [u8; 4].
    let data = unsafe { data.cast::<Vec4u8>() };
    transpose::vec4u8_aos_to_soa(data, output_buf);
    filter_argb8888(output_buf);
}

#[instrument(skip_all, level = "debug")]
pub fn unfilter(data: &mut Vec4u8s, output_buf: &mut [u8]) {
    let output_buf = bytemuck::cast_slice_mut(output_buf);
    unfilter_argb8888(data);
    transpose::vec4u8_soa_to_aos(data, output_buf);
}

// https://afrantzis.com/pixel-format-guide/wayland_drm.html

#[instrument(skip_all, level = "debug")]
pub fn filter_argb8888(data: &mut Vec4u8s) {
    let mut prev = Vec4u8::new();
    for vec4 in data.iter_mut() {
        let b = vec4.0.wrapping_sub(prev.0);
        let g = vec4.1.wrapping_sub(prev.1);
        let r = vec4.2.wrapping_sub(prev.2);
        let a = vec4.3.wrapping_sub(prev.3);

        prev.0 = *vec4.0;
        prev.1 = *vec4.1;
        prev.2 = *vec4.2;
        prev.3 = *vec4.3;

        *vec4.0 = g;
        *vec4.1 = b.wrapping_sub(g);
        *vec4.2 = r.wrapping_sub(g);
        *vec4.3 = a;
    }
}

#[instrument(skip_all, level = "debug")]
pub fn unfilter_argb8888(data: &mut Vec4u8s) {
    for vec4 in data.iter_mut() {
        let g = *vec4.0;
        let b = vec4.1.wrapping_add(g);
        let r = vec4.2.wrapping_add(g);
        let a = *vec4.3;

        *vec4.0 = b;
        *vec4.1 = g;
        *vec4.2 = r;
        *vec4.3 = a;
    }

    let (p0, p1, p2, p3) = data.parts_mut();
    debug_span!("prefix_sum").in_scope(|| {
        lagoon::ThreadPool::global().scoped(|s| {
            s.run(move || prefix_sum::prefix_sum(p0));
            s.run(move || prefix_sum::prefix_sum(p1));
            s.run(move || prefix_sum::prefix_sum(p2));
            s.run(move || prefix_sum::prefix_sum(p3));
        });
    });
}
