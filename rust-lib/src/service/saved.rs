//! The module's saved state: what uplink's wallet keeps in `referral.json` and `lez_core`
//! doesn't expose. One JSON document in the module's data directory.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::invitation::Invitation;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// The deployment in use: the referral program's account (the program loader's header
    /// account), hex. Selects the `Deployment` below.
    pub program_account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermsAcceptance {
    pub version: u32,
    pub sha256: String,
    pub accepted_at_ms: u64,
    /// Set when an enrolment binds the acceptance to a node.
    pub node: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantRecord {
    pub invitation: Option<Invitation>,
    /// The node this participant enrolled (or is enrolling) with, hex.
    pub node: Option<String>,
    pub terms: Option<TermsAcceptance>,
    /// Receipts found the last time the node key was given (or a cash-out settled), so they
    /// can be listed without reading the key again. Public facts: index, account, points.
    #[serde(default)]
    pub receipts: Vec<KnownReceipt>,
    /// Each child's join epoch (hex node → epoch), from its join note. The program keeps it only
    /// until a claim settles that epoch, so it's remembered here the first time it's seen.
    #[serde(default)]
    pub children_since: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownReceipt {
    pub index: u64,
    pub account: String,
    pub points: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    Register,
    Claim,
    CashOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationStatus {
    /// Recorded, not yet handed to `lez_core` (or the module stopped while it was).
    Submitting,
    Pending,
    Settled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub reference: String,
    pub kind: OperationKind,
    pub participant: String,
    pub status: OperationStatus,
    pub tx_hash: Option<String>,
    pub error: Option<String>,
    pub created_at_ms: u64,
    /// Kind-specific facts, e.g. a cash-out's receipt index and points.
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl Operation {
    pub fn is_open(&self) -> bool {
        matches!(self.status, OperationStatus::Submitting | OperationStatus::Pending)
    }
}

/// Everything kept for one deployment of the program. Each network (devnet, testnet, …)
/// has its own program account, so its own section: epochs, invitations and operations
/// from one network never mix with another's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    /// Accepted terms not yet bound to a node by an enrolment.
    pub terms: Option<TermsAcceptance>,
    pub participants: BTreeMap<String, ParticipantRecord>,
    pub operations: BTreeMap<String, Operation>,
    /// Every publication seen: `"<epoch>:<digest hex>"` → active nodes, hex.
    pub publications: BTreeMap<String, Vec<String>>,
    /// How far the indexer's history of the registry has been read.
    pub history_offset: u64,
}

impl Deployment {
    pub const EMPTY: Self = Self {
        terms: None,
        participants: BTreeMap::new(),
        operations: BTreeMap::new(),
        publications: BTreeMap::new(),
        history_offset: 0,
    };
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub config: Config,
    /// One section per program account (hex).
    pub deployments: BTreeMap<String, Deployment>,
}
