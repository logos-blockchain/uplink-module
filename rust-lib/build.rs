//! Finds the referral program binary, uplink's committed `artifacts/programs/referral/referral.bin`
//! at the uplink commit `referral_core` is pinned to, and hands its path to the crate as
//! `UPLINK_REFERRAL_BIN`. The nix build passes the path in (flake.nix fetches uplink at that
//! commit); a plain cargo build finds the file in cargo's own checkout of uplink, which it has
//! for `referral_core`. So moving the pin in Cargo.toml moves the binary with it.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const VAR: &str = "UPLINK_REFERRAL_BIN";
const ARTIFACT: &str = "artifacts/programs/referral/referral.bin";

fn main() {
    println!("cargo:rerun-if-env-changed={VAR}");
    println!("cargo:rerun-if-changed=Cargo.lock");
    let path = env::var(VAR).map(PathBuf::from).unwrap_or_else(|_| from_cargo_checkout());
    assert!(path.is_file(), "{VAR}: {} is not a file", path.display());
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rustc-env={VAR}={}", path.display());
}

/// `referral_core`'s commit, from Cargo.lock's `source = "git+…uplink?rev=…#<commit>"`.
fn pinned_commit() -> String {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let lock = fs::read_to_string(manifest_dir.join("Cargo.lock")).expect("rust-lib/Cargo.lock");
    lock.split("[[package]]")
        .find(|package| package.contains("name = \"referral_core\""))
        .and_then(|package| package.lines().find(|line| line.starts_with("source = ")))
        .and_then(|source| source.trim_end_matches('"').split('#').nth(1))
        .map(str::to_owned)
        .expect("Cargo.lock pins referral_core to an uplink commit")
}

fn from_cargo_checkout() -> PathBuf {
    let commit = pinned_commit();
    let cargo_home = env::var("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(&env::var("HOME").expect("HOME")).join(".cargo"));
    let checkouts = cargo_home.join("git/checkouts");
    fs::read_dir(&checkouts)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|repo| repo.file_name().to_string_lossy().starts_with("uplink-"))
        .map(|repo| repo.path().join(&commit[..7]).join(ARTIFACT))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            panic!("no {ARTIFACT} in cargo's checkout of uplink {commit} under {}; set {VAR} to it", checkouts.display())
        })
}
