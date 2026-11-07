//! The typed error of the evidence crate.

use std::path::{Path, PathBuf};

/// The diagnostic code of a missing evaluation report.
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
/// The diagnostic code of a per-view regression.
pub const REGRESSION: &str = "REGRESSION";
/// The diagnostic code of a missing independent ground truth.
pub const NO_GROUND_TRUTH: &str = "NO_GROUND_TRUTH";
/// The diagnostic code of an evaluation id that is already registered.
pub const EVALUATION_ALREADY_REGISTERED: &str = "EVALUATION_ALREADY_REGISTERED";
/// The diagnostic code of a broken evidence document.
pub const EVIDENCE_INVALID: &str = "EVIDENCE_INVALID";

/// A failure of the evidence store.
///
/// Every variant carries the diagnostic code that the accept gate prints and
/// a human readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    /// The evaluation report does not exist or cannot be read.
    Missing {
        /// The human readable reason.
        message: String,
    },
    /// The evaluation report is not registered in the registry of the run.
    Unregistered {
        /// The human readable reason.
        message: String,
    },
    /// The evaluation report does not match the pins of the run.
    Stale {
        /// The human readable reason.
        message: String,
    },
    /// The inputs of the run changed after the evaluation.
    InputChanged {
        /// The human readable reason.
        message: String,
    },
    /// The visible (projection) result does not pass.
    VisibleFailed {
        /// The human readable reason.
        message: String,
    },
    /// A required author geometric condition does not pass.
    AuthorConditionFailed {
        /// The human readable reason.
        message: String,
    },
    /// A source view regressed against the base revision.
    Regression {
        /// The human readable reason.
        message: String,
    },
    /// A claim needs a ground truth that is not available.
    NoGroundTruth {
        /// The human readable reason.
        message: String,
    },
    /// An evaluation id is already registered with a different digest.
    AlreadyRegistered {
        /// The human readable reason.
        message: String,
    },
    /// A file or directory could not be read, written or renamed.
    Io {
        /// The path that could not be used.
        path: PathBuf,
        /// The human readable reason.
        message: String,
    },
    /// An evidence document does not follow its schema.
    Invalid {
        /// The human readable reason.
        message: String,
    },
}

impl EvidenceError {
    /// Build an IO error for a path.
    pub fn io(path: &Path, message: impl Into<String>) -> EvidenceError {
        EvidenceError::Io {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }

    /// The diagnostic code of the error.
    pub fn code(&self) -> &'static str {
        match self {
            EvidenceError::Missing { .. } => EVALUATION_MISSING,
            EvidenceError::Unregistered { .. } => EVALUATION_UNREGISTERED,
            EvidenceError::Stale { .. } => EVALUATION_STALE,
            EvidenceError::InputChanged { .. } => INPUT_CHANGED,
            EvidenceError::VisibleFailed { .. } => VISIBLE_FAILED,
            EvidenceError::AuthorConditionFailed { .. } => AUTHOR_CONDITION_FAILED,
            EvidenceError::Regression { .. } => REGRESSION,
            EvidenceError::NoGroundTruth { .. } => NO_GROUND_TRUTH,
            EvidenceError::AlreadyRegistered { .. } => EVALUATION_ALREADY_REGISTERED,
            EvidenceError::Io { .. } => "IO_ERROR",
            EvidenceError::Invalid { .. } => EVIDENCE_INVALID,
        }
    }

    /// The human readable message of the error.
    pub fn message(&self) -> &str {
        match self {
            EvidenceError::Missing { message }
            | EvidenceError::Unregistered { message }
            | EvidenceError::Stale { message }
            | EvidenceError::InputChanged { message }
            | EvidenceError::VisibleFailed { message }
            | EvidenceError::AuthorConditionFailed { message }
            | EvidenceError::Regression { message }
            | EvidenceError::NoGroundTruth { message }
            | EvidenceError::AlreadyRegistered { message }
            | EvidenceError::Invalid { message } => message,
            EvidenceError::Io { message, .. } => message,
        }
    }
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvidenceError::Io { path, message } => {
                write!(
                    formatter,
                    "the path '{}' cannot be used: {message}",
                    path.display()
                )
            }
            other => write!(formatter, "{}: {}", other.code(), other.message()),
        }
    }
}

impl std::error::Error for EvidenceError {}
