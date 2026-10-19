//! The real ports: `lez_core` and `lez_indexer_module` over the Logos protocol, the
//! module's data directory, the filesystem and the clock.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use zeroize::Zeroizing;

use crate::glue::modules;
use crate::ports::{AccountEntry, Clock, Files, History, Lez, PortResult, Store};

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

/// `lez_indexer_module`, an optional dependency: never loaded for us, so calls fail when it
/// isn't running, and the service then carries on without history.
pub struct Indexer;

/// A call to a module that isn't running waits the protocol's full deadline (20 s) before
/// failing; a page of history needs far less.
const INDEXER_TIMEOUT: Duration = Duration::from_secs(5);

impl History for Indexer {
    fn transactions_by_account(&self, account: &str, offset: u64, limit: u64) -> PortResult<String> {
        let page = modules()
            .lez_indexer_module
            .get_transactions_by_account_with_timeout(account, &offset.to_string(), &limit.to_string(), INDEXER_TIMEOUT)
            .map_err(|e| format!("lez_indexer_module.getTransactionsByAccount: {e}"))?;
        if page.is_empty() { Err("lez_indexer_module: no answer (not running?)".into()) } else { Ok(page) }
    }

    fn status(&self) -> PortResult<String> {
        let status = modules()
            .lez_indexer_module
            .get_status_with_timeout(INDEXER_TIMEOUT)
            .map_err(|e| format!("lez_indexer_module.getStatus: {e}"))?;
        if status.is_empty() { Err("lez_indexer_module: not started".into()) } else { Ok(status) }
    }
}

/// One JSON file in the module's data directory, replaced atomically.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// The service's saved state.
    pub fn new(dir: &str) -> Self {
        Self::named(dir, "uplink_state.json")
    }

    pub fn named(dir: &str, file: &str) -> Self {
        Self { path: PathBuf::from(dir).join(file) }
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

    fn set_aside(&self) -> PortResult<String> {
        let ms = SystemClock.now_ms();
        let aside = self.path.with_extension(format!("json.unreadable-{ms}"));
        std::fs::rename(&self.path, &aside).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(aside.display().to_string())
    }
}

pub struct LocalFiles;

/// Set to `1` where the module may read keystores from paths (`Files::keystore_paths`).
pub const KEYSTORE_PATHS_ENV: &str = "UPLINK_KEYSTORE_PATHS";

/// A node's keystore.yaml is a few KB; anything far bigger is the wrong file.
const MAX_SECRET_FILE: u64 = 64 * 1024;

impl Files for LocalFiles {
    fn read_secret(&self, path: &str) -> PortResult<Zeroizing<String>> {
        use std::io::Read as _;
        let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        // Allocated once, so no reallocation leaves an unwiped copy behind.
        let mut text = Zeroizing::new(String::with_capacity(MAX_SECRET_FILE as usize + 1));
        file.take(MAX_SECRET_FILE + 1).read_to_string(&mut text).map_err(|e| format!("{path}: {e}"))?;
        if text.len() as u64 > MAX_SECRET_FILE {
            return Err(format!("{path}: larger than {} KB, not a keystore", MAX_SECRET_FILE / 1024));
        }
        Ok(text)
    }

    fn keystore_paths(&self) -> bool {
        std::env::var(KEYSTORE_PATHS_ENV).is_ok_and(|v| v == "1")
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
    }
}
