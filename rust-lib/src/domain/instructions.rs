//! The referral program's instructions, as the bytes `lez_core` sends.

use std::collections::BTreeSet;

use referral_core::{Claim, Instruction, NodeId, Referrer};

fn encode(instruction: &Instruction) -> Vec<u8> {
    borsh::to_vec(instruction).expect("borsh serialization is infallible")
}

/// `epoch` is the registry's current epoch; the program checks it.
pub fn register(
    node: NodeId,
    referrer: Option<Referrer>,
    node_signature: [u8; 64],
    pool: BTreeSet<NodeId>,
    epoch: u32,
) -> Vec<u8> {
    encode(&Instruction::Register { node, referrer, node_signature, pool, epoch })
}

pub fn claim(claim: Claim) -> Vec<u8> {
    encode(&Instruction::Claim(claim))
}

pub fn cash_out(node: NodeId, points: u128, blinding_factor: [u8; 32]) -> Vec<u8> {
    encode(&Instruction::CashOut { node, points, blinding_factor })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_what_the_program_decodes() {
        let node = NodeId::new([7; 32]);
        let bytes = register(node, None, [5; 64], BTreeSet::from([node]), 3);
        let Instruction::Register { node: decoded, referrer, pool, epoch, .. } = borsh::from_slice(&bytes).unwrap() else {
            panic!("not a Register");
        };
        assert_eq!((decoded, referrer, pool, epoch), (node, None, BTreeSet::from([node]), 3));

        let Instruction::CashOut { points, .. } = borsh::from_slice(&cash_out(node, 9, [1; 32])).unwrap() else {
            panic!("not a CashOut");
        };
        assert_eq!(points, 9);
    }
}
