//! The module's API. Every method returns JSON: `{"ok": true, …}` or
//! `{"ok": false, "code": "<reason>", "error": "<text>"}`. Ids are hex; `reference` is a
//! caller-chosen 32-byte hex id that makes resubmitting idempotent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::adapters::{FileStore, Indexer, LezCore, LocalFiles, SystemClock};
use crate::domain::DomainError;
use crate::domain::program::Binary;
use crate::service::{Service, recorder};

pub trait UplinkModule: 'static {
    /// Selected program account, terms version, whether history is on.
    fn info(&mut self) -> String;
    /// Selects the deployment by its program account ("" clears it, back to the env var).
    fn configure(&mut self, program_account: String) -> String;
    /// `{programAccount, imageId}` when the selected account runs this module's program;
    /// else `not_configured`, `program_not_deployed` or `program_mismatch`.
    fn check_program(&mut self) -> String;

    /// The program's terms: `{version, lastUpdated, sections, sha256}`.
    fn terms(&mut self) -> String;
    fn terms_status(&mut self) -> String;
    /// Accepts the current terms; `sha256` must be the one `terms()` returned.
    fn accept_terms(&mut self, version: i64, sha256: String) -> String;

    /// `{epoch, nodes, active}`.
    fn registry(&mut self) -> String;
    /// For each node (comma-separated hex), active in each of the last `count` published epochs:
    /// `{epochs: [..], nodes: {node: [1|0|-1, ..]}}`, oldest first; `-1` = unknown.
    fn activity(&mut self, nodes: String, count: i64) -> String;

    /// A private LEZ account for the program, labelled so it can be found again.
    fn create_participant(&mut self, label: String) -> String;
    /// The participant carrying `label`, and whether it is registered.
    fn find_participant(&mut self, label: String) -> String;
    /// `{registered, node, referrer, children, childrenSince, rewardBalance}` from the chain.
    fn participant(&mut self, participant: String) -> String;
    /// The invitation code to give a node you invite.
    fn invitation(&mut self, participant: String, node: String) -> String;
    /// Reads a pasted invitation without saving it: `{referrer, registered}`.
    fn check_invitation(&mut self, blob: String) -> String;
    /// Saves an invitation you received; returns the referrer's node.
    fn import_invitation(&mut self, participant: String, blob: String) -> String;

    /// Joins: reads `node`'s key from `keystore_path`, signs, submits Register. Pass the
    /// running node's provider id as `node` so a keystore for another node is refused
    /// (`keystore_mismatch`); `""` uses the keystore's BlendSigning key.
    fn enrol(&mut self, reference: String, participant: String, keystore_path: String, node: String) -> String;
    /// The bytes to sign, for signing elsewhere.
    fn enrolment_message(&mut self, participant: String, node: String) -> String;
    /// Joins with a signature made elsewhere.
    fn enrol_with_signature(&mut self, reference: String, participant: String, node: String, signature: String) -> String;
    /// The node a keystore is for, from its public keys only: `{node}`.
    fn keystore_node(&mut self, keystore_path: String) -> String;
    /// An operation's status (`submitting`, `pending`, `settled`, `failed`), settling it when its transaction is in.
    fn status(&mut self, reference: String) -> String;
    /// Every recorded operation, or one participant's ("" for all).
    fn operations(&mut self, participant: String) -> String;

    /// Syncs the wallet to the tip so new notes are found.
    fn sync(&mut self) -> String;
    /// New referrals and credits addressed to the participant.
    fn notes(&mut self, participant: String) -> String;
    /// Points claimable now, by batch.
    fn claimable(&mut self, participant: String) -> String;
    /// Submits the oldest claimable batch (up to 16 epochs).
    fn claim(&mut self, reference: String, participant: String) -> String;
    /// Claims one batch when something is claimable and nothing is pending; call it on every
    /// poll. Points are never lost by waiting: each epoch pays once, whenever it's claimed.
    fn auto_claim(&mut self, participant: String) -> String;

    /// Burns the whole reward balance into a receipt; the key derives the receipt's secret.
    fn cash_out(&mut self, reference: String, participant: String, keystore_path: String) -> String;
    /// Receipts; with a keystore path they're found on-chain and remembered, with `""` the
    /// remembered list (no key read).
    fn receipts(&mut self, participant: String, keystore_path: String) -> String;
    /// The signed code for the payout form.
    fn payout_code(&mut self, participant: String, index: i64, keystore_path: String) -> String;
    fn on_context_ready(&mut self, _ctx: &RustModuleContext) {}
}

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

#[derive(Default)]
struct UplinkModuleImpl {
    service: Option<Service>,
}

fn reply(result: Result<Value, DomainError>) -> String {
    match result {
        Ok(Value::Object(mut fields)) => {
            fields.insert("ok".into(), json!(true));
            Value::Object(fields).to_string()
        }
        Ok(value) => json!({ "ok": true, "value": value }).to_string(),
        Err(e) => json!({ "ok": false, "code": e.code, "error": e.message }).to_string(),
    }
}

impl UplinkModuleImpl {
    fn with<T: Into<Result<Value, DomainError>>>(&mut self, f: impl FnOnce(&mut Service) -> T) -> String {
        match self.service.as_mut() {
            Some(service) => reply(f(service).into()),
            None => reply(Err(DomainError::new("not_ready", "the module has no data directory yet"))),
        }
    }
}

impl UplinkModule for UplinkModuleImpl {
    fn on_context_ready(&mut self, ctx: &RustModuleContext) {
        let dir = &ctx.instance_persistence_path;
        self.service = Some(Service::new(
            Box::new(LezCore),
            Some(Box::new(Indexer)),
            Box::new(FileStore::new(dir)),
            Some(Box::new(FileStore::named(dir, RECORDER_LOG))),
            Box::new(LocalFiles),
            Box::new(SystemClock),
            Binary::embedded(),
        ));
        start_recorder(dir);
    }

    fn info(&mut self) -> String {
        self.with(|s| Ok(s.info()))
    }
    fn configure(&mut self, program_account: String) -> String {
        self.with(|s| s.configure(&program_account))
    }
    fn check_program(&mut self) -> String {
        self.with(|s| s.check_program())
    }
    fn terms(&mut self) -> String {
        self.with(|s| Ok(s.terms()))
    }
    fn terms_status(&mut self) -> String {
        self.with(|s| Ok(s.terms_status()))
    }
    fn accept_terms(&mut self, version: i64, sha256: String) -> String {
        self.with(|s| match u32::try_from(version) {
            Ok(version) => s.accept_terms(version, &sha256),
            Err(_) => Err(DomainError::new("terms_changed", "these are not the current terms")),
        })
    }
    fn registry(&mut self) -> String {
        self.with(Service::registry)
    }
    fn activity(&mut self, nodes: String, count: i64) -> String {
        let nodes: Vec<String> = nodes.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).collect();
        let count = u32::try_from(count.clamp(1, 64)).unwrap_or(16);
        self.with(|s| s.activity(&nodes, count))
    }
    fn create_participant(&mut self, label: String) -> String {
        self.with(|s| s.create_participant(&label))
    }
    fn find_participant(&mut self, label: String) -> String {
        self.with(|s| s.find_participant(&label))
    }
    fn participant(&mut self, participant: String) -> String {
        self.with(|s| s.participant(&participant))
    }
    fn invitation(&mut self, participant: String, node: String) -> String {
        self.with(|s| s.invitation(&participant, &node))
    }
    fn check_invitation(&mut self, blob: String) -> String {
        self.with(|s| s.check_invitation(&blob))
    }
    fn import_invitation(&mut self, participant: String, blob: String) -> String {
        self.with(|s| s.import_invitation(&participant, &blob))
    }
    fn enrol(&mut self, reference: String, participant: String, keystore_path: String, node: String) -> String {
        self.with(|s| s.enrol(&reference, &participant, &keystore_path, &node))
    }
    fn enrolment_message(&mut self, participant: String, node: String) -> String {
        self.with(|s| s.enrolment_message(&participant, &node))
    }
    fn enrol_with_signature(&mut self, reference: String, participant: String, node: String, signature: String) -> String {
        self.with(|s| s.enrol_with_signature(&reference, &participant, &node, &signature))
    }
    fn keystore_node(&mut self, keystore_path: String) -> String {
        self.with(|s| s.keystore_node(&keystore_path))
    }
    fn status(&mut self, reference: String) -> String {
        self.with(|s| s.status(&reference))
    }
    fn operations(&mut self, participant: String) -> String {
        self.with(|s| s.operations(&participant))
    }
    fn sync(&mut self) -> String {
        self.with(|s| s.sync())
    }
    fn notes(&mut self, participant: String) -> String {
        self.with(|s| s.notes(&participant))
    }
    fn claimable(&mut self, participant: String) -> String {
        self.with(|s| s.claimable(&participant))
    }
    fn claim(&mut self, reference: String, participant: String) -> String {
        self.with(|s| s.claim(&reference, &participant))
    }
    fn auto_claim(&mut self, participant: String) -> String {
        self.with(|s| s.auto_claim(&participant))
    }
    fn cash_out(&mut self, reference: String, participant: String, keystore_path: String) -> String {
        self.with(|s| s.cash_out(&reference, &participant, &keystore_path))
    }
    fn receipts(&mut self, participant: String, keystore_path: String) -> String {
        self.with(|s| s.receipts(&participant, &keystore_path))
    }
    fn payout_code(&mut self, participant: String, index: i64, keystore_path: String) -> String {
        self.with(|s| match u64::try_from(index) {
            Ok(index) => s.payout_code(&participant, index, &keystore_path),
            Err(_) => Err(DomainError::new("unknown_receipt", "no receipt with this index")),
        })
    }
}

/// The recorder's own file, next to the service's state (see `service::recorder`).
const RECORDER_LOG: &str = "uplink_epochs.json";

static RECORDER_RUNNING: AtomicBool = AtomicBool::new(false);
static RECORDER_STOP: AtomicBool = AtomicBool::new(false);

/// Starts the recorder thread, once: it looks at the registry every `recorder::INTERVAL`
/// for as long as the module is loaded, whether or not any app is calling it.
fn start_recorder(dir: &str) {
    if RECORDER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (state, log) = (FileStore::new(dir), FileStore::named(dir, RECORDER_LOG));
    let spawned = std::thread::Builder::new().name("uplink-recorder".into()).spawn(move || {
        while !RECORDER_STOP.load(Ordering::SeqCst) {
            // A failed look (no wallet open yet, lez_core busy proving, nothing configured)
            // is just tried again on the next one.
            let _ = recorder::record_once(&LezCore, &state, &log);
            let mut slept = Duration::ZERO;
            while slept < recorder::INTERVAL && !RECORDER_STOP.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(250));
                slept += Duration::from_millis(250);
            }
        }
        RECORDER_RUNNING.store(false, Ordering::SeqCst);
        logos_rust_sdk::unload_finished();
    });
    if spawned.is_err() {
        RECORDER_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Stops the recorder before the host unloads the module: the host waits (`Asynchronous`)
/// until the thread is out of the library's code, at most one look (one lez_core call).
impl logos_rust_sdk::AboutToUnload for UplinkModuleImpl {
    fn about_to_unload(&self) -> logos_rust_sdk::Shutdown {
        RECORDER_STOP.store(true, Ordering::SeqCst);
        if RECORDER_RUNNING.load(Ordering::SeqCst) {
            logos_rust_sdk::Shutdown::Asynchronous
        } else {
            logos_rust_sdk::Shutdown::Synchronous
        }
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    logos_install!(UplinkModuleImpl);
}
