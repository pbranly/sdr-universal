use anyhow::Result;
use std::sync::mpsc::Receiver;

use crate::core::{Event, IqBlock};

pub trait Backend {
    fn connect(&mut self) -> Result<()>;
    fn disconnect(&mut self);
    fn apply_event(&mut self, event: &Event) -> Result<()>;
    fn take_iq_receiver(&mut self) -> Option<Receiver<IqBlock>>;

    /// Tâches périodiques hors callbacks (acquittements, messages de surcharge...).
    fn service(&mut self) {}
}

pub mod bandwidth;
pub mod gain;
pub mod mock;

#[cfg(feature = "sdrplay")]
pub mod sdrplay;
