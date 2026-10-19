//! The module's saved state: what uplink's wallet keeps in `referral.json` and `lez_core`
//! doesn't expose. One JSON document in the module's data directory.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

/// Everything kept for one deployment of the program. Each network (devnet, testnet, …)
/// has its own program account, so its own section: epochs, invitations and operations
/// from one network never mix with another's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    /// Accepted terms not yet bound to a node by an enrolment.
    pub terms: Option<TermsAcceptance>,
}

impl Deployment {
    pub const EMPTY: Self = Self {
        terms: None,
    };
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub config: Config,
    /// One section per program account (hex).
    pub deployments: BTreeMap<String, Deployment>,
}
