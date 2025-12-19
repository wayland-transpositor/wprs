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

use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Result;

use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;
#[cfg(any(feature = "server", feature = "wayland-client"))]
use smithay::utils;
#[cfg(any(feature = "server", feature = "wayland-client"))]
use smithay::utils::Coordinate;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct Point<N> {
    pub x: N,
    pub y: N,
}

impl<N> Debug for ArchivedPoint<N>
where
    N: Debug + Archive,
    <N as Archive>::Archived: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_struct("Point")
            .field("x", &self.x)
            .field("y", &self.y)
            .finish()
    }
}

impl<N> From<(N, N)> for Point<N> {
    fn from((x, y): (N, N)) -> Self {
        Self { x, y }
    }
}

impl<N> From<Point<N>> for (N, N) {
    fn from(point: Point<N>) -> Self {
        (point.x, point.y)
    }
}

#[cfg(any(feature = "server", feature = "wayland-client"))]
impl<N, T> From<Point<N>> for utils::Point<N, T> {
    fn from(point: Point<N>) -> Self {
        <(N, N)>::from(point).into()
    }
}

#[cfg(any(feature = "server", feature = "wayland-client"))]
impl<N, T> From<utils::Point<N, T>> for Point<N> {
    fn from(point: utils::Point<N, T>) -> Self {
        <(N, N)>::from(point).into()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct Size<N> {
    pub w: N,
    pub h: N,
}

impl<N> Debug for ArchivedSize<N>
where
    N: Debug + Archive,
    <N as Archive>::Archived: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_struct("Size")
            .field("w", &self.w)
            .field("h", &self.h)
            .finish()
    }
}

impl<N> From<(N, N)> for Size<N> {
    fn from((w, h): (N, N)) -> Self {
        Self { w, h }
    }
}

impl<N> From<Size<N>> for (N, N) {
    fn from(size: Size<N>) -> Self {
        (size.w, size.h)
    }
}

#[cfg(any(feature = "server", feature = "wayland-client"))]
impl<N: Coordinate, T> From<Size<N>> for utils::Size<N, T> {
    fn from(size: Size<N>) -> Self {
        <(N, N)>::from(size).into()
    }
}

#[cfg(any(feature = "server", feature = "wayland-client"))]
impl<N, T> From<utils::Size<N, T>> for Size<N> {
    fn from(size: utils::Size<N, T>) -> Self {
        <(N, N)>::from(size).into()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct Rectangle<N> {
    pub loc: Point<N>,
    pub size: Size<N>,
}

impl<N> Debug for ArchivedRectangle<N>
where
    N: Debug + Archive,
    <N as Archive>::Archived: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_struct("Rectangle")
            .field("loc", &self.loc)
            .field("size", &self.size)
            .finish()
    }
}

impl<N> Rectangle<N> {
    pub fn new(x: N, y: N, w: N, h: N) -> Self {
        Self {
            loc: Point { x, y },
            size: Size { w, h },
        }
    }
}

#[cfg(any(feature = "server", feature = "wayland-client"))]
impl<N, T> From<utils::Rectangle<N, T>> for Rectangle<N> {
    fn from(rect: utils::Rectangle<N, T>) -> Self {
        Self {
            loc: rect.loc.into(),
            size: rect.size.into(),
        }
    }
}
