//! `uplink_module`: the Uplink node referral program (logos-blockchain/uplink) as a Logos
//! core module, over `lez_core`'s generic API.
//!
//! - `domain`: the program's formats and the referral rules. Pure, `cargo test`.
//! - `ports`: what the service needs from outside, as traits.
//! - `service`: the use cases (enrol, claim, cash out) and the saved state.
//! - `adapters` + `glue` (feature `logos_module`): the real ports and the module API.

pub mod domain;
pub mod ports;
pub mod service;

#[cfg(feature = "logos_module")]
mod adapters;
#[cfg(feature = "logos_module")]
mod glue;
