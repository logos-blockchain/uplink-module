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
    /// Its children as its participant state names them (hex), so their activity is kept
    /// when epoch sets are dropped (`lists`).
    #[serde(default)]
    pub children: std::collections::BTreeSet<String>,
    /// The registry epoch its last claim plan looked up to, and the epochs that plan claims.
    /// An epoch's full set is kept while some participant's plan hasn't looked at it yet or
    /// claims it.
    #[serde(default)]
    pub planned_through: Option<u32>,
    #[serde(default)]
    pub needs: Vec<u32>,
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
    /// The LEZ block height when `lez_core` accepted the transaction, to give up on it if no
    /// block includes it (`operations::INCLUSION_BLOCKS`).
    #[serde(default)]
    pub pending_since_block: Option<i64>,
    /// How many times this reference was sent (a failed one may be sent again).
    #[serde(default = "one")]
    pub attempts: u32,
    /// The LEZ block height when it last failed, to space out automatic retries.
    #[serde(default)]
    pub failed_at_block: Option<i64>,
    /// Kind-specific facts, e.g. a cash-out's receipt index and points.
    #[serde(default)]
    pub detail: serde_json::Value,
}

fn one() -> u32 {
    1
}

impl Operation {
    pub fn is_open(&self) -> bool {
        matches!(self.status, OperationStatus::Submitting | OperationStatus::Pending)
    }

    pub fn fail(&mut self, error: String, height: Option<i64>) {
        self.status = OperationStatus::Failed;
        self.error = Some(error);
        self.failed_at_block = height;
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
    /// Active sets as earlier versions saved them (`"<epoch>:<digest hex>"` → nodes, hex).
    /// Read once, moved to `uplink_lists.json` (`lists`), never written again.
    #[serde(default, skip_serializing)]
    pub publications: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing)]
    pub history_offset: u64,
    /// The latest epoch taken in from the recorder's log, which it may then forget.
    #[serde(default)]
    pub recorded_through: Option<u32>,
}

impl Deployment {
    pub const EMPTY: Self = Self {
        terms: None,
        participants: BTreeMap::new(),
        operations: BTreeMap::new(),
        publications: BTreeMap::new(),
        history_offset: 0,
        recorded_through: None,
    };
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub config: Config,
    /// One section per program account (hex).
    pub deployments: BTreeMap<String, Deployment>,
}
