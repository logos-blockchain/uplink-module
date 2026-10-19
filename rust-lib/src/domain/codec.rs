//! Hex ids and `lez_core`'s account JSON.

use lee_core::account::AccountId;
use referral_core::NodeId;
use serde_json::Value;

use super::{DomainError, Result};

pub fn bytes32(hex_str: &str) -> Result<[u8; 32]> {
    let raw = hex::decode(hex_str.trim().trim_start_matches("0x"))
        .map_err(|e| DomainError::new("invalid_hex", format!("{hex_str}: {e}")))?;
    raw.try_into()
        .map_err(|_| DomainError::new("invalid_hex", format!("{hex_str}: expected 32 bytes")))
}

pub fn bytes(hex_str: &str) -> Result<Vec<u8>> {
    hex::decode(hex_str.trim().trim_start_matches("0x"))
        .map_err(|e| DomainError::new("invalid_hex", format!("{e}")))
}

pub fn account(hex_str: &str) -> Result<AccountId> {
    bytes32(hex_str).map(AccountId::new)
}

/// An account as hex, or as base58 the way LEZ tools print it (the uplink devnet, wallets).
pub fn account_any(id: &str) -> Result<AccountId> {
    let id = id.trim();
    if id.trim_start_matches("0x").len() == 64 {
        return account(id);
    }
    id.parse::<AccountId>()
        .map_err(|e| DomainError::new("invalid_account", format!("{id}: neither 32 bytes of hex nor base58 ({e})")))
}

pub fn node(hex_str: &str) -> Result<NodeId> {
    bytes32(hex_str).map(NodeId::new)
}

/// `""` means no referrer.
pub fn optional_node(hex_str: &str) -> Result<Option<NodeId>> {
    if hex_str.trim().is_empty() { Ok(None) } else { node(hex_str).map(Some) }
}

pub fn account_hex(id: &AccountId) -> String {
    hex::encode(id.value())
}

pub fn node_hex(node: &NodeId) -> String {
    hex::encode(node.to_bytes())
}

/// The data one program keeps in an account, from `lez_core`'s
/// `{"nonce", "shards": [{"program", "data"}]}`. Empty when the program has none.
pub fn shard(account_json: &str, program: &AccountId) -> Result<Vec<u8>> {
    let value: Value = serde_json::from_str(account_json)
        .map_err(|e| DomainError::new("bad_account", format!("{e}")))?;
    let shards = value
        .get("shards")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("bad_account", "no shards"))?;
    let want = account_hex(program);
    for shard in shards {
        let program_hex = shard.get("program").and_then(Value::as_str).unwrap_or_default();
        if program_hex.trim_start_matches("0x").eq_ignore_ascii_case(&want) {
            return bytes(shard.get("data").and_then(Value::as_str).unwrap_or_default());
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_programs_shard_and_nothing_else() {
        let program = AccountId::new([9; 32]);
        let json = format!(
            r#"{{"nonce":"00","shards":[{{"program":"{}","data":"0102"}},{{"program":"{}","data":"aa"}}]}}"#,
            hex::encode([1; 32]),
            hex::encode([9; 32]),
        );
        assert_eq!(shard(&json, &program).unwrap(), vec![0xaa]);
        assert!(shard(&json, &AccountId::new([7; 32])).unwrap().is_empty());
    }

    #[test]
    fn hex_ids_accept_a_prefix_and_reject_wrong_lengths() {
        assert_eq!(bytes32(&format!("0x{}", hex::encode([3; 32]))).unwrap(), [3; 32]);
        assert_eq!(bytes32("abcd").unwrap_err().code, "invalid_hex");
        assert_eq!(optional_node("").unwrap(), None);
    }
}
