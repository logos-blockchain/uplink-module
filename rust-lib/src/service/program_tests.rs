//! The module's transactions, run through the real referral program (`referral_program`'s
//! `plan` and `apply`, from uplink), offline. The fake chain from `tests.rs` holds the
//! program's state; each transaction the module sends is checked by the program's rules
//! and its effects are written back, as the sequencer would. No `lez_core`, wallet or network.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use lee_core::account::AccountId;
use lee_core::program::{AccountMeta, PlanInput};
use referral_core::{Effect, Instruction, NodeId, ORACLE_ACCOUNT_ID, ParticipantAuthorizationV1, State, epoch_account};
use sha2::{Digest, Sha256};

use super::tests::{FakeHistory, PARTICIPANT, World, account_json, keys_json, node_key, program, publish_json, reference};
use crate::domain::invitation::Invitation;
use crate::domain::keystore::NodeKey;
use crate::domain::{codec, terms};
use crate::ports::AccountEntry;

/// Where an account the program writes lives.
#[derive(Clone, Copy)]
enum Home {
    /// A private account in this module's (fake) wallet.
    Wallet,
    /// A public account (registry, archives, receipts).
    Public,
    /// Somebody else's private account: another participant, or a note for them.
    Elsewhere,
}

fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "the program rejected it".into())
}

impl World {
    fn shard(&self, id: &AccountId, home: Home) -> Vec<u8> {
        let hex_id = codec::account_hex(id);
        let chain = self.lez.0.borrow();
        let json = match home {
            Home::Wallet => chain.private.get(&hex_id).map(|(account, _)| account.clone()),
            Home::Public => chain.public.get(&hex_id).cloned(),
            Home::Elsewhere => chain.elsewhere.get(&hex_id).cloned(),
        };
        json.filter(|json| !json.is_empty())
            .map(|json| codec::shard(&json, &program()).unwrap())
            .unwrap_or_default()
    }

    fn write(&self, id: &AccountId, home: Home, data: Vec<u8>, keys: Option<String>) {
        let (hex_id, json) = (codec::account_hex(id), account_json(&data));
        let mut chain = self.lez.0.borrow_mut();
        match home {
            Home::Wallet => {
                let keys = keys.or_else(|| chain.private.get(&hex_id).map(|(_, k)| k.clone())).expect("wallet keys");
                chain.private.insert(hex_id, (json, keys));
            }
            Home::Public => {
                chain.public.insert(hex_id, json);
            }
            Home::Elsewhere => {
                chain.elsewhere.insert(hex_id, json);
            }
        }
    }

    /// Runs one instruction through the real program; all-or-nothing, like a transaction.
    /// `accounts`: (id, authorized, where it lives, wallet keys for a new wallet account).
    fn execute(&self, accounts: Vec<(AccountId, bool, Home, Option<String>)>, instruction: Instruction) -> Result<(), String> {
        let input = PlanInput {
            self_account_id: program(),
            caller_account_id: None,
            accounts: accounts.iter().map(|(id, authorized, _, _)| AccountMeta::new(*id, *authorized, program())).collect(),
            instruction_data: Vec::new(),
        };
        let planned = catch_unwind(AssertUnwindSafe(|| referral_program::plan(&input, instruction))).map_err(panic_text)?;

        let mut posts: BTreeMap<AccountId, Vec<u8>> = BTreeMap::new();
        for shard_effect in &planned.output().effects {
            let id = shard_effect.selector.account_id;
            let home = accounts.iter().find(|(a, ..)| *a == id).map(|(_, _, home, _)| *home).expect("a planned account");
            let pre = posts.get(&id).cloned().unwrap_or_else(|| self.shard(&id, home));
            let effect: Effect = borsh::from_slice(&shard_effect.data).map_err(|e| e.to_string())?;
            let pre = lee_core::account::ShardData::try_from(pre).unwrap();
            if let Some(post) = catch_unwind(AssertUnwindSafe(|| referral_program::apply(effect, &pre))).map_err(panic_text)? {
                posts.insert(id, post.into_inner());
            }
        }
        for (id, data) in posts {
            let (_, _, home, keys) = accounts.iter().find(|(a, ..)| *a == id).expect("a planned account").clone();
            self.write(&id, home, data, keys);
        }
        Ok(())
    }

    /// Settles the last transaction the module sent, through the real program.
    fn settle_last(&self) -> Result<(), String> {
        let (entries, bytes) = self.sent().pop().ok_or("nothing was sent")?;
        let mut accounts = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            accounts.push(match entry {
                AccountEntry::Own(id) => (codec::account(id).unwrap(), true, Home::Wallet, None),
                AccountEntry::PublicNoSign(id) => (codec::account(id).unwrap(), false, Home::Public, None),
                // A fresh account under another user's keys; where it ends up is their wallet.
                AccountEntry::PrivateForeign { npk, .. } => {
                    let id = Sha256::new().chain_update(npk).chain_update(bytes.as_slice()).chain_update([index as u8]).finalize();
                    (AccountId::new(id.into()), false, Home::Elsewhere, None)
                }
            });
        }
        self.execute(accounts, borsh::from_slice(&bytes).map_err(|e| e.to_string())?)?;
        self.lez.0.borrow_mut().settled = true;
        Ok(())
    }

    /// The oracle publishing `epoch`.
    fn publish(&self, epoch: u32, active: &[NodeId]) {
        let accounts = vec![
            (ORACLE_ACCOUNT_ID, true, Home::Public, None),
            (epoch_account(program(), epoch), false, Home::Public, None),
        ];
        self.execute(accounts, Instruction::Publish { epoch, active: active.iter().copied().collect() }).unwrap();
    }

    /// Another node registering under `referrer`, from its own wallet. When `referrer` is this
    /// module's node, the child note lands in this wallet, addressed to the participant.
    fn register_other(&self, seed: u8, referrer: Option<(NodeId, String)>) -> NodeId {
        let key = NodeKey::from_secret(&[seed; 32]);
        let participant = AccountId::new([seed ^ 0xf0; 32]);
        let registry = crate::domain::state::registry(&self.shard(&ORACLE_ACCOUNT_ID, Home::Public)).unwrap();
        let referrer_keys = referrer.as_ref().map(|(node, keys)| Invitation::from_keys(*node, keys).unwrap().referrer_keys().unwrap());
        let message = ParticipantAuthorizationV1::new(program(), key.node(), participant, referrer.as_ref().map(|(n, _)| *n)).message();
        let mut accounts = vec![(participant, true, Home::Elsewhere, None), (ORACLE_ACCOUNT_ID, false, Home::Public, None)];
        if let Some((parent, keys)) = &referrer {
            let ours = *parent == node_key().node();
            let note = AccountId::new([seed ^ 0x0f; 32]);
            accounts.push((note, false, if ours { Home::Wallet } else { Home::Elsewhere }, ours.then(|| keys.clone())));
        }
        self.execute(
            accounts,
            Instruction::Register {
                node: key.node(),
                referrer: referrer_keys,
                node_signature: key.sign(&message),
                pool: registry.registered_nodes,
                epoch: registry.epoch,
            },
        )
        .unwrap();
        key.node()
    }

    fn balance(&self) -> u128 {
        crate::domain::state::participant(&self.shard(&codec::account(PARTICIPANT).unwrap(), Home::Wallet))
            .unwrap()
            .reward_balance
    }
}

#[test]
fn enrol_claim_late_and_cash_out_pass_the_real_program() {
    let world = World::empty();
    let mut service = world.service();
    let me = node_key().node();
    let participant_keys = keys_json(0x11);

    // An inviter registers from its own wallet; we import its invitation and join under it.
    let inviter_keys = keys_json(0x22);
    let inviter = world.register_other(2, None);
    service.import_invitation(PARTICIPANT, &Invitation::from_keys(inviter, &inviter_keys).unwrap().encode().unwrap()).unwrap();
    service.accept_terms(terms::version(), &terms::sha256()).unwrap();
    service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", &codec::node_hex(&me)).unwrap();
    world.settle_last().expect("the program accepts our Register");
    assert_eq!(service.participant(PARTICIPANT).unwrap()["referrer"], codec::node_hex(&inviter));

    // A node joins under us: its Register leaves a child note in our wallet.
    let child = world.register_other(3, Some((me, participant_keys)));

    // Epochs 1 and 2 are published; we see 1 live, then claim both at once (late).
    world.publish(1, &[inviter, me, child]);
    service.registry().unwrap();
    world.publish(2, &[me, child]);
    assert_eq!(service.claimable(PARTICIPANT).unwrap()["points"], "2");
    service.claim(&reference(2), PARTICIPANT).unwrap();
    world.settle_last().expect("the program accepts our multi-epoch Claim");
    assert_eq!(world.balance(), 2);
    // The inviter was active in epoch 1 only: exactly one credit went up.
    let credits: Vec<State> = world
        .lez
        .0
        .borrow()
        .elsewhere
        .values()
        .filter_map(|json| crate::domain::state::note(&codec::shard(json, &program()).unwrap()))
        .filter(|state| matches!(state, State::Credit { .. }))
        .collect();
    assert_eq!(credits.len(), 1);
    assert!(matches!(credits[0], State::Credit { recipient_node, amount: 1, epoch: 1, .. } if recipient_node == inviter));

    // Nothing left to claim, and the program agrees.
    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["skipped"], "nothing_to_claim");

    // Cash out the whole balance into a receipt only our node can find.
    service.cash_out(&reference(3), PARTICIPANT, "/node/keystore.yaml").unwrap();
    world.settle_last().expect("the program accepts our CashOut");
    assert_eq!(world.balance(), 0);
    let receipts = service.receipts(PARTICIPANT, "/node/keystore.yaml").unwrap();
    assert_eq!(receipts["receipts"][0]["points"], "2");
    assert!(service.payout_code(PARTICIPANT, 0, "/node/keystore.yaml").unwrap()["code"].as_str().unwrap().starts_with("uplink-payout-v2:"));
}

#[test]
fn the_program_rejects_a_claim_built_against_the_wrong_publication() {
    // Guards the program check itself: a tampered transaction must fail here.
    let world = World::empty();
    let me = node_key().node();
    world.publish(1, &[me]);
    let tampered = Instruction::Claim(referral_core::Claim {
        epochs: vec![referral_core::EpochClaim { epoch: 1, active: BTreeSet::from([me, NodeId::new([9; 32])]), notes: vec![], total: 1 }],
        referrer: None,
    });
    let accounts = vec![
        (codec::account(PARTICIPANT).unwrap(), true, Home::Wallet, None),
        (epoch_account(program(), 1), false, Home::Public, None),
    ];
    assert!(world.execute(accounts, tampered).is_err());
}

#[test]
fn an_epoch_learned_late_is_claimed_after_a_later_one_and_the_program_pays_it() {
    let world = World::empty();
    let mut service = world.service();
    let me = node_key().node();
    service.accept_terms(terms::version(), &terms::sha256()).unwrap();
    service.enrol(&reference(1), PARTICIPANT, "/node/keystore.yaml", &codec::node_hex(&me)).unwrap();
    world.settle_last().expect("the program accepts our Register");
    let child = world.register_other(3, Some((me, keys_json(0x11))));

    // Epoch 1 is published while the module isn't looking; epoch 2 it sees live.
    world.publish(1, &[me, child]);
    world.publish(2, &[me, child]);
    service.registry().unwrap();
    let claimable = service.claimable(PARTICIPANT).unwrap();
    assert_eq!((claimable["points"].clone(), claimable["history"]["missingEpochs"].clone()), (serde_json::json!("1"), serde_json::json!([1])));

    // Epoch 2 alone, first.
    assert_eq!(service.claim(&reference(2), PARTICIPANT).unwrap()["detail"]["epochs"], serde_json::json!([2]));
    world.settle_last().expect("the program accepts a claim that skips epoch 1");
    assert_eq!(world.balance(), 1);

    // An indexer supplies epoch 1; it's claimed after 2, and the program pays it.
    let mut service = world.service_with(Some(Box::new(FakeHistory(publish_json(1, BTreeSet::from([me, child]))))));
    let claimable = service.claimable(PARTICIPANT).unwrap();
    assert_eq!((claimable["points"].clone(), claimable["history"].clone()), (serde_json::json!("1"), serde_json::Value::Null));
    assert_eq!(service.claim(&reference(3), PARTICIPANT).unwrap()["detail"]["epochs"], serde_json::json!([1]));
    world.settle_last().expect("the program accepts epoch 1 after epoch 2");
    assert_eq!(world.balance(), 2);

    // Each epoch paid once: nothing left, and the program refuses epoch 2 again.
    assert_eq!(service.auto_claim(PARTICIPANT).unwrap()["skipped"], "nothing_to_claim");
    let again = Instruction::Claim(referral_core::Claim {
        epochs: vec![referral_core::EpochClaim { epoch: 2, active: BTreeSet::from([me, child]), notes: vec![], total: 1 }],
        referrer: None,
    });
    let accounts = vec![
        (codec::account(PARTICIPANT).unwrap(), true, Home::Wallet, None),
        (epoch_account(program(), 2), false, Home::Public, None),
    ];
    assert!(world.execute(accounts, again).is_err());
    assert_eq!(world.balance(), 2);
}
