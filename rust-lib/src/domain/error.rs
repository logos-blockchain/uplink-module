#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct DomainError {
    /// Stable, machine-readable reason, e.g. `invalid_hex`, `not_registered`.
    pub code: &'static str,
    pub message: String,
}

impl DomainError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

pub type Result<T> = std::result::Result<T, DomainError>;
