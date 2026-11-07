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
    /// The base revision of the candidate is not current any more (`STALE_BASE`).
    StaleBase {
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
            | EditError::StaleBase { message } => message,
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
