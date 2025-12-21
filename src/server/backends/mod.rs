#[cfg(feature = "server")]
pub mod wayland;

use crate::prelude::*;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::Request;
use crate::protocols::wprs::Serializer;

pub trait ServerBackend {
    fn run(self: Box<Self>, serializer: Serializer<Request, Event>) -> Result<()>;
}
