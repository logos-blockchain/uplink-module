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
    let secret = key.receipt_secret();
    core_blinding_factor(&secret, uplink, participant, index)
}

pub fn receipt(uplink: AccountId, node: NodeId, blinding_factor: [u8; 32]) -> AccountId {
    cash_out_receipt(uplink, node, blinding_factor)
}

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
