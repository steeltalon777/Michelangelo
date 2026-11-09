//! Machine-readable diagnostics for the intake pipeline.
//!
//! Every diagnostic has a stable code, a severity and an optional subject
//! (the declared reference or file it applies to). A plan with any
//! `Error`-severity diagnostic is rejected; `Warning` diagnostics are
//! recorded and carried into the snapshot manifest.

use serde::{Deserialize, Serialize};

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Stable diagnostic codes. Serialized as snake_case strings; these names are
/// part of the provisional internal DTO (see `crate::dto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// Source format is not supported (e.g. `.blend`, remote URI).
    UnsupportedFormat,
    /// Declared format does not match file content (e.g. `.glb` magic).
    FormatMismatch,
    /// The glTF declares `extensionsRequired` that this PoC does not support.
    UnsupportedRequiredExtension,
    /// A buffer/image carries an extension that changes its payload encoding.
    UnsupportedBufferExtension,
    /// glTF `asset.version` is not 2.x.
    UnsupportedVersion,
    /// A relative reference escapes the source directory (`..`).
    PathTraversal,
    /// A reference is an absolute filesystem path.
    AbsoluteReference,
    /// A reference uses a network / URI scheme (`http:`, `https:`, `file:`, ...).
    NetworkReference,
    /// A reference contains percent-encoding; conservatively rejected.
    PercentEncodedReference,
    /// The resolved target escapes the source directory via symlink/junction.
    SymlinkEscape,
    /// The referenced file does not exist.
    MissingDependency,
    /// The referenced file exists but could not be read.
    DependencyReadError,
    /// Two distinct declared references resolve to the same canonical file.
    PathCollision,
    /// A dependency exceeds `max_dependency_bytes`.
    DependencyTooLarge,
    /// Total dependency bytes exceed `max_total_dependency_bytes`.
    TotalSizeExceeded,
    /// Number of dependencies exceeds `max_dependency_count`.
    TooManyDependencies,
    /// A declared reference exceeds `max_reference_len`.
    ReferenceTooLong,
    /// GLB file is shorter than its declared total length / chunk layout.
    TruncatedGlb,
    /// GLB chunk structure is inconsistent (bad type order, trailing bytes,
    /// buffer `byteLength` larger than the BIN chunk).
    GlbChunkMismatch,
    /// A `data:` URI could not be decoded.
    DataUriError,
    /// Text asset is not valid UTF-8; parsed lossily with a warning.
    NonUtf8Text,
    /// A text reference contains replacement characters after lossy decoding,
    /// so the referenced filename is ambiguous.
    AmbiguousEncoding,
    /// No license information was provided for the source.
    UnknownLicense,
    /// Informational note about OBJ parsing coverage (geometry is not validated).
    ObjParseNote,
}

/// One diagnostic record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    /// The declared reference or file the diagnostic applies to, if any.
    pub subject: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn severity_str(&self) -> &'static str {
        match self.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    pub fn error(
        code: DiagnosticCode,
        subject: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            subject,
            message: message.into(),
        }
    }

    pub fn warning(
        code: DiagnosticCode,
        subject: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            code,
            severity: Severity::Warning,
            subject,
            message: message.into(),
        }
    }

    pub fn info(code: DiagnosticCode, subject: Option<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Info,
            subject,
            message: message.into(),
        }
    }
}

/// Overall status derived from the diagnostics of a plan or snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Clean,
    WithWarnings,
    Rejected,
}

pub fn report_status(diagnostics: &[Diagnostic]) -> ReportStatus {
    if diagnostics.iter().any(Diagnostic::is_error) {
        ReportStatus::Rejected
    } else if !diagnostics.is_empty() {
        ReportStatus::WithWarnings
    } else {
        ReportStatus::Clean
    }
}

/// Machine-readable payload of [`crate::error::IntakeError::PlanRejected`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRejection {
    /// Identity could be computed before the rejection; otherwise `None`.
    pub identity: Option<String>,
    pub format: Option<crate::format::SourceFormat>,
    pub diagnostics: Vec<Diagnostic>,
    pub status: ReportStatus,
}
