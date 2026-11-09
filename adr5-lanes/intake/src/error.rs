//! Hard failure modes of the intake PoC.
//!
//! Diagnostics that describe the *content* of a rejected input live in
//! [`crate::report`] and are returned wrapped in
//! [`IntakeError::PlanRejected`]. `IntakeError` variants below represent
//! operational failures that are not content diagnostics.

use std::path::PathBuf;

use crate::report::PlanRejection;

/// Top-level error of `michelangelo_asset_intake`.
#[derive(Debug)]
pub enum IntakeError {
    /// Filesystem failure.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The source format is explicitly not supported. `.blend` and remote
    /// URIs are deliberately NOT supported by this PoC (ADR 0005 §13.2).
    UnsupportedFormat { path: PathBuf, reason: String },
    /// The source file exceeds `max_source_bytes` before it is read.
    SourceTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    /// GLB container structure is invalid. `code` is the diagnostic code the
    /// plan pipeline should record for the rejection.
    MalformedGlb {
        code: crate::report::DiagnosticCode,
        reason: String,
    },
    /// The plan was rejected because at least one diagnostic has severity
    /// `Error`. The rejection carries the full machine-readable report.
    PlanRejected { rejection: PlanRejection },
    /// A pinned input changed between planning and execution. This is the
    /// IT-8D-aligned failure: a substituted dependency is never a silent
    /// success.
    InputChanged {
        path: PathBuf,
        expected_sha256: String,
        actual_sha256: String,
    },
    /// The snapshot directory for this identity already exists but does not
    /// match the expected manifest. Nothing is ever overwritten.
    SnapshotDirConflict { dir: PathBuf, reason: String },
    /// An existing snapshot failed verification (corrupted copy or manifest).
    SnapshotCorrupt { dir: PathBuf, reason: String },
    /// The requested output path already exists (no-clobber export).
    OutputExists { path: PathBuf },
    /// A written copy did not match its pinned hash when read back.
    CopyVerificationFailed {
        path: PathBuf,
        expected_sha256: String,
        actual_sha256: String,
    },
    /// Format conversion was requested. This PoC performs no conversion at
    /// all: no hidden remesh, retopo or decimation exists anywhere in this
    /// crate.
    ConversionNotImplemented,
    /// The snapshot manifest could not be parsed as JSON.
    ManifestJson {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntakeError::Io { path, source } => {
                write!(f, "io error at {}: {source}", path.display())
            }
            IntakeError::UnsupportedFormat { path, reason } => {
                write!(f, "unsupported source format {}: {reason}", path.display())
            }
            IntakeError::SourceTooLarge { path, size, limit } => write!(
                f,
                "source {} is {size} bytes, above the limit of {limit} bytes",
                path.display()
            ),
            IntakeError::MalformedGlb { code, reason } => {
                write!(f, "malformed GLB container ({code:?}): {reason}")
            }
            IntakeError::PlanRejected { rejection } => write!(
                f,
                "intake plan rejected ({} error diagnostics)",
                rejection.diagnostics.iter().filter(|d| d.is_error()).count()
            ),
            IntakeError::InputChanged { path, expected_sha256, actual_sha256 } => write!(
                f,
                "input changed after planning: {path} expected sha256 {expected_sha256}, actual {actual_sha256} (INPUT_CHANGED)",
                path = path.display()
            ),
            IntakeError::SnapshotDirConflict { dir, reason } => {
                write!(f, "snapshot directory {} already exists and does not match: {reason}", dir.display())
            }
            IntakeError::SnapshotCorrupt { dir, reason } => {
                write!(f, "existing snapshot {} failed verification: {reason}", dir.display())
            }
            IntakeError::OutputExists { path } => {
                write!(f, "output already exists, no-clobber: {}", path.display())
            }
            IntakeError::CopyVerificationFailed { path, expected_sha256, actual_sha256 } => write!(
                f,
                "copy verification failed for {path}: expected sha256 {expected_sha256}, actual {actual_sha256}",
                path = path.display()
            ),
            IntakeError::ConversionNotImplemented => {
                write!(f, "format conversion is not implemented in this PoC; any conversion would require an explicit loss report")
            }
            IntakeError::ManifestJson { path, source } => {
                write!(f, "snapshot manifest {} is not valid JSON: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for IntakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IntakeError::Io { source, .. } => Some(source),
            IntakeError::ManifestJson { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IntakeError {
    fn from(source: std::io::Error) -> Self {
        IntakeError::Io {
            path: PathBuf::new(),
            source,
        }
    }
}
