//! What the service needs from outside, as traits: `lez_core`, the LEZ indexer, our saved
//! state and key files. `adapters` implements them for real; tests use fakes.

use zeroize::Zeroizing;

pub type PortResult<T> = Result<T, String>;

/// One account in a private transaction, in the form `lez_core` takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountEntry {
    /// One of the wallet's own private accounts (participant, notes).
    Own(String),
    /// A public account the transaction reads or updates without a signature (registry, receipt).
    PublicNoSign(String),
    /// A new private note for another user, from their public keys.
    PrivateForeign { npk: String, vpk: String },
}

impl AccountEntry {
    /// The account descriptor `lez_core.send_generic_private_transaction_with_accounts` takes
    /// (logos-execution-zone-module#69). A foreign note gets a random identifier from lez_core.
    pub fn descriptor(&self) -> String {
        match self {
            Self::Own(id) => serde_json::json!({ "kind": "private_owned", "account_id": id }).to_string(),
            Self::PublicNoSign(id) => serde_json::json!({ "kind": "public_no_sign", "account_id": id }).to_string(),
            Self::PrivateForeign { npk, vpk } => serde_json::json!({
                "kind": "private_foreign",
                "nullifier_public_key": npk,
                "viewing_public_key": vpk,
            })
            .to_string(),
        }
    }
}

/// The `lez_core` calls the module uses. Ids are hex; account reads return `lez_core`'s
/// JSON (`{"nonce", "shards": [{"program", "data"}]}`), or `""` when there is none.
pub trait Lez {
    fn create_account_private(&self) -> PortResult<String>;
    /// `(account id, is_public)` for every wallet account.
    fn list_accounts(&self) -> PortResult<Vec<(String, bool)>>;
    fn get_account_public(&self, id: &str) -> PortResult<String>;
    fn get_account_private(&self, id: &str) -> PortResult<String>;
    /// `{"nullifier_public_key", "viewing_public_key"}`.
    fn get_private_account_keys(&self, id: &str) -> PortResult<String>;
    fn add_label(&self, label: &str, id: &str, is_private: bool) -> PortResult<()>;
    /// `""` when no account has the label.
    fn resolve_label(&self, label: &str) -> PortResult<String>;
    fn get_current_block_height(&self) -> PortResult<i64>;
    fn sync_to_block(&self, block: i64) -> PortResult<()>;
    /// `lez_core.send_generic_private_transaction_with_accounts`; replies
    /// `{"success", "tx_hash", "error"}`.
    fn send_private_transaction(
        &self,
        accounts: &[AccountEntry],
        instruction: &[u8],
        program_elf: &[u8],
        program_kind_json: &str,
    ) -> PortResult<String>;
    fn poll_transaction_status(&self, tx_hash: &str) -> PortResult<bool>;
}

/// The LEZ indexer (`lez_indexer_module`), for publications this module didn't see live.
pub trait History {
    /// A JSON array of transactions touching `account`, oldest first.
    fn transactions_by_account(&self, account: &str, offset: u64, limit: u64) -> PortResult<String>;
    /// The indexer's status JSON (`{"state": "Syncing" | "CaughtUp" | …, "last_error",
    /// "indexed_block_id", …}`, as `indexer_core` writes it).
    fn status(&self) -> PortResult<String>;
}

/// The module's saved state, as one JSON document.
pub trait Store {
    fn load(&self) -> PortResult<Option<String>>;
    fn save(&self, json: &str) -> PortResult<()>;
    /// Moves the saved state out of the way, keeping it, so the next save starts fresh.
    /// Returns where it went.
    fn set_aside(&self) -> PortResult<String>;
}

/// Files the user points the module at.
pub trait Files {
    fn read_secret(&self, path: &str) -> PortResult<Zeroizing<String>>;
    /// Whether the module may read a keystore from a path a caller gives. Off unless the
    /// operator turns it on (`UPLINK_KEYSTORE_PATHS=1`): any caller can name a path, so
    /// where the key has another holder (Basecamp's Blockchain app), it must stay there.
    fn keystore_paths(&self) -> bool;
}

pub trait Clock {
    fn now_ms(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_use_lez_cores_field_names() {
        let own: serde_json::Value = serde_json::from_str(&AccountEntry::Own("ab".into()).descriptor()).unwrap();
        assert_eq!(own, serde_json::json!({ "kind": "private_owned", "account_id": "ab" }));
        let foreign: serde_json::Value =
            serde_json::from_str(&AccountEntry::PrivateForeign { npk: "01".into(), vpk: "02".into() }.descriptor()).unwrap();
        assert_eq!(foreign["nullifier_public_key"], "01");
        assert_eq!(foreign["viewing_public_key"], "02");
        assert!(foreign.get("identifier").is_none());
    }
}
