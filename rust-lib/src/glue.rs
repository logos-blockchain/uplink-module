//! The module's API. Every method returns JSON: `{"ok": true, …}` or
//! `{"ok": false, "code": "<reason>", "error": "<text>"}`. Ids are hex; `reference` is a
//! caller-chosen 32-byte hex id that makes resubmitting idempotent.

use serde_json::{Value, json};

use crate::adapters::{FileStore, LezCore};
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
            Box::new(FileStore::new(&ctx.instance_persistence_path)),
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
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<UplinkModuleImpl>();
}
