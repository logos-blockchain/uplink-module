//! Old publications from the LEZ indexer: each `Publish` the oracle made, from
//! `lez_indexer_module.getTransactionsByAccount(ORACLE_ACCOUNT_ID, …)`. Ported from
//! uplink's wallet `published`.

use std::collections::BTreeSet;

use indexer_service_protocol::Transaction;
use lee_core::account::AccountId;
use referral_core::{Effect, Instruction, NodeId, ORACLE_ACCOUNT_ID};

/// The (epoch, active set) a transaction published, if it is one of the program's `Publish`es.
pub fn published(transaction: &Transaction, program: AccountId) -> Option<(u32, BTreeSet<NodeId>)> {
    let program = *program.value();
    match transaction {
        Transaction::Public(public) => {
            if public.message.program_account_id.value != program {
                return None;
            }
            match borsh::from_slice(&public.message.instruction_data).ok()? {
                Instruction::Publish { epoch, active } => Some((epoch, active)),
                _ => None,
            }
        }
        Transaction::PrivacyPreserving(private) => private
            .message
            .public_actions
            .iter()
            .filter(|action| action.account_id.value == *ORACLE_ACCOUNT_ID.value())
            .flat_map(|action| &action.effects)
            .filter(|effect| effect.program_account_id.value == program)
            .find_map(|effect| match borsh::from_slice(&effect.data).ok()? {
                Effect::Publish { epoch, active } => Some((epoch, active)),
                _ => None,
            }),
    }
}

/// Every publication in a page of indexer JSON (an array of transactions), with its position
/// in the page. Entries that don't parse are skipped, so a newer indexer's extra kinds don't
/// break older pages.
pub fn page(json: &str, program: AccountId) -> (usize, Vec<(usize, u32, BTreeSet<NodeId>)>) {
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return (0, Vec::new());
    };
    let found = entries
        .iter()
        .enumerate()
        .filter_map(|(at, entry)| Some((at, serde_json::from_value::<Transaction>(entry.clone()).ok()?)))
        .filter_map(|(at, transaction)| published(&transaction, program).map(|(epoch, active)| (at, epoch, active)))
        .collect();
    (entries.len(), found)
}

#[cfg(test)]
mod tests {
    use indexer_service_protocol::{HashType, PublicMessage, PublicTransaction, WitnessSet};

    use super::*;

    fn publish(program: [u8; 32], epoch: u32, active: BTreeSet<NodeId>) -> Transaction {
        Transaction::Public(PublicTransaction {
            hash: HashType([0; 32]),
            message: PublicMessage {
                program_account_id: indexer_service_protocol::AccountId { value: program },
                shard_selectors: vec![],
                nonces: vec![],
                instruction_data: borsh::to_vec(&Instruction::Publish { epoch, active }).unwrap(),
                fee: None,
            },
            witness_set: WitnessSet { signatures_and_public_keys: vec![], proof: None },
        })
    }

    #[test]
    fn reads_publishes_for_this_program_only() {
        let active = BTreeSet::from([NodeId::new([7; 32])]);
        let page_json = serde_json::to_string(&vec![
            publish([1; 32], 5, active.clone()),
            publish([9; 32], 4, active.clone()),
        ])
        .unwrap();
        let (count, found) = page(&page_json, AccountId::new([9; 32]));
        assert_eq!(count, 2);
        assert_eq!(found, vec![(1, 4, active)], "with its position in the page");
    }
}
