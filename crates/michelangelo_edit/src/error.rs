//! The typed error of the edit boundary.

use std::path::{Path, PathBuf};

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
}

impl EditError {
    /// The diagnostic code of the error.
    pub fn code(&self) -> &'static str {
        match self {
            EditError::Io { .. } => "IO_ERROR",
            EditError::Schema { .. } => "INTERNAL_ERROR",
            EditError::Scene { .. } => "INTERNAL_ERROR",
            EditError::Policy { .. } => "POLICY_DENIED",
            EditError::Internal { .. } => "INTERNAL_ERROR",
        }
    }

    /// The CLI exit code of the error (2 — usage or IO error).
    pub fn exit_code(&self) -> u8 {
        2
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
            EditError::Schema { message } => write!(formatter, "{message}"),
            EditError::Scene { message } => write!(formatter, "{message}"),
            EditError::Policy { message } => write!(formatter, "{message}"),
            EditError::Internal { message } => write!(formatter, "{message}"),
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
