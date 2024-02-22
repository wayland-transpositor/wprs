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

/// vec4 u8 struct-of-array data structures.
use bytemuck::Pod;
use bytemuck::Zeroable;
use itertools::izip;
use rkyv::bytecheck;
use rkyv::with::Raw;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;
use static_assertions::assert_eq_size;

/// Convenience layer for operating on arrays of u8s that represent 4-vectors.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
#[repr(C, packed)]
pub struct Vec4u8(pub u8, pub u8, pub u8, pub u8);

assert_eq_size!(Vec4u8, [u8; 4]);

// SAFETY:
// * is inhabited
// * all bit patterns, including all-zeroes, are valid
// * has no uninit/padding bytes
// * u8 is Pod
// * is repr(C)
// * contains no pointer types or interior mutability
// * a shared reference allows only read-only access
unsafe impl Zeroable for Vec4u8 {}
unsafe impl Pod for Vec4u8 {}

impl Vec4u8 {
    pub fn new() -> Self {
        Self(0, 0, 0, 0)
    }
}

impl Default for Vec4u8 {
    fn default() -> Self {
        Self::new()
    }
}

/// 4-vectors of u8s in struct-of-array format.
///
/// Not interchangable with `Vec<Vec4u8>`, that is in array-of-struct format.
#[derive(Clone, Debug, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct Vec4u8s(#[with(Raw)] Vec<u8>);

impl Vec4u8s {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// # Panics
    /// If len is not a multiple of 4.
    pub fn with_total_size(n: usize) -> Self {
        assert!(n % 4 == 0);
        Self(vec![0; n])
    }

    pub fn len(&self) -> usize {
        self.0.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn resize(&mut self, new_len: usize) {
        self.0.resize(new_len * 4, 0);
    }

    pub fn parts(&self) -> (&[u8], &[u8], &[u8], &[u8]) {
        let len = self.len();
        let (part0, rest) = self.0.split_at(len);
        let (part1, rest) = rest.split_at(len);
        let (part2, part3) = rest.split_at(len);
        (part0, part1, part2, part3)
    }

    pub fn parts_mut(&mut self) -> (&mut [u8], &mut [u8], &mut [u8], &mut [u8]) {
        let len = self.len();
        let (part0, rest) = self.0.split_at_mut(len);
        let (part1, rest) = rest.split_at_mut(len);
        let (part2, part3) = rest.split_at_mut(len);
        (part0, part1, part2, part3)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u8, &u8, &u8, &u8)> {
        let (p0, p1, p2, p3) = self.parts();
        izip!(p0, p1, p2, p3)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&mut u8, &mut u8, &mut u8, &mut u8)> {
        let (p0, p1, p2, p3) = self.parts_mut();
        izip!(p0, p1, p2, p3)
    }

    pub fn chunks(&self, n: usize) -> impl Iterator<Item = (&[u8], &[u8], &[u8], &[u8])> {
        let (p0, p1, p2, p3) = self.parts();
        izip!(p0.chunks(n), p1.chunks(n), p2.chunks(n), p3.chunks(n))
    }

    pub fn chunks_mut(
        &mut self,
        n: usize,
    ) -> impl Iterator<Item = (&mut [u8], &mut [u8], &mut [u8], &mut [u8])> {
        let (p0, p1, p2, p3) = self.parts_mut();
        izip!(
            p0.chunks_mut(n),
            p1.chunks_mut(n),
            p2.chunks_mut(n),
            p3.chunks_mut(n)
        )
    }
}

impl Default for Vec4u8s {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec4u8s> for Vec<u8> {
    fn from(vec4u8: Vec4u8s) -> Self {
        vec4u8.0
    }
}

impl From<Vec<u8>> for Vec4u8s {
    fn from(vec: Vec<u8>) -> Self {
        assert!(vec.len() % 4 == 0);
        Self(vec)
    }
}

impl AsRef<[u8]> for Vec4u8s {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_pointer::BufferPointer;

    // The main value from these two tests is running the cast code through miri.
    #[test]
    fn test_cast_vec4u8_to_u8() {
        let v: Vec<Vec4u8> = vec![
            Vec4u8(0, 0, 0, 0),
            Vec4u8(1, 1, 1, 1),
            Vec4u8(2, 2, 2, 2),
            Vec4u8(3, 3, 3, 3),
        ];
        let v_ptr = v.as_ptr();
        let v_bufptr = unsafe { BufferPointer::new(&v_ptr, v.len()) };
        let v_u8 = unsafe { v_bufptr.cast::<u8>() };
        assert_eq!(
            v_u8.into_iter().collect::<Vec<u8>>(),
            vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3]
        );
    }

    #[test]
    fn test_cast_u8_to_vec4u8() {
        let v: Vec<u8> = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3];
        let v_ptr = v.as_ptr();
        let v_bufptr = unsafe { BufferPointer::new(&v_ptr, v.len()) };
        let v_vec4u8 = unsafe { v_bufptr.cast::<Vec4u8>() };

        assert_eq!(
            v_vec4u8.into_iter().collect::<Vec<Vec4u8>>(),
            vec![
                Vec4u8(0, 0, 0, 0),
                Vec4u8(1, 1, 1, 1),
                Vec4u8(2, 2, 2, 2),
                Vec4u8(3, 3, 3, 3)
            ]
        );
    }

    #[test]
    fn test_vec4u8s_with_total_size() {
        assert_eq!(Vec4u8s::with_total_size(0).len(), 0);
        assert_eq!(Vec4u8s::with_total_size(4).len(), 1);
        assert_eq!(Vec4u8s::with_total_size(128).len(), 32);
    }

    #[test]
    #[should_panic]
    fn test_vec4u8s_with_total_size_invalid_size() {
        Vec4u8s::with_total_size(5);
    }

    #[test]
    fn test_vec4u8s_from_vec_u8() {
        let v: Vec<u8> = Vec::new();
        let vs: Vec4u8s = v.into();
        assert_eq!(vs.len(), 0);

        let v: Vec<u8> = vec![1, 2, 3, 4];
        let vs: Vec4u8s = v.into();
        assert_eq!(vs.len(), 1);

        let v: Vec<u8> = vec![0; 128];
        let vs: Vec4u8s = v.into();
        assert_eq!(vs.len(), 32);
    }

    #[test]
    #[should_panic]
    fn test_vec4u8s_from_vec_u8_invalid_size() {
        let v: Vec<u8> = vec![1, 2, 3, 4, 5];
        let _: Vec4u8s = v.into();
    }

    #[test]
    fn test_vec4u8s_resize() {
        let mut v = Vec4u8s::with_total_size(16);
        assert_eq!(v.len(), 4);

        v.resize(32);
        assert_eq!(v.len(), 32);

        v.resize(0);
        assert_eq!(v.len(), 0);

        v.resize(64);
        assert_eq!(v.len(), 64);
    }

    #[test]
    fn test_vec4u8s_parts() {
        let v: Vec4u8s = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        let (part0, part1, part2, part3) = v.parts();

        assert_eq!(part0, vec![0, 0, 0, 0]);
        assert_eq!(part1, vec![1, 1, 1, 1]);
        assert_eq!(part2, vec![2, 2, 2, 2]);
        assert_eq!(part3, vec![3, 3, 3, 3]);
    }

    #[test]
    fn test_vec4u8s_parts_mut() {
        let mut v: Vec4u8s = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        let (part0, part1, part2, part3) = v.parts_mut();

        part1.copy_from_slice(&[5, 6, 7, 8]);
        part3.copy_from_slice(&[8, 7, 6, 5]);

        assert_eq!(part0, vec![0, 0, 0, 0]);
        assert_eq!(part1, vec![5, 6, 7, 8]);
        assert_eq!(part2, vec![2, 2, 2, 2]);
        assert_eq!(part3, vec![8, 7, 6, 5]);
    }

    #[test]
    fn test_vec4u8s_iter() {
        let v: Vec4u8s = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        let vi: Vec<(&u8, &u8, &u8, &u8)> = v.iter().collect();

        assert_eq!(
            vi,
            vec![
                (&0, &1, &2, &3),
                (&0, &1, &2, &3),
                (&0, &1, &2, &3),
                (&0, &1, &2, &3)
            ]
        );
    }

    #[test]
    fn test_vec4u8s_iter_mut() {
        let mut v: Vec4u8s = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        let mut vi: Vec<(&mut u8, &mut u8, &mut u8, &mut u8)> = v.iter_mut().collect();

        *vi[1].0 = 5;
        *vi[1].1 = 6;
        *vi[1].2 = 7;
        *vi[1].3 = 8;

        assert_eq!(
            vi,
            vec![
                (&mut 0, &mut 1, &mut 2, &mut 3),
                (&mut 5, &mut 6, &mut 7, &mut 8),
                (&mut 0, &mut 1, &mut 2, &mut 3),
                (&mut 0, &mut 1, &mut 2, &mut 3)
            ]
        );
    }

    #[test]
    fn test_vec4u8s_chunks() {
        let v: Vec4u8s = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        let mut chunks = v.chunks(2);
        let (chunks0, chunks1, chunks2, chunks3): (&[u8], &[u8], &[u8], &[u8]) =
            chunks.next().unwrap();
        assert_eq!(chunks0, vec![0, 0]);
        assert_eq!(chunks1, vec![1, 1]);
        assert_eq!(chunks2, vec![2, 2]);
        assert_eq!(chunks3, vec![3, 3]);

        let (chunks0, chunks1, chunks2, chunks3): (&[u8], &[u8], &[u8], &[u8]) =
            chunks.next().unwrap();
        assert_eq!(chunks0, vec![0, 0]);
        assert_eq!(chunks1, vec![1, 1]);
        assert_eq!(chunks2, vec![2, 2]);
        assert_eq!(chunks3, vec![3, 3]);
    }

    #[test]
    fn test_vec4u8s_chunks_mut() {
        let mut v: Vec4u8s = vec![0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        {
            let mut chunks = v.chunks_mut(2);
            let (_, chunks1, _, _): (&mut [u8], &mut [u8], &mut [u8], &mut [u8]) =
                chunks.next().unwrap();
            chunks1.copy_from_slice(&[5, 6]);
        }

        let v2: Vec4u8s = vec![0, 0, 0, 0, 5, 6, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3].into();

        assert_eq!(v, v2);
    }
}
