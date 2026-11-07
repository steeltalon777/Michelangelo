//! The optional workspace edit policy.
//!
//! A workspace may carry the file `<workspace>/.michelangelo/edit_policy.json`.
//! The file is optional: when it is absent, only an explicit author
//! acceptance may apply a candidate (the manual path of ADR 0004 §13.3).
//!
//! ```json
//! {
//!   "schema": "michelangelo.edit-policy/0",
//!   "auto_accept": true,
//!   "allowed_origins": ["human", "ai_author", "test_backend"]
//! }
//! ```
//!
//! * `auto_accept` — whether a candidate that does not require an author
//!   acceptance may be applied by `submit` alone,
//! * `allowed_origins` — the origins the policy accepts (all of them when
//!   the field is absent).
//!
//! A policy file that exists but cannot be used is an error: the boundary
//! never guesses the intent of a broken policy.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate::{CandidateOrigin, EditCandidate};
use crate::error::EditError;

/// The relative path of the policy file inside a workspace.
pub const POLICY_FILE_RELATIVE: &str = ".michelangelo/edit_policy.json";
/// The schema of the policy file.
pub const POLICY_SCHEMA: &str = "michelangelo.edit-policy/0";

/// The edit policy of a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditPolicy {
    /// The schema identifier.
    #[serde(default = "policy_schema")]
    pub schema: String,
    /// Whether a candidate may be applied without an explicit author
    /// acceptance.
    #[serde(default)]
    pub auto_accept: bool,
    /// The origins the policy accepts (all of them when absent).
    #[serde(default)]
    pub allowed_origins: Option<Vec<CandidateOrigin>>,
}

/// The default schema of a policy file.
fn policy_schema() -> String {
    POLICY_SCHEMA.to_string()
}

impl Default for EditPolicy {
    fn default() -> Self {
        EditPolicy {
            schema: POLICY_SCHEMA.to_string(),
            auto_accept: false,
            allowed_origins: None,
        }
    }
}

/// The outcome of the policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The policy allows an automatic acceptance.
    Allow,
    /// The workspace carries no policy: an explicit author acceptance is
    /// required.
    NoPolicy,
    /// The acceptance is deferred to an explicit author acceptance.
    Deferred {
        /// The human readable reason of the deferral.
        reason: String,
    },
    /// The policy denies an automatic acceptance.
    Denied {
        /// The human readable reason of the denial.
        reason: String,
    },
}

/// The path of the policy file that applies to a run (None when there is none).
pub fn find_policy_file(run_dir: &Path) -> Option<PathBuf> {
    let mut current = Some(run_dir);
    while let Some(directory) = current {
        let candidate = directory.join(POLICY_FILE_RELATIVE);
        if candidate.is_file() {
            return Some(candidate);
        }
        current = directory.parent();
    }
    None
}

/// Load the policy that applies to a run.
///
/// None is returned when the workspace carries no policy file; a policy file
/// that cannot be used is an error.
pub fn load_policy(run_dir: &Path) -> Result<Option<EditPolicy>, EditError> {
    let path = match find_policy_file(run_dir) {
        Some(path) => path,
        None => return Ok(None),
    };
    let text = std::fs::read_to_string(&path).map_err(|err| EditError::Policy {
        message: format!("the policy file '{}' cannot be read: {err}", path.display()),
    })?;
    let policy: EditPolicy = serde_json::from_str(&text).map_err(|err| EditError::Policy {
        message: format!("the policy file '{}' is not usable: {err}", path.display()),
    })?;
    if policy.schema != POLICY_SCHEMA {
        return Err(EditError::Policy {
            message: format!(
                "the policy file '{}' declares the schema '{}' which is not supported",
                path.display(),
                policy.schema
            ),
        });
    }
    Ok(Some(policy))
}

/// Evaluate the policy for a candidate.
pub fn evaluate(policy: Option<&EditPolicy>, candidate: &EditCandidate) -> PolicyDecision {
    let policy = match policy {
        Some(policy) => policy,
        None => return PolicyDecision::NoPolicy,
    };
    if !policy.auto_accept {
        return PolicyDecision::Denied {
            reason: "the edit policy does not allow an automatic acceptance".to_string(),
        };
    }
    if let Some(origins) = &policy.allowed_origins {
        if !origins.contains(&candidate.provenance.origin) {
            return PolicyDecision::Denied {
                reason: format!(
                    "the edit policy does not accept the origin '{}'",
                    candidate.provenance.origin.as_str()
                ),
            };
        }
    }
    PolicyDecision::Allow
}
