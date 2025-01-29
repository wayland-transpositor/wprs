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

/// ArcSlice allows partitioning a read-only slice into non-overlapping chunks
/// and sharing those chunks between threads.
///
/// Unlike [bytes](https://github.com/tokio-rs/bytes) and
/// [divbuf](https://github.com/asomers/divbuf), ArcSlice supports "adopting" an
/// existing data structure and does not need to copy data into itself. Another
/// possible alternative is
/// [owning-ref-rs](https://github.com/Kimundi/owning-ref-rs), but it is
/// unmaintained, has unsafe code, and has known soundness issues. It does have
/// additional features (such as mut support), but we don't need them here.
use std::cmp;
use std::fmt;
use std::ops::Deref;
use std::ops::Range;
use std::sync::Arc;

use wprs_common::utils;

pub struct ArcSlice<T> {
    // Ideally we'd make this generic over Send and Sync (i.e., ArcSlice would
    // be Send + Sync iff data is Send + Sync, but we'd need higher-kinded types
    // to do that correctly without putting the Arc into the generic parameter.
    data: Arc<dyn AsRef<[T]> + Send + Sync>,
    offset: usize,
    len: usize,
}

impl<T> ArcSlice<T> {
    pub fn new_from_arc(data: Arc<dyn AsRef<[T]> + Send + Sync + 'static>) -> Self {
        let len = (*data).as_ref().len();
        Self {
            data,
            offset: 0,
            len,
        }
    }

    pub fn new(data: impl AsRef<[T]> + Send + Sync + 'static) -> Self {
        Self::new_from_arc(Arc::new(data))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// # Panics
    /// If index > len.
    pub fn index(&self, index: Range<usize>) -> Self {
        assert!(index.end <= self.len());
        let mut output = self.clone();
        output.offset += index.start;
        output.len = index.end - index.start;
        output
    }

    /// # Panics
    /// If mid > len.
    pub fn split_at(&self, mid: usize) -> (Self, Self) {
        assert!(mid <= self.len());
        (self.index(0..mid), self.index(mid..self.len()))
    }

    /// # Panics
    /// If chunk_size == 0.
    pub fn chunks(self, chunk_size: usize) -> Chunks<T> {
        assert!(chunk_size != 0);
        Chunks::new(self, chunk_size)
    }

    /// # Panics
    /// If chunk_size == 0.
    pub fn chunks_exact(self, chunk_size: usize) -> (Chunks<T>, Self) {
        assert!(chunk_size != 0);
        let rem_len = self.len() % chunk_size;
        let fst_len = self.len() - rem_len;
        let (fst, snd) = self.split_at(fst_len);
        (Chunks::new(fst, chunk_size), snd)
    }
}

impl<T: fmt::Debug> fmt::Debug for ArcSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T> Clone for ArcSlice<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            offset: self.offset,
            len: self.len,
        }
    }
}

impl<T> AsRef<[T]> for ArcSlice<T> {
    fn as_ref(&self) -> &[T] {
        &(*self.data).as_ref()[self.offset..(self.offset + self.len)]
    }
}

impl<T> Deref for ArcSlice<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_ref()
    }
}

#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct Chunks<T> {
    arc_slice: ArcSlice<T>,
    chunk_size: usize,
}

impl<T> Chunks<T> {
    #[inline]
    fn new(arc_slice: ArcSlice<T>, chunk_size: usize) -> Self {
        assert!(chunk_size != 0);
        Self {
            arc_slice,
            chunk_size,
        }
    }
}

impl<T> Iterator for Chunks<T> {
    type Item = ArcSlice<T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.arc_slice.is_empty() {
            None
        } else {
            let chunk_size = cmp::min(self.arc_slice.len(), self.chunk_size);
            let (fst, snd) = self.arc_slice.split_at(chunk_size);
            self.arc_slice = snd;
            Some(fst)
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = utils::n_chunks(self.arc_slice.len(), self.chunk_size);
        (n, Some(n))
    }
}

impl<T> ExactSizeIterator for Chunks<T> {}
