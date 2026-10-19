//! The module's API. Every method returns JSON: `{"ok": true, …}` or
//! `{"ok": false, "code": "<reason>", "error": "<text>"}`.

use serde_json::json;

pub trait UplinkModule: 'static {
    /// The module's name and version.
    fn info(&mut self) -> String;

    fn on_context_ready(&mut self, _ctx: &RustModuleContext) {}
}

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

#[derive(Default)]
struct UplinkModuleImpl;

impl UplinkModule for UplinkModuleImpl {
    fn info(&mut self) -> String {
        json!({ "ok": true, "name": "uplink_module", "version": env!("CARGO_PKG_VERSION") }).to_string()
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<UplinkModuleImpl>();
}
