//! The typed error of the evaluator.

use std::path::PathBuf;

/// The diagnostic code of a missing input of the evaluation.
pub const EVALUATION_INPUT_MISSING: &str = "EVALUATION_INPUT_MISSING";
/// The diagnostic code of a broken input of the evaluation.
pub const EVALUATION_INPUT_INVALID: &str = "EVALUATION_INPUT_INVALID";
/// The diagnostic code of a failed render.
pub const RENDER_FAILED: &str = "RENDER_FAILED";
/// The diagnostic code of a failed comparison.
pub const COMPARE_FAILED: &str = "COMPARE_FAILED";
/// The diagnostic code of a broken evidence document of the run.
pub const EVIDENCE_BROKEN: &str = "EVIDENCE_BROKEN";

/// A failure of the evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// A required input of the evaluation is missing.
    Missing {
        /// The human readable reason.
        message: String,
    },
    /// An input of the evaluation cannot be used.
    Invalid {
        /// The human readable reason.
        message: String,
    },
    /// The render of the geometry failed.
    Render {
        /// The human readable reason.
        message: String,
    },
    /// The comparison of the renders failed.
    Compare {
        /// The human readable reason.
        message: String,
    },
    /// The evidence of the run (the snapshot, the scene) is broken.
    Evidence {
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
}

impl EvalError {
    /// Build an IO error for a path.
    pub fn io(path: &std::path::Path, message: impl Into<String>) -> EvalError {
        EvalError::Io {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }

    /// The diagnostic code of the error.
    pub fn code(&self) -> &'static str {
        match self {
            EvalError::Missing { .. } => EVALUATION_INPUT_MISSING,
            EvalError::Invalid { .. } => EVALUATION_INPUT_INVALID,
            EvalError::Render { .. } => RENDER_FAILED,
            EvalError::Compare { .. } => COMPARE_FAILED,
            EvalError::Evidence { .. } => EVIDENCE_BROKEN,
            EvalError::Io { .. } => "IO_ERROR",
        }
    }

    /// The human readable message of the error.
    pub fn message(&self) -> &str {
        match self {
            EvalError::Missing { message }
            | EvalError::Invalid { message }
            | EvalError::Render { message }
            | EvalError::Compare { message }
            | EvalError::Evidence { message } => message,
            EvalError::Io { message, .. } => message,
        }
    }
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Io { path, message } => {
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

impl std::error::Error for EvalError {}

impl From<michelangelo_edit::EditError> for EvalError {
    fn from(error: michelangelo_edit::EditError) -> EvalError {
        EvalError::Evidence {
            message: error.message().to_string(),
        }
    }
}
