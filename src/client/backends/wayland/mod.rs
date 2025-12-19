use bimap::BiMap;
use smithay_client_toolkit::reexports::client::backend::ObjectId as SctkObjectId;

use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::ObjectId;

pub(crate) type ObjectBimap = BiMap<(ClientId, ObjectId), SctkObjectId>;

pub mod backend;
mod sctk;

pub mod server_handlers;
pub mod smithay_handlers;
mod subsurface;
mod xdg_shell;

pub use backend::WaylandClientBackend;
pub use sctk::*;
