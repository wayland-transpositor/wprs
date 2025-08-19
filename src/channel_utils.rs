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
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use smithay::reexports::calloop::channel;

pub trait Sender: Clone {
    type T;
    type E;
    fn send(&self, msg: Self::T) -> Result<(), Self::E>;
}

impl<T> Sender for mpsc::Sender<T> {
    type T = T;
    type E = mpsc::SendError<T>;
    fn send(&self, msg: Self::T) -> Result<(), Self::E> {
        Self::send(self, msg)
    }
}

impl<T> Sender for mpsc::SyncSender<T> {
    type T = T;
    type E = mpsc::SendError<T>;
    fn send(&self, msg: Self::T) -> Result<(), Self::E> {
        Self::send(self, msg)
    }
}

impl<T> Sender for channel::Sender<T> {
    type T = T;
    type E = mpsc::SendError<T>;
    fn send(&self, msg: Self::T) -> Result<(), Self::E> {
        Self::send(self, msg)
    }
}

impl<T> Sender for channel::SyncSender<T> {
    type T = T;
    type E = mpsc::SendError<T>;
    fn send(&self, msg: Self::T) -> Result<(), Self::E> {
        Self::send(self, msg)
    }
}

impl<T> Sender for crossbeam_channel::Sender<T> {
    type T = T;
    type E = crossbeam_channel::SendError<T>;
    fn send(&self, msg: Self::T) -> Result<(), Self::E> {
        Self::send(self, msg)
    }
}

// TODO: ideally you'd be able to use this as DiscardingSender<T> and not care
// about the type of the sender.
pub struct DiscardingSender<S: Sender> {
    pub sender: S,
    pub actually_send: Arc<AtomicBool>,
}

impl<S: Sender + Clone> Clone for DiscardingSender<S> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            actually_send: self.actually_send.clone(),
        }
    }
}

impl<S: Sender> DiscardingSender<S> {
    pub fn send(&self, t: S::T) -> Result<(), S::E> {
        if self.actually_send.load(Ordering::Acquire) {
            self.sender.send(t)
        } else {
            Ok(())
        }
    }
}

impl<S: Sender> Sender for DiscardingSender<S> {
    type T = S::T;
    type E = S::E;
    fn send(&self, msg: Self::T) -> Result<(), Self::E> {
        self.sender.send(msg)
    }
}

/// A sender whose channnel is promised (as opposed to guaranteed) to be open.
///
/// Useful when the lifetime of the sender and receiver (including clones
/// thereof) are known to be the same according to program logic but that can't
/// be proven with lifetimes due to the endpoints being cloned.
///
/// A sender whose channel is guaranteed to be open could be made by storing a
/// clone of the receiver in this struct, but the current implementation is more
/// likely to expose bugs by failing loudly (panicing) instead of failing
/// silently by sending into an empty void (a channel that nothing will read
/// ever read from)
#[derive(Debug, Clone)]
pub struct InfallibleSender<'a, S>(S, PhantomData<&'a ()>)
where
    S: Sender,
    S::E: Debug;

impl<'a, S> InfallibleSender<'a, S>
where
    S: Sender,
    S::E: Debug,
{
    pub fn new<L>(sender: S, _l: &'a L) -> Self {
        Self(sender, PhantomData)
    }

    /// # Panics
    /// If the receiver has actually been dropped, despite the promise to the contrary.
    pub fn send(&self, t: S::T) {
        self.0.send(t).unwrap();
    }

    pub fn into_inner(self) -> S {
        self.0
    }
}
