//! The module's API. Every method returns JSON: `{"ok": true, …}` or
//! `{"ok": false, "code": "<reason>", "error": "<text>"}`. Ids are hex; `reference` is a
//! caller-chosen 32-byte hex id that makes resubmitting idempotent.

use serde_json::{Value, json};

use crate::adapters::{FileStore, Indexer, LezCore, SystemClock};
use crate::domain::DomainError;
use crate::domain::program::Binary;
use crate::service::Service;

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
        self.service = Some(Service::new(
            Box::new(LezCore),
            Some(Box::new(Indexer)),
            Box::new(FileStore::new(&ctx.instance_persistence_path)),
            Box::new(SystemClock),
            Binary::embedded(),
        ));
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
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<UplinkModuleImpl>();
}
