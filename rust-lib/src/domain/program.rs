//! The referral program binary the module proves with, and the check that a deployment runs it.
//!
//! `assets/referral.bin` is uplink's reproducibly built `artifacts/programs/referral/referral.bin`,
//! copied from the commit `referral_core` is pinned to. A program account holds a
//! [`ProgramHeader`] in the loader's shard, whose `image_id` the loader computed from the
//! deployed bytecode, so comparing it with ours shows whether the deployment is this program.

use std::sync::OnceLock;

use lee_core::program::{PROGRAM_LOADER_ACCOUNT_ID, ProgramHeader, ProgramId};

use super::{DomainError, Result, codec};

pub const EMBEDDED: &[u8] = include_bytes!("../../assets/referral.bin");

/// The binary and its image id, computed once on first use.
pub struct Binary {
    pub elf: &'static [u8],
    image_id: OnceLock<ProgramId>,
}

impl Binary {
    pub fn embedded() -> Self {
        Self { elf: EMBEDDED, image_id: OnceLock::new() }
    }

    /// A binary whose image id is already known (tests, where the bytes aren't a real program).
    pub fn known(elf: &'static [u8], image_id: ProgramId) -> Self {
        Self { elf, image_id: OnceLock::from(image_id) }
    }

    pub fn image_id(&self) -> Result<ProgramId> {
        if let Some(id) = self.image_id.get() {
            return Ok(*id);
        }
        let id: ProgramId = risc0_binfmt::compute_image_id(self.elf)
            .map_err(|e| DomainError::new("program_binary", format!("the embedded referral.bin does not decode: {e}")))?
            .into();
        Ok(*self.image_id.get_or_init(|| id))
    }
}

/// The image id deployed at a program account, from `lez_core.get_account_public`'s JSON;
/// `None` when nothing is deployed there.
pub fn deployed_image_id(account_json: &str) -> Result<Option<ProgramId>> {
    if account_json.trim().is_empty() {
        return Ok(None);
    }
    let shard = codec::shard(account_json, &PROGRAM_LOADER_ACCOUNT_ID)?;
    if shard.is_empty() {
        return Ok(None);
    }
    ProgramHeader::from_bytes(&shard)
        .map(|header| Some(header.image_id))
        .ok_or_else(|| DomainError::new("bad_account", "the program account's header does not decode"))
}

pub fn image_id_hex(id: &ProgramId) -> String {
    hex::encode(id.iter().flat_map(|word| word.to_le_bytes()).collect::<Vec<_>>())
}

/// A program account's JSON holding a header for `image_id`.
#[cfg(test)]
pub(crate) fn header_json(image_id: ProgramId) -> String {
    let header = ProgramHeader {
        image_id,
        program_first_segment: lee_core::account::AccountId::new([3; 32]),
        immutable: true,
    };
    serde_json::json!({
        "nonce": "00",
        "shards": [{ "program": codec::account_hex(&PROGRAM_LOADER_ACCOUNT_ID), "data": hex::encode(header.to_bytes()) }]
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    /// Moving the pin means copying the new binary in: this catches a pin moved without it.
    #[test]
    fn the_embedded_binary_is_uplinks_at_the_pinned_commit() {
        assert_eq!(
            hex::encode(Sha256::digest(EMBEDDED)),
            "39ab20f193373a8185a364f8db678fc6626b2ccd21e93bc4eb1c048072db2726",
            "assets/referral.bin is not uplink 8021f33's artifacts/programs/referral/referral.bin"
        );
        Binary::embedded().image_id().expect("the embedded binary has an image id");
    }

    #[test]
    fn reads_the_image_id_a_program_account_holds() {
        assert_eq!(deployed_image_id(&header_json([7; 8])).unwrap(), Some([7; 8]));
        assert_eq!(deployed_image_id("").unwrap(), None);
        assert_eq!(deployed_image_id(r#"{"nonce":"00","shards":[]}"#).unwrap(), None);
    }
}
