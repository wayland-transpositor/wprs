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
use std::num::NonZeroU32;

use rkyv::bytecheck;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::Mode as KdeDecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_popup;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_surface;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
use smithay::reexports::wayland_server::backend;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::ToplevelStateSet;
use smithay::wayland::shell::xdg::PopupSurface;
use smithay::wayland::shell::xdg::PositionerState;
use smithay_client_toolkit::reexports::csd_frame::WindowState as CsdWindowState;
use smithay_client_toolkit::shell::xdg::popup::ConfigureKind;
use smithay_client_toolkit::shell::xdg::popup::PopupConfigure as SctkPopupConfigure;
use smithay_client_toolkit::shell::xdg::window::DecorationMode as SctkDecorationMode;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;

use crate::prelude::*;
use crate::serialization;
use crate::serialization::geometry::Point;
use crate::serialization::geometry::Rectangle;
use crate::serialization::geometry::Size;
use crate::serialization::wayland::WlSurfaceId;
use crate::serialization::ClientId;

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgSurfaceId(pub u64);

impl XdgSurfaceId {
    pub fn new(xdg_surface: &xdg_surface::XdgSurface) -> Self {
        Self(serialization::hash(&xdg_surface.id()))
    }
}

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgToplevelId(pub u64);

impl XdgToplevelId {
    pub fn new(xdg_toplevel: &XdgToplevel) -> Self {
        Self(serialization::hash(&xdg_toplevel.id()))
    }
}

impl From<&backend::ObjectId> for XdgToplevelId {
    fn from(object_id: &backend::ObjectId) -> Self {
        Self(serialization::hash(object_id))
    }
}

#[derive(Archive, Deserialize, Serialize, Debug, Copy, Clone, Hash, Eq, PartialEq)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgPopupId(pub u64);

impl XdgPopupId {
    pub fn new(xdg_popup: &xdg_popup::XdgPopup) -> Self {
        Self(serialization::hash(&xdg_popup.id()))
    }
}

impl From<backend::ObjectId> for XdgPopupId {
    fn from(object_id: backend::ObjectId) -> Self {
        Self(serialization::hash(&object_id))
    }
}

impl From<&backend::ObjectId> for XdgPopupId {
    fn from(object_id: &backend::ObjectId) -> Self {
        Self(serialization::hash(object_id))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgPositioner {
    pub width: i32,
    pub height: i32,
    pub anchor_rect: Rectangle<i32>,
    pub anchor_edges: u32,
    pub gravity: u32,
    pub constraint_adjustment: u32,
    pub offset: Point<i32>,
    pub reactive: bool,
    pub parent_size: Option<Size<i32>>,
    pub parent_configure: Option<u32>,
}

impl XdgPositioner {
    pub fn new(positioner: &PositionerState) -> Self {
        Self {
            width: positioner.rect_size.w,
            height: positioner.rect_size.h,
            anchor_rect: positioner.anchor_rect.into(),
            anchor_edges: positioner.anchor_edges.into(),
            gravity: positioner.gravity.into(),
            constraint_adjustment: positioner.constraint_adjustment.into(),
            offset: positioner.offset.into(),
            reactive: positioner.reactive,
            parent_size: positioner.parent_size.map(Into::into),
            parent_configure: positioner.parent_configure.map(Into::into),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgSurfaceState {
    pub window_geometry: Option<Rectangle<i32>>,
    pub max_size: Size<i32>,
    pub min_size: Size<i32>,
}

impl XdgSurfaceState {
    pub fn new() -> Self {
        Self {
            window_geometry: None,
            max_size: (0, 0).into(),
            min_size: (0, 0).into(),
        }
    }
}

impl Default for XdgSurfaceState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub enum DecorationMode {
    Client,
    Server,
}

impl From<DecorationMode> for XdgDecorationMode {
    fn from(decoration_mode: DecorationMode) -> Self {
        match decoration_mode {
            DecorationMode::Client => Self::ClientSide,
            DecorationMode::Server => Self::ServerSide,
        }
    }
}

impl From<DecorationMode> for SctkDecorationMode {
    fn from(decoration_mode: DecorationMode) -> Self {
        match decoration_mode {
            DecorationMode::Client => Self::Client,
            DecorationMode::Server => Self::Server,
        }
    }
}

impl From<SctkDecorationMode> for DecorationMode {
    fn from(decoration_mode: SctkDecorationMode) -> Self {
        match decoration_mode {
            SctkDecorationMode::Client => Self::Client,
            SctkDecorationMode::Server => Self::Server,
        }
    }
}

impl TryFrom<XdgDecorationMode> for DecorationMode {
    type Error = anyhow::Error;
    fn try_from(decoration_mode: XdgDecorationMode) -> Result<Self> {
        match decoration_mode {
            XdgDecorationMode::ClientSide => Ok(Self::Client),
            XdgDecorationMode::ServerSide => Ok(Self::Server),
            _ => Err(anyhow!("unknown decoration mode {decoration_mode:?}")),
        }
    }
}

impl TryFrom<KdeDecorationMode> for DecorationMode {
    type Error = anyhow::Error;
    fn try_from(decoration_mode: KdeDecorationMode) -> Result<Self> {
        match decoration_mode {
            KdeDecorationMode::Client => Ok(Self::Client),
            KdeDecorationMode::Server => Ok(Self::Server),
            _ => Err(anyhow!("unknown decoration mode {decoration_mode:?}")),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgToplevelState {
    pub id: XdgToplevelId,
    pub parent: Option<WlSurfaceId>,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub decoration_mode: Option<DecorationMode>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
}

impl XdgToplevelState {
    pub fn new(toplevel: &ToplevelSurface) -> Self {
        Self {
            id: XdgToplevelId::new(toplevel.xdg_toplevel()),
            parent: None,
            title: None,
            app_id: None,
            decoration_mode: None,
            maximized: None,
            fullscreen: None,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct XdgPopupState {
    pub id: XdgPopupId,
    pub parent_surface_id: WlSurfaceId,
    pub positioner: XdgPositioner,
    pub grab_requested: bool,
}

impl XdgPopupState {
    pub fn new(popup: &PopupSurface, positioner: &PositionerState) -> Result<Self> {
        Ok(Self {
            id: XdgPopupId::new(popup.xdg_popup()),
            parent_surface_id: WlSurfaceId::new(&popup.get_parent_surface().location(loc!())?),
            positioner: XdgPositioner::new(positioner),
            grab_requested: false,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct WindowState(u16);

impl From<WindowState> for ToplevelStateSet {
    fn from(window_state: WindowState) -> Self {
        let mut states = Self::default();
        let window_states = CsdWindowState::from_bits(window_state.0).unwrap();
        if window_states.contains(CsdWindowState::MAXIMIZED) {
            states.set(State::Maximized);
        };
        if window_states.contains(CsdWindowState::FULLSCREEN) {
            states.set(State::Fullscreen);
        };
        if window_states.contains(CsdWindowState::RESIZING) {
            states.set(State::Resizing);
        };
        if window_states.contains(CsdWindowState::ACTIVATED) {
            states.set(State::Activated);
        };
        if window_states.contains(CsdWindowState::TILED_LEFT) {
            states.set(State::TiledLeft);
        };
        if window_states.contains(CsdWindowState::TILED_RIGHT) {
            states.set(State::TiledRight);
        };
        if window_states.contains(CsdWindowState::TILED_TOP) {
            states.set(State::TiledTop);
        };
        if window_states.contains(CsdWindowState::TILED_BOTTOM) {
            states.set(State::TiledBottom);
        };
        states
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct ToplevelConfigure {
    pub surface_id: WlSurfaceId,
    pub new_size: Size<Option<NonZeroU32>>,
    pub suggested_bounds: Option<Size<u32>>,
    pub decoration_mode: DecorationMode,
    pub state: WindowState,
}

impl ToplevelConfigure {
    pub fn from_smithay(surface_id: &WlSurfaceId, configure: WindowConfigure) -> Self {
        Self {
            surface_id: *surface_id,
            new_size: configure.new_size.into(),
            suggested_bounds: configure.suggested_bounds.map(Into::into),
            decoration_mode: configure.decoration_mode.into(),
            state: WindowState(configure.state.bits()),
        }
    }
}

// TODO: do we need this? We're never reading it.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub enum PopupConfigureKind {
    Initial,
    Reactive,
    Reposition { token: u32 },
}

impl From<ConfigureKind> for PopupConfigureKind {
    fn from(kind: ConfigureKind) -> Self {
        match kind {
            ConfigureKind::Initial => Self::Initial,
            ConfigureKind::Reactive => Self::Reactive,
            ConfigureKind::Reposition { token } => Self::Reposition { token },
            _ => {
                unreachable!()
            }, // TODO
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct PopupConfigure {
    pub surface_id: WlSurfaceId,
    pub position: Point<i32>,
    pub width: i32,
    pub height: i32,
    pub kind: PopupConfigureKind,
}

impl PopupConfigure {
    pub fn from_smithay(surface_id: &WlSurfaceId, configure: SctkPopupConfigure) -> Self {
        Self {
            surface_id: *surface_id,
            position: configure.position.into(),
            width: configure.width,
            height: configure.height,
            kind: configure.kind.into(),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct Move {
    pub serial: u32,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct Resize {
    pub serial: u32,
    pub edge: u32,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub enum ToplevelRequestPayload {
    Destroyed,

    // After these requests, "the compositor will respond by emitting a
    // configure event", even if the state is already set, so these needs to be
    // one-requests and can't be sent as part of the XdgToplevelState. I.e.,
    // these are not idempotent. Even if we checked if they changed and only
    // applied the change if necessary, the compositor is still free to ignore
    // the request, so they state could get out of sync.
    SetMaximized,
    UnsetMaximized,
    SetFullscreen, // TODO: specify output?
    UnsetFullscreen,
    // "There is no way to know if the surface is currently minimized, nor is
    // there any way to unset minimization on this surface."
    SetMinimized,

    Move(Move),
    Resize(Resize),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct ToplevelRequest {
    pub client: ClientId,
    pub surface: WlSurfaceId,
    pub payload: ToplevelRequestPayload,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub enum ToplevelEvent {
    Configure(ToplevelConfigure),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub enum PopupRequestPayload {
    Destroyed,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub struct PopupRequest {
    pub client: ClientId,
    pub surface: WlSurfaceId,
    pub payload: PopupRequestPayload,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Archive, Deserialize, Serialize)]
#[archive_attr(derive(bytecheck::CheckBytes, Debug))]
pub enum PopupEvent {
    Configure(PopupConfigure),
}
