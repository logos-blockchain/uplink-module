//! The program's terms: shown, accepted, and required before joining.

use serde_json::{Value, json};

use super::Service;
use super::saved::TermsAcceptance;
use crate::domain::{DomainError, Result, terms};

impl Service {
    pub fn terms(&self) -> Value {
        let mut terms: Value = serde_json::from_str(terms::TERMS_JSON).expect("assets/terms.json parses");
        terms["sha256"] = json!(terms::sha256());
        terms
    }

    pub fn accept_terms(&mut self, version: u32, sha256: &str) -> Result<Value> {
        terms::check(version, sha256)?;
        let accepted_at_ms = self.clock.now_ms();
        self.deployment_mut()?.terms = Some(TermsAcceptance {
            version,
            sha256: sha256.to_lowercase(),
            accepted_at_ms,
            node: None,
        });
        self.persist()?;
        Ok(self.terms_status())
    }

    pub fn terms_status(&self) -> Value {
        let accepted = self.deployment().terms.as_ref();
        json!({
            "accepted": accepted.is_some_and(|t| terms::check(t.version, &t.sha256).is_ok()),
            "acceptedVersion": accepted.map(|t| t.version),
            "currentVersion": terms::version(),
        })
    }

    pub(super) fn require_terms(&self) -> Result<TermsAcceptance> {
        self.deployment()
            .terms
            .clone()
            .filter(|t| terms::check(t.version, &t.sha256).is_ok())
            .ok_or_else(|| DomainError::new("terms_not_accepted", "accept the programme's terms first"))
    }
}
