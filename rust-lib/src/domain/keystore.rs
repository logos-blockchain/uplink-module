//! Reading the node's BlendSigning key from its `keystore.yaml` (node 0.3.1 format):
//!
//! ```yaml
//! public_keys:
//!   BlendSigning: <hex public key>
//! secret_keys:
//!   BlendSigning: !Ed25519 <hex 32-byte secret>
//!   BlendZk: !Zk <hex>
//! ```
//!
//! Only Ed25519 secrets are kept, in zeroizing buffers; ZK secrets are skipped unread.
//! The parser's own buffers are not wiped.

use std::collections::BTreeMap;

use ed25519_dalek::{Signer as _, SigningKey};
use referral_core::NodeId;
use serde::Deserialize;
use serde::de::IgnoredAny;
use zeroize::Zeroizing;

use super::{DomainError, Result};

pub const BLEND_SIGNING: &str = "BlendSigning";

/// A YAML error without its message: serde_yaml quotes the value it failed on, which here
/// can be a secret key. The position is enough to find the problem.
fn bad_yaml(error: &serde_yaml::Error) -> DomainError {
    let at = error
        .location()
        .map(|at| format!(" at line {}, column {}", at.line(), at.column()))
        .unwrap_or_default();
    DomainError::new("bad_keystore", format!("the keystore is not valid YAML{at}"))
}

#[derive(Deserialize)]
enum SecretKey {
    Ed25519(Zeroizing<String>),
    #[allow(dead_code)]
    Zk(IgnoredAny),
}

#[derive(Deserialize)]
struct Keystore {
    #[serde(default)]
    secret_keys: BTreeMap<String, SecretKey>,
}

#[derive(Deserialize)]
struct PublicKeys {
    #[serde(default)]
    public_keys: BTreeMap<String, String>,
}

/// The node a keystore is for: its `BlendSigning` public key. Reads no secret key.
pub fn blend_signing_public(keystore_yaml: &str) -> Result<NodeId> {
    let keys: PublicKeys = serde_yaml::from_str(keystore_yaml).map_err(|e| bad_yaml(&e))?;
    let hex_key = keys
        .public_keys
        .get(BLEND_SIGNING)
        .ok_or_else(|| DomainError::new("key_not_found", "the keystore has no BlendSigning key"))?;
    let bytes: [u8; 32] = hex::decode(hex_key.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| DomainError::new("bad_keystore", "the BlendSigning public key is not 32 bytes of hex"))?;
    Ok(NodeId::new(bytes))
}

/// The node's BlendSigning key. Wiped from memory when dropped.
pub struct NodeKey(SigningKey);

impl NodeKey {
    pub fn from_secret(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    pub fn node(&self) -> NodeId {
        NodeId::new(self.0.verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }

    /// The node's cash-out receipt secret (`super::cashout::receipt_secret`).
    pub fn receipt_secret(&self) -> Zeroizing<[u8; 32]> {
        let seed = Zeroizing::new(self.0.to_bytes());
        super::cashout::receipt_secret(&seed)
    }
}

/// The Ed25519 key for `node`, or the `BlendSigning` entry when `node` is unknown.
/// Matching by public key means a renamed title can't make it sign for another node.
pub fn blend_signing_key(keystore_yaml: &str, node: Option<NodeId>) -> Result<NodeKey> {
    let keystore: Keystore = serde_yaml::from_str(keystore_yaml).map_err(|e| bad_yaml(&e))?;

    let mut by_title = None;
    for (title, secret) in &keystore.secret_keys {
        let SecretKey::Ed25519(hex_secret) = secret else { continue };
        let key = ed25519_secret(hex_secret)?;
        match node {
            Some(node) if key.node() == node => return Ok(key),
            None if title == BLEND_SIGNING => by_title = Some(key),
            _ => {}
        }
    }
    by_title.ok_or_else(|| match node {
        Some(node) => DomainError::new(
            "keystore_mismatch",
            format!("this keystore has no key for node {}", hex::encode(node.to_bytes())),
        ),
        None => DomainError::new("key_not_found", "the keystore has no BlendSigning key"),
    })
}

fn ed25519_secret(hex_secret: &str) -> Result<NodeKey> {
    let raw = Zeroizing::new(
        hex::decode(hex_secret.trim()).map_err(|_| DomainError::new("bad_keystore", "an Ed25519 secret is not hex"))?,
    );
    let secret: Zeroizing<[u8; 32]> = Zeroizing::new(
        raw.as_slice()
            .try_into()
            .map_err(|_| DomainError::new("bad_keystore", "an Ed25519 secret is not 32 bytes"))?,
    );
    Ok(NodeKey::from_secret(&secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keystore() -> String {
        format!(
            "public_keys:\n  BlendSigning: {pb}\n  NetworkSwarm: {pn}\nsecret_keys:\n  BlendSigning: !Ed25519 {sb}\n  NetworkSwarm: !Ed25519 {sn}\n  BlendZk: !Zk {sz}\nWARNING: Do not share your secret keys\n",
            pb = hex::encode(NodeKey::from_secret(&[1; 32]).node().to_bytes()),
            pn = hex::encode(NodeKey::from_secret(&[2; 32]).node().to_bytes()),
            sb = hex::encode([1; 32]),
            sn = hex::encode([2; 32]),
            sz = hex::encode([3; 32]),
        )
    }

    #[test]
    fn picks_the_key_whose_public_key_is_the_node() {
        let swarm = NodeKey::from_secret(&[2; 32]).node();
        assert_eq!(blend_signing_key(&keystore(), Some(swarm)).unwrap().node(), swarm);
    }

    #[test]
    fn falls_back_to_the_blend_signing_title_without_a_node() {
        let blend = NodeKey::from_secret(&[1; 32]).node();
        assert_eq!(blend_signing_key(&keystore(), None).unwrap().node(), blend);
    }

    #[test]
    fn names_the_node_from_the_public_keys_alone() {
        assert_eq!(blend_signing_public(&keystore()).unwrap(), NodeKey::from_secret(&[1; 32]).node());
    }

    #[test]
    fn refuses_a_node_the_keystore_does_not_hold() {
        let err = blend_signing_key(&keystore(), Some(NodeId::new([9; 32]))).err().unwrap();
        assert_eq!(err.code, "keystore_mismatch");
        assert_eq!(blend_signing_key("secret_keys: [", None).err().unwrap().code, "bad_keystore");
    }

    #[test]
    fn errors_never_quote_the_keystore() {
        let secret = hex::encode([7; 32]);
        // A secret where a map is expected, under each section the two readers parse.
        for yaml in [format!("secret_keys: {secret}\n"), format!("public_keys: {secret}\n")] {
            let raw = serde_yaml::from_str::<Keystore>(&yaml).err().or(serde_yaml::from_str::<PublicKeys>(&yaml).err());
            assert!(raw.unwrap().to_string().contains(&secret), "serde_yaml's own message quotes the value");
            for err in [blend_signing_key(&yaml, None).err(), blend_signing_public(&yaml).err()].into_iter().flatten() {
                assert!(!err.message.contains(&secret[..16]), "{}", err.message);
            }
        }
    }
}
