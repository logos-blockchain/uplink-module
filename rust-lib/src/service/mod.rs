//! The module's use cases, over the domain and the ports. Every call returns JSON
//! (`serde_json::Value`) or a `DomainError`; `glue` turns those into the module's replies.

mod activity;
mod cashout;
mod claims;
mod deployment;
mod enrol;
mod notes;
mod operations;
mod participants;
mod registry;
pub mod saved;
mod terms;

#[cfg(test)]
mod tests;

use sha2::{Digest, Sha256};

use crate::domain::program::Binary;
use crate::domain::{DomainError, Result, codec};
use crate::ports::{Clock, Files, History, Lez, Store};
use saved::{Deployment, Saved};

static NO_DEPLOYMENT: Deployment = Deployment::EMPTY;

pub const PROGRAM_ACCOUNT_ENV: &str = "UPLINK_PROGRAM";

pub struct Service {
    lez: Box<dyn Lez>,
    history: Option<Box<dyn History>>,
    store: Box<dyn Store>,
    files: Box<dyn Files>,
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
        history: Option<Box<dyn History>>,
        store: Box<dyn Store>,
        files: Box<dyn Files>,
        clock: Box<dyn Clock>,
        binary: Binary,
    ) -> Self {
        let saved = store
            .load()
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { lez, history, store, files, clock, binary, checked: None, saved }
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

/// References are caller-chosen 32-byte hex ids; resubmitting one is idempotent.
fn reference_id(reference: &str) -> Result<String> {
    codec::bytes32(reference).map(hex::encode)
}

/// A reference derived from another, for operations one call submits several of.
fn derived_reference(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}
