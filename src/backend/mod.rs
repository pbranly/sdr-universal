use anyhow::Result;

use crate::core::Event;

pub trait Backend {
    fn connect(&mut self) -> Result<()>;
    fn disconnect(&mut self);
    fn apply_event(&mut self, event: &Event) -> Result<()>;
}

pub mod sdrplay;
