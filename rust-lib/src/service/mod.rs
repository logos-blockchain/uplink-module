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
mod program_tests;
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
    /// Set when the saved state couldn't be used at startup; `info()` reports it.
    state_problem: Option<StateProblem>,
}

/// What went wrong with the saved state at startup.
enum StateProblem {
    /// It wasn't valid; it was moved to `aside` and the module started empty.
    SetAside { aside: String },
    /// It couldn't be read (or moved). Nothing is saved, so it isn't overwritten.
    Unreadable { error: String },
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
        let (saved, state_problem) = load(store.as_ref());
        Self { lez, history, store, files, clock, binary, checked: None, saved, state_problem }
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
        if let Some(StateProblem::Unreadable { error }) = &self.state_problem {
            return Err(DomainError::new("state_unreadable", format!("the saved state can't be read, so nothing is saved over it: {error}")));
        }
        let json = serde_json::to_string(&self.saved).expect("saved state serializes");
        self.store.save(&json).map_err(|e| DomainError::new("store", e))
    }
}

/// The saved state, or an empty one and why. A file that doesn't parse is moved aside
/// rather than overwritten by the next save; one that can't be read at all is left alone.
fn load(store: &dyn Store) -> (Saved, Option<StateProblem>) {
    match store.load() {
        Ok(None) => (Saved::default(), None),
        Ok(Some(json)) => match serde_json::from_str(&json) {
            Ok(saved) => (saved, None),
            Err(_) => match store.set_aside() {
                Ok(aside) => (Saved::default(), Some(StateProblem::SetAside { aside })),
                Err(error) => (Saved::default(), Some(StateProblem::Unreadable { error })),
            },
        },
        Err(error) => (Saved::default(), Some(StateProblem::Unreadable { error })),
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
