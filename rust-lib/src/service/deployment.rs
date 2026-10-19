//! Which deployment of the program to use, and whether that account runs this module's program.

use lee_core::account::AccountId;
use serde_json::{Value, json};

use super::{Service, StateProblem, port, PROGRAM_ACCOUNT_ENV};
use crate::domain::program;
use crate::domain::{DomainError, Result, codec, terms};

impl Service {
    /// Selects the deployment by its program account (hex or base58); `""` clears it, back to
    /// `UPLINK_PROGRAM`. Switching back to a deployment finds its state as it was.
    pub fn configure(&mut self, program_account: &str) -> Result<Value> {
        self.saved.config.program_account = if program_account.trim().is_empty() {
            None
        } else {
            Some(codec::account_hex(&codec::account_any(program_account)?))
        };
        self.persist()?;
        Ok(self.info())
    }

    /// Whether the selected program account runs the program this module was built with:
    /// `program_not_deployed` when nothing is deployed there (often another network), and
    /// `program_mismatch` when another program or version is.
    pub fn check_program(&mut self) -> Result<Value> {
        let program = self.verified_program()?;
        Ok(json!({ "programAccount": codec::account_hex(&program), "imageId": program::image_id_hex(&self.binary.image_id()?) }))
    }

    pub(super) fn verified_program(&mut self) -> Result<AccountId> {
        let program = self.program()?;
        let hex_id = codec::account_hex(&program);
        if self.checked.as_deref() == Some(hex_id.as_str()) {
            return Ok(program);
        }
        let expected = self.binary.image_id()?;
        let account = self.lez.get_account_public(&hex_id).map_err(port)?;
        match program::deployed_image_id(&account)? {
            None => Err(DomainError::new(
                "program_not_deployed",
                format!("no program is deployed at {program} on the network the wallet is connected to"),
            )),
            Some(deployed) if deployed != expected => Err(DomainError::new(
                "program_mismatch",
                format!(
                    "the program at {program} is {}, not the referral program this module runs ({})",
                    program::image_id_hex(&deployed),
                    program::image_id_hex(&expected)
                ),
            )),
            Some(_) => {
                self.checked = Some(hex_id);
                Ok(program)
            }
        }
    }

    pub fn info(&self) -> Value {
        json!({
            "programAccount": self.program_hex(),
            "deployments": self.saved.deployments.keys().collect::<Vec<_>>(),
            "termsVersion": terms::version(),
            "history": self.history.is_some(),
            "stateProblem": self.state_problem_json(),
        })
    }

    fn state_problem_json(&self) -> Value {
        match &self.state_problem {
            None => Value::Null,
            Some(StateProblem::SetAside { aside }) => json!({ "code": "state_set_aside", "file": aside }),
            Some(StateProblem::Unreadable { error }) => json!({ "code": "state_unreadable", "error": error }),
        }
    }

    pub(super) fn program_hex(&self) -> Option<String> {
        self.saved
            .config
            .program_account
            .clone()
            .or_else(|| {
                let env = std::env::var(PROGRAM_ACCOUNT_ENV).ok()?;
                // Unparseable stays as given, so program() reports it.
                Some(codec::account_any(&env).map_or(env, |id| codec::account_hex(&id)))
            })
    }

    pub(super) fn program(&self) -> Result<AccountId> {
        let hex_id = self
            .program_hex()
            .ok_or_else(|| DomainError::new("not_configured", format!("set the referral program account ({PROGRAM_ACCOUNT_ENV} or configure)")))?;
        codec::account(&hex_id)
    }
}
