//! The typed error of the edit boundary.

use std::path::{Path, PathBuf};

/// The diagnostic code of a store that is locked by another process.
pub const STORE_BUSY: &str = "STORE_BUSY";
/// The diagnostic code of evidence that does not match its digest.
pub const EVIDENCE_MISMATCH: &str = "EVIDENCE_MISMATCH";
/// The diagnostic code of a candidate without a snapshot.
pub const SNAPSHOT_MISSING: &str = "SNAPSHOT_MISSING";
/// The diagnostic code of a candidate whose base is not current any more.
pub const STALE_BASE: &str = "STALE_BASE";
/// The diagnostic code of a candidate that was already accepted.
pub const CANDIDATE_ALREADY_ACCEPTED: &str = "CANDIDATE_ALREADY_ACCEPTED";
/// The diagnostic code of an acceptance without an evaluation report.
pub const EVALUATION_MISSING: &str = "EVALUATION_MISSING";
/// The diagnostic code of an evaluation report that does not match its pins.
pub const EVALUATION_STALE: &str = "EVALUATION_STALE";
/// The diagnostic code of an evaluation report that is not registered.
pub const EVALUATION_UNREGISTERED: &str = "EVALUATION_UNREGISTERED";
/// The diagnostic code of inputs that changed after the evaluation.
pub const INPUT_CHANGED: &str = "INPUT_CHANGED";
/// The diagnostic code of a failed projection conformance.
pub const VISIBLE_FAILED: &str = "VISIBLE_FAILED";
/// The diagnostic code of a failed author geometric condition.
pub const AUTHOR_CONDITION_FAILED: &str = "AUTHOR_CONDITION_FAILED";
/// The diagnostic code of a per-view regression against the base revision.
pub const REGRESSION: &str = "REGRESSION";
/// The diagnostic code of a missing independent ground truth.
pub const NO_GROUND_TRUTH: &str = "NO_GROUND_TRUTH";

/// A failure of the local edit boundary.
///
/// Every variant carries a human readable message and the diagnostic code
/// that is used in the decision report of the operation that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// A file or directory could not be read, written or renamed.
    Io {
        /// The path that could not be used.
        path: PathBuf,
        /// The human readable reason.
        message: String,
    },
    /// The candidate (or another document) does not follow its schema.
    Schema {
        /// The human readable reason.
        message: String,
    },
    /// The scene store is inconsistent (missing revision, broken pointer).
    Scene {
        /// The human readable reason.
        message: String,
    },
    /// The edit policy file exists but cannot be used.
    Policy {
        /// The human readable reason.
        message: String,
    },
    /// An unexpected internal failure.
    Internal {
        /// The human readable reason.
        message: String,
    },
    /// The store is locked by another process (`STORE_BUSY`).
    StoreBusy {
        /// The human readable reason.
        message: String,
    },
    /// Evidence does not match its digest (`EVIDENCE_MISMATCH`).
    EvidenceMismatch {
        /// The human readable reason.
        message: String,
    },
    /// A candidate without a snapshot cannot be accepted (`SNAPSHOT_MISSING`).
    SnapshotMissing {
        /// The human readable reason.
        message: String,
    },
    /// The base revision of the candidate is not current any more (STALE_BASE).
    StaleBase {
        /// The human readable reason.
        message: String,
    },
    /// The acceptance carries no evaluation report (EVALUATION_MISSING).
    EvaluationMissing {
        /// The human readable reason.
        message: String,
    },
    /// The evaluation report does not match its pins (EVALUATION_STALE).
    EvaluationStale {
        /// The human readable reason.
        message: String,
    },
    /// The evaluation report is not registered (EVALUATION_UNREGISTERED).
    EvaluationUnregistered {
        /// The human readable reason.
        message: String,
    },
    /// The inputs of the run changed after the evaluation (INPUT_CHANGED).
    InputChanged {
        /// The human readable reason.
        message: String,
    },
    /// The projection conformance does not pass (VISIBLE_FAILED).
    VisibleFailed {
        /// The human readable reason.
        message: String,
    },
    /// A required author geometric condition does not pass
    /// (AUTHOR_CONDITION_FAILED).
    AuthorConditionFailed {
        /// The human readable reason.
        message: String,
    },
    /// A source view regresses against the base revision (REGRESSION).
    Regression {
        /// The human readable reason.
        message: String,
    },
    /// A claim needs a ground truth that is not available (NO_GROUND_TRUTH).
    NoGroundTruth {
        /// The human readable reason.
        message: String,
    },
}

impl EditError {
    /// The message of the error.
    pub fn message(&self) -> &str {
        match self {
            EditError::Io { message, .. }
            | EditError::Schema { message }
            | EditError::Scene { message }
            | EditError::Policy { message }
            | EditError::Internal { message }
            | EditError::StoreBusy { message }
            | EditError::EvidenceMismatch { message }
            | EditError::SnapshotMissing { message }
            | EditError::StaleBase { message }
            | EditError::EvaluationMissing { message }
            | EditError::EvaluationStale { message }
            | EditError::EvaluationUnregistered { message }
            | EditError::InputChanged { message }
            | EditError::VisibleFailed { message }
            | EditError::AuthorConditionFailed { message }
            | EditError::Regression { message }
            | EditError::NoGroundTruth { message } => message,
        }
    }

    /// The diagnostic code of the error.
    pub fn code(&self) -> &'static str {
        match self {
            EditError::Io { .. } => "IO_ERROR",
            EditError::Schema { .. } => "INTERNAL_ERROR",
            EditError::Scene { .. } => "INTERNAL_ERROR",
            EditError::Policy { .. } => "POLICY_DENIED",
            EditError::Internal { .. } => "INTERNAL_ERROR",
            EditError::StoreBusy { .. } => STORE_BUSY,
            EditError::EvidenceMismatch { .. } => EVIDENCE_MISMATCH,
            EditError::SnapshotMissing { .. } => SNAPSHOT_MISSING,
            EditError::StaleBase { .. } => STALE_BASE,
            EditError::EvaluationMissing { .. } => EVALUATION_MISSING,
            EditError::EvaluationStale { .. } => EVALUATION_STALE,
            EditError::EvaluationUnregistered { .. } => EVALUATION_UNREGISTERED,
            EditError::InputChanged { .. } => INPUT_CHANGED,
            EditError::VisibleFailed { .. } => VISIBLE_FAILED,
            EditError::AuthorConditionFailed { .. } => AUTHOR_CONDITION_FAILED,
            EditError::Regression { .. } => REGRESSION,
            EditError::NoGroundTruth { .. } => NO_GROUND_TRUTH,
        }
    }

    /// The CLI exit code of the error.
    ///
    /// `2` is the usage and IO code, `1` is the code of a refused store
    /// condition (the candidate is not applied).
    pub fn exit_code(&self) -> u8 {
        match self.code() {
            STORE_BUSY => 2,
            EVIDENCE_MISMATCH | SNAPSHOT_MISSING | STALE_BASE => 1,
            EVALUATION_MISSING
            | EVALUATION_STALE
            | EVALUATION_UNREGISTERED
            | INPUT_CHANGED
            | VISIBLE_FAILED
            | AUTHOR_CONDITION_FAILED
            | REGRESSION
            | NO_GROUND_TRUTH => 1,
            _ => 2,
        }
    }
}

impl std::fmt::Display for EditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::Io { path, message } => {
                write!(
                    formatter,
                    "the path '{}' cannot be used: {message}",
                    path.display()
                )
            }
            EditError::Schema { message }
            | EditError::Scene { message }
            | EditError::Policy { message }
            | EditError::Internal { message } => write!(formatter, "{message}"),
            EditError::StoreBusy { message } => {
                write!(formatter, "{STORE_BUSY}: {message}")
            }
            EditError::EvidenceMismatch { message } => {
                write!(formatter, "{EVIDENCE_MISMATCH}: {message}")
            }
            EditError::SnapshotMissing { message } => {
                write!(formatter, "{SNAPSHOT_MISSING}: {message}")
            }
            EditError::StaleBase { message } => {
                write!(formatter, "{STALE_BASE}: {message}")
            }
            EditError::EvaluationMissing { message } => {
                write!(formatter, "{EVALUATION_MISSING}: {message}")
            }
            EditError::EvaluationStale { message } => {
                write!(formatter, "{EVALUATION_STALE}: {message}")
            }
            EditError::EvaluationUnregistered { message } => {
                write!(formatter, "{EVALUATION_UNREGISTERED}: {message}")
            }
            EditError::InputChanged { message } => {
                write!(formatter, "{INPUT_CHANGED}: {message}")
            }
            EditError::VisibleFailed { message } => {
                write!(formatter, "{VISIBLE_FAILED}: {message}")
            }
            EditError::AuthorConditionFailed { message } => {
                write!(formatter, "{AUTHOR_CONDITION_FAILED}: {message}")
            }
            EditError::Regression { message } => {
                write!(formatter, "{REGRESSION}: {message}")
            }
            EditError::NoGroundTruth { message } => {
                write!(formatter, "{NO_GROUND_TRUTH}: {message}")
            }
        }
    }
}

impl std::error::Error for EditError {}

/// Build an IO error for a path.
pub fn io_error(path: &Path, message: impl Into<String>) -> EditError {
    EditError::Io {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

impl From<michelangelo_evidence::EvidenceError> for EditError {
    /// Map the typed error of the evidence store onto the error of the
    /// boundary: the diagnostic code of the gate is preserved.
    fn from(error: michelangelo_evidence::EvidenceError) -> EditError {
        let message = error.message().to_string();
        match error {
            michelangelo_evidence::EvidenceError::Missing { .. } => {
                EditError::EvaluationMissing { message }
            }
            michelangelo_evidence::EvidenceError::Unregistered { .. } => {
                EditError::EvaluationUnregistered { message }
            }
            michelangelo_evidence::EvidenceError::Stale { .. } => {
                EditError::EvaluationStale { message }
            }
            michelangelo_evidence::EvidenceError::InputChanged { .. } => {
                EditError::InputChanged { message }
            }
            michelangelo_evidence::EvidenceError::VisibleFailed { .. } => {
                EditError::VisibleFailed { message }
            }
            michelangelo_evidence::EvidenceError::AuthorConditionFailed { .. } => {
                EditError::AuthorConditionFailed { message }
            }
            michelangelo_evidence::EvidenceError::Regression { .. } => {
                EditError::Regression { message }
            }
            michelangelo_evidence::EvidenceError::NoGroundTruth { .. } => {
                EditError::NoGroundTruth { message }
            }
            other => EditError::Internal {
                message: other.message().to_string(),
            },
        }
    }
}
