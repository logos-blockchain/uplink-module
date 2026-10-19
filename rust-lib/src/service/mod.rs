//! The module's use cases, over the domain and the ports. Every call returns JSON
//! (`serde_json::Value`) or a `DomainError`; `glue` turns those into the module's replies.

mod deployment;
pub mod saved;

#[cfg(test)]
mod tests;

use crate::domain::program::Binary;
use crate::domain::DomainError;
use crate::ports::{Lez, Store};
use saved::Saved;

pub const PROGRAM_ACCOUNT_ENV: &str = "UPLINK_PROGRAM";

pub struct Service {
    lez: Box<dyn Lez>,
    store: Box<dyn Store>,
    binary: Binary,
    /// The program account last found running `binary`, this session.
    checked: Option<String>,
    saved: Saved,
}

pub(crate) fn port(error: String) -> DomainError {
    DomainError::new("lez_core", error)
}

impl Service {
    pub fn new(
        lez: Box<dyn Lez>,
        store: Box<dyn Store>,
        binary: Binary,
    ) -> Self {
        let saved = store
            .load()
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { lez, store, binary, checked: None, saved }
    }

    fn persist(&self) -> crate::domain::Result<()> {
        let json = serde_json::to_string(&self.saved).expect("saved state serializes");
        self.store.save(&json).map_err(|e| DomainError::new("store", e))
    }
}
