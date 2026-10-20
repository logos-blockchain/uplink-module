//! Old publications from the LEZ indexer: each `Publish` the oracle made, from
//! `lez_indexer_module.getTransactionsByAccount(ORACLE_ACCOUNT_ID, …)`. Ported from
//! uplink's wallet `published`.
//!
//! lez_indexer_module answers in its own JSON, not the indexer service's: a flat object per
//! transaction, `{"type": "Public", "program_account_id": <base58>, "instruction_data": <hex>,
//! …}`. Its privacy-preserving entries carry only counts, so a publication made in one can't
//! be read from it; the oracle publishes in public transactions. The service's format is
//! read too, as uplink's own tools get it.

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

/// The (epoch, active set) a lez_indexer_module entry published, if it is one of the
/// program's `Publish`es.
pub fn module_published(entry: &serde_json::Value, program: AccountId) -> Option<(u32, BTreeSet<NodeId>)> {
    if entry["type"] != "Public" {
        return None;
    }
    if entry["program_account_id"].as_str()?.parse::<AccountId>().ok()? != program {
        return None;
    }
    let data = hex::decode(entry["instruction_data"].as_str()?).ok()?;
    match borsh::from_slice(&data).ok()? {
        Instruction::Publish { epoch, active } => Some((epoch, active)),
        _ => None,
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
        .filter_map(|(at, entry)| {
            module_published(entry, program)
                .or_else(|| published(&serde_json::from_value::<Transaction>(entry.clone()).ok()?, program))
                .map(|(epoch, active)| (at, epoch, active))
        })
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

    // An entry as lez_indexer_module returned it on uplink's devnet: the oracle publishing
    // epoch 3 with no active nodes. History never filled before this format was read.
    #[test]
    fn reads_lez_indexer_modules_own_format() {
        let program = "fnmC5mLF5LujgzVnPcNLXWH7WWkAEdGsq7DJpegrvg5";
        let entry = serde_json::json!({
            "type": "Public",
            "hash": "7caa9cd5a3c30ecc4f46f95a3d067785aa85200253168432f8958b77004ffed3",
            "program_account_id": program,
            "accounts": [
                { "account_id": "6YWUkiCsExdhHHUuFxNRRMjjXkcAdwu4jyB8EN89Bjpv", "program_account_id": program, "nonce": "0" },
                { "account_id": "2dPXhuLG8sZghZU19N43vSKnAeQjQ2i44TcHXsNXLNgd", "program_account_id": program, "nonce": "0" }
            ],
            "instruction_data": "010300000000000000",
            "signature_count": 1,
            "fee": { "payer": "6YWUkiCsExdhHHUuFxNRRMjjXkcAdwu4jyB8EN89Bjpv", "gas_limit": "2000000", "tip": "0", "max_fee": "134400000" }
        });
        let other = serde_json::json!({ "type": "PrivacyPreserving", "hash": "00", "accounts": [], "private_actions_count": 1 });
        let (count, found) = page(&serde_json::json!([other, entry]).to_string(), program.parse().unwrap());
        assert_eq!(count, 2);
        assert_eq!(found, vec![(1, 3, BTreeSet::new())]);
        assert!(page(&serde_json::json!([entry]).to_string(), AccountId::new([9; 32])).1.is_empty(), "another program's");
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
