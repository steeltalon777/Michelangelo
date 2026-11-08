//! The evaluation orchestration: profile -> context -> check dispatch ->
//! assembled immutable report. This is the `validation.evaluate` port of the
//! contract gate §7.5 (module-local implementation, not shared Core).

use std::collections::BTreeMap;

use crate::check::{applicability, Applicability, CheckId, CheckRecord, CheckStatus};
use crate::checks::{appearance, backend, geometry, integrity, rig};
use crate::context::ValidationContext;
use crate::error::ValidationError;
use crate::profile::{ProfileKind, ValidationProfile};
use crate::report::{BundleRef, ProfileRef, ValidationReport};

/// The request for one evaluation. `created_at` is supplied by the caller so
/// that tests and integrators control the timestamp (reports are immutable,
/// so the caller owns the instant).
pub struct EvaluationRequest<'a> {
    /// The reserved report id (reservation happens before evaluation).
    pub report_id: String,
    /// RFC 3339 UTC instant of the report.
    pub created_at: String,
    /// The workspace root (disposable validation workspace).
    pub workspace_root: &'a std::path::Path,
    /// The workspace-relative manifest path of the measured bundle.
    pub manifest_path: String,
    /// The pinned profile (already validated).
    pub profile: ValidationProfile,
    /// The export roundtrip verifying backend (None = honest unavailable).
    pub roundtrip_backend: Option<&'a dyn backend::ExportRoundtripBackend>,
    /// The clip playback verifying backend (None = honest unavailable).
    pub playback_backend: Option<&'a dyn backend::ClipPlaybackBackend>,
}

/// The evaluator versions recorded into reports.
pub fn default_evaluator_versions() -> BTreeMap<String, String> {
    let mut versions = BTreeMap::new();
    versions.insert(
        "michelangelo_asset_validation".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    versions.insert(
        "check_matrix".to_string(),
        "glm1-contract-gate-draft-0.1".to_string(),
    );
    versions
}

/// Evaluate the bundle under the pinned profile and assemble the report.
///
/// Guarantees (contract gate §7.5): every required check gets exactly one
/// result; unavailable checks stay unavailable; `not_required` only for
/// optional checks with an applicability reference; the verdict folds as
/// fail > unavailable > pass; the report records the digests of the bytes
/// actually read. Operational errors (unreadable workspace, broken pinned
/// reference) abort with `Err` instead of fabricating a result.
pub fn evaluate(request: EvaluationRequest<'_>) -> Result<(ValidationReport, String), ValidationError> {
    let profile = &request.profile;
    // The kind gate runs before profile validation: a reconstruction profile
    // is a valid document that this module must refuse (it belongs to the
    // IT-8D evaluator), not a schema conflict.
    ValidationContext::check_kind_supported(profile)?;
    profile.validate()?;

    let mut context = ValidationContext::open(request.workspace_root, &request.manifest_path)?;

    // Core integrity checks run first and populate the input digest log the
    // other checks reuse (every artifact byte is read once, hashed once).
    let mut records: Vec<CheckRecord> = Vec::new();

    let path_record = integrity::check_path_containment(&mut context)?;
    records.push(path_record);
    let payload_record = integrity::check_payload_integrity(&mut context, profile)?;
    records.push(payload_record);
    let digest_record = integrity::check_asset_digest_full(&mut context, profile)?;
    records.push(digest_record);
    let closure_record = integrity::check_dependency_closure(&mut context, profile)?;
    records.push(closure_record);

    // Required checks beyond the core four.
    for check in &profile.required_checks {
        if matches!(
            check,
            CheckId::PathContainment
                | CheckId::PayloadIntegrity
                | CheckId::AssetDigestFull
                | CheckId::DependencyClosure
        ) {
            continue;
        }
        let record = dispatch_required(&mut context, profile, *check, &request)?;
        records.push(record);
    }

    // Optional checks: only when the bundle actually carries the relevant
    // content (probe); otherwise an explicit not_required with an
    // applicability reference (F16-compliant).
    for check in &profile.optional_checks {
        if profile.required_checks.contains(check) {
            continue;
        }
        let record = match dispatch_optional_probe(&mut context, profile, *check, &request)? {
            OptionalOutcome::Applicable(record) => record,
            OptionalOutcome::NotApplicable(reason) => CheckRecord {
                check_id: *check,
                result: CheckStatus::NotRequired,
                value: None,
                detail: None,
                applicability_ref: Some(reason),
                evidence: Vec::new(),
            },
        };
        records.push(record);
    }

    let input_digests = context.input_digests().clone();
    let identity = context.identity();

    // Coverage guard: every required check must have a record (F8 behavioral
    // analog). The dispatch always yields a record, so this is a defensive
    // invariant, but it keeps assembly honest under future edits.
    let missing: Vec<&CheckId> = profile
        .required_checks
        .iter()
        .filter(|required| !records.iter().any(|record| &record.check_id == *required))
        .collect();
    if !missing.is_empty() {
        let names: Vec<String> = missing.iter().map(|check| check.as_str().to_string()).collect();
        return Err(ValidationError::rule(
            crate::error::MISSING_REQUIRED_CHECK,
            format!("the required checks {:?} have no results", names),
        ));
    }

    let report = ValidationReport::assemble(
        request.report_id.clone(),
        request.created_at.clone(),
        BundleRef {
            kind: identity.kind,
            id: identity.id.clone(),
            sha256: identity.manifest_sha256.clone(),
        },
        ProfileRef {
            profile_id: profile.profile_id.clone(),
            revision: profile.revision,
            profile_sha256: profile.digest()?,
        },
        input_digests,
        records,
        default_evaluator_versions(),
    )?;
    let digest = report.digest()?;
    Ok((report, digest))
}

/// The matrix guard: which checks may be recorded at all for the kind. Only
/// the NotApplicable class is blocked; Required / RequiredWithCondition /
/// Optional / Human are all legitimate.
fn optional_allowed(profile: &ValidationProfile, check: CheckId) -> bool {
    !matches!(applicability(profile.kind, check), Applicability::NotApplicable)
}

/// Run one required check (or the human-review stub).
fn dispatch_required(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
    check: CheckId,
    request: &EvaluationRequest<'_>,
) -> Result<CheckRecord, ValidationError> {
    if !optional_allowed(profile, check) && check != CheckId::StyleReview {
        // The profile validation already refuses not-applicable required
        // checks; a defensive refusal keeps the invariant explicit.
        return Ok(CheckRecord {
            check_id: check,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(format!(
                "APPLICABILITY_CONFLICT: the check '{}' is not applicable to the profile kind '{}'",
                check.as_str(),
                profile.kind.as_str()
            )),
            applicability_ref: None,
            evidence: Vec::new(),
        });
    }
    dispatch_run(context, profile, check, request)
}

/// Run one optional check (or the human-review stub).
fn dispatch_optional_probe(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
    check: CheckId,
    request: &EvaluationRequest<'_>,
) -> Result<OptionalOutcome, ValidationError> {
    // Human review is never automated (ADR §4.1): an automated report marks
    // it unavailable so the verdict waits for the author procedure.
    if check == CheckId::StyleReview {
        return Ok(OptionalOutcome::Applicable(CheckRecord {
            check_id: check,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(
                "human author review required; an automated score cannot substitute the \
                 review (ADR 0005 §4.1)"
                    .to_string(),
            ),
            applicability_ref: None,
            evidence: Vec::new(),
        }));
    }
    if !optional_allowed(profile, check) {
        // Not applicable for this kind (e.g. rig_bindings for family_static):
        // the matrix exempts the check from the approved постановка.
        let reason = format!(
            "profile_applicability:{}:{}",
            profile.kind.as_str(),
            check.as_str()
        );
        return Ok(OptionalOutcome::NotApplicable(reason));
    }
    // Content probes: an optional check over content the bundle does not
    // carry is explicitly not_required (with the applicability reference).
    let manifest = context.manifest();
    let has_mtl_dep = manifest.dependencies.iter().any(|d| d.path.ends_with(".mtl"));
    let has_obj = manifest
        .artifacts
        .iter()
        .any(|a| a.content_kind == "geometry" && a.path.ends_with(".obj"));
    let probe_exempt = |condition: bool, reason: String| {
        if condition {
            None
        } else {
            Some(format!("bundle_scope:{reason}"))
        }
    };
    let exempt = match check {
        CheckId::MaterialPresence => probe_exempt(
            manifest.artifacts.iter().any(|a| a.content_kind == "material") || has_obj || has_mtl_dep,
            "the bundle declares no material content".to_string(),
        ),
        CheckId::TextureIntegrity => probe_exempt(
            manifest
                .artifacts
                .iter()
                .any(|a| a.content_kind == "texture")
                || has_mtl_dep,
            "the bundle declares no texture content".to_string(),
        ),
        CheckId::UvPreservation => probe_exempt(
            has_obj || profile.reference_digest(integrity::reference_keys::EXPECTED_UV_COUNTS).is_some(),
            "the bundle carries no OBJ geometry with UV data".to_string(),
        ),
        CheckId::FormatLossDeclared => probe_exempt(
            !manifest.conversions.is_empty(),
            "the bundle declares no format conversions".to_string(),
        ),
        CheckId::ConnectorCompatibility => probe_exempt(
            profile
                .reference_digest(integrity::reference_keys::CONNECTOR_PROFILE)
                .is_some(),
            "the profile pins no connector profile".to_string(),
        ),
        CheckId::MinimumClearance => probe_exempt(
            profile
                .threshold(CheckId::MinimumClearance, "min_clearance_m")
                .is_some(),
            "the profile declares no min_clearance_m threshold".to_string(),
        ),
        CheckId::MaximumDimension => probe_exempt(
            profile
                .thresholds
                .iter()
                .any(|t| t.check_id == CheckId::MaximumDimension),
            "the profile declares no max_extent thresholds".to_string(),
        ),
        CheckId::AnimationClips => probe_exempt(
            !profile.references.animation_clips.is_empty(),
            "the profile pins no animation clips".to_string(),
        ),
        CheckId::ExportRoundtrip => probe_exempt(
            manifest.outputs.export_glb,
            "the bundle declares no export_glb output".to_string(),
        ),
        CheckId::SilhouetteConformance => probe_exempt(
            profile
                .reference_digest(integrity::reference_keys::SILHOUETTE_REFERENCE)
                .is_some(),
            "the profile pins no silhouette reference; an intentionally changed silhouette \
             is not compared with the master (ADR §8.1)"
                .to_string(),
        ),
        CheckId::RigBindings => probe_exempt(
            manifest
                .artifacts
                .iter()
                .any(|a| a.content_kind == "rig_definition"),
            "the bundle declares no rig_definition artifact".to_string(),
        ),
        CheckId::ProtectedRoles => probe_exempt(
            manifest
                .artifacts
                .iter()
                .any(|a| a.content_kind == "scene_layout")
                && profile
                    .reference_digest(integrity::reference_keys::PROTECTED_SCENE_LAYOUT)
                    .is_some(),
            "the bundle declares no scene_layout with protected components or the profile \
             pins no protected expectations"
                .to_string(),
        ),
        _ => None,
    };
    match exempt {
        Some(reason) => Ok(OptionalOutcome::NotApplicable(reason)),
        None => {
            let record = dispatch_run(context, profile, check, request)?;
            Ok(OptionalOutcome::Applicable(record))
        }
    }
}

enum OptionalOutcome {
    Applicable(CheckRecord),
    NotApplicable(String),
}

/// The actual runner dispatch.
fn dispatch_run(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
    check: CheckId,
    request: &EvaluationRequest<'_>,
) -> Result<CheckRecord, ValidationError> {
    match check {
        CheckId::PathContainment => integrity::check_path_containment(context),
        CheckId::PayloadIntegrity => integrity::check_payload_integrity(context, profile),
        CheckId::AssetDigestFull => integrity::check_asset_digest_full(context, profile),
        CheckId::DependencyClosure => integrity::check_dependency_closure(context, profile),
        CheckId::BudgetGeometry => geometry::check_budget_geometry(context, profile),
        CheckId::ProtectedRoles => geometry::check_protected_roles(context, profile),
        CheckId::ConnectorCompatibility => geometry::check_connector_compatibility(context, profile),
        CheckId::MinimumClearance => geometry::check_minimum_clearance(context, profile),
        CheckId::MaximumDimension => geometry::check_maximum_dimension(context, profile),
        CheckId::MaterialPresence => appearance::check_material_presence(context),
        CheckId::TextureIntegrity => appearance::check_texture_integrity(context),
        CheckId::UvPreservation => appearance::check_uv_preservation(context, profile),
        CheckId::FormatLossDeclared => appearance::check_format_loss_declared(context),
        CheckId::RigBindings => rig::check_rig_bindings(context, profile),
        CheckId::AnimationClips => {
            let default_backend = backend::NoBackend;
            let backend_ref = request.playback_backend.unwrap_or(&default_backend);
            backend::check_animation_clips(context, profile, backend_ref)
        }
        CheckId::ExportRoundtrip => {
            let default_backend = backend::NoBackend;
            let backend_ref = request.roundtrip_backend.unwrap_or(&default_backend);
            backend::check_export_roundtrip(context, backend_ref)
        }
        CheckId::SilhouetteConformance => Ok(CheckRecord {
            check_id: check,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(
                "no silhouette evaluator exists in this module; calibrated-reference \
                 comparison belongs to the reconstruction evaluator (IT-8D) and is not \
                 re-implemented here"
                    .to_string(),
            ),
            applicability_ref: None,
            evidence: Vec::new(),
        }),
        CheckId::StyleReview => Ok(CheckRecord {
            check_id: check,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(
                "human author review required; the automated report never substitutes the \
                 review and never creates author approval"
                    .to_string(),
            ),
            applicability_ref: None,
            evidence: Vec::new(),
        }),
    }
}

/// Compile-time cross-check used by tests: the evaluator versions record the
/// draft matrix generation the implementation follows.
#[doc(hidden)]
pub fn matrix_generation() -> &'static str {
    "glm1-contract-gate-draft-0.1"
}

/// Silence the unused import for ProfileKind when only re-exported.
#[allow(unused_imports)]
use ProfileKind as _ProfileKindUnused;
