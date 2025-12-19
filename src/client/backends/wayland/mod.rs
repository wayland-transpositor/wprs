use bimap::BiMap;
use smithay_client_toolkit::reexports::client::backend::ObjectId as SctkObjectId;

use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::ObjectId;

pub(crate) type ObjectBimap = BiMap<(ClientId, ObjectId), SctkObjectId>;

mod sctk;
pub mod backend;

pub mod server_handlers;
pub mod smithay_handlers;
mod subsurface;
mod xdg_shell;


pub use sctk::*;
pub use backend::WaylandClientBackend;
