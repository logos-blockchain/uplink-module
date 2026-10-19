//! `uplink_module`: the Uplink node referral program (logos-blockchain/uplink) as a Logos
//! core module, over `lez_core`'s generic API.
//!
//! - `domain`: the program's formats and the referral rules. Pure, `cargo test`.
//! - `glue` (feature `logos_module`): the module API.

pub mod domain;

#[cfg(feature = "logos_module")]
mod glue;
