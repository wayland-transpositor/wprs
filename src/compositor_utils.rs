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
use smithay::utils::Buffer as BufferCoords;
use smithay::utils::Logical;
use smithay::utils::Point;
use smithay::utils::Rectangle;
use smithay::utils::Size;
use smithay::utils::Transform;
use smithay::utils::user_data::UserDataMap;
use smithay::wayland::compositor::SurfaceAttributes;
use smithay::wayland::shm;
use smithay::wayland::shm::BufferAccessError;
use smithay::wayland::shm::BufferData;

use crate::buffer_pointer::BufferPointer;
use crate::prelude::*;
use crate::serialization::wayland::OutputInfo;
use crate::serialization::wayland::ViewportState;

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

/// Convert a damage rectangle from surface coordinates to buffer coordinates.
///
/// The viewport is part of this mapping, not just buffer_scale: chromium leaves
/// buffer_scale at 1 and scales entirely through the viewport, so using
/// buffer_scale alone makes this the identity and the damage then covers only
/// the top-left corner of the buffer.
pub fn surface_damage_to_buffer(
    rect: Rectangle<i32, Logical>,
    buffer_scale: i32,
    transform: Transform,
    viewport_state: Option<&ViewportState>,
    buffer_size: Option<(i32, i32)>,
) -> Rectangle<i32, BufferCoords> {
    let src = viewport_state.and_then(|viewport_state| viewport_state.src);
    let dst = viewport_state.and_then(|viewport_state| viewport_state.dst);

    // The whole buffer in surface-local coordinates: to_logical applies
    // buffer_scale and transposes the axes for a rotating transform.
    let buffer_logical_size: Option<Size<f64, Logical>> = buffer_size.map(|(width, height)| {
        Size::<i32, BufferCoords>::from((width, height))
            .to_f64()
            .to_logical(f64::from(buffer_scale.max(1)), transform)
    });

    // The part of it the viewport samples, before the destination scaling.
    let src_size: Size<f64, Logical> = match (src, buffer_logical_size) {
        (Some(src), _) => Size::from((src.size.w, src.size.h)),
        (None, Some(size)) => size,
        // Nothing to scale against.
        (None, None) => return rect.to_buffer(buffer_scale, transform.invert(), &rect.size),
    };

    let (scale_w, scale_h) = match dst {
        Some(dst) if dst.w > 0 && dst.h > 0 => {
            (src_size.w / f64::from(dst.w), src_size.h / f64::from(dst.h))
        },
        _ => (1.0, 1.0),
    };
    let (offset_x, offset_y) = src.map_or((0.0, 0.0), |src| (src.loc.x, src.loc.y));

    let rect = rect.to_f64();
    let surface_local_rect = Rectangle::new(
        Point::from((
            rect.loc.x * scale_w + offset_x,
            rect.loc.y * scale_h + offset_y,
        )),
        Size::from((rect.size.w * scale_w, rect.size.h * scale_h)),
    );

    // The rect spans the whole surface-local space now that src.loc has been
    // added, so a mirroring transform measures against that, not the crop.
    let area_size = buffer_logical_size.unwrap_or(src_size);
    let area = Size::from((area_size.w.ceil() as i32, area_size.h.ceil() as i32));

    // Round outwards: too much damage costs a repaint, too little leaves stale
    // pixels. buffer_transform is what the client already applied to the
    // buffer, so mapping back to buffer coordinates undoes it.
    surface_local_rect
        .to_i32_up::<i32>()
        .to_buffer(buffer_scale, transform.invert(), &area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::geometry;

    fn viewport(src: Option<geometry::Rectangle<f64>>, dst: Option<(i32, i32)>) -> ViewportState {
        ViewportState {
            src,
            dst: dst.map(|(w, h)| geometry::Size { w, h }),
        }
    }

    fn damage(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    fn buffer_rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, BufferCoords> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    /// A surface with no viewport and a buffer matching its size needs no
    /// conversion at all.
    #[test]
    fn identity() {
        assert_eq!(
            surface_damage_to_buffer(
                damage(10, 20, 30, 40),
                1,
                Transform::Normal,
                None,
                Some((100, 100))
            ),
            buffer_rect(10, 20, 30, 40),
        );
    }

    /// Without a buffer there is nothing to scale against, so buffer_scale is
    /// all we can apply.
    #[test]
    fn no_buffer_uses_buffer_scale_only() {
        assert_eq!(
            surface_damage_to_buffer(damage(10, 20, 30, 40), 2, Transform::Normal, None, None),
            buffer_rect(20, 40, 60, 80),
        );
    }

    #[test]
    fn buffer_scale_without_viewport() {
        assert_eq!(
            surface_damage_to_buffer(
                damage(10, 20, 30, 40),
                2,
                Transform::Normal,
                None,
                Some((200, 200))
            ),
            buffer_rect(20, 40, 60, 80),
        );
    }

    /// Regression test for damage covering only the top-left corner of the
    /// buffer. Chromium leaves buffer_scale at 1 and scales through the
    /// viewport instead, so the destination ratio is the only thing that
    /// relates surface coordinates to the 2x buffer.
    #[test]
    fn viewport_destination_scales_damage() {
        let viewport = viewport(None, Some((1733, 729)));
        let buffer = Some((3466, 1458));

        // Full-surface damage has to reach the whole buffer.
        assert_eq!(
            surface_damage_to_buffer(
                damage(0, 0, 1733, 729),
                1,
                Transform::Normal,
                Some(&viewport),
                buffer
            ),
            buffer_rect(0, 0, 3466, 1458),
        );

        // And a partial rect keeps its position relative to the buffer.
        assert_eq!(
            surface_damage_to_buffer(
                damage(16, 153, 1701, 403),
                1,
                Transform::Normal,
                Some(&viewport),
                buffer
            ),
            buffer_rect(32, 306, 3402, 806),
        );
    }

    /// The viewport source rectangle is defined in surface-local coordinates,
    /// so it both sets the pre-destination size and offsets the result.
    #[test]
    fn viewport_source_crops_and_offsets() {
        let viewport = viewport(
            Some(geometry::Rectangle::new(10.0, 10.0, 100.0, 50.0)),
            Some((200, 100)),
        );
        assert_eq!(
            surface_damage_to_buffer(
                damage(0, 0, 200, 100),
                1,
                Transform::Normal,
                Some(&viewport),
                Some((400, 200))
            ),
            buffer_rect(10, 10, 100, 50),
        );
    }

    /// A rotating transform mirrors the rect within the whole buffer, not
    /// within the viewport source rectangle.
    #[test]
    fn viewport_source_with_transform() {
        let viewport = viewport(
            Some(geometry::Rectangle::new(10.0, 10.0, 100.0, 50.0)),
            Some((200, 100)),
        );
        assert_eq!(
            surface_damage_to_buffer(
                damage(0, 0, 200, 100),
                1,
                Transform::_180,
                Some(&viewport),
                Some((400, 200))
            ),
            buffer_rect(290, 140, 100, 50),
        );
    }

    /// Damage must never come out too small: a rect that lands on fractional
    /// coordinates has to grow to cover them.
    #[test]
    fn rounds_outward() {
        let viewport = viewport(None, Some((100, 100)));
        // 3/2 scaling: x 1 -> 1.5 floors to 1, right edge 2 -> 3.0 stays 3.
        assert_eq!(
            surface_damage_to_buffer(
                damage(1, 1, 1, 1),
                1,
                Transform::Normal,
                Some(&viewport),
                Some((150, 150))
            ),
            buffer_rect(1, 1, 2, 2),
        );
    }

    /// Every transform, against the region the compositor actually samples.
    ///
    /// Confirmed against mutter: with any of these wrong the window stops
    /// repainting entirely.
    #[test]
    fn every_transform_maps_to_the_displayed_region() {
        // A 1200x800 buffer shown through a viewport that crops to 560x360 and
        // scales it down to a 600x400 surface.
        let viewport = viewport(
            Some(geometry::Rectangle::new(20.0, 20.0, 560.0, 360.0)),
            Some((600, 400)),
        );
        let buffer = Some((1200, 800));

        let cases = [
            (Transform::Normal, buffer_rect(20, 20, 560, 360)),
            (Transform::_90, buffer_rect(20, 220, 360, 560)),
            (Transform::_180, buffer_rect(620, 420, 560, 360)),
            (Transform::_270, buffer_rect(820, 20, 360, 560)),
            (Transform::Flipped, buffer_rect(620, 20, 560, 360)),
            (Transform::Flipped90, buffer_rect(20, 20, 360, 560)),
            (Transform::Flipped180, buffer_rect(20, 420, 560, 360)),
            (Transform::Flipped270, buffer_rect(820, 220, 360, 560)),
        ];

        for (transform, expected) in cases {
            let damage = surface_damage_to_buffer(
                damage(0, 0, 600, 400),
                1,
                transform,
                Some(&viewport),
                buffer,
            );
            assert_eq!(damage, expected, "{transform:?}");

            // Whatever the transform, the damage has to land inside the buffer.
            assert!(
                damage.loc.x >= 0 && damage.loc.y >= 0,
                "{transform:?} damage starts outside the buffer: {damage:?}"
            );
            assert!(
                damage.loc.x + damage.size.w <= 1200 && damage.loc.y + damage.size.h <= 800,
                "{transform:?} damage runs past the buffer: {damage:?}"
            );
        }
    }
}
