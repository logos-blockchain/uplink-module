//! The programme's terms, owned by the module so every way in (Uplink, `logoscore`) shows
//! the same text. Acceptance names the version and the SHA-256 of this exact file.

use sha2::{Digest, Sha256};

use super::{DomainError, Result};

pub const TERMS_JSON: &str = include_str!("../../assets/terms.json");

pub fn sha256() -> String {
    hex::encode(Sha256::digest(TERMS_JSON.as_bytes()))
}

pub fn version() -> u32 {
    serde_json::from_str::<serde_json::Value>(TERMS_JSON)
        .ok()
        .and_then(|terms| terms.get("version")?.as_u64())
        .and_then(|version| u32::try_from(version).ok())
        .expect("assets/terms.json has a version")
}

/// Accepting is only valid for the exact current text.
pub fn check(version: u32, sha256: &str) -> Result<()> {
    if version != self::version() || !sha256.eq_ignore_ascii_case(&self::sha256()) {
        return Err(DomainError::new("terms_changed", "these are not the current terms"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_text_can_be_accepted() {
        assert!(check(version(), &sha256()).is_ok());
        assert_eq!(check(version(), "00").unwrap_err().code, "terms_changed");
        assert_eq!(check(version() + 1, &sha256()).unwrap_err().code, "terms_changed");
    }
}
