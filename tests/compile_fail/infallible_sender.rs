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

use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;

use wprs::utils::channel::InfallibleSender;

struct ChannelParent(Sender<()>, Receiver<()>);
impl ChannelParent {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self(tx, rx)
    }

    fn sender<'a>(&'a self) -> Sender<()> {
        self.0.clone()
    }

    fn infallible_sender(&self) -> InfallibleSender<Sender<()>> {
        InfallibleSender::new(self.0.clone(), self)
    }
}

fn main() {
    {
        let _sender = {
            let channel_parent = ChannelParent::new();
            channel_parent.sender()
        };
    }

    {
        let channel_parent = ChannelParent::new();
        let _infallible_sender = channel_parent.infallible_sender();
    }

    {
        let _infallible_sender = {
            let channel_parent = ChannelParent::new();
            channel_parent.infallible_sender()
        };
    }
}
