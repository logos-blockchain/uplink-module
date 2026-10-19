use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use lee_core::account::AccountId;
use referral_core::{Instruction, NodeId, ORACLE_ACCOUNT_ID, Participant, Referrer, Registry, State, active_digest, epoch_account};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::Service;
use crate::domain::invitation::Invitation;
use crate::domain::keystore::NodeKey;
use crate::domain::program::Binary;
use crate::domain::{codec, enrolment};
use crate::ports::{AccountEntry, Clock, Files, History, Lez, PortResult, Store};

pub(super) const PROGRAM: [u8; 32] = [0x50; 32];
pub(super) const IMAGE_ID: [u32; 8] = [7; 8];
pub(super) const PARTICIPANT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
pub(super) const VPK_LEN: usize = 1184;
/// What `lez_core` returns for a private account no program has written to yet.
pub(super) const EMPTY_ACCOUNT: &str = r#"{"nonce":"00","shards":[]}"#;

pub(super) fn program() -> AccountId {
    AccountId::new(PROGRAM)
}

pub(super) fn account_json(data: &[u8]) -> String {
    json!({ "nonce": "00", "shards": [{ "program": hex::encode(PROGRAM), "data": hex::encode(data) }] }).to_string()
}

pub(super) fn keys_json(npk: u8) -> String {
    json!({ "nullifier_public_key": hex::encode([npk; 32]), "viewing_public_key": hex::encode([npk; VPK_LEN]) }).to_string()
}

#[derive(Default)]
pub(super) struct Chain {
    pub(super) private: BTreeMap<String, (String, String)>,
    pub(super) public: BTreeMap<String, String>,
    /// Accounts in other users' wallets (their participants, notes for them).
    pub(super) elsewhere: BTreeMap<String, String>,
    labels: BTreeMap<String, String>,
    sent: Vec<(Vec<AccountEntry>, Vec<u8>)>,
    reply: Option<String>,
    pub(super) settled: bool,
    pub(super) height: i64,
    created: u8,
}

#[derive(Clone, Default)]
pub(super) struct FakeLez(pub(super) Rc<RefCell<Chain>>);

impl Lez for FakeLez {
    fn create_account_private(&self) -> PortResult<String> {
        let mut chain = self.0.borrow_mut();
        chain.created += 1;
        let n = 0xa0 + chain.created;
        let id = hex::encode([n; 32]);
        chain.private.insert(id.clone(), (EMPTY_ACCOUNT.into(), keys_json(n)));
        Ok(id)
    }
    fn list_accounts(&self) -> PortResult<Vec<(String, bool)>> {
        let chain = self.0.borrow();
        Ok(chain.private.keys().map(|id| (id.clone(), false)).chain(chain.public.keys().map(|id| (id.clone(), true))).collect())
    }
    fn get_account_public(&self, id: &str) -> PortResult<String> {
        Ok(self.0.borrow().public.get(id).cloned().unwrap_or_default())
    }
    fn get_account_private(&self, id: &str) -> PortResult<String> {
        Ok(self.0.borrow().private.get(id).map(|(account, _)| account.clone()).unwrap_or_default())
    }
    fn get_private_account_keys(&self, id: &str) -> PortResult<String> {
        self.0.borrow().private.get(id).map(|(_, keys)| keys.clone()).ok_or_else(|| "not found".into())
    }
    fn add_label(&self, label: &str, id: &str, _: bool) -> PortResult<()> {
        self.0.borrow_mut().labels.insert(label.into(), format!("Private/{id}"));
        Ok(())
    }
    fn resolve_label(&self, label: &str) -> PortResult<String> {
        Ok(self.0.borrow().labels.get(label).cloned().unwrap_or_default())
    }
    fn get_current_block_height(&self) -> PortResult<i64> {
        Ok(self.0.borrow().height)
    }
    fn sync_to_block(&self, _: i64) -> PortResult<()> {
        Ok(())
    }
    fn send_private_transaction(&self, accounts: &[AccountEntry], instruction: &[u8], elf: &[u8], kind: &str) -> PortResult<String> {
        assert_eq!(elf, b"ELF");
        assert!(kind.contains(&hex::encode(PROGRAM)));
        let mut chain = self.0.borrow_mut();
        chain.sent.push((accounts.to_vec(), instruction.to_vec()));
        Ok(chain.reply.clone().unwrap_or_else(|| json!({ "success": true, "tx_hash": format!("tx{}", chain.sent.len()) }).to_string()))
    }
    fn poll_transaction_status(&self, _: &str) -> PortResult<bool> {
        Ok(self.0.borrow().settled)
    }
}

#[derive(Default)]
pub(super) struct Disk {
    pub(super) json: Option<String>,
    /// What `set_aside` moved away, oldest first.
    pub(super) aside: Vec<String>,
    /// When set, `load` fails with it, like a file without read permission.
    pub(super) unreadable: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct FakeStore(pub(super) Rc<RefCell<Disk>>);
impl Store for FakeStore {
    fn load(&self) -> PortResult<Option<String>> {
        let disk = self.0.borrow();
        disk.unreadable.clone().map_or_else(|| Ok(disk.json.clone()), Err)
    }
    fn save(&self, json: &str) -> PortResult<()> {
        self.0.borrow_mut().json = Some(json.to_owned());
        Ok(())
    }
    fn set_aside(&self) -> PortResult<String> {
        let mut disk = self.0.borrow_mut();
        let json = disk.json.take().ok_or("nothing to set aside")?;
        disk.aside.push(json);
        Ok(format!("aside-{}", disk.aside.len()))
    }
}

pub(super) struct FakeFiles(String);
impl Files for FakeFiles {
    fn read_secret(&self, path: &str) -> PortResult<Zeroizing<String>> {
        if path == "/node/keystore.yaml" { Ok(Zeroizing::new(self.0.clone())) } else { Err("no such file".into()) }
    }
}

pub(super) struct FakeClock;
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        1_000
    }
}

/// A caught-up indexer whose history is one page.
pub(super) struct FakeHistory(String);
impl History for FakeHistory {
    fn transactions_by_account(&self, _: &str, offset: u64, _: u64) -> PortResult<String> {
        Ok(if offset == 0 { self.0.clone() } else { "[]".into() })
    }
    fn status(&self) -> PortResult<String> {
        Ok(json!({ "state": "CaughtUp", "last_error": null, "indexed_block_id": 50 }).to_string())
    }
}

/// An indexer still replaying the chain: nothing indexed yet past block 12.
pub(super) struct SyncingHistory;
impl History for SyncingHistory {
    fn transactions_by_account(&self, _: &str, _: u64, _: u64) -> PortResult<String> {
        Ok("[]".into())
    }
    fn status(&self) -> PortResult<String> {
        Ok(json!({ "state": "Syncing", "last_error": null, "indexed_block_id": 12 }).to_string())
    }
}

pub(super) fn node_key() -> NodeKey {
    NodeKey::from_secret(&[1; 32])
}

pub(super) fn keystore_yaml() -> String {
    format!(
        "public_keys:\n  BlendSigning: {}\nsecret_keys:\n  BlendSigning: !Ed25519 {}\n  BlendZk: !Zk {}\n",
        codec::node_hex(&node_key().node()),
        hex::encode([1; 32]),
        hex::encode([9; 32]),
    )
}

pub(super) const REFERRER: NodeId = NodeId::new([8; 32]);

pub(super) struct World {
    pub(super) lez: FakeLez,
    pub(super) store: FakeStore,
}

impl World {
    fn new() -> Self {
        let world = Self::empty();
        world.set_registry(1, &[REFERRER], &[REFERRER]);
        world
    }

    /// A wallet with an unregistered participant, and a chain with nothing published.
    pub(super) fn empty() -> Self {
        let world = Self { lez: FakeLez::default(), store: FakeStore::default() };
        world.lez.0.borrow_mut().private.insert(PARTICIPANT.into(), (EMPTY_ACCOUNT.into(), keys_json(0x11)));
        world.deploy(&hex::encode(PROGRAM), IMAGE_ID);
        world
    }

    pub(super) fn service(&self) -> Service {
        self.service_with(None)
    }

    fn service_with(&self, history: Option<Box<dyn History>>) -> Service {
        let mut service = self.started(history);
        service.configure(&hex::encode(PROGRAM)).unwrap();
        service
    }

    /// A service as the module starts it, before anything is configured.
    pub(super) fn started(&self, history: Option<Box<dyn History>>) -> Service {
        Service::new(
            Box::new(self.lez.clone()),
            history,
            Box::new(self.store.clone()),
            Box::new(FakeFiles(keystore_yaml())),
            Box::new(FakeClock),
            Binary::known(b"ELF", IMAGE_ID),
        )
    }

    /// A program running `image_id`, deployed at `account`.
    pub(super) fn deploy(&self, account: &str, image_id: [u32; 8]) {
        self.lez.0.borrow_mut().public.insert(account.into(), crate::domain::program::header_json(image_id));
    }

    /// The registry at `epoch`, with `active` published for it (and archived).
    fn set_registry(&self, epoch: u32, nodes: &[NodeId], active: &[NodeId]) {
        let active: BTreeSet<NodeId> = active.iter().copied().collect();
        let registry = State::Registry(Registry {
            registered_nodes: nodes.iter().copied().collect(),
            epoch,
            active_nodes: active.clone(),
        });
        self.lez
            .0
            .borrow_mut()
            .public
            .insert(codec::account_hex(&ORACLE_ACCOUNT_ID), account_json(&registry.to_data().into_inner()));
        if epoch > 0 {
            self.archive(epoch, &active);
        }
    }

    /// The program's archived publication of `epoch`.
    fn archive(&self, epoch: u32, active: &BTreeSet<NodeId>) {
        let archived = State::Publication { epoch, active_digest: active_digest(active) };
        self.lez
            .0
            .borrow_mut()
            .public
            .insert(codec::account_hex(&epoch_account(program(), epoch)), account_json(&archived.to_data().into_inner()));
    }

    fn set_participant(&self, participant: Participant) {
        let data = State::Participant(participant).to_data().into_inner();
        self.lez.0.borrow_mut().private.get_mut(PARTICIPANT).unwrap().0 = account_json(&data);
    }

    fn add_note(&self, id: u8, note: State) {
        self.lez
            .0
            .borrow_mut()
            .private
            .insert(hex::encode([id; 32]), (account_json(&note.to_data().into_inner()), keys_json(0x11)));
    }

    pub(super) fn sent(&self) -> Vec<(Vec<AccountEntry>, Vec<u8>)> {
        self.lez.0.borrow().sent.clone()
    }
}

pub(super) fn invitation_blob() -> String {
    Invitation::from_keys(REFERRER, &keys_json(0x22)).unwrap().encode().unwrap()
}

pub(super) fn reference(byte: u8) -> String {
    hex::encode([byte; 32])
}

pub(super) fn accept_terms(service: &mut Service) {
    service.accept_terms(crate::domain::terms::version(), &crate::domain::terms::sha256()).unwrap();
}

pub(super) fn referrer() -> Referrer {
    Invitation::from_keys(REFERRER, &keys_json(0x22)).unwrap().referrer_keys().unwrap()
}

pub(super) fn enrolled(world: &World) -> NodeId {
    let me = node_key().node();
    world.set_participant(Participant::new(me, Some(referrer())));
    me
}

pub(super) fn referrer_entry() -> AccountEntry {
    AccountEntry::PrivateForeign { npk: hex::encode([0x22; 32]), vpk: hex::encode([0x22; VPK_LEN]) }
}

/// A page of indexer history holding the oracle's `Publish` of `epoch`.
pub(super) fn publish_json(epoch: u32, active: BTreeSet<NodeId>) -> String {
    let publish = indexer_service_protocol::Transaction::Public(indexer_service_protocol::PublicTransaction {
        hash: indexer_service_protocol::HashType([0; 32]),
        message: indexer_service_protocol::PublicMessage {
            program_account_id: indexer_service_protocol::AccountId { value: PROGRAM },
            shard_selectors: vec![],
            nonces: vec![],
            instruction_data: borsh::to_vec(&Instruction::Publish { epoch, active }).unwrap(),
            fee: None,
        },
        witness_set: indexer_service_protocol::WitnessSet { signatures_and_public_keys: vec![], proof: None },
    });
    serde_json::to_string(&vec![publish]).unwrap()
}

/// An indexer that never answers, like one that isn't running.
pub(super) struct DeadIndexer(Rc<RefCell<usize>>);
impl History for DeadIndexer {
    fn transactions_by_account(&self, _: &str, _: u64, _: u64) -> PortResult<String> {
        *self.0.borrow_mut() += 1;
        Err("object_unavailable".into())
    }
    fn status(&self) -> PortResult<String> {
        Err("object_unavailable".into())
    }
}

mod activity;
mod cashout;
mod claims;
mod deployment;
mod enrol;
mod participants;
mod saved_state;
mod terms;
