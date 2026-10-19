//! The module's use cases, over the domain and the ports. Every call returns JSON
//! (`serde_json::Value`) or a `DomainError`; `glue` turns those into the module's replies.

mod deployment;
pub mod saved;
mod terms;

#[cfg(test)]
mod tests;

use crate::domain::program::Binary;
use crate::domain::{DomainError, Result, codec};
use crate::ports::{Clock, Lez, Store};
use saved::{Deployment, Saved};

static NO_DEPLOYMENT: Deployment = Deployment::EMPTY;

pub const PROGRAM_ACCOUNT_ENV: &str = "UPLINK_PROGRAM";

pub struct Service {
    lez: Box<dyn Lez>,
    store: Box<dyn Store>,
    clock: Box<dyn Clock>,
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
        clock: Box<dyn Clock>,
        binary: Binary,
    ) -> Self {
        let saved = store
            .load()
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { lez, store, clock, binary, checked: None, saved }
    }

    /// The current deployment's saved state (empty if none is configured or seen yet).
    fn deployment(&self) -> &Deployment {
        self.program_hex().and_then(|program| self.saved.deployments.get(&program)).unwrap_or(&NO_DEPLOYMENT)
    }

    /// The current deployment's saved state, for writing. Needs a configured program.
    fn deployment_mut(&mut self) -> Result<&mut Deployment> {
        let program = codec::account_hex(&self.program()?);
        Ok(self.saved.deployments.entry(program).or_default())
    }

    fn persist(&self) -> Result<()> {
        let json = serde_json::to_string(&self.saved).expect("saved state serializes");
        self.store.save(&json).map_err(|e| DomainError::new("store", e))
    }
}
