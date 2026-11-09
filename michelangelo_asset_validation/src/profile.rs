//! The versioned `ValidationProfile` (contract gate §2.11, draft-0.1).
//!
//! The profile is pinned BEFORE generation (ADR 0005 §8.1). An unknown
//! mandatory check blocks acceptance (`VALIDATION_UNKNOWN_REQUIRED`, F8) and
//! a required check that the applicability matrix marks not-applicable for
//! the kind is a profile conflict (`VALIDATION_APPLICABILITY_CONFLICT`) — an
//! agent cannot smuggle an inapplicable "requirement" past the validator.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::canonical::digest_document;
use crate::check::{applicability, Applicability, CheckId, DocRef};
use crate::error::ValidationError;
use crate::ids::{validate_id, validate_rfc3339, validate_sha256, validate_token32};

/// The draft schema tag mirrored from the GLM-1 contract gate.
pub const PROFILE_SCHEMA: &str = "michelangelo.validation-profile/draft-0.1";

/// The profile kind (ADR 0005 §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileKind {
    #[serde(rename = "reconstruction")]
    Reconstruction,
    #[serde(rename = "passthrough_preservation")]
    PassthroughPreservation,
    #[serde(rename = "family_static")]
    FamilyStatic,
    #[serde(rename = "family_rig_compatible")]
    FamilyRigCompatible,
    #[serde(rename = "anatomy_change")]
    AnatomyChange,
}

impl ProfileKind {
    /// The wire name of the kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProfileKind::Reconstruction => "reconstruction",
            ProfileKind::PassthroughPreservation => "passthrough_preservation",
            ProfileKind::FamilyStatic => "family_static",
            ProfileKind::FamilyRigCompatible => "family_rig_compatible",
            ProfileKind::AnatomyChange => "anatomy_change",
        }
    }
}

/// The applicability declaration of a profile. Unknown fields are rejected
/// (M3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliesTo {
    pub targets: Vec<TargetKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub export_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,
}

/// The kind of document the profile applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetKind {
    #[serde(rename = "candidate_asset_bundle")]
    CandidateAssetBundle,
    #[serde(rename = "asset_revision")]
    AssetRevision,
    #[serde(rename = "source_snapshot")]
    SourceSnapshot,
}

/// One threshold entry: metric, comparison, value. Unknown fields are
/// rejected (M3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    pub check_id: CheckId,
    pub metric: String,
    pub comparison: Comparison,
    pub value: f64,
}

/// Threshold comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    #[serde(rename = "le")]
    AtMost,
    #[serde(rename = "ge")]
    AtLeast,
    #[serde(rename = "eq")]
    Equal,
}

impl Comparison {
    /// Evaluate `measured <comparison> value`.
    pub fn holds(&self, measured: f64, value: f64) -> bool {
        match self {
            Comparison::AtMost => measured <= value,
            Comparison::AtLeast => measured >= value,
            Comparison::Equal => (measured - value).abs() < f64::EPSILON,
        }
    }

    /// The wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Comparison::AtMost => "le",
            Comparison::AtLeast => "ge",
            Comparison::Equal => "eq",
        }
    }
}

/// Hash-pinned references the evaluators may resolve. Unknown fields are
/// rejected (M3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProfileReferences {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reference_digests: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_profile_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animation_clips: Vec<DocRef>,
}

/// The versioned validation profile. Immutable once pinned; a change of any
/// field changes the digest and invalidates reports pinned to the old digest
/// (F19). Unknown fields are rejected (M3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationProfile {
    pub schema: String,
    pub profile_id: String,
    pub revision: u32,
    pub created_at: String,
    pub kind: ProfileKind,
    pub applies_to: AppliesTo,
    pub required_checks: Vec<CheckId>,
    #[serde(default)]
    pub optional_checks: Vec<CheckId>,
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
    #[serde(default)]
    pub references: ProfileReferences,
    pub evaluator_versions: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_notes: Option<String>,
}

impl ValidationProfile {
    /// Build and validate a profile from raw JSON bytes. Unknown check ids
    /// are refused with the gate codes (F8) before typed parsing.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<ValidationProfile, ValidationError> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the validation profile is not usable: {err}"),
            )
        })?;
        for (field, code) in [
            ("required_checks", crate::error::VALIDATION_UNKNOWN_REQUIRED),
            ("optional_checks", crate::error::VALIDATION_UNKNOWN_OPTIONAL),
        ] {
            if let Some(list) = value.get(field).and_then(|item| item.as_array()) {
                for item in list {
                    if let Some(name) = item.as_str() {
                        if CheckId::parse(name).is_err() {
                            return Err(ValidationError::rule(
                                code,
                                format!(
                                    "the {field} entry '{name}' is outside the closed \
                                         check_id registry v0.1 (VALIDATION_MATRIX §1)"
                                ),
                            ));
                        }
                    }
                }
            }
        }
        let profile: ValidationProfile = serde_json::from_value(value).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the validation profile is not usable: {err}"),
            )
        })?;
        profile.validate()?;
        Ok(profile)
    }

    /// Structural and applicability validation.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema != PROFILE_SCHEMA {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!(
                    "the profile schema '{}' is not '{}'",
                    self.schema, PROFILE_SCHEMA
                ),
            ));
        }
        validate_id(&self.profile_id, "profile_id")?;
        validate_rfc3339(&self.created_at)?;
        if self.revision == 0 {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                "the profile revision must be at least 1",
            ));
        }
        if self.applies_to.targets.is_empty() {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                "the profile must declare at least one target kind",
            ));
        }
        if self.required_checks.is_empty() {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                "the profile must require at least one check",
            ));
        }

        // Closed registry + uniqueness. serde deserialization already rejects
        // names outside the enum; a defensive re-check keeps the guarantee
        // explicit for hand-assembled values.
        for check in &self.required_checks {
            validate_check_known(*check, true)?;
        }
        for check in &self.optional_checks {
            validate_check_known(*check, false)?;
        }

        // Uniqueness of required/optional entries.
        let mut seen_required = std::collections::BTreeSet::new();
        for check in &self.required_checks {
            if !seen_required.insert(*check) {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    format!("the check '{}' is required twice", check.as_str()),
                ));
            }
        }
        let mut seen_optional = std::collections::BTreeSet::new();
        for check in &self.optional_checks {
            if !seen_optional.insert(*check) {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    format!("the check '{}' is optional twice", check.as_str()),
                ));
            }
            if seen_required.contains(check) {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    format!(
                        "the check '{}' is both required and optional",
                        check.as_str()
                    ),
                ));
            }
        }

        // Applicability matrix guard (VALIDATION_MATRIX §1). The
        // reconstruction kind is skipped: it is a valid profile kind whose
        // evaluation belongs to the existing IT-8D evaluator, which refuses
        // it with UNSUPPORTED_PROFILE_KIND before any check runs.
        if self.kind != ProfileKind::Reconstruction {
            self.validate_applicability()?;
        }

        // Thresholds: unique per (check, metric); non-negative finite values.
        let mut seen_thresholds = std::collections::BTreeSet::new();
        for threshold in &self.thresholds {
            let key = (threshold.check_id, threshold.metric.clone());
            if !seen_thresholds.insert(key) {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    format!(
                        "the threshold '{}'/'{}' is declared twice",
                        threshold.check_id.as_str(),
                        threshold.metric
                    ),
                ));
            }
            if !(threshold.value.is_finite()) {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    "threshold values must be finite",
                ));
            }
        }

        // References: keys and digests.
        for (key, digest) in &self.references.reference_digests {
            validate_token32(key, "reference key")?;
            validate_sha256(digest, "reference digest")?;
        }
        if let Some(skeleton) = &self.references.skeleton_profile_sha256 {
            validate_sha256(skeleton, "skeleton_profile_sha256")?;
        }
        for clip in &self.references.animation_clips {
            crate::ids::validate_rel_path(&clip.path)?;
            validate_sha256(&clip.sha256, "clip digest")?;
        }
        if self.evaluator_versions.is_empty() {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                "the profile must declare at least one evaluator version",
            ));
        }
        Ok(())
    }

    /// The canonical digest of the profile (the value pinned into reports).
    pub fn digest(&self) -> Result<String, ValidationError> {
        digest_document(self)
    }

    /// The applicability matrix guard (VALIDATION_MATRIX §1): a required
    /// check that the matrix marks not-applicable for the kind is a profile
    /// conflict; a conditional required check without its condition token is
    /// a conflict too.
    ///
    /// The optional side is guarded symmetrically (M2): `not_required` is
    /// legal only where the matrix does not require the check. An optional
    /// entry for an unconditionally required check, or for a conditional
    /// check whose condition token is present, would let a content probe mask
    /// the check under `not_required`; both are profile conflicts.
    fn validate_applicability(&self) -> Result<(), ValidationError> {
        for check in &self.required_checks {
            match applicability(self.kind, *check) {
                Applicability::NotApplicable => {
                    return Err(ValidationError::rule(
                        crate::error::VALIDATION_APPLICABILITY_CONFLICT,
                        format!(
                            "the check '{}' is not applicable to the profile kind '{}'",
                            check.as_str(),
                            self.kind.as_str()
                        ),
                    ));
                }
                Applicability::RequiredWithCondition(condition)
                    if !self.has_condition(condition) =>
                {
                    return Err(ValidationError::rule(
                        crate::error::VALIDATION_APPLICABILITY_CONFLICT,
                        format!(
                            "the check '{}' requires the profile condition '{}'",
                            check.as_str(),
                            condition
                        ),
                    ));
                }
                _ => {}
            }
        }
        for check in &self.optional_checks {
            match applicability(self.kind, *check) {
                Applicability::Required => {
                    return Err(ValidationError::rule(
                        crate::error::VALIDATION_APPLICABILITY_CONFLICT,
                        format!(
                            "the check '{}' is unconditionally required by the matrix for the \
                             kind '{}' and cannot be optional",
                            check.as_str(),
                            self.kind.as_str()
                        ),
                    ));
                }
                Applicability::RequiredWithCondition(condition)
                    if self.has_condition(condition) =>
                {
                    return Err(ValidationError::rule(
                        crate::error::VALIDATION_APPLICABILITY_CONFLICT,
                        format!(
                            "the check '{}' is listed as optional but the profile carries the \
                             condition '{}' that makes it required",
                            check.as_str(),
                            condition
                        ),
                    ));
                }
                _ => {}
            }
        }

        // Reverse coverage (P2): the forward loops above only inspect the
        // entries the profile lists, so an OMITTED matrix-required check
        // would never be measured while the report still folds to pass. The
        // matrix legend (§1) treats absence of an R check as unavailable and
        // blocking; an omission is therefore a profile conflict, symmetric to
        // the not_required masking guard. Every Required check, and every
        // RequiredWithCondition check whose token the profile carries, MUST be
        // present in required_checks. NotApplicable / Optional / Human are not
        // required; a conditional check without its token is not required.
        for check in CheckId::ALL {
            let required = match applicability(self.kind, check) {
                Applicability::Required => true,
                Applicability::RequiredWithCondition(condition) => self.has_condition(condition),
                _ => false,
            };
            if required && !self.required_checks.contains(&check) {
                return Err(ValidationError::rule(
                    crate::error::VALIDATION_APPLICABILITY_CONFLICT,
                    format!(
                        "the check '{}' is required by the applicability matrix for the kind \
                         '{}' but is missing from required_checks; omission would mask the \
                         measurement",
                        check.as_str(),
                        self.kind.as_str()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The threshold for a check metric, if declared.
    pub fn threshold(&self, check: CheckId, metric: &str) -> Option<&Threshold> {
        self.thresholds
            .iter()
            .find(|threshold| threshold.check_id == check && threshold.metric == metric)
    }

    /// Whether the profile carries a named condition token.
    pub fn has_condition(&self, condition: &str) -> bool {
        self.applies_to.conditions.iter().any(|c| c == condition)
    }

    /// The digest pinned for a reference key.
    pub fn reference_digest(&self, key: &str) -> Option<&String> {
        self.references.reference_digests.get(key)
    }
}

/// A defensive re-check that the check id belongs to the closed registry.
fn validate_check_known(check: CheckId, required: bool) -> Result<(), ValidationError> {
    let known = CheckId::ALL.contains(&check);
    if known {
        Ok(())
    } else if required {
        Err(ValidationError::rule(
            crate::error::VALIDATION_UNKNOWN_REQUIRED,
            format!(
                "the required check '{}' is outside the registry",
                check.as_str()
            ),
        ))
    } else {
        Err(ValidationError::rule(
            crate::error::VALIDATION_UNKNOWN_OPTIONAL,
            format!(
                "the optional check '{}' is outside the registry",
                check.as_str()
            ),
        ))
    }
}
