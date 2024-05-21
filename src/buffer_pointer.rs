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

use std::cmp;
use std::marker::PhantomData;
use std::mem;
use std::ops::Range;

use crate::utils;

/// A slice-like wrapper around a pointer and length.
///
/// The unsafe boundary is at construction: `ptr` must be non-null, aligned, and
/// valid for reads of `len` elements for the lifetime of the BufferPointer.
/// Assuming the pointer isn't {d,r}eallocated by anything else, during the
/// lifetime of BufferPointer, the implemented operations on BufferPointer will
/// be safe.
///
/// Useful when you have a raw pointer that cannot be wrapped in a slice (using
/// slice::from_raw_parts), for example if it points to shared memory that could
/// be mutated during the lifetime of the slice.
#[derive(Debug, Eq, PartialEq)]
pub struct BufferPointer<'a, T: 'a> {
    ptr: *const T,
    len: usize,
    lifetime: PhantomData<&'a T>,
}

impl<'a, T: 'a> Copy for BufferPointer<'a, T> {}

#[allow(clippy::non_canonical_clone_impl)]
impl<'a, T: 'a> Clone for BufferPointer<'a, T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            len: self.len,
            lifetime: PhantomData,
        }
    }
}

impl<'a, T: 'a> BufferPointer<'a, T> {
    /// # Safety: same as new.
    unsafe fn new_impl(ptr: *const T, len: usize, lifetime: PhantomData<&'a T>) -> Self {
        assert!(!ptr.is_null());
        // TODO(https://github.com/rust-lang/rust/issues/96284): use ptr.is_aligned.
        assert!(ptr.align_offset(mem::align_of::<T>()) == 0);
        Self {
            ptr: ptr.to_owned(),
            len,
            lifetime,
        }
    }

    /// # Safety
    /// * `ptr` must be non-null, aligned, and valid for reads of `len` elements for
    ///   the lifetime of the BufferPointer.
    pub unsafe fn new(ptr: &'a *const T, len: usize) -> Self {
        unsafe { Self::new_impl(ptr.to_owned(), len, PhantomData::<&'a T>) }
    }

    pub fn ptr(self) -> *const T {
        self.ptr
    }

    pub fn len(self) -> usize {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// # Safety
    /// All possible bit patterns of T must be valid bit patterns of U, including any
    /// padding.
    /// # Panics
    /// If the current type and the new type are don't have the same alignment or
    /// if the computer size of the buffer would be different after the cast.
    pub unsafe fn cast<U>(self) -> BufferPointer<'a, U> {
        // TODO(https://github.com/rust-lang/rust/issues/96284): use ptr.is_aligned.
        let new_ptr = self.ptr().cast::<U>();
        assert!(new_ptr.align_offset(mem::align_of::<U>()) == 0);

        let old_size = mem::size_of::<T>();
        let new_size = mem::size_of::<U>();
        let new_len = (self.len() * old_size) / new_size;
        assert!(
            old_size * self.len() == new_size * new_len,
            "old_size = {old_size}, self.len() = {}, new_size = {new_size}, new_len = {new_len}",
            self.len()
        );

        // SAFETY: we verify that the new pointer is aligned and that length in
        // bytes doesn't change.
        unsafe { BufferPointer::new_impl(new_ptr, new_len, PhantomData) }
    }

    /// Splits BufferPointer([0, len]) into BufferPointer([0, mid-1]) and
    /// BufferPointer([mid, end]).
    ///
    /// # Panics
    /// If mid > len.
    pub fn split_at(self, mid: usize) -> (Self, Self) {
        assert!(mid <= self.len());
        // SAFETY: the lengths of the outputs are less then the length of self.
        unsafe {
            (
                BufferPointer::new_impl(self.ptr, mid, self.lifetime),
                BufferPointer::new_impl(self.ptr().add(mid), self.len - mid, self.lifetime),
            )
        }
    }

    /// # Panics
    /// If chunk_size == 0.
    pub fn chunks(self, chunk_size: usize) -> Chunks<'a, T> {
        assert!(chunk_size != 0);
        Chunks::new(self, chunk_size)
    }

    /// # Panics
    /// If chunk_size == 0.
    pub fn chunks_exact(self, chunk_size: usize) -> (Chunks<'a, T>, Self) {
        assert!(chunk_size != 0);
        let rem_len = self.len() % chunk_size;
        let fst_len = self.len() - rem_len;
        let (fst, snd) = self.split_at(fst_len);
        (Chunks::new(fst, chunk_size), snd)
    }

    fn as_ptr_range(&self) -> Range<*const T> {
        // SAFETY: precondition for BufferPointer::new requires self.ptr to be valid
        // for reads of self.len elements.
        self.ptr..unsafe { self.ptr.add(self.len) }
    }

    /// # Panics
    /// If dst.len > len or if dst overlaps with self.
    pub fn copy_to_nonoverlapping(self, dst: &mut [T]) {
        assert!(dst.len() >= self.len());
        assert!(!self.as_ptr_range().contains(&dst.as_ptr()));
        assert!(!dst.as_ptr_range().contains(&self.ptr));

        // SAFETY: precondition for BufferPointer::new requires self.ptr to be
        // valid for reads of self.len elements and we verified that dst.len()
        // >= self.len() and that self and dst are non-overlapping.
        unsafe {
            self.ptr()
                .copy_to_nonoverlapping(dst.as_mut_ptr(), self.len());
        }
    }
}

// SAFETY: pointers are unsafe when they are dereferenced, not created, so it
// makes no sense for pointers to not be Send, but that can't be changed at this
// point for backward compatibility reasons. See
// https://doc.rust-lang.org/nomicon/send-and-sync.html.
unsafe impl<'a, T: 'a> Send for BufferPointer<'a, T> {}

impl<'a, T: 'a> IntoIterator for &'a BufferPointer<'a, T> {
    type Item = T;
    type IntoIter = BufferPointerIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        BufferPointerIter::new(self)
    }
}

#[derive(Debug)]
pub struct BufferPointerIter<'a, T: 'a> {
    ptr: &'a BufferPointer<'a, T>,
    idx: usize,
}

impl<'a, T: 'a> BufferPointerIter<'a, T> {
    fn new(ptr: &'a BufferPointer<'a, T>) -> Self {
        Self { ptr, idx: 0 }
    }
}

impl<T> Iterator for BufferPointerIter<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.idx >= self.ptr.len {
            None
        } else {
            // SAFETY: precondition for BufferPointer::new requires self.ptr to be
            // valid for reads of self.len elements.
            let ret = unsafe { self.ptr.ptr().add(self.idx).read() };
            self.idx += 1;
            Some(ret)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.ptr.len, Some(self.ptr.len))
    }
}

impl<T> ExactSizeIterator for BufferPointerIter<'_, T> {}

#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct Chunks<'a, T: 'a> {
    ptr: BufferPointer<'a, T>,
    chunk_size: usize,
}

impl<'a, T: 'a> Chunks<'a, T> {
    #[inline]
    fn new(ptr: BufferPointer<'a, T>, chunk_size: usize) -> Self {
        assert!(chunk_size != 0);
        Self { ptr, chunk_size }
    }
}

impl<'a, T: 'a> Iterator for Chunks<'a, T> {
    type Item = BufferPointer<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.ptr.len == 0 {
            None
        } else {
            let chunk_size = cmp::min(self.ptr.len, self.chunk_size);
            let (fst, snd) = self.ptr.split_at(chunk_size);
            self.ptr = snd;
            Some(fst)
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = utils::n_chunks(self.ptr.len, self.chunk_size);
        (n, Some(n))
    }
}

impl<'a, T: 'a> ExactSizeIterator for Chunks<'_, T> {}

#[cfg(test)]
mod tests {
    use std::ptr;
    use std::slice;

    use super::*;

    fn test_arr<const LEN: usize>() -> [u32; LEN] {
        (0..(LEN as u32)).collect::<Vec<u32>>().try_into().unwrap()
    }

    #[test]
    #[should_panic]
    fn test_nullptr() {
        let a_ptr: *const u32 = ptr::null();
        let _a_bufptr = unsafe { BufferPointer::new(&a_ptr, 8) };
    }

    #[test]
    fn test_cast_aligned() {
        let a = [0u32; 4];
        let a_ptr = a.as_ptr();
        let bufptr_u32 = unsafe { BufferPointer::new(&a_ptr, a.len()) };
        let _bufptr_u8: BufferPointer<u8> = unsafe { bufptr_u32.cast() };
    }

    #[test]
    #[should_panic]
    fn test_cast_unsaligned() {
        let a = [0u8; 1];
        let a_ptr = a.as_ptr();
        let bufptr_u8 = unsafe { BufferPointer::new(&a_ptr, a.len()) };
        let _bufptr_u32: BufferPointer<u32> = unsafe { bufptr_u8.cast() };
    }

    #[test]
    #[should_panic]
    fn test_cast_unequal_size() {
        let a = [0u8; 5];
        let a_ptr = a.as_ptr();
        let bufptr_struct = unsafe { BufferPointer::new(&a_ptr, a.len()) };
        let _bufptr_u32: BufferPointer<u32> = unsafe { bufptr_struct.cast() };
    }

    #[test]
    fn test_split_at() {
        let a = [0u32; 5];
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };

        let (a_bufptr_0, a_bufptr_1) = a_bufptr.split_at(2);
        assert_eq!(a_bufptr_0.len(), 2);
        assert_eq!(a_bufptr_1.len(), 3);

        let (a_bufptr_00, a_bufptr_01) = a_bufptr_0.split_at(1);
        let (a_bufptr_10, a_bufptr_11) = a_bufptr_1.split_at(1);
        assert_eq!(a_bufptr_00.len(), 1);
        assert_eq!(a_bufptr_01.len(), 1);
        assert_eq!(a_bufptr_10.len(), 1);
        assert_eq!(a_bufptr_11.len(), 2);

        let (a_bufptr_000, a_bufptr_001) = a_bufptr_00.split_at(0);
        assert_eq!(a_bufptr_000.len(), 0);
        assert_eq!(a_bufptr_001.len(), 1);

        let (a_bufptr_100, a_bufptr_101) = a_bufptr_10.split_at(1);
        assert_eq!(a_bufptr_100.len(), 1);
        assert_eq!(a_bufptr_101.len(), 0);
    }

    #[test]
    #[should_panic]
    fn test_split_at_past_end() {
        let a = [0u32; 5];
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };
        let (_a_bufptr_0, _a_bufptr_1) = a_bufptr.split_at(6);
    }

    #[test]
    fn test_chunks() {
        let a: [u32; 5] = test_arr();
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };

        let chunks = a_bufptr.chunks(2);
        assert_eq!(
            chunks.collect::<Vec<_>>(),
            vec![
                unsafe { BufferPointer::new(&a_ptr.offset(0), 2) },
                unsafe { BufferPointer::new(&a_ptr.offset(2), 2) },
                unsafe { BufferPointer::new(&a_ptr.offset(4), 1) },
            ]
        );
    }

    #[test]
    fn test_chunks_exact() {
        let a: [u32; 5] = test_arr();
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };

        let (chunks, rem) = a_bufptr.chunks_exact(2);
        assert_eq!(
            chunks.collect::<Vec<_>>(),
            vec![unsafe { BufferPointer::new(&a_ptr.offset(0), 2) }, unsafe {
                BufferPointer::new(&a_ptr.offset(2), 2)
            },]
        );
        assert_eq!(rem, unsafe { BufferPointer::new(&a_ptr.offset(4), 1) })
    }

    #[test]
    fn test_copy_to_nonoverlapping() {
        let a: [u32; 5] = test_arr();
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };

        let mut b = [0u32; 6];
        a_bufptr.copy_to_nonoverlapping(&mut b);
    }

    #[test]
    #[should_panic]
    fn test_copy_to_nonoverlapping_dst_too_small() {
        let a: [u32; 5] = test_arr();
        let a_ptr = unsafe { a.as_ptr().add(1) };
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };

        let mut b = [0u32; 4];
        a_bufptr.copy_to_nonoverlapping(&mut b);
    }

    #[test]
    #[should_panic]
    fn test_copy_to_nonoverlapping_actually_overlapping() {
        let mut a: [u32; 5] = test_arr();
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len() - 1) };

        let a_subslice = unsafe { slice::from_raw_parts_mut(a.as_mut_ptr().add(1), a.len() - 1) };
        a_bufptr.copy_to_nonoverlapping(a_subslice);
    }

    #[test]
    #[should_panic]
    fn test_copy_to_nonoverlapping_actually_overlapping_other_direction() {
        let mut a: [u32; 5] = test_arr();
        let a_ptr = unsafe { a.as_ptr().add(1) };
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len() - 1) };

        a_bufptr.copy_to_nonoverlapping(&mut a);
    }

    #[test]
    fn test_iter() {
        let a: [u32; 5] = test_arr();
        let a_ptr = a.as_ptr();
        let a_bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };

        let iter = a_bufptr.into_iter();
        assert_eq!(iter.collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
    }
}
