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

use smithay_client_toolkit::data_device_manager::data_device::DataDevice;
use smithay_client_toolkit::primary_selection::device::PrimarySelectionDevice;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;

#[derive(Debug)]
pub(crate) struct SeatObject<P> {
    pub(crate) seat: WlSeat,
    pub(crate) keyboard: Option<WlKeyboard>,
    pub(crate) pointer: Option<P>,
    pub(crate) data_device: DataDevice,
    pub(crate) primary_selection_device: Option<PrimarySelectionDevice>,
}
