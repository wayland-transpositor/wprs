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

use std::sync::Mutex;
use std::time::Duration;

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::user_data::UserDataMap;
use smithay::wayland::compositor::SurfaceAttributes;
use smithay::wayland::shm;
use smithay::wayland::shm::BufferAccessError;
use smithay::wayland::shm::BufferData;

use crate::buffer_pointer::BufferPointer;
use crate::prelude::*;

/// # Panics
/// If smithay has a bug and with_buffer_contents gives us an invalid pointer.
pub fn with_buffer_contents<F, T>(buffer: &WlBuffer, f: F) -> Result<T, BufferAccessError>
where
    F: FnOnce(BufferPointer<u8>, BufferData) -> T,
{
    shm::with_buffer_contents(buffer, |ptr, len, spec| {
        assert!(!ptr.is_null());
        let start = spec.offset as usize;
        let buffer_len = (spec.height * spec.stride) as usize;
        assert!(
            start + buffer_len <= len,
            "start = {start}, buf_len = {buffer_len}, len = {len}"
        );
        // SAFETY: smithay promises to give us a valid pointer and we check that
        // our calculated start and offset are within the length given by
        // smithay.
        unsafe {
            let ptr = ptr.add(start);
            let buf = BufferPointer::new(&ptr, buffer_len);
            f(buf, spec)
        }
    })
}

// Based on https://github.com/Smithay/smithay/blob/b1c682742ac7b9fa08736476df3e651489709ac2/src/desktop/wayland/utils.rs.
#[derive(Debug, Default)]
struct SurfaceFrameThrottlingState(Mutex<Option<Duration>>);

impl SurfaceFrameThrottlingState {
    pub fn update(&self, time: Duration, throttle: Duration) -> bool {
        let mut guard = self.0.lock().unwrap();
        let send_throttled_frame = guard
            .map(|last| time.saturating_sub(last) > throttle)
            .unwrap_or(true);
        if send_throttled_frame {
            *guard = Some(time);
        }
        send_throttled_frame
    }
}

pub fn send_frames(
    surface: &WlSurface,
    data_map: &UserDataMap,
    surface_attributes: &mut SurfaceAttributes,
    time: Duration,
    throttle: Duration,
) -> Result<()> {
    data_map.insert_if_missing_threadsafe(SurfaceFrameThrottlingState::default);
    let surface_frame_throttling_state = data_map
        .get::<SurfaceFrameThrottlingState>()
        .location(loc!())?;
    let frame_overdue = surface_frame_throttling_state.update(time, throttle);

    if frame_overdue {
        for callback in surface_attributes.frame_callbacks.drain(..) {
            debug!(
                "Sending callback for surface {:?}: {:?}",
                surface.id(),
                callback.id()
            );
            callback.done(time.as_millis() as u32);
        }
    }
    Ok(())
}
