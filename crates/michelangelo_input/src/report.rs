//! Report types for input package validation.
//!
//! The report is the stable machine readable contract of the validator; the
//! CLI prints it verbatim. The top level keys are fixed:
//! schema, package_id, revision, status, levels, completeness, diagnostics,
//! resources.

use serde::{Deserialize, Serialize};

/// Schema identifier of the validation report.
pub const REPORT_SCHEMA: &str = "michelangelo.input-package-report/0";

/// Overall validation status of a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportStatus {
    /// The package is complete for metric reconstruction.
    #[serde(rename = "valid_complete")]
    ValidComplete,
    /// The package is valid but partial.
    #[serde(rename = "valid_partial")]
    ValidPartial,
    /// The package is invalid.
    #[serde(rename = "invalid")]
    Invalid,
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// The diagnostic blocks acceptance.
    #[serde(rename = "error")]
    Error,
    /// The diagnostic is informational.
    #[serde(rename = "warning")]
    Warning,
}

/// Summary of one validation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelSummary {
    /// True when the level has no errors.
    pub ok: bool,
    /// Number of errors of the level.
    pub errors: u32,
    /// Number of warnings of the level.
    pub warnings: u32,
}

/// Result summary of both validation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationLevels {
    /// Level 1, format and safety.
    pub format_safety: LevelSummary,
    /// Level 2, sufficiency.
    pub sufficiency: LevelSummary,
}

/// Completeness of the package for metric reconstruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletenessStatus {
    /// At least two independent calibrated views spanning two axes.
    #[serde(rename = "complete")]
    Complete,
    /// Not enough information for metric reconstruction.
    #[serde(rename = "partial")]
    Partial,
}

/// Completeness details of a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completeness {
    /// Complete or partial.
    pub status: CompletenessStatus,
    /// Number of usable calibrated views.
    pub calibrated_views: u32,
    /// Number of pairwise distinct view directions.
    pub distinct_directions: u32,
    /// Coordinate axes spanned by the distinct directions.
    pub axes_covered: Vec<String>,
    /// Diagnostic codes that describe what is missing.
    pub missing: Vec<String>,
}

/// One diagnostic of the validation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Validation level, 1 or 2.
    pub level: u8,
    /// Stable diagnostic code.
    pub code: String,
    /// Error or warning.
    pub severity: Severity,
    /// JSON-path like address of the offending element.
    pub address: String,
    /// Human readable message.
    pub message: String,
}

/// One referenced resource of the package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceReport {
    /// Address of the resource, for example "views[0].image".
    pub address: String,
    /// Package relative path of the resource.
    pub path: String,
    /// Lowercase hex SHA-256 of the file content, empty when unreadable.
    pub sha256: String,
    /// Size of the file in bytes.
    pub bytes: u64,
    /// True when the computed hash matches the declared hash.
    pub verified: bool,
}

/// The validation report of one input package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputPackageReport {
    /// Schema identifier, always [REPORT_SCHEMA].
    pub schema: String,
    /// Package identifier, empty when the manifest could not be read.
    pub package_id: String,
    /// Package revision, 0 when the manifest could not be read.
    pub revision: u32,
    /// Overall status.
    pub status: ReportStatus,
    /// Result summary of both validation levels.
    pub levels: ValidationLevels,
    /// Completeness details.
    pub completeness: Completeness,
    /// All diagnostics, sorted by (level, address, code).
    pub diagnostics: Vec<Diagnostic>,
    /// All referenced resources.
    pub resources: Vec<ResourceReport>,
}

impl InputPackageReport {
    /// Build an empty report for a package that could not be read.
    pub fn empty() -> Self {
        InputPackageReport {
            schema: REPORT_SCHEMA.to_string(),
            package_id: String::new(),
            revision: 0,
            status: ReportStatus::Invalid,
            levels: ValidationLevels {
                format_safety: LevelSummary {
                    ok: false,
                    errors: 0,
                    warnings: 0,
                },
                sufficiency: LevelSummary {
                    ok: true,
                    errors: 0,
                    warnings: 0,
                },
            },
            completeness: Completeness {
                status: CompletenessStatus::Partial,
                calibrated_views: 0,
                distinct_directions: 0,
                axes_covered: Vec::new(),
                missing: Vec::new(),
            },
            diagnostics: Vec::new(),
            resources: Vec::new(),
        }
    }

    /// True when the report contains at least one error diagnostic.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Return all diagnostics with the given code.
    pub fn diagnostics_with_code<'a>(
        &'a self,
        code: &'a str,
    ) -> impl Iterator<Item = &'a Diagnostic> {
        self.diagnostics.iter().filter(move |d| d.code == code)
    }
}
