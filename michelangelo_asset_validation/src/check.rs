//! The closed check_id registry (contract gate §6 / OP-4), the applicability
//! matrix (VALIDATION_MATRIX §1) and the NEW result model.
//!
//! Boundary note (OP-7): `CheckStatus::NotRequired` exists ONLY in this new
//! report model (`michelangelo.validation-report/draft-0.1`). It must never
//! be back-ported into the existing IT-8 evaluation enum
//! (`michelangelo.evaluation-report/0`, verdict `pass/fail/unavailable`) and
//! can never be converted into `pass`. The `Verdict` type of this module
//! intentionally has no `not_required` variant.

use serde::{Deserialize, Serialize};

use crate::error::ValidationError;

/// The closed registry of check identifiers, v0.1 (contract gate §6).
/// Extension requires a new schema revision, never an unknown id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckId {
    #[serde(rename = "payload_integrity")]
    PayloadIntegrity,
    #[serde(rename = "dependency_closure")]
    DependencyClosure,
    #[serde(rename = "path_containment")]
    PathContainment,
    #[serde(rename = "asset_digest_full")]
    AssetDigestFull,
    #[serde(rename = "budget_geometry")]
    BudgetGeometry,
    #[serde(rename = "protected_roles")]
    ProtectedRoles,
    #[serde(rename = "connector_compatibility")]
    ConnectorCompatibility,
    #[serde(rename = "minimum_clearance")]
    MinimumClearance,
    #[serde(rename = "maximum_dimension")]
    MaximumDimension,
    #[serde(rename = "material_presence")]
    MaterialPresence,
    #[serde(rename = "texture_integrity")]
    TextureIntegrity,
    #[serde(rename = "export_roundtrip")]
    ExportRoundtrip,
    #[serde(rename = "uv_preservation")]
    UvPreservation,
    #[serde(rename = "rig_bindings")]
    RigBindings,
    #[serde(rename = "animation_clips")]
    AnimationClips,
    #[serde(rename = "silhouette_conformance")]
    SilhouetteConformance,
    #[serde(rename = "style_review")]
    StyleReview,
    #[serde(rename = "format_loss_declared")]
    FormatLossDeclared,
}

impl CheckId {
    /// All registry members in canonical order.
    pub const ALL: [CheckId; 18] = [
        CheckId::PayloadIntegrity,
        CheckId::DependencyClosure,
        CheckId::PathContainment,
        CheckId::AssetDigestFull,
        CheckId::BudgetGeometry,
        CheckId::ProtectedRoles,
        CheckId::ConnectorCompatibility,
        CheckId::MinimumClearance,
        CheckId::MaximumDimension,
        CheckId::MaterialPresence,
        CheckId::TextureIntegrity,
        CheckId::ExportRoundtrip,
        CheckId::UvPreservation,
        CheckId::RigBindings,
        CheckId::AnimationClips,
        CheckId::SilhouetteConformance,
        CheckId::StyleReview,
        CheckId::FormatLossDeclared,
    ];

    /// The wire name of the check.
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckId::PayloadIntegrity => "payload_integrity",
            CheckId::DependencyClosure => "dependency_closure",
            CheckId::PathContainment => "path_containment",
            CheckId::AssetDigestFull => "asset_digest_full",
            CheckId::BudgetGeometry => "budget_geometry",
            CheckId::ProtectedRoles => "protected_roles",
            CheckId::ConnectorCompatibility => "connector_compatibility",
            CheckId::MinimumClearance => "minimum_clearance",
            CheckId::MaximumDimension => "maximum_dimension",
            CheckId::MaterialPresence => "material_presence",
            CheckId::TextureIntegrity => "texture_integrity",
            CheckId::ExportRoundtrip => "export_roundtrip",
            CheckId::UvPreservation => "uv_preservation",
            CheckId::RigBindings => "rig_bindings",
            CheckId::AnimationClips => "animation_clips",
            CheckId::SilhouetteConformance => "silhouette_conformance",
            CheckId::StyleReview => "style_review",
            CheckId::FormatLossDeclared => "format_loss_declared",
        }
    }

    /// Parse from the wire name; an unknown name is rejected (never accepted).
    pub fn parse(name: &str) -> Result<CheckId, ValidationError> {
        CheckId::ALL
            .iter()
            .copied()
            .find(|check| check.as_str() == name)
            .ok_or_else(|| {
                ValidationError::rule(
                    crate::error::VALIDATION_UNKNOWN_REQUIRED,
                    format!("the check id '{name}' is outside the closed registry v0.1"),
                )
            })
    }
}

/// The result status of one check in the NEW report model.
///
/// `NotRequired` is legal only for optional checks and only with a
/// non-empty `applicability_ref` (F16). `Unavailable` is the honest outcome
/// for a check whose ground truth or verifying backend is missing; it is
/// never folded into `pass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "not_required")]
    NotRequired,
}

/// One recorded check of a validation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckRecord {
    pub check_id: CheckId,
    pub result: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(
        rename = "applicability_ref",
        skip_serializing_if = "Option::is_none"
    )]
    pub applicability_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<DocRef>,
}

/// The aggregate verdict of a report. The fold is the IT-8 fold:
/// any `fail` -> fail; else any `unavailable` -> unavailable; else pass.
/// There is deliberately no `not_required` verdict (contract gate §5, OP-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "unavailable")]
    Unavailable,
}

impl Verdict {
    /// Fold the check records into a verdict, refusing inconsistent input.
    pub fn fold(records: &[CheckRecord]) -> Result<Verdict, ValidationError> {
        let mut verdict = Verdict::Pass;
        for record in records {
            match record.result {
                CheckStatus::Fail => return Ok(Verdict::Fail),
                CheckStatus::Unavailable => verdict = Verdict::Unavailable,
                CheckStatus::Pass | CheckStatus::NotRequired => {}
            }
        }
        Ok(verdict)
    }

    /// The wire name of the verdict.
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::Unavailable => "unavailable",
        }
    }
}

/// Hash-pinned document reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocRef {
    pub path: String,
    pub sha256: String,
}

/// The applicability class of a check for a profile kind (VALIDATION_MATRIX §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    /// Required unconditionally.
    Required,
    /// Required only when the profile carries the named condition token.
    RequiredWithCondition(&'static str),
    /// Allowed as optional (result may be measured or not_required).
    Optional,
    /// Human/author procedure; an automated report records `unavailable`.
    Human,
    /// Not applicable for the kind; a required entry is a profile conflict.
    NotApplicable,
}

use crate::profile::ProfileKind;

/// The applicability matrix (draft of VALIDATION_MATRIX §1, simplified to the
/// classes above; deviations are documented in INTEGRATION_NOTES.md).
pub fn applicability(kind: ProfileKind, check: CheckId) -> Applicability {
    use Applicability::*;
    use CheckId::*;
    match kind {
        // Reconstruction stays with the existing IT-8D evaluator; the
        // module refuses the whole kind before any check runs.
        ProfileKind::Reconstruction => NotApplicable,
        ProfileKind::PassthroughPreservation => match check {
            PayloadIntegrity | DependencyClosure | PathContainment | AssetDigestFull
            | BudgetGeometry | FormatLossDeclared => Required,
            ProtectedRoles => Optional,
            MaterialPresence => RequiredWithCondition("materials_preserved"),
            TextureIntegrity => RequiredWithCondition("textures_preserved"),
            UvPreservation => RequiredWithCondition("uv_preserved"),
            ExportRoundtrip => RequiredWithCondition("export:glb"),
            ConnectorCompatibility
            | MinimumClearance
            | MaximumDimension
            | RigBindings
            | AnimationClips
            | SilhouetteConformance
            | StyleReview => NotApplicable,
        },
        ProfileKind::FamilyStatic => match check {
            PayloadIntegrity | DependencyClosure | PathContainment | AssetDigestFull
            | BudgetGeometry | ProtectedRoles | MaterialPresence => Required,
            ConnectorCompatibility => RequiredWithCondition("constraints:connector"),
            MinimumClearance => RequiredWithCondition("constraints:clearance"),
            MaximumDimension => RequiredWithCondition("constraints:dimension"),
            TextureIntegrity => RequiredWithCondition("textures_declared"),
            UvPreservation => RequiredWithCondition("uv_preserved"),
            ExportRoundtrip => RequiredWithCondition("export:glb"),
            RigBindings | AnimationClips | SilhouetteConformance => NotApplicable,
            StyleReview => Human,
            FormatLossDeclared => Optional,
        },
        ProfileKind::FamilyRigCompatible => match check {
            PayloadIntegrity | DependencyClosure | PathContainment | AssetDigestFull
            | BudgetGeometry | ProtectedRoles | MaterialPresence | UvPreservation
            | RigBindings => Required,
            AnimationClips => RequiredWithCondition("clips_pinned"),
            ConnectorCompatibility => RequiredWithCondition("constraints:connector"),
            MinimumClearance => RequiredWithCondition("constraints:clearance"),
            MaximumDimension => RequiredWithCondition("constraints:dimension"),
            TextureIntegrity => RequiredWithCondition("textures_declared"),
            ExportRoundtrip => RequiredWithCondition("export:glb"),
            SilhouetteConformance => Optional,
            StyleReview => Human,
            FormatLossDeclared => Optional,
        },
        ProfileKind::AnatomyChange => match check {
            PayloadIntegrity | DependencyClosure | PathContainment | AssetDigestFull
            | BudgetGeometry | ProtectedRoles | MaterialPresence | RigBindings
            | AnimationClips => Required,
            ConnectorCompatibility | MinimumClearance | MaximumDimension | TextureIntegrity
            | UvPreservation | FormatLossDeclared => Optional,
            ExportRoundtrip => RequiredWithCondition("export:glb"),
            SilhouetteConformance => NotApplicable,
            StyleReview => Human,
        },
    }
}
