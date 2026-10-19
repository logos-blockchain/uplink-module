//! The statement a node signs to join: uplink's `ParticipantAuthorizationV1`.

use lee_core::account::AccountId;
use referral_core::{NodeId, ParticipantAuthorizationV1};

pub fn authorization(
    program: AccountId,
    node: NodeId,
    participant: AccountId,
    referrer: Option<NodeId>,
) -> ParticipantAuthorizationV1 {
    ParticipantAuthorizationV1::new(program, node, participant, referrer)
}

/// The exact bytes the node's BlendSigning key signs.
pub fn message(program: AccountId, node: NodeId, participant: AccountId, referrer: Option<NodeId>) -> Vec<u8> {
    authorization(program, node, participant, referrer).message()
}

/// The same check the program runs on Register.
pub fn verify(
    program: AccountId,
    node: NodeId,
    participant: AccountId,
    referrer: Option<NodeId>,
    signature: &[u8; 64],
) -> bool {
    authorization(program, node, participant, referrer).verify(signature)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    #[test]
    fn a_signature_over_the_message_verifies_and_nothing_else_does() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let node = NodeId::new(key.verifying_key().to_bytes());
        let (program, participant) = (AccountId::new([9; 32]), AccountId::new([3; 32]));
        let referrer = Some(NodeId::new([2; 32]));

        let message = message(program, node, participant, referrer);
        assert_eq!(message.len(), 31 + 3 * 32 + 1 + 32);
        let signature = key.sign(&message).to_bytes();
        assert!(verify(program, node, participant, referrer, &signature));
        assert!(!verify(program, node, participant, None, &signature));
        assert_eq!(super::message(program, node, participant, None).len(), 31 + 3 * 32 + 1);
    }
}
