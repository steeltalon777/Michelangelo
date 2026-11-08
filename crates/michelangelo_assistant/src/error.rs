//! The typed failures of the assistant surface.
//!
//! Every failure of the assistant carries a stable code, a human readable
//! message and the exit code of the CLI. The codes are mapped to the
//! JSON-RPC error codes of the MCP adapter (see `crate::mcp`).

use std::path::{Path, PathBuf};

/// A failure of the assistant surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantError {
    /// A file or directory could not be read, written or renamed.
    Io {
        /// The path that could not be used.
        path: PathBuf,
        /// The human readable reason.
        message: String,
    },
    /// A requested object does not exist.
    NotFound {
        /// The human readable reason.
        message: String,
    },
    /// The task (or a candidate against the task) is not usable.
    TaskInvalid {
        /// The human readable reason.
        message: String,
    },
    /// The policy of the workspace refuses the operation.
    PolicyDenied {
        /// The human readable reason.
        message: String,
    },
    /// The budget of the run is exhausted.
    BudgetExhausted {
        /// The human readable reason.
        message: String,
    },
    /// A path argument resolves outside the workspace or the run store.
    PathNotAllowed {
        /// The human readable reason.
        message: String,
    },
    /// The request or a document is not usable.
    Invalid {
        /// The human readable reason.
        message: String,
    },
}

impl AssistantError {
    /// The stable code of the failure.
    pub fn code(&self) -> &'static str {
        match self {
            AssistantError::Io { .. } => "IO_ERROR",
            AssistantError::NotFound { .. } => "NOT_FOUND",
            AssistantError::TaskInvalid { .. } => "TASK_INVALID",
            AssistantError::PolicyDenied { .. } => "POLICY_DENIED",
            AssistantError::BudgetExhausted { .. } => "BUDGET_EXHAUSTED",
            AssistantError::PathNotAllowed { .. } => "PATH_NOT_ALLOWED",
            AssistantError::Invalid { .. } => "INVALID_REQUEST",
        }
    }

    /// The exit code of the failure (1 for a refusal, 2 for usage and IO).
    pub fn exit_code(&self) -> u8 {
        match self {
            AssistantError::PolicyDenied { .. } | AssistantError::BudgetExhausted { .. } => 1,
            _ => 2,
        }
    }

    /// Build an IO error for a path.
    pub fn io(path: &Path, message: impl Into<String>) -> Self {
        AssistantError::Io {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AssistantError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssistantError::Io { path, message } => {
                write!(
                    formatter,
                    "the path '{}' cannot be used: {message}",
                    path.display()
                )
            }
            AssistantError::NotFound { message } => write!(formatter, "{message}"),
            AssistantError::TaskInvalid { message } => write!(formatter, "{message}"),
            AssistantError::PolicyDenied { message } => write!(formatter, "{message}"),
            AssistantError::BudgetExhausted { message } => write!(formatter, "{message}"),
            AssistantError::PathNotAllowed { message } => write!(formatter, "{message}"),
            AssistantError::Invalid { message } => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for AssistantError {}

impl From<serde_json::Error> for AssistantError {
    fn from(error: serde_json::Error) -> Self {
        AssistantError::Invalid {
            message: format!("the document cannot be (de)serialized: {error}"),
        }
    }
}

impl From<michelangelo_edit::EditError> for AssistantError {
    fn from(error: michelangelo_edit::EditError) -> Self {
        match error {
            michelangelo_edit::EditError::Io { path, message } => {
                AssistantError::Io { path, message }
            }
            michelangelo_edit::EditError::Policy { message } => {
                AssistantError::PolicyDenied { message }
            }
            michelangelo_edit::EditError::Schema { message }
            | michelangelo_edit::EditError::Scene { message }
            | michelangelo_edit::EditError::Internal { message }
            | michelangelo_edit::EditError::StoreBusy { message }
            | michelangelo_edit::EditError::EvidenceMismatch { message }
            | michelangelo_edit::EditError::SnapshotMissing { message }
            | michelangelo_edit::EditError::StaleBase { message } => {
                AssistantError::Invalid { message }
            }
            // The refusals of the accept gate keep their diagnostic code.
            michelangelo_edit::EditError::EvaluationMissing { message } => {
                AssistantError::Invalid {
                    message: format!("EVALUATION_MISSING: {message}"),
                }
            }
            michelangelo_edit::EditError::EvaluationStale { message } => AssistantError::Invalid {
                message: format!("EVALUATION_STALE: {message}"),
            },
            michelangelo_edit::EditError::EvaluationUnregistered { message } => {
                AssistantError::Invalid {
                    message: format!("EVALUATION_UNREGISTERED: {message}"),
                }
            }
            michelangelo_edit::EditError::InputChanged { message } => AssistantError::Invalid {
                message: format!("INPUT_CHANGED: {message}"),
            },
            michelangelo_edit::EditError::VisibleFailed { message } => AssistantError::Invalid {
                message: format!("VISIBLE_FAILED: {message}"),
            },
            michelangelo_edit::EditError::AuthorConditionFailed { message } => {
                AssistantError::Invalid {
                    message: format!("AUTHOR_CONDITION_FAILED: {message}"),
                }
            }
            michelangelo_edit::EditError::Regression { message } => AssistantError::Invalid {
                message: format!("REGRESSION: {message}"),
            },
            michelangelo_edit::EditError::NoGroundTruth { message } => AssistantError::Invalid {
                message: format!("NO_GROUND_TRUTH: {message}"),
            },
            // The refusals of the unknown state machine keep their code.
            michelangelo_edit::EditError::UnknownNotFound { message } => AssistantError::Invalid {
                message: format!("UNKNOWN_NOT_FOUND: {message}"),
            },
            michelangelo_edit::EditError::BindingMissing { message } => AssistantError::Invalid {
                message: format!("BINDING_MISSING: {message}"),
            },
            michelangelo_edit::EditError::ConfirmationMissing { message } => {
                AssistantError::Invalid {
                    message: format!("CONFIRMATION_MISSING: {message}"),
                }
            }
            michelangelo_edit::EditError::ResolutionRefused { message } => {
                AssistantError::Invalid {
                    message: format!("RESOLUTION_REFUSED: {message}"),
                }
            }
            michelangelo_edit::EditError::BindingMismatch { message } => AssistantError::Invalid {
                message: format!("BINDING_MISMATCH: {message}"),
            },
            michelangelo_edit::EditError::UnknownPolicyViolation { message } => {
                AssistantError::Invalid {
                    message: format!("UNKNOWN_POLICY_VIOLATION: {message}"),
                }
            }
            michelangelo_edit::EditError::StubForbidden { message } => AssistantError::Invalid {
                message: format!("STUB_FORBIDDEN: {message}"),
            },
            michelangelo_edit::EditError::AgentConfirmationRefused { message } => {
                AssistantError::Invalid {
                    message: format!("AGENT_CONFIRMATION_REFUSED: {message}"),
                }
            }
            michelangelo_edit::EditError::Usage { message } => AssistantError::Invalid { message },
        }
    }
}
