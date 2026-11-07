//! The optional workspace policy of the assistant surface.
//!
//! A workspace may carry the file
//! `<workspace>/.michelangelo/assistant_policy.json`:
//!
//! ```json
//! {
//!   "allow_apply": false,
//!   "allow_submit": true,
//!   "allowed_tools": ["get_task", "validate_package", "submit_candidate", "get_report"],
//!   "max_calls_per_run": 3,
//!   "max_candidate_bytes": 67108864,
//!   "allow_external_service": false
//! }
//! ```
//!
//! The file is optional: an absent file means the defaults above. The file is
//! strict — an unknown field, an unknown tool or an invalid value is a
//! startup error, never a silent fallback.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssistantError;

/// The relative path of the policy file inside a workspace.
pub const POLICY_FILE_RELATIVE: &str = ".michelangelo/assistant_policy.json";
/// The default number of assistant calls of one run.
pub const DEFAULT_MAX_CALLS_PER_RUN: u64 = 3;
/// The default size limit of one candidate (64 MiB).
pub const DEFAULT_MAX_CANDIDATE_BYTES: u64 = 67_108_864;
/// The smallest usable size limit of a candidate (1 KiB).
pub const MIN_CANDIDATE_BYTES: u64 = 1_024;
/// The largest usable size limit of a candidate (1 GiB).
pub const MAX_CANDIDATE_BYTES: u64 = 1_073_741_824;

/// The tool that returns the bounded task.
pub const TOOL_GET_TASK: &str = "get_task";
/// The tool that validates an input package.
pub const TOOL_VALIDATE_PACKAGE: &str = "validate_package";
/// The tool that submits a candidate.
pub const TOOL_SUBMIT_CANDIDATE: &str = "submit_candidate";
/// The tool that returns the reconstruction report.
pub const TOOL_GET_REPORT: &str = "get_report";

/// The four tools of the assistant surface.
pub const ALL_TOOLS: [&str; 4] = [
    TOOL_GET_TASK,
    TOOL_VALIDATE_PACKAGE,
    TOOL_SUBMIT_CANDIDATE,
    TOOL_GET_REPORT,
];

/// The policy of a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPolicy {
    /// The schema identifier.
    #[serde(default = "default_schema")]
    pub schema: String,
    /// Whether a candidate may be applied without an author acceptance.
    ///
    /// The value is always `false` in this stage: the assistant never
    /// applies a candidate.
    #[serde(default)]
    pub allow_apply: bool,
    /// Whether a candidate may be submitted at all.
    #[serde(default = "default_true")]
    pub allow_submit: bool,
    /// The tools the surface exposes.
    #[serde(default = "default_tools")]
    pub allowed_tools: Vec<String>,
    /// The maximum number of assistant calls of one run.
    #[serde(default = "default_max_calls")]
    pub max_calls_per_run: u64,
    /// The maximum number of bytes one run may submit.
    #[serde(default = "default_max_bytes")]
    pub max_candidate_bytes: u64,
    /// The declaration of the human that an external service may be used.
    ///
    /// The flag is a declaration for a future stage: this stage never opens a
    /// network connection and never calls an external service.
    #[serde(default)]
    pub allow_external_service: bool,
}

/// The default schema of a policy file.
fn default_schema() -> String {
    crate::POLICY_SCHEMA.to_string()
}

/// The default value of `allow_submit`.
fn default_true() -> bool {
    true
}

/// The default tool set of the surface.
fn default_tools() -> Vec<String> {
    ALL_TOOLS.iter().map(|name| name.to_string()).collect()
}

/// The default number of calls of one run.
fn default_max_calls() -> u64 {
    DEFAULT_MAX_CALLS_PER_RUN
}

/// The default size limit of a candidate.
fn default_max_bytes() -> u64 {
    DEFAULT_MAX_CANDIDATE_BYTES
}

impl Default for AssistantPolicy {
    fn default() -> Self {
        AssistantPolicy {
            schema: default_schema(),
            allow_apply: false,
            allow_submit: true,
            allowed_tools: default_tools(),
            max_calls_per_run: DEFAULT_MAX_CALLS_PER_RUN,
            max_candidate_bytes: DEFAULT_MAX_CANDIDATE_BYTES,
            allow_external_service: false,
        }
    }
}

impl AssistantPolicy {
    /// True when the tool is exposed by the policy.
    pub fn allows_tool(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|name| name == tool)
    }

    /// The strictest number of calls of the policy and of a task budget.
    pub fn effective_max_calls(&self, task_max_calls: u64) -> u64 {
        self.max_calls_per_run.min(task_max_calls)
    }

    /// The strictest size limit of the policy and of a task budget.
    pub fn effective_max_candidate_bytes(&self, task_max_bytes: u64) -> u64 {
        self.max_candidate_bytes.min(task_max_bytes)
    }

    /// Parse and validate a policy document.
    pub fn parse(text: &str) -> Result<AssistantPolicy, AssistantError> {
        let policy: AssistantPolicy =
            serde_json::from_str(text).map_err(|error| AssistantError::Invalid {
                message: format!("the assistant policy is not usable: {error}"),
            })?;
        policy.validate()?;
        Ok(policy)
    }

    /// Validate the values of the policy.
    pub fn validate(&self) -> Result<(), AssistantError> {
        if self.schema != crate::POLICY_SCHEMA {
            return Err(AssistantError::Invalid {
                message: format!(
                    "the assistant policy declares the schema '{}' which is not supported",
                    self.schema
                ),
            });
        }
        if self.allow_apply {
            return Err(AssistantError::Invalid {
                message: "the assistant policy must not allow an automatic application of a candidate: the acceptance is always an explicit author decision".to_string(),
            });
        }
        if self.allowed_tools.is_empty() {
            return Err(AssistantError::Invalid {
                message: "the assistant policy allows no tool".to_string(),
            });
        }
        let mut seen: Vec<&str> = Vec::new();
        for tool in &self.allowed_tools {
            if !ALL_TOOLS.contains(&tool.as_str()) {
                return Err(AssistantError::Invalid {
                    message: format!(
                        "the tool '{tool}' is not one of the four tools of the surface"
                    ),
                });
            }
            if seen.contains(&tool.as_str()) {
                return Err(AssistantError::Invalid {
                    message: format!("the tool '{tool}' is listed twice"),
                });
            }
            seen.push(tool.as_str());
        }
        if self.max_calls_per_run < 1 {
            return Err(AssistantError::Invalid {
                message: "the field 'max_calls_per_run' must be at least 1".to_string(),
            });
        }
        if self.max_candidate_bytes < MIN_CANDIDATE_BYTES
            || self.max_candidate_bytes > MAX_CANDIDATE_BYTES
        {
            return Err(AssistantError::Invalid {
                message: format!(
                    "the field 'max_candidate_bytes' must be in {MIN_CANDIDATE_BYTES}..={MAX_CANDIDATE_BYTES}"
                ),
            });
        }
        Ok(())
    }
}

/// The path of the policy file inside a workspace.
pub fn policy_path(workspace: &Path) -> PathBuf {
    workspace.join(POLICY_FILE_RELATIVE)
}

/// Find the policy file that applies to a directory.
///
/// The search walks up the directory tree, like the edit policy of Stage C.
pub fn find_policy_file(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(directory) = current {
        let candidate = policy_path(directory);
        if candidate.is_file() {
            return Some(candidate);
        }
        current = directory.parent();
    }
    None
}

/// Load the policy that applies to a directory.
///
/// The defaults are returned when the workspace carries no policy file; a
/// policy file that exists but cannot be used is an error.
pub fn load_policy(start: &Path) -> Result<AssistantPolicy, AssistantError> {
    let path = match find_policy_file(start) {
        Some(path) => path,
        None => return Ok(AssistantPolicy::default()),
    };
    let text = std::fs::read_to_string(&path).map_err(|error| {
        AssistantError::io(
            &path,
            format!("the assistant policy cannot be read: {error}"),
        )
    })?;
    let policy = AssistantPolicy::parse(&text)?;
    Ok(policy)
}
