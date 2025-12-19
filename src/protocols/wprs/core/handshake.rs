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

use std::sync::Arc;

use crate::prelude::*;
use crate::protocols::wprs::Capabilities;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::SendType;
use crate::protocols::wprs::wayland::Buffer;
use crate::protocols::wprs::wayland::BufferAssignment;
use crate::protocols::wprs::wayland::BufferData;
use crate::protocols::wprs::wayland::CompressedBufferData;
use crate::protocols::wprs::wayland::SurfaceState;

fn externalize_compressed_buffer(state: &mut SurfaceState) -> Option<SendType<Request>> {
    let Some(BufferAssignment::New(Buffer { data, .. })) = state.buffer.as_mut() else {
        return None;
    };

    match data {
        BufferData::Compressed(CompressedBufferData(shards)) => {
            let msg = SendType::RawBuffer(Arc::clone(shards));
            *data = BufferData::External;
            Some(msg)
        }
        BufferData::External | BufferData::Uncompressed(_) => None,
    }
}

pub fn surface_messages(state: SurfaceState) -> Result<Vec<SendType<Request>>> {
    let mut out = Vec::new();
    let mut state = state;

    if let Some(raw) = externalize_compressed_buffer(&mut state) {
        out.push(raw);
    }
    out.push(SendType::Object(Request::Surface(
        super::surface_request_from_state(state),
    )));
    Ok(out)
}

pub fn initial_messages(
    xwayland_enabled: bool,
    surfaces: impl IntoIterator<Item = SurfaceState>,
) -> Result<Vec<SendType<Request>>> {
    let mut out = Vec::new();
    out.push(SendType::Object(Request::Capabilities(Capabilities {
        xwayland: xwayland_enabled,
    })));

    for mut surface in surfaces {
        if let Some(raw) = externalize_compressed_buffer(&mut surface) {
            out.push(raw);
        }

        out.extend(surface_messages(surface).location(loc!())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;

    use crate::arc_slice::ArcSlice;
    use crate::sharding_compression::CompressedShards;
    use crate::sharding_compression::ShardingCompressor;
    use crate::protocols::wprs::ClientId;
    use crate::protocols::wprs::wayland::BufferFormat;
    use crate::protocols::wprs::wayland::BufferMetadata;
    use crate::protocols::wprs::wayland::SurfaceRequestPayload;
    use crate::protocols::wprs::wayland::UncompressedBufferData;
    use crate::protocols::wprs::wayland::WlSurfaceId;

    fn make_compressed_shards(payload: &[u8]) -> Arc<CompressedShards> {
        let mut compressor = ShardingCompressor::new(NonZeroUsize::new(1).unwrap(), 1).unwrap();
        Arc::new(compressor.compress(
            NonZeroUsize::new(1).unwrap(),
            ArcSlice::new(payload.to_vec()),
        ))
    }

    fn dummy_surface_state(buffer: Option<BufferAssignment>) -> SurfaceState {
        SurfaceState {
            client: ClientId(1),
            id: WlSurfaceId(2),
            buffer,
            role: None,
            buffer_scale: 1,
            buffer_transform: None,
            opaque_region: None,
            input_region: None,
            z_ordered_children: Vec::new(),
            damage: None,
            output_ids: Vec::new(),
            viewport_state: None,
            xdg_surface_state: None,
        }
    }

    fn dummy_metadata() -> BufferMetadata {
        BufferMetadata {
            width: 1,
            height: 1,
            stride: 4,
            format: BufferFormat::Argb8888,
        }
    }

    fn assert_surface_commit_has_external_buffer(msg: &SendType<Request>) {
        let SendType::Object(Request::Surface(req)) = msg else {
            panic!("expected surface request");
        };

        let SurfaceRequestPayload::Commit(st) = &req.payload else {
            panic!("expected commit");
        };

        let Some(BufferAssignment::New(buf)) = &st.buffer else {
            panic!("expected a buffer assignment");
        };

        assert!(matches!(buf.data, BufferData::External));
    }

    #[test]
    fn initial_messages_externalize_compressed_buffers() {
        let compressed = make_compressed_shards(&[1, 2, 3, 4]);

        let surface = dummy_surface_state(Some(BufferAssignment::New(Buffer {
            metadata: dummy_metadata(),
            data: BufferData::Compressed(CompressedBufferData(compressed)),
        })));

        let msgs = initial_messages(false, [surface]).unwrap();
        assert!(matches!(
            msgs[0],
            SendType::Object(Request::Capabilities(_))
        ));
        assert!(matches!(msgs[1], SendType::RawBuffer(_)));
        assert_surface_commit_has_external_buffer(&msgs[2]);
    }

    #[test]
    fn surface_messages_externalize_compressed_buffers() {
        let compressed = make_compressed_shards(&[1, 2, 3, 4]);

        let surface = dummy_surface_state(Some(BufferAssignment::New(Buffer {
            metadata: dummy_metadata(),
            data: BufferData::Compressed(CompressedBufferData(compressed)),
        })));

        let msgs = surface_messages(surface).unwrap();
        assert!(matches!(msgs[0], SendType::RawBuffer(_)));
        assert_surface_commit_has_external_buffer(&msgs[1]);
    }

    #[test]
    fn initial_messages_without_buffer_sends_only_surface_commit() {
        let surface = dummy_surface_state(None);
        let msgs = initial_messages(false, [surface]).unwrap();

        assert!(matches!(
            msgs.as_slice(),
            [_, SendType::Object(Request::Surface(_))]
        ));
    }

    #[test]
    fn initial_messages_with_removed_buffer_sends_only_surface_commit() {
        let surface = dummy_surface_state(Some(BufferAssignment::Removed));
        let msgs = initial_messages(false, [surface]).unwrap();

        assert!(matches!(
            msgs.as_slice(),
            [_, SendType::Object(Request::Surface(_))]
        ));
    }

    #[test]
    fn initial_messages_with_external_buffer_does_not_send_raw_buffer() {
        let surface = dummy_surface_state(Some(BufferAssignment::New(Buffer {
            metadata: dummy_metadata(),
            data: BufferData::External,
        })));

        let msgs = initial_messages(false, [surface]).unwrap();
        assert!(matches!(
            msgs.as_slice(),
            [_, SendType::Object(Request::Surface(_))]
        ));
    }

    #[test]
    fn initial_messages_multiple_surfaces_preserve_per_surface_order() {
        let compressed1 = make_compressed_shards(&[1, 2, 3, 4]);
        let compressed2 = make_compressed_shards(&[5, 6, 7, 8]);

        let mut s1 = dummy_surface_state(Some(BufferAssignment::New(Buffer {
            metadata: dummy_metadata(),
            data: BufferData::Compressed(CompressedBufferData(compressed1)),
        })));
        s1.id = WlSurfaceId(10);

        let mut s2 = dummy_surface_state(Some(BufferAssignment::New(Buffer {
            metadata: dummy_metadata(),
            data: BufferData::Compressed(CompressedBufferData(compressed2)),
        })));
        s2.id = WlSurfaceId(20);

        let msgs = initial_messages(false, [s1, s2]).unwrap();

        assert!(matches!(msgs[0], SendType::Object(Request::Capabilities(_))));
        assert!(matches!(msgs[1], SendType::RawBuffer(_)));
        assert_surface_commit_has_external_buffer(&msgs[2]);
        assert!(matches!(msgs[3], SendType::RawBuffer(_)));
        assert_surface_commit_has_external_buffer(&msgs[4]);
    }

    #[test]
    fn initial_messages_keep_uncompressed_buffers_inline() {
        let surface = dummy_surface_state(Some(BufferAssignment::New(Buffer {
            metadata: dummy_metadata(),
            data: BufferData::Uncompressed(UncompressedBufferData(
                crate::vec4u8::Vec4u8s::with_total_size(4),
            )),
        })));

        let msgs = initial_messages(false, [surface]).unwrap();
        assert!(matches!(
            msgs.as_slice(),
            [_, SendType::Object(Request::Surface(_))]
        ));
    }
}
