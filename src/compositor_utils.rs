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

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use smithay::output::Mode;
use smithay::output::Output;
use smithay::output::Scale;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::user_data::UserDataMap;
use smithay::wayland::compositor::SurfaceAttributes;
use smithay::wayland::shm;
use smithay::wayland::shm::BufferAccessError;
use smithay::wayland::shm::BufferData;

use crate::buffer_pointer::BufferPointer;
use crate::prelude::*;
use crate::serialization::wayland::OutputInfo;

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
pub(crate) struct SurfaceFrameThrottlingState(Mutex<Option<Duration>>);

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

pub fn update_output(local_output: &mut Output, output: OutputInfo) {
    let current_mode = local_output.current_mode().unwrap_or(Mode {
        size: (0, 0).into(),
        refresh: 0,
    });
    let received_mode = Mode {
        size: output.mode.dimensions.into(),
        refresh: output.mode.refresh_rate,
    };
    if current_mode != received_mode {
        local_output.delete_mode(current_mode);
    }

    local_output.change_current_state(
        Some(received_mode),
        Some(output.transform.into()),
        Some(Scale::Integer(output.scale_factor)),
        Some(output.location.into()),
    );

    if output.mode.preferred {
        local_output.set_preferred(received_mode);
    }
}

pub fn update_surface_outputs<'a, F>(
    surface: &WlSurface,
    new_ids: &HashSet<u32>,
    old_ids: &HashSet<u32>,
    output_accessor: F,
) where
    F: Fn(&u32) -> Option<&'a Output>,
{
    let entered_ids = new_ids.difference(old_ids);
    let left_ids = old_ids.difference(new_ids);

    // careful, a surface can be on multiple outputs, and the surface scale is the largest scale among them
    for id in entered_ids {
        let output = output_accessor(id);
        if let Some(output) = output {
            output.enter(surface);
        }
    }

    for id in left_ids {
        let output = output_accessor(id);
        if let Some(output) = output {
            output.leave(surface);
        }
    }
}
