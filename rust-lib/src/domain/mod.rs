//! Pure referral logic: the program's formats, the enrolment message, keys, claims and
//! cash-out. No I/O; everything here is tested with plain `cargo test`.

pub mod cashout;
pub mod codec;
pub mod enrolment;
pub mod error;
pub mod instructions;
pub mod invitation;
pub mod keystore;
pub mod program;
pub mod state;
pub mod terms;

pub use error::{DomainError, Result};
pub use referral_core as core;
