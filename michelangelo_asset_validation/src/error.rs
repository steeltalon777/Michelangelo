//! Typed errors of the validation layer.
//!
//! The codes follow the failure registry of the GLM-1 contract gate
//! (`CONTRACT_GATE.md` §5) plus validation-specific extensions. The codes of
//! the existing accept gate (`EVALUATION_*`, `INPUT_CHANGED`, ...) are NOT
//! reused: this module never writes into the IT-8 evaluation report schema.

/// Unknown check_id in `required_checks` (F8).
pub const VALIDATION_UNKNOWN_REQUIRED: &str = "VALIDATION_UNKNOWN_REQUIRED";
/// Unknown check_id in `optional_checks` (F8 family; same refusal class).
pub const VALIDATION_UNKNOWN_OPTIONAL: &str = "VALIDATION_UNKNOWN_OPTIONAL";
/// Required check conflicts with the applicability matrix of the profile kind.
pub const VALIDATION_APPLICABILITY_CONFLICT: &str = "VALIDATION_APPLICABILITY_CONFLICT";
/// Input bytes changed after the evaluation produced the report (F2).
pub const VALIDATION_INPUT_CHANGED: &str = "VALIDATION_INPUT_CHANGED";
/// Input bytes referenced by a registered report are missing (F3 class).
pub const VALIDATION_INPUT_MISSING: &str = "VALIDATION_INPUT_MISSING";
/// Pinned profile/threshold digest differs from the basis of the report (F19).
pub const VALIDATION_PROFILE_CHANGED: &str = "VALIDATION_PROFILE_CHANGED";
/// Acceptance cites a report digest that is not registered (F14).
pub const VALIDATION_REPORT_NOT_REGISTERED: &str = "VALIDATION_REPORT_NOT_REGISTERED";
/// Acceptance of an `accepted` decision cites a report whose verdict is not pass.
pub const VALIDATION_VERDICT_NOT_PASS: &str = "VALIDATION_VERDICT_NOT_PASS";
/// A required check of the profile has no result in the report.
pub const MISSING_REQUIRED_CHECK: &str = "MISSING_REQUIRED_CHECK";
/// A `not_required` check result lacks its applicability reference (F16).
pub const INVALID_NOT_REQUIRED: &str = "INVALID_NOT_REQUIRED";
/// The verdict does not match the fold of the check results (F16).
pub const INVALID_VERDICT_FOLD: &str = "INVALID_VERDICT_FOLD";
/// A registered acceptance record is replayed for a different bundle digest (F5).
pub const ACCEPTANCE_REPLAY: &str = "ACCEPTANCE_REPLAY";
/// The canonical accept Core seam is not implemented; validation never publishes.
pub const ACCEPT_SEAM_NOT_AVAILABLE: &str = "ACCEPT_SEAM_NOT_AVAILABLE";
/// A registered document or report would be overwritten (no-clobber rule).
pub const NO_CLOBBER_VIOLATION: &str = "NO_CLOBBER_VIOLATION";
/// Registration without a prior reservation of the report id (IT-8D D1.3 analog).
pub const RESERVATION_REQUIRED: &str = "RESERVATION_REQUIRED";
/// The report id is reserved by another registration in flight.
pub const RESERVATION_CONFLICT: &str = "RESERVATION_CONFLICT";
/// A path leaves the workspace container.
pub const PATH_ESCAPE: &str = "PATH_ESCAPE";
/// An element of the transitive dependency closure is missing (F3).
pub const DEPENDENCY_MISSING: &str = "DEPENDENCY_MISSING";
/// The digest of an actual dependency does not match the pinned value.
pub const DEPENDENCY_CHANGED: &str = "DEPENDENCY_CHANGED";
/// The recomputed digest does not match the declared/pinned snapshot (F1).
pub const SNAPSHOT_MISMATCH: &str = "SNAPSHOT_MISMATCH";
/// The profile kind is out of scope of this module (reconstruction stays with IT-8D).
pub const UNSUPPORTED_PROFILE_KIND: &str = "UNSUPPORTED_PROFILE_KIND";
/// A pinned reference named by the profile is missing from the workspace.
pub const REFERENCE_MISSING: &str = "REFERENCE_MISSING";
/// The document is not usable.
pub const DOCUMENT_INVALID: &str = "DOCUMENT_INVALID";
/// Unwrapped IO failure.
pub const IO_ERROR: &str = "IO_ERROR";

/// A failure of the validation layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// A contract rule is violated; `code` is one of the constants above.
    Rule {
        code: &'static str,
        message: String,
    },
    /// A file or directory could not be read or written.
    Io {
        path: std::path::PathBuf,
        message: String,
    },
}

impl ValidationError {
    /// Build a rule violation.
    pub fn rule(code: &'static str, message: impl Into<String>) -> ValidationError {
        ValidationError::Rule {
            code,
            message: message.into(),
        }
    }

    /// Build an IO failure for a path.
    pub fn io(path: &std::path::Path, message: impl Into<String>) -> ValidationError {
        ValidationError::Io {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }

    /// The diagnostic code of the failure.
    pub fn code(&self) -> &'static str {
        match self {
            ValidationError::Rule { code, .. } => code,
            ValidationError::Io { .. } => IO_ERROR,
        }
    }

    /// The human readable message.
    pub fn message(&self) -> &str {
        match self {
            ValidationError::Rule { message, .. } => message,
            ValidationError::Io { message, .. } => message,
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::Rule { code, message } => write!(formatter, "{code}: {message}"),
            ValidationError::Io { path, message } => {
                write!(formatter, "the path '{}' cannot be used: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ValidationError {}
