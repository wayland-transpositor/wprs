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

use std::fmt;
use std::fmt::Debug;
use std::num::NonZeroU32;
use std::sync::Arc;

#[cfg(any(feature = "server", feature = "wayland-client"))]
use anyhow::Error;
use enum_as_inner::EnumAsInner;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;
#[cfg(feature = "server")]
use smithay::backend::input::AxisSource as SmithayAxisSource;
#[cfg(feature = "server")]
use smithay::output::Subpixel as SmithaySubpixel;
#[cfg(feature = "server")]
use smithay::reexports::wayland_server::Resource;
#[cfg(feature = "server")]
use smithay::reexports::wayland_server::backend;
#[cfg(feature = "server")]
use smithay::reexports::wayland_server::protocol::wl_output::Transform as SmithayWlTransform;
#[cfg(feature = "server")]
use smithay::reexports::wayland_server::protocol::wl_shm::Format as SmithayBufferFormat;
#[cfg(feature = "server")]
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
#[cfg(feature = "server")]
use smithay::utils::Transform as SmithayTransform;
#[cfg(feature = "server")]
use smithay::wayland::compositor::RectangleKind as SmithayRectangleKind;
#[cfg(feature = "server")]
use smithay::wayland::compositor::RegionAttributes;
#[cfg(feature = "server")]
use smithay::wayland::selection::data_device::SourceMetadata as SmithaySourceMetadata;
#[cfg(feature = "server")]
use smithay::wayland::shm::BufferData as SmithayBufferData;
#[cfg(feature = "server")]
use smithay::wayland::viewporter::ViewportCachedState;

#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::compositor::CompositorState;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::compositor::Region as SctkRegion;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::output::Mode as SctkMode;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::output::OutputInfo as SctkOutputInfo;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction as SctkWlDndAction;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::reexports::client::protocol::wl_output::Subpixel as SctkSubpixel;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::reexports::client::protocol::wl_output::Transform as SctkTransform;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::AxisSource as SctkAxisSource;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::reexports::client::protocol::wl_shm::Format as SctkBufferFormat;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::seat::keyboard::Modifiers as SmithayModifiers;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::seat::keyboard::RepeatInfo as SctkRepeatInfo;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::seat::pointer::AxisScroll as SctkAxisScroll;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::seat::pointer::PointerEvent as SctkPointerEvent;
#[cfg(feature = "wayland-client")]
use smithay_client_toolkit::seat::pointer::PointerEventKind as SctkPointerEventKind;

use super::ClientId;
use super::geometry::Point;
use super::geometry::Rectangle;
use super::geometry::Size;
use super::tuple::Tuple2;
use super::xdg_shell;
#[cfg(feature = "server")]
use crate::buffer_pointer::BufferPointer;
use crate::config;
#[cfg(feature = "server")]
use crate::filtering;
use crate::prelude::*;
use crate::sharding_compression::CompressedShards;
#[cfg(feature = "server")]
use crate::sharding_compression::ShardingCompressor;
use crate::vec4u8::Vec4u8s;

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct WlSurfaceId(pub u64);

impl WlSurfaceId {
    #[cfg(feature = "server")]
    pub fn new(wl_surface: &WlSurface) -> Self {
        Self(super::hash(&wl_surface.id()))
    }
}

#[cfg(feature = "server")]
impl From<&backend::ObjectId> for WlSurfaceId {
    fn from(object_id: &backend::ObjectId) -> Self {
        Self(super::hash(object_id))
    }
}

// TODO: consider removing
#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct ClientSurface {
    pub client: ClientId,
    pub surface: WlSurfaceId,
}

impl ClientSurface {
    #[cfg(feature = "server")]
    pub fn new(wl_surface: &WlSurface) -> Result<Self> {
        Ok(Self {
            client: ClientId::new(&wl_surface.client().location(loc!())?),
            surface: WlSurfaceId::new(wl_surface),
        })
    }
}

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct SubSurfaceId(pub u64);

impl SubSurfaceId {
    pub fn new(subsurface_id: &WlSurfaceId) -> Self {
        Self(super::hash(&subsurface_id))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, EnumAsInner, Archive, Deserialize, Serialize)]
pub enum BufferFormat {
    Argb8888,
    Xrgb8888,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct BufferMetadata {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format: BufferFormat,
}

#[cfg(feature = "server")]
impl TryFrom<SmithayBufferFormat> for BufferFormat {
    type Error = Error;
    fn try_from(format: SmithayBufferFormat) -> Result<Self> {
        match format {
            SmithayBufferFormat::Argb8888 => Ok(Self::Argb8888),
            SmithayBufferFormat::Xrgb8888 => Ok(Self::Xrgb8888),
            _ => bail!("invalid buffer format {:?}", format),
        }
    }
}

#[cfg(feature = "wayland-client")]
impl TryFrom<SctkBufferFormat> for BufferFormat {
    type Error = Error;
    fn try_from(format: SctkBufferFormat) -> Result<Self> {
        match format {
            SctkBufferFormat::Argb8888 => Ok(Self::Argb8888),
            SctkBufferFormat::Xrgb8888 => Ok(Self::Xrgb8888),
            _ => bail!("invalid buffer format {:?}", format),
        }
    }
}

#[cfg(feature = "wayland-client")]
impl From<BufferFormat> for SctkBufferFormat {
    fn from(format: BufferFormat) -> Self {
        match format {
            BufferFormat::Argb8888 => Self::Argb8888,
            BufferFormat::Xrgb8888 => Self::Xrgb8888,
        }
    }
}

impl BufferMetadata {
    // TODO: replace with impl From
    #[cfg(feature = "server")]
    pub fn from_buffer_data(spec: &SmithayBufferData) -> Result<Self> {
        Ok(Self {
            width: spec.width,
            height: spec.height,
            stride: spec.stride,
            format: spec.format.try_into().location(loc!())?,
        })
    }

    pub fn pixel_bytes(&self) -> i32 {
        self.stride / self.width
    }

    pub fn len(&self) -> usize {
        (self.height * self.stride) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct UncompressedBufferData(pub Vec4u8s);

impl fmt::Debug for UncompressedBufferData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UncompressedBufferData")
            .field(&format_args!("Vec4u8s[{}]", self.0.len()))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct CompressedBufferData(pub Arc<CompressedShards>);

impl fmt::Debug for CompressedBufferData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CompressedBufferData")
            .field(&format_args!(
                "CompressedShards[{:?}]",
                self.0.uncompressed_size()
            ))
            .finish()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, EnumAsInner, Archive, Deserialize, Serialize)]
pub enum BufferData {
    External,
    Uncompressed(UncompressedBufferData),
    Compressed(CompressedBufferData),
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct Buffer {
    pub metadata: BufferMetadata,
    pub data: BufferData,
}

impl Buffer {
    #[cfg(feature = "server")]
    pub fn new(
        metadata: &SmithayBufferData,
        data: BufferPointer<u8>,
        compressor: &mut ShardingCompressor,
    ) -> Result<Self> {
        let metadata = BufferMetadata::from_buffer_data(metadata).location(loc!())?;
        let compressed_data = BufferData::Compressed(CompressedBufferData(Arc::new(
            filtering::filter_and_compress(data, compressor),
        )));
        debug!(
            "New Buffer: size {:?}, width {:?}, height {:?}, stride {:?}, data {:?} ",
            &data.len(),
            metadata.width,
            metadata.height,
            metadata.stride,
            compressed_data,
        );
        Ok(Self {
            metadata,
            data: compressed_data,
        })
    }

    #[allow(clippy::missing_panics_doc)]
    #[cfg(feature = "server")]
    pub fn update(
        &mut self,
        metadata: &SmithayBufferData,
        data: BufferPointer<u8>,
        compressor: &mut ShardingCompressor,
    ) -> Result<()> {
        self.metadata = BufferMetadata::from_buffer_data(metadata).location(loc!())?;
        self.data = BufferData::Compressed(CompressedBufferData(Arc::new(
            filtering::filter_and_compress(data, compressor),
        )));
        Ok(())
    }
}

// TODO: consider splitting SurfaceState, this only really makes sense for the
// surface state we're sending, not the one we're storing.
#[derive(Debug, Clone, Eq, PartialEq, EnumAsInner, Archive, Deserialize, Serialize)]
pub enum BufferAssignment {
    New(Buffer),
    Removed,
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum CursorImageStatus {
    Hidden,
    Named(String),
    Surface {
        client_surface: ClientSurface,
        hotspot: Point<i32>,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct CursorImage {
    pub serial: u32,
    pub status: CursorImageStatus,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum KeyState {
    Released,
    Pressed,
    Repeated,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum RepeatInfo {
    Repeat { rate: NonZeroU32, delay: u32 },
    Disable,
}

#[cfg(feature = "wayland-client")]
impl From<SctkRepeatInfo> for RepeatInfo {
    fn from(info: SctkRepeatInfo) -> Self {
        match info {
            SctkRepeatInfo::Repeat { rate, delay } => Self::Repeat { rate, delay },
            SctkRepeatInfo::Disable => Self::Disable,
        }
    }
}

// Make this a separate struct so we can override debug just for this variant instead of the entire enum.
#[derive(Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct KeyInner {
    pub serial: u32,
    pub raw_code: u32,
    pub state: KeyState,
}

impl fmt::Debug for KeyInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("serial", &self.serial)
            .field(
                "raw_code",
                if config::get_log_priv_data() {
                    &self.raw_code
                } else {
                    &"<redacted>"
                },
            )
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub caps_lock: bool,
    pub logo: bool,
    pub num_lock: bool,
}

#[cfg(feature = "wayland-client")]
impl From<SmithayModifiers> for ModifierState {
    fn from(modifiers: SmithayModifiers) -> Self {
        Self {
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            caps_lock: modifiers.caps_lock,
            logo: modifiers.logo,
            num_lock: modifiers.num_lock,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum KeyboardEvent {
    Enter {
        serial: u32,
        surface_id: WlSurfaceId,
        keycodes: Vec<u32>,
        keysyms: Vec<u32>,
    },
    Leave {
        serial: u32,
    },
    Key(KeyInner),
    RepeatInfo(RepeatInfo),
    Keymap(String),
    Modifiers {
        modifier_state: ModifierState,
        layout_index: u32,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub struct AxisScroll {
    pub absolute: f64,
    pub discrete: i32,
    pub stop: bool,
}

#[cfg(feature = "wayland-client")]
impl From<SctkAxisScroll> for AxisScroll {
    fn from(axis_scroll: SctkAxisScroll) -> Self {
        Self {
            absolute: axis_scroll.absolute,
            discrete: axis_scroll.discrete,
            stop: axis_scroll.stop,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum AxisSource {
    Finger,
    Continuous,
    Wheel,
    WheelTilt,
}

#[cfg(feature = "wayland-client")]
impl From<SctkAxisSource> for AxisSource {
    fn from(axis_source: SctkAxisSource) -> Self {
        match axis_source {
            SctkAxisSource::Wheel => Self::Wheel,
            SctkAxisSource::Finger => Self::Finger,
            SctkAxisSource::Continuous => Self::Continuous,
            SctkAxisSource::WheelTilt => Self::WheelTilt,
            _ => unreachable!(), // TODO: error message
        }
    }
}

#[cfg(feature = "server")]
impl From<AxisSource> for SmithayAxisSource {
    fn from(axis_source: AxisSource) -> Self {
        match axis_source {
            AxisSource::Wheel => Self::Wheel,
            AxisSource::Finger => Self::Finger,
            AxisSource::Continuous => Self::Continuous,
            AxisSource::WheelTilt => Self::WheelTilt,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub enum PointerEventKind {
    Enter {
        serial: u32,
    },
    Leave {
        serial: u32,
    },
    Motion,
    Press {
        button: u32,
        serial: u32,
    },
    Release {
        button: u32,
        serial: u32,
    },
    Axis {
        horizontal: AxisScroll,
        vertical: AxisScroll,
        source: Option<AxisSource>,
    },
}

#[cfg(feature = "wayland-client")]
impl From<SctkPointerEventKind> for PointerEventKind {
    fn from(event: SctkPointerEventKind) -> Self {
        match event {
            SctkPointerEventKind::Enter { serial } => Self::Enter { serial },
            SctkPointerEventKind::Leave { serial } => Self::Leave { serial },
            SctkPointerEventKind::Motion { time: _ } => Self::Motion,
            SctkPointerEventKind::Press {
                time: _,
                button,
                serial,
            } => Self::Press { button, serial },
            SctkPointerEventKind::Release {
                time: _,
                button,
                serial,
            } => Self::Release { button, serial },
            SctkPointerEventKind::Axis {
                time: _,
                horizontal,
                vertical,
                source,
            } => Self::Axis {
                horizontal: horizontal.into(),
                vertical: vertical.into(),
                source: source.map(Into::into),
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub struct PointerEvent {
    pub surface_id: WlSurfaceId,
    pub position: Point<f64>,
    pub kind: PointerEventKind,
}

impl PointerEvent {
    #[cfg(feature = "wayland-client")]
    pub fn from_smithay(surface_id: &WlSurfaceId, event: &SctkPointerEvent) -> Self {
        Self {
            surface_id: *surface_id,
            position: event.position.into(),
            kind: event.kind.clone().into(),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct SubSurfaceState {
    pub parent: WlSurfaceId,
    pub location: Point<i32>,
    pub sync: bool,
}

impl SubSurfaceState {
    #[cfg(feature = "server")]
    pub fn new(parent: &WlSurface) -> Self {
        Self {
            parent: WlSurfaceId::new(parent),
            location: (0, 0).into(),
            sync: true,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, EnumAsInner, Archive, Deserialize, Serialize)]
pub enum Role {
    Cursor(Point<i32>),
    SubSurface(SubSurfaceState),
    XdgToplevel(xdg_shell::XdgToplevelState),
    XdgPopup(xdg_shell::XdgPopupState),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum RectangleKind {
    Add,
    Subtract,
}

#[cfg(feature = "server")]
impl From<&SmithayRectangleKind> for RectangleKind {
    fn from(kind: &SmithayRectangleKind) -> Self {
        match kind {
            SmithayRectangleKind::Add => Self::Add,
            SmithayRectangleKind::Subtract => Self::Subtract,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct Region {
    rects: Vec<Tuple2<RectangleKind, Rectangle<i32>>>,
}

#[cfg(feature = "server")]
impl From<&RegionAttributes> for Region {
    fn from(region: &RegionAttributes) -> Self {
        Self {
            rects: region
                .rects
                .iter()
                .map(|(kind, rect)| (kind.into(), (*rect).into()).into())
                .collect(),
        }
    }
}

impl Region {
    pub fn new() -> Self {
        Self { rects: Vec::new() }
    }

    #[cfg(feature = "wayland-client")]
    pub fn create_compositor_region(
        &self,
        compositor_state: &CompositorState,
    ) -> Result<SctkRegion> {
        let region = SctkRegion::new(compositor_state).location(loc!())?;
        self.rects.iter().for_each(|rect| match rect.0 {
            RectangleKind::Add => {
                region.add(rect.1.loc.x, rect.1.loc.y, rect.1.size.w, rect.1.size.h);
            },
            RectangleKind::Subtract => {
                region.subtract(rect.1.loc.x, rect.1.loc.y, rect.1.size.w, rect.1.size.h);
            },
        });
        Ok(region)
    }
}

impl Default for Region {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum Transform {
    Normal,
    _90,
    _180,
    _270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[cfg(feature = "wayland-client")]
impl From<SctkTransform> for Transform {
    fn from(transform: SctkTransform) -> Self {
        match transform {
            SctkTransform::Normal => Self::Normal,
            SctkTransform::_90 => Self::_90,
            SctkTransform::_180 => Self::_180,
            SctkTransform::_270 => Self::_270,
            SctkTransform::Flipped => Self::Flipped,
            SctkTransform::Flipped90 => Self::Flipped90,
            SctkTransform::Flipped180 => Self::Flipped180,
            SctkTransform::Flipped270 => Self::Flipped270,
            _ => {
                warn!("Unknown transformation {transform:?}, using Normal instead.");
                Self::Normal
            },
        }
    }
}

#[cfg(feature = "wayland-client")]
impl From<Transform> for SctkTransform {
    fn from(transform: Transform) -> Self {
        match transform {
            Transform::Normal => Self::Normal,
            Transform::_90 => Self::_90,
            Transform::_180 => Self::_180,
            Transform::_270 => Self::_270,
            Transform::Flipped => Self::Flipped,
            Transform::Flipped90 => Self::Flipped90,
            Transform::Flipped180 => Self::Flipped180,
            Transform::Flipped270 => Self::Flipped270,
        }
    }
}

#[cfg(feature = "server")]
impl From<SmithayTransform> for Transform {
    fn from(transform: SmithayTransform) -> Self {
        match transform {
            SmithayTransform::Normal => Self::Normal,
            SmithayTransform::_90 => Self::_90,
            SmithayTransform::_180 => Self::_180,
            SmithayTransform::_270 => Self::_270,
            SmithayTransform::Flipped => Self::Flipped,
            SmithayTransform::Flipped90 => Self::Flipped90,
            SmithayTransform::Flipped180 => Self::Flipped180,
            SmithayTransform::Flipped270 => Self::Flipped270,
        }
    }
}

#[cfg(feature = "server")]
impl From<SmithayWlTransform> for Transform {
    fn from(transform: SmithayWlTransform) -> Self {
        match transform {
            SmithayWlTransform::Normal => Self::Normal,
            SmithayWlTransform::_90 => Self::_90,
            SmithayWlTransform::_180 => Self::_180,
            SmithayWlTransform::_270 => Self::_270,
            SmithayWlTransform::Flipped => Self::Flipped,
            SmithayWlTransform::Flipped90 => Self::Flipped90,
            SmithayWlTransform::Flipped180 => Self::Flipped180,
            SmithayWlTransform::Flipped270 => Self::Flipped270,
            _ => {
                warn!("Unknown transformation {transform:?}, using Normal instead.");
                Self::Normal
            },
        }
    }
}

#[cfg(feature = "server")]
impl From<Transform> for SmithayTransform {
    fn from(transform: Transform) -> Self {
        match transform {
            Transform::Normal => Self::Normal,
            Transform::_90 => Self::_90,
            Transform::_180 => Self::_180,
            Transform::_270 => Self::_270,
            Transform::Flipped => Self::Flipped,
            Transform::Flipped90 => Self::Flipped90,
            Transform::Flipped180 => Self::Flipped180,
            Transform::Flipped270 => Self::Flipped270,
        }
    }
}

/// An entry for a vector of child surfaces. The (x, y) position is stored
/// explicitly, the z position (stacking order) is stored implicitly based on
/// the index of the item in the vector.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct SubsurfacePosition {
    pub id: WlSurfaceId,
    pub position: Point<i32>,
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub struct SurfaceState {
    pub client: ClientId,
    pub id: WlSurfaceId,
    pub buffer: Option<BufferAssignment>,
    pub role: Option<Role>,
    // TODO: include buffer_delta, transform from SurfaceAttributes
    pub buffer_scale: i32,
    pub buffer_transform: Option<Transform>,
    pub opaque_region: Option<Region>,
    pub input_region: Option<Region>,
    pub z_ordered_children: Vec<SubsurfacePosition>,
    pub damage: Option<Vec<Rectangle<i32>>>,
    // server-side only
    pub output_ids: Vec<u32>,
    pub viewport_state: Option<ViewportState>,

    // Interfaces
    pub xdg_surface_state: Option<xdg_shell::XdgSurfaceState>,
}

impl SurfaceState {
    #[cfg(feature = "server")]
    pub fn new(surface: &WlSurface, buffer: Option<BufferAssignment>) -> Result<Self> {
        Ok(Self {
            client: ClientId::new(&surface.client().location(loc!())?),
            id: WlSurfaceId::new(surface),
            buffer,
            role: None,
            buffer_scale: 1,
            buffer_transform: None,
            opaque_region: None,
            input_region: None,
            // TODO: insert own id into z_ordered_children after figuring out
            // client isolation.
            z_ordered_children: Vec::new(),
            damage: None,
            output_ids: Vec::new(),
            viewport_state: None,
            xdg_surface_state: None,
        })
    }

    #[instrument(skip(data, compressor), level = "debug")]
    #[cfg(feature = "server")]
    pub fn set_buffer(
        &mut self,
        metadata: &SmithayBufferData,
        data: BufferPointer<u8>,
        compressor: &mut ShardingCompressor,
    ) -> Result<()> {
        match &mut self.buffer {
            // Only buffer data was updated, we can reuse the buffer.
            Some(BufferAssignment::New(buffer)) => {
                buffer.update(metadata, data, compressor).location(loc!())?;
            },
            Some(BufferAssignment::Removed) | None => {
                self.buffer = Some(BufferAssignment::New(
                    Buffer::new(metadata, data, compressor).location(loc!())?,
                ));
            },
        }
        Ok(())
    }

    #[cfg(feature = "server")]
    pub fn update_with_external_buffer(
        &mut self,
        buffer: &Option<BufferAssignment>,
    ) -> Result<Arc<CompressedShards>> {
        self.buffer.clone_from(buffer);
        // set_buffer (found above) sets buffer to
        // Some(BufferAssignment::New(...)), so the 4 unwraps below should
        // never fail.

        let raw_buffer_to_send = buffer
            .as_ref()
            .location(loc!())?
            .as_new()
            .location(loc!())?
            .data
            .as_compressed()
            .location(loc!())?
            .0
            .clone();

        self.buffer
            .as_mut()
            .location(loc!())?
            .as_new_mut()
            .location(loc!())?
            .data = BufferData::External;

        Ok(raw_buffer_to_send)
    }

    #[instrument(skip_all, level = "debug")]
    pub fn clone_without_buffer(&self) -> Self {
        let mut clone = self.clone();
        clone.buffer = None;
        clone
    }

    pub fn get_role(&self) -> Result<&Role> {
        self.role.as_ref().ok_or(anyhow!("Role was None."))
    }

    pub fn get_role_mut(&mut self) -> Result<&mut Role> {
        self.role.as_mut().ok_or(anyhow!("Role was None."))
    }

    pub fn xdg_toplevel(&self) -> Result<&xdg_shell::XdgToplevelState> {
        self.get_role()
            .location(loc!())?
            .as_xdg_toplevel()
            .ok_or(anyhow!("Role was not XdgToplevel."))
    }

    pub fn xdg_popup(&self) -> Result<&xdg_shell::XdgPopupState> {
        self.get_role()
            .location(loc!())?
            .as_xdg_popup()
            .ok_or(anyhow!("Role was not XdgPopup."))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum Subpixel {
    Unknown,
    None,
    HorizontalRgb,
    HorizontalBgr,
    VerticalRgb,
    VerticalBgr,
}

#[cfg(feature = "wayland-client")]
impl From<SctkSubpixel> for Subpixel {
    fn from(subpixel: SctkSubpixel) -> Self {
        match subpixel {
            SctkSubpixel::Unknown => Self::Unknown,
            SctkSubpixel::None => Self::None,
            SctkSubpixel::HorizontalRgb => Self::HorizontalRgb,
            SctkSubpixel::HorizontalBgr => Self::HorizontalBgr,
            SctkSubpixel::VerticalRgb => Self::VerticalRgb,
            SctkSubpixel::VerticalBgr => Self::VerticalBgr,
            _ => Self::Unknown,
        }
    }
}

#[cfg(feature = "server")]
impl From<Subpixel> for SmithaySubpixel {
    fn from(subpixel: Subpixel) -> Self {
        match subpixel {
            Subpixel::Unknown => Self::Unknown,
            Subpixel::None => Self::None,
            Subpixel::HorizontalRgb => Self::HorizontalRgb,
            Subpixel::HorizontalBgr => Self::HorizontalBgr,
            Subpixel::VerticalRgb => Self::VerticalRgb,
            Subpixel::VerticalBgr => Self::VerticalBgr,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct Mode {
    pub dimensions: Size<i32>,
    pub refresh_rate: i32,
    pub current: bool,
    pub preferred: bool,
}

#[cfg(feature = "wayland-client")]
impl From<&SctkMode> for Mode {
    fn from(mode: &SctkMode) -> Self {
        Self {
            dimensions: mode.dimensions.into(),
            refresh_rate: mode.refresh_rate,
            current: mode.current,
            preferred: mode.preferred,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct OutputInfo {
    pub id: u32,
    pub model: String,
    pub make: String,
    pub location: Point<i32>,
    pub physical_size: Size<i32>,
    pub subpixel: Subpixel,
    pub transform: Transform,
    pub scale_factor: i32,
    pub mode: Mode,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[cfg(feature = "wayland-client")]
impl From<SctkOutputInfo> for OutputInfo {
    fn from(output: SctkOutputInfo) -> Self {
        Self {
            id: output.id,
            model: output.model.clone(),
            make: output.make.clone(),
            location: output.location.into(),
            physical_size: output.physical_size.into(),
            subpixel: output.subpixel.into(),
            transform: output.transform.into(),
            scale_factor: output.scale_factor,
            mode: output
                .modes
                .iter()
                .filter(|mode| mode.current)
                .next_back()
                .unwrap()
                .into(),
            name: output.name.clone(),
            description: output.description.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub enum SurfaceRequestPayload {
    Commit(SurfaceState),
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub struct SurfaceRequest {
    pub client: ClientId,
    pub surface: WlSurfaceId,
    pub payload: SurfaceRequestPayload,
}

impl SurfaceRequest {
    #[cfg(feature = "server")]
    pub fn new(surface: &WlSurface, payload: SurfaceRequestPayload) -> Result<Self> {
        Ok(Self {
            client: ClientId::new(&surface.client().location(loc!())?),
            surface: WlSurfaceId::new(surface),
            payload,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct SourceMetadata {
    pub mime_types: Vec<String>,
    pub dnd_actions: u32,
}

impl SourceMetadata {
    pub fn from_mime_types(mime_types: Vec<String>) -> Self {
        Self {
            mime_types,
            dnd_actions: 0,
        }
    }

    #[cfg(feature = "wayland-client")]
    pub fn from_dnd_actions(dnd_actions: SctkWlDndAction) -> Self {
        Self {
            mime_types: Vec::new(),
            dnd_actions: dnd_actions.into(),
        }
    }
}

#[cfg(feature = "server")]
impl From<SmithaySourceMetadata> for SourceMetadata {
    fn from(source_metadata: SmithaySourceMetadata) -> Self {
        Self {
            mime_types: source_metadata.mime_types,
            dnd_actions: source_metadata.dnd_action.into(),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataSource {
    Selection,
    DnD,
    Primary,
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub struct DragEnter {
    pub serial: u32,
    pub surface: WlSurfaceId,
    pub loc: Point<f64>,
    pub source_actions: u32,
    pub selected_action: u32,
    pub mime_types: Vec<String>,
}

#[derive(Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub struct DataToTransfer(pub Vec<u8>);

impl fmt::Debug for DataToTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataToTransfer")
            .field(
                "0",
                if config::get_log_priv_data() {
                    &self.0
                } else {
                    &"<redacted>"
                },
            )
            .finish()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataSourceRequest {
    // wl_data_source requests
    // DnDSetSourceActions(u32),

    // wl_data_device requests
    StartDrag(SourceMetadata, Option<Tuple2<ClientId, WlSurfaceId>>),
    SetSelection(DataSource, SourceMetadata),
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataSourceEvent {
    // wl_data_source events
    DnDMimeTypeAcceptedByDestination(Option<String>),
    MimeTypeSendRequestedByDestination(DataSource, String),
    DnDActionSelected(u32),
    DnDDropPerformed,
    DnDCancelled,
    DnDFinished,
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataDestinationRequest {
    // wl_data_offer requests
    DnDAcceptMimeType(Option<String>),
    RequestDataTransfer(DataSource, String),
    DnDFinish,
    DnDSetDestinationActions(u32),
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataDestinationEvent {
    // wl_data_offer events
    // DnDActionsOfferedBySource(u32),
    DnDActionSelected(u32),

    // wl_data_device events
    DnDEnter(DragEnter),
    DnDLeave,
    DnDMotion(Point<f64>),
    DnDDrop,
    SelectionSet(DataSource, SourceMetadata),
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataRequest {
    // source is remote application, destination is local application
    // Requests from remote source application to local compositor.
    // E.g.: set the selection, start a dnd.
    SourceRequest(DataSourceRequest),

    // // source -is local application, destination is remote application
    // // Feedback from wprsd compositor to local source application.
    // // E.g.: destination accepted a mime type, destination requested data transfer.
    // SourceEvent(DataSourceEvent),

    // source is remote application, destination is local application Not needed
    // because wprsd forwards the source events to the local compositor and lets
    // it interpret them and generate events for the local destination.
    // DestinationEvent(DataDestinationEvent),

    // source is local application, destination is remote application
    // Feedback from remote destination to local compositor.
    // E.g.: accept mime type, request data transfer.
    DestinationRequest(DataDestinationRequest),

    TransferData(DataSource, DataToTransfer),
}

#[derive(Debug, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub enum DataEvent {
    // source is remote application, destination is local application
    // Feedback from local compositor to remote source application.
    // E.g., destination accepted a mime type, destination requested data transfer.
    SourceEvent(DataSourceEvent),

    // // source -is local application, destination is remote application
    // // Events from local compositor to wprsd as remote compositor.
    // //
    // // E.g., a selection was set so set one for remote clients, a dnd was
    // // started, so tart one for remote clients.
    // SourceRequest(DataSourceRequest),

    // source is local application, destination is remote application
    // Events for remote data destination.
    // E.g.: a selection was set, dnd motion over dest's surface.
    DestinationEvent(DataDestinationEvent),

    // // source is remote application, destination is local application
    // // Feedback from local destination to wprsd as remote compositor.
    // // E.g.: accept mime type, request data transfer.
    // DestinationRequest(DataDestinationRequest),
    TransferData(DataSource, DataToTransfer),
}

#[derive(Debug, Clone, PartialEq, Eq, Archive, Deserialize, Serialize)]
pub enum OutputEvent {
    New(OutputInfo),
    Update(OutputInfo),
    Destroy(OutputInfo),
}

#[derive(Debug, Clone, PartialEq, Eq, Archive, Deserialize, Serialize)]
pub struct Output {
    pub id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Archive, Deserialize, Serialize)]
pub enum SurfaceEventPayload {
    OutputsChanged(Vec<Output>),
}

#[derive(Debug, Clone, PartialEq, Eq, Archive, Deserialize, Serialize)]
pub struct SurfaceEvent {
    pub surface_id: WlSurfaceId,
    pub payload: SurfaceEventPayload,
}

#[derive(Debug, Copy, Clone, PartialEq, Archive, Deserialize, Serialize)]
pub struct ViewportState {
    pub src: Option<Rectangle<f64>>,
    pub dst: Option<Size<i32>>,
}

#[cfg(feature = "server")]
impl From<&ViewportCachedState> for ViewportState {
    fn from(viewport_state: &ViewportCachedState) -> Self {
        Self {
            src: viewport_state.src.map(Into::into),
            dst: viewport_state.dst.map(Into::into),
        }
    }
}
