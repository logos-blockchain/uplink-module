//! Invitations: what a referrer hands to the node they invite. It names the referrer's
//! node and its participant's public keys, so the invitee's Register can leave a note
//! only the referrer can read.
//!
//! Blob: `uplink-invite-v1:` + base64url(node ‖ npk ‖ vpk), no padding. The JSON the `uplink`
//! CLI writes (`referral_core::Invitation`) is accepted too.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lee_core::NullifierPublicKey;
use lee_core::encryption::ViewingPublicKey;
use referral_core::{NodeId, Referrer};
use serde::{Deserialize, Serialize};

use super::{DomainError, Result, codec};

pub const PREFIX: &str = "uplink-invite-v1:";
const VPK_LEN: usize = 1184;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitation {
    /// The referrer's node, hex.
    pub node: String,
    /// The referrer participant's nullifier public key, hex.
    pub npk: String,
    /// The referrer participant's viewing public key, hex (ML-KEM-768, 1184 bytes).
    pub vpk: String,
}

impl Invitation {
    /// From the referrer's node and `lez_core.get_private_account_keys(participant)`.
    pub fn from_keys(node: NodeId, keys_json: &str) -> Result<Self> {
        let keys: serde_json::Value =
            serde_json::from_str(keys_json).map_err(|e| DomainError::new("bad_keys", format!("{e}")))?;
        let field = |name: &str| {
            keys.get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| DomainError::new("bad_keys", format!("no {name}")))
        };
        let invitation = Self { node: codec::node_hex(&node), npk: field("nullifier_public_key")?, vpk: field("viewing_public_key")? };
        invitation.check()?;
        Ok(invitation)
    }

    pub fn referrer(&self) -> Result<NodeId> {
        codec::node(&self.node)
    }

    pub fn encode(&self) -> Result<String> {
        let mut raw = codec::bytes32(&self.node)?.to_vec();
        raw.extend_from_slice(&codec::bytes32(&self.npk)?);
        raw.extend_from_slice(&codec::bytes(&self.vpk)?);
        Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(raw)))
    }

    /// The referrer as Register records it, so later claims can address it from chain state.
    pub fn referrer_keys(&self) -> Result<Referrer> {
        let vpk = ViewingPublicKey::from_bytes(codec::bytes(&self.vpk)?)
            .map_err(|e| DomainError::new("bad_invitation", format!("viewing key: {e:?}")))?;
        Ok(Referrer { node: self.referrer()?, npk: NullifierPublicKey(codec::bytes32(&self.npk)?), vpk })
    }

    pub fn decode(blob: &str) -> Result<Self> {
        if let Ok(cli) = serde_json::from_str::<referral_core::Invitation>(blob.trim()) {
            return Ok(Self {
                node: codec::node_hex(&cli.parent_node),
                npk: hex::encode(cli.npk.0),
                vpk: hex::encode(cli.vpk.to_bytes()),
            });
        }
        let body = blob
            .trim()
            .strip_prefix(PREFIX)
            .ok_or_else(|| DomainError::new("bad_invitation", "not an Uplink invitation"))?;
        let raw = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| DomainError::new("bad_invitation", "the invitation is damaged"))?;
        if raw.len() != 64 + VPK_LEN {
            return Err(DomainError::new("bad_invitation", "the invitation is damaged"));
        }
        Ok(Self { node: hex::encode(&raw[..32]), npk: hex::encode(&raw[32..64]), vpk: hex::encode(&raw[64..]) })
    }

    fn check(&self) -> Result<()> {
        codec::bytes32(&self.node)?;
        codec::bytes32(&self.npk)?;
        if codec::bytes(&self.vpk)?.len() != VPK_LEN {
            return Err(DomainError::new("bad_keys", "the viewing key is not 1184 bytes"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_json() -> String {
        format!(r#"{{"nullifier_public_key":"{}","viewing_public_key":"{}"}}"#, hex::encode([4; 32]), hex::encode([5; VPK_LEN]))
    }

    #[test]
    fn round_trips_through_the_blob() {
        let invitation = Invitation::from_keys(NodeId::new([7; 32]), &keys_json()).unwrap();
        let blob = invitation.encode().unwrap();
        assert!(blob.starts_with(PREFIX));
        assert_eq!(Invitation::decode(&blob).unwrap(), invitation);
        assert_eq!(invitation.referrer().unwrap(), NodeId::new([7; 32]));
        assert_eq!(invitation.referrer_keys().unwrap().npk.0, [4; 32]);
    }

    #[test]
    fn accepts_the_uplink_clis_json() {
        let keys = Invitation::from_keys(NodeId::new([7; 32]), &keys_json()).unwrap().referrer_keys().unwrap();
        let cli = referral_core::Invitation::new(keys.node, keys.npk, keys.vpk);
        let decoded = Invitation::decode(&serde_json::to_string_pretty(&cli).unwrap()).unwrap();
        assert_eq!(decoded, Invitation::from_keys(NodeId::new([7; 32]), &keys_json()).unwrap());
    }

    #[test]
    fn rejects_other_and_damaged_blobs() {
        assert_eq!(Invitation::decode("uplink-payout-v2:abc").unwrap_err().code, "bad_invitation");
        assert_eq!(Invitation::decode(&format!("{PREFIX}AAAA")).unwrap_err().code, "bad_invitation");
    }
}
