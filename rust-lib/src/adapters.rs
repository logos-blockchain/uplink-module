//! The real ports: `lez_core` and `lez_indexer_module` over the Logos protocol, the
//! module's data directory, the filesystem and the clock.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::glue::modules;
use crate::ports::{AccountEntry, Lez, PortResult, Store};

fn call<T, E: std::fmt::Display>(method: &str, result: Result<T, E>) -> PortResult<T> {
    result.map_err(|e| format!("lez_core.{method}: {e}"))
}

/// Proving a private transaction takes minutes without RISC0_DEV_MODE, far past the 20 s default.
/// The SDK has no "wait forever" (<= 0 means the default), so a generous bound.
const SEND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct LezCore;

impl Lez for LezCore {
    fn create_account_private(&self) -> PortResult<String> {
        let id = call("create_account_private", modules().lez_core.create_account_private())?;
        if id.is_empty() { Err("lez_core.create_account_private: no wallet open".into()) } else { Ok(id) }
    }

    fn list_accounts(&self) -> PortResult<Vec<(String, bool)>> {
        let entries = call("list_accounts", modules().lez_core.list_accounts())?;
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let id = entry.get("account_id")?.as_str()?.to_owned();
                Some((id, entry.get("is_public").and_then(Value::as_bool).unwrap_or(false)))
            })
            .collect())
    }

    fn get_account_public(&self, id: &str) -> PortResult<String> {
        call("get_account_public", modules().lez_core.get_account_public(id))
    }

    fn get_account_private(&self, id: &str) -> PortResult<String> {
        call("get_account_private", modules().lez_core.get_account_private(id))
    }

    fn get_private_account_keys(&self, id: &str) -> PortResult<String> {
        call("get_private_account_keys", modules().lez_core.get_private_account_keys(id))
    }

    fn add_label(&self, label: &str, id: &str, is_private: bool) -> PortResult<()> {
        match call("add_label", modules().lez_core.add_label(label, id, is_private))? {
            0 => Ok(()),
            code => Err(format!("lez_core.add_label: wallet error {code}")),
        }
    }

    fn resolve_label(&self, label: &str) -> PortResult<String> {
        call("resolve_label", modules().lez_core.resolve_label(label))
    }

    fn get_current_block_height(&self) -> PortResult<i64> {
        call("get_current_block_height", modules().lez_core.get_current_block_height())
    }

    fn sync_to_block(&self, block: i64) -> PortResult<()> {
        match call("sync_to_block", modules().lez_core.sync_to_block(block))? {
            0 => Ok(()),
            code => Err(format!("lez_core.sync_to_block: wallet error {code}")),
        }
    }

    fn send_private_transaction(
        &self,
        accounts: &[AccountEntry],
        instruction: &[u8],
        program_elf: &[u8],
        program_kind_json: &str,
    ) -> PortResult<String> {
        let descriptors: Vec<String> = accounts.iter().map(AccountEntry::descriptor).collect();
        call(
            "send_generic_private_transaction_with_accounts",
            modules().lez_core.send_generic_private_transaction_with_accounts_with_timeout(
                &descriptors,
                instruction,
                program_elf,
                program_kind_json,
                &[],
                &[],
                &[],
                SEND_TIMEOUT,
            ),
        )
    }

    fn poll_transaction_status(&self, tx_hash: &str) -> PortResult<bool> {
        call("poll_transaction_status", modules().lez_core.poll_transaction_status(tx_hash))
    }
}

/// One JSON file in the module's data directory, replaced atomically.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(dir: &str) -> Self {
        Self { path: PathBuf::from(dir).join("uplink_state.json") }
    }
}

impl Store for FileStore {
    fn load(&self) -> PortResult<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(json) => Ok(Some(json)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", self.path.display())),
        }
    }

    fn save(&self, json: &str) -> PortResult<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

