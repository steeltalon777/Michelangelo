//! The pinned immutable `ValidationReport` (contract gate §2.12, draft-0.1).
//!
//! Measurements only — never a decision. The report records the digests of
//! the bytes actually read during the evaluation (IT-8D D1.2 analog), the
//! profile digest it was evaluated against, and per-check results from the
//! NEW model. The aggregate verdict folds as fail > unavailable > pass and
//! has no `not_required` value. Registration is no-clobber: a repeated
//! report is never overwritten (registry.rs).

use serde::{Deserialize, Serialize};

use crate::canonical::{parse_json, to_pretty_json};
use crate::check::{CheckRecord, Verdict};
use crate::error::ValidationError;
use crate::ids::{validate_id, validate_rfc3339, validate_rel_path, validate_sha256};

/// The draft schema tag mirrored from the GLM-1 contract gate.
pub const REPORT_SCHEMA: &str = "michelangelo.validation-report/draft-0.1";

/// The kind of document the measurements bind to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BundleKind {
    #[serde(rename = "candidate_asset_bundle")]
    CandidateAssetBundle,
    #[serde(rename = "asset_revision")]
    AssetRevision,
    #[serde(rename = "source_snapshot")]
    SourceSnapshot,
}

/// The reference to the measured document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    pub kind: BundleKind,
    pub id: String,
    pub sha256: String,
}

/// The reference to the pinned profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRef {
    pub profile_id: String,
    pub revision: u32,
    pub profile_sha256: String,
}

/// The immutable validation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub schema: String,
    pub report_id: String,
    pub created_at: String,
    pub bundle_ref: BundleRef,
    pub profile_ref: ProfileRef,
    /// Digests of the bytes actually read for this evaluation.
    pub input_digests: std::collections::BTreeMap<String, String>,
    pub checks: Vec<CheckRecord>,
    pub verdict: Verdict,
    pub evaluator_versions: std::collections::BTreeMap<String, String>,
}

impl ValidationReport {
    /// Assemble a report from its parts, refusing internally inconsistent
    /// input (F16: `not_required` without an applicability reference, a
    /// verdict that does not match the fold).
    pub fn assemble(
        report_id: String,
        created_at: String,
        bundle_ref: BundleRef,
        profile_ref: ProfileRef,
        input_digests: std::collections::BTreeMap<String, String>,
        mut checks: Vec<CheckRecord>,
        evaluator_versions: std::collections::BTreeMap<String, String>,
    ) -> Result<ValidationReport, ValidationError> {
        validate_id(&report_id, "report_id")?;
        validate_rfc3339(&created_at)?;
        validate_id(&bundle_ref.id, "bundle_ref.id")?;
        validate_sha256(&bundle_ref.sha256, "bundle_ref.sha256")?;
        validate_sha256(&profile_ref.profile_sha256, "profile_sha256")?;
        validate_id(&profile_ref.profile_id, "profile_id")?;

        for (path, digest) in &input_digests {
            validate_rel_path(path)?;
            validate_sha256(digest, "input digest")?;
        }

        // At most one record per check; deterministic order.
        checks.sort_by_key(|record| record.check_id);
        let mut seen = std::collections::BTreeSet::new();
        for record in &checks {
            if !seen.insert(record.check_id) {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    format!(
                        "the check '{}' has more than one result",
                        record.check_id.as_str()
                    ),
                ));
            }
            match record.result {
                crate::check::CheckStatus::NotRequired => {
                    match &record.applicability_ref {
                        Some(reference) if !reference.trim().is_empty() => {}
                        _ => {
                            return Err(ValidationError::rule(
                                crate::error::INVALID_NOT_REQUIRED,
                                format!(
                                    "the check '{}' is not_required without an applicability_ref",
                                    record.check_id.as_str()
                                ),
                            ));
                        }
                    }
                }
                crate::check::CheckStatus::Pass
                | crate::check::CheckStatus::Fail
                | crate::check::CheckStatus::Unavailable => {
                    if record.applicability_ref.is_some() {
                        return Err(ValidationError::rule(
                            crate::error::DOCUMENT_INVALID,
                            format!(
                                "the check '{}' carries an applicability_ref without a not_required result",
                                record.check_id.as_str()
                            ),
                        ));
                    }
                }
            }
            for evidence in &record.evidence {
                validate_rel_path(&evidence.path)?;
                validate_sha256(&evidence.sha256, "evidence digest")?;
            }
        }

        if checks.is_empty() {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                "the report must contain at least one check record",
            ));
        }

        let verdict = Verdict::fold(&checks)?;
        let report = ValidationReport {
            schema: REPORT_SCHEMA.to_string(),
            report_id,
            created_at,
            bundle_ref,
            profile_ref,
            input_digests,
            checks,
            verdict,
            evaluator_versions,
        };
        Ok(report)
    }

    /// The digest of the report: SHA-256 over the EXACT serialized bytes
    /// (the value the no-clobber registry pins and the acceptance record
    /// cites; the registry stores the digest of the bytes actually written,
    /// following the IT-8D evidence-registry precedent).
    pub fn digest(&self) -> Result<String, ValidationError> {
        Ok(crate::canonical::sha256_hex(&self.to_json_bytes()?))
    }

    /// Serialize the report bytes (pretty JSON + trailing newline).
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        to_pretty_json(self)
    }

    /// Parse and structurally re-verify a report from bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<ValidationReport, ValidationError> {
        // Deny unknown fields via a strict round-trip: parse into Value first
        // to control the schema tag, then into the typed struct.
        let value = parse_json(bytes, "the validation report")?;
        let report: ValidationReport = serde_json::from_value(value).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the validation report is not usable: {err}"),
            )
        })?;
        if report.schema != REPORT_SCHEMA {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!(
                    "the report schema '{}' is not '{}'",
                    report.schema, REPORT_SCHEMA
                ),
            ));
        }
        // Re-assemble to re-run every structural guard (fold consistency,
        // not_required applicability, one record per check).
        ValidationReport::assemble(
            report.report_id.clone(),
            report.created_at.clone(),
            report.bundle_ref.clone(),
            report.profile_ref.clone(),
            report.input_digests.clone(),
            report.checks.clone(),
            report.evaluator_versions.clone(),
        )
        .map(|reconstructed| {
            if reconstructed.verdict != report.verdict {
                Err(ValidationError::rule(
                    crate::error::INVALID_VERDICT_FOLD,
                    "the stored verdict does not match the fold of the check results",
                ))
            } else {
                Ok(reconstructed)
            }
        })?
    }
}
