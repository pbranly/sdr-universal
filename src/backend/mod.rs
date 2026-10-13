use anyhow::Result;
use std::sync::mpsc::Receiver;

use crate::core::{Event, IqBlock};

pub trait Backend {
    fn connect(&mut self) -> Result<()>;
    fn disconnect(&mut self);
    fn apply_event(&mut self, event: &Event) -> Result<()>;
    fn take_iq_receiver(&mut self) -> Option<Receiver<IqBlock>>;

    /// Periodic tasks outside the API callbacks (acknowledgements, overload messages...).
    fn service(&mut self) {}

    /// Whether the receiver is still there. Checked regularly by the main loop.
    fn health(&mut self) -> Health {
        Health::Healthy
    }

    /// Called repeatedly after `health()` reported the receiver lost, until it
    /// succeeds: tears the session down, opens the receiver again and re-applies
    /// the settings that were in effect.
    fn recover(&mut self) -> Result<()> {
        Err(anyhow::anyhow!(
            "this backend cannot recover a lost receiver"
        ))
    }
}

/// State of the receiver, as seen by the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Healthy,
    /// Unplugged, failed, or silent. The text says why.
    Lost(String),
}

pub mod gain;
pub mod mock;
pub mod replay;

#[cfg(feature = "sdrplay")]
pub mod sdrplay;
