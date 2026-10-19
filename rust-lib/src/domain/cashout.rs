//! Cash-out receipts, as logos-blockchain/uplink defines them:
//!
//! ```text
//! receipt_secret     = SHA256("Uplink/ReceiptSecret/v1\0" || node secret key)
//! blinding_factor(i) = referral_core::blinding_factor(receipt_secret, uplink, participant, i)
//! receipt(i)         = referral_core::cash_out_receipt(uplink, node, blinding_factor(i))
//! ```
//!
//! Only the node's operator can link a receipt to the node, and a keystore backup recovers
//! every receipt.

use lee_core::account::AccountId;
use referral_core::{NodeId, blinding_factor as core_blinding_factor, cash_out_receipt};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::keystore::NodeKey;

const RECEIPT_SECRET_DOMAIN: &[u8; 24] = b"Uplink/ReceiptSecret/v1\0";

/// The node's receipt secret. Matches `uplink_wallet::receipt_secret`.
pub fn receipt_secret(node_secret: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(Sha256::new().chain_update(RECEIPT_SECRET_DOMAIN).chain_update(node_secret).finalize().into())
}

pub fn blinding_factor(key: &NodeKey, uplink: AccountId, participant: AccountId, index: u64) -> [u8; 32] {
    ReceiptKeys::from_key(key).blinding_factor(uplink, participant, index)
}

pub fn receipt(uplink: AccountId, node: NodeId, blinding_factor: [u8; 32]) -> AccountId {
    cash_out_receipt(uplink, node, blinding_factor)
}

/// What cash-out and receipts need from the node: its ID and its receipt secret. From the node
/// key itself (a keystore, on the command line), or handed in by whoever holds the key (the
/// Blockchain app, which derives the secret and keeps the key). Wiped when dropped.
pub struct ReceiptKeys {
    pub node: NodeId,
    secret: Zeroizing<[u8; 32]>,
}

impl ReceiptKeys {
    pub fn from_key(key: &NodeKey) -> Self {
        Self { node: key.node(), secret: key.receipt_secret() }
    }

    pub fn from_secret(node: NodeId, secret: Zeroizing<[u8; 32]>) -> Self {
        Self { node, secret }
    }

    pub fn blinding_factor(&self, uplink: AccountId, participant: AccountId, index: u64) -> [u8; 32] {
        core_blinding_factor(&self.secret, uplink, participant, index)
    }

    /// A fingerprint of the secret, safe to keep: the same secret always gives the same one,
    /// so a different secret handed in later can be refused.
    pub fn fingerprint(&self) -> String {
        hex::encode(Sha256::new().chain_update(RECEIPT_FINGERPRINT_DOMAIN).chain_update(*self.secret).finalize())
    }
}

const RECEIPT_FINGERPRINT_DOMAIN: &[u8; 29] = b"Uplink/ReceiptFingerprint/v1\0";

/// What the node signs for a payout code: the receipt's opening as JSON, keys sorted.
pub fn payout_message(blinding_factor: [u8; 32], node: NodeId, uplink: AccountId) -> String {
    let opening = std::collections::BTreeMap::from([
        ("blinding_factor", hex::encode(blinding_factor)),
        ("node", hex::encode(node.to_bytes())),
        ("program_account", hex::encode(uplink.value())),
    ]);
    serde_json::to_string(&opening).expect("strings serialize")
}

/// The payout code: the opening and the node's signature over it, base64url JSON after
/// `PAYOUT_PREFIX`. `None` unless the signature is the node's over `message`.
pub fn payout_code(message: &str, node: NodeId, signature: &[u8; 64]) -> Option<String> {
    use base64::Engine as _;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&node.to_bytes()).ok()?;
    key.verify_strict(message.as_bytes(), &ed25519_dalek::Signature::from_bytes(signature)).ok()?;
    let mut opening: std::collections::BTreeMap<String, String> = serde_json::from_str(message).ok()?;
    opening.insert("signature".into(), hex::encode(signature));
    let json = serde_json::to_string(&opening).expect("strings serialize");
    Some(format!("{PAYOUT_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)))
}

/// The prefix of payout codes Uplink understands (`PayoutCode.h`).
pub const PAYOUT_PREFIX: &str = "uplink-payout-v2:";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_receipt_secret_matches_uplinks_test_vector() {
        assert_eq!(
            *receipt_secret(&[7; 32]),
            [
                0x7f, 0xa0, 0x48, 0xd0, 0xe5, 0x46, 0x69, 0x25, 0xa9, 0xff, 0x21, 0x3b, 0x16, 0x8f, 0xcb, 0x2d, 0xd0, 0x8d,
                0x61, 0xb6, 0xee, 0xed, 0xf7, 0x79, 0x50, 0x2a, 0x13, 0x65, 0x03, 0x41, 0x9f, 0x30,
            ]
        );
    }

    #[test]
    fn a_handed_in_secret_works_like_the_key_it_came_from() {
        let key = NodeKey::from_secret(&[1; 32]);
        let handed = ReceiptKeys::from_secret(key.node(), receipt_secret(&[1; 32]));
        let (uplink, participant) = (AccountId::new([9; 32]), AccountId::new([3; 32]));
        assert_eq!(handed.blinding_factor(uplink, participant, 4), blinding_factor(&key, uplink, participant, 4));
        assert_eq!(handed.fingerprint(), ReceiptKeys::from_key(&key).fingerprint());
        assert_ne!(handed.fingerprint(), ReceiptKeys::from_key(&NodeKey::from_secret(&[2; 32])).fingerprint());
        assert!(!handed.fingerprint().contains(&hex::encode(*receipt_secret(&[1; 32]))));
    }

    #[test]
    fn a_payout_code_needs_the_nodes_signature_over_the_message() {
        let key = NodeKey::from_secret(&[1; 32]);
        let message = payout_message([5; 32], key.node(), AccountId::new([9; 32]));
        assert!(payout_code(&message, key.node(), &key.sign(message.as_bytes())).unwrap().starts_with(PAYOUT_PREFIX));
        let other = NodeKey::from_secret(&[2; 32]);
        assert!(payout_code(&message, key.node(), &other.sign(message.as_bytes())).is_none());
        assert!(payout_code(&message, key.node(), &key.sign(b"something else")).is_none());
    }

    #[test]
    fn is_deterministic_and_distinct_per_receipt() {
        let key = NodeKey::from_secret(&[1; 32]);
        let (uplink, participant) = (AccountId::new([9; 32]), AccountId::new([3; 32]));
        let first = blinding_factor(&key, uplink, participant, 0);
        assert_eq!(first, blinding_factor(&NodeKey::from_secret(&[1; 32]), uplink, participant, 0));
        assert_ne!(first, blinding_factor(&key, uplink, participant, 1));
        assert_ne!(first, blinding_factor(&NodeKey::from_secret(&[2; 32]), uplink, participant, 0));
        assert_ne!(receipt(uplink, key.node(), first), receipt(uplink, key.node(), [0; 32]));
    }
}
