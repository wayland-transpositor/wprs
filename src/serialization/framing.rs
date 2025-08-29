// Copyright 2025 Google LLC
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

use std::convert::TryInto;
use std::io::Read;
use std::io::Write;
use std::mem;
use std::num::NonZeroUsize;

use rkyv::util::AlignedVec;
use static_assertions::const_assert;

use crate::prelude::*;

const_assert!(mem::size_of::<usize>() >= mem::size_of::<u32>());

pub trait Framed: Sized {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()>;
    fn framed_read<R: Read>(stream: &mut R) -> Result<Self>;
}

impl Framed for u8 {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        stream.write_all(&self.to_be_bytes()).location(loc!())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let mut buf = [0u8; mem::size_of::<Self>()];
        stream.read_exact(&mut buf).location(loc!())?;
        Ok(Self::from_be_bytes(buf))
    }
}

impl Framed for bool {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        (*self as u8).framed_write(stream)
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        Ok(u8::framed_read(stream).location(loc!())? != 0)
    }
}

impl Framed for u32 {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        stream.write_all(&self.to_be_bytes()).location(loc!())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let mut buf = [0u8; mem::size_of::<Self>()];
        stream.read_exact(&mut buf).location(loc!())?;
        Ok(Self::from_be_bytes(buf))
    }
}

impl Framed for usize {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        (*self as u32).framed_write(stream)
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        u32::framed_read(stream).map(|u| u.try_into().unwrap())
    }
}

impl Framed for NonZeroUsize {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        self.get().framed_write(stream)
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        Self::new(
            u32::framed_read(stream)
                .location(loc!())?
                // Asserted at top of file that usize >= u32.
                .try_into()
                .unwrap(),
        )
        .context(loc!(), "data was 0")
    }
}

impl Framed for Vec<u8> {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        self.len().framed_write(stream).location(loc!())?;
        stream.write_all(self).location(loc!())?;
        Ok(())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let len = u32::framed_read(stream).location(loc!())?;
        let mut buf = vec![0; len as usize];
        stream.read_exact(&mut buf).location(loc!())?;
        Ok(buf)
    }
}

impl Framed for AlignedVec {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        self.len().framed_write(stream).location(loc!())?;
        stream.write_all(self).location(loc!())?;
        Ok(())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let len = u32::framed_read(stream).location(loc!())?;
        let mut buf = Self::new();
        buf.resize(len as usize, 0);
        stream.read_exact(&mut buf).location(loc!())?;
        Ok(buf)
    }
}

impl Framed for String {
    fn framed_write<W: Write>(&self, stream: &mut W) -> Result<()> {
        let bytes = self.as_bytes();
        bytes.len().framed_write(stream).location(loc!())?;
        stream.write_all(bytes).location(loc!())?;
        Ok(())
    }

    fn framed_read<R: Read>(stream: &mut R) -> Result<Self> {
        let bytes = Vec::<u8>::framed_read(stream).location(loc!())?;
        Self::from_utf8(bytes).location(loc!())
    }
}
