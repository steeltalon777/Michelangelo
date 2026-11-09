//! Adversarial behavioral tests (GLM-1 NEGATIVE_TEST_MATRIX, GLM-6 lane):
//! F1 (geometry+manifest substitution), F2 (input changed after evaluation),
//! F5 (replay / sibling evidence reuse), F8 (unknown required check,
//! behavioral), F9 (parent transform / shared material hitting a protected
//! role), F13 (path escape), F14 (acceptance without registered report),
//! F16 (not_required / unavailable -> pass refusals), F19 (stale approval
//! basis), plus reservation and no-clobber guarantees.

use michelangelo_asset_validation::acceptance::{
    AcceptanceRecord, CanonicalAcceptSeam, LocalReplayIndex, RegistryBackedSeam,
};
use michelangelo_asset_validation::canonical::sha256_hex;
use michelangelo_asset_validation::check::{CheckId, CheckRecord, CheckStatus, Verdict};
use michelangelo_asset_validation::profile::{Comparison, ProfileKind, ProfileReferences};
use michelangelo_asset_validation::registry::{
    load_registry, report_file_path, store_pinned_profile, verify_registered_report,
    write_and_register, write_registry, RegistryEntry, Reservation,
};
use michelangelo_asset_validation::report::{BundleRef, ProfileRef, ValidationReport};
use michelangelo_asset_validation::testing::*;

/// Rewrite one artifact's bytes AND repair the manifest consistently (the
/// "competent attacker" of F1: declared digests are recomputed).
fn rewrite_artifact_and_manifest(ws: &FinalizedWorkspace, artifact_id: &str, bytes: &[u8]) {
    let path = ws
        .manifest
        .artifacts
        .iter()
        .find(|a| a.artifact_id == artifact_id)
        .expect("artifact")
        .path
        .clone();
    std::fs::write(ws.root().join(&path), bytes).expect("artifact rewrite");
    let mut manifest = ws.manifest.clone();
    for entry in manifest.artifacts.iter_mut() {
        if entry.artifact_id == artifact_id {
            entry.sha256 = sha256_hex(bytes);
            entry.size_bytes = bytes.len() as u64;
        }
    }
    let entries: Vec<(String, String, String, String)> = manifest
        .artifacts
        .iter()
        .map(|a| {
            (
                a.artifact_id.clone(),
                a.path.clone(),
                a.content_kind.clone(),
                a.sha256.clone(),
            )
        })
        .collect();
    manifest.asset_digest = Some(michelangelo_asset_validation::canonical::asset_digest(
        &entries,
    ));
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
    std::fs::write(ws.root().join("manifest.json"), manifest_bytes).expect("manifest rewrite");
}

/// Rewrite the scene_layout artifact bytes (and repair the manifest).
fn rewrite_layout(
    ws: &FinalizedWorkspace,
    layout: &michelangelo_asset_validation::checks::parsing::LayoutDoc,
) {
    let bytes = serde_json::to_vec_pretty(layout).expect("layout bytes");
    rewrite_artifact_and_manifest(ws, "layout", &bytes);
}

/// Rewrite one dependency's bytes AND repair the manifest (digests and the
/// dependency aggregate).
fn rewrite_dependency_and_manifest(ws: &FinalizedWorkspace, dep_id: &str, bytes: &[u8]) {
    let path = ws
        .manifest
        .dependencies
        .iter()
        .find(|d| d.dep_id == dep_id)
        .expect("dependency")
        .path
        .clone();
    std::fs::write(ws.root().join(&path), bytes).expect("dependency rewrite");
    let mut manifest = ws.manifest.clone();
    for dep in manifest.dependencies.iter_mut() {
        if dep.dep_id == dep_id {
            dep.sha256 = sha256_hex(bytes);
        }
    }
    let entries: Vec<(String, String, String)> = manifest
        .dependencies
        .iter()
        .map(|d| (d.dep_id.clone(), d.revision.to_string(), d.sha256.clone()))
        .collect();
    manifest.dependency_digest = Some(michelangelo_asset_validation::canonical::dependency_digest(
        &entries,
    ));
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
    std::fs::write(ws.root().join("manifest.json"), manifest_bytes).expect("manifest rewrite");
}

/// Register a hand-assembled report by writing the report file and the
/// registry index directly. This bypasses `write_and_register` on purpose:
/// the independent verification and seam guards must refuse the report too.
fn force_register(root: &std::path::Path, report: &ValidationReport) -> String {
    let bytes = report.to_json_bytes().expect("report bytes");
    let digest = sha256_hex(&bytes);
    let path = report_file_path(root, &report.report_id);
    std::fs::create_dir_all(path.parent().expect("report parent")).expect("report dir");
    std::fs::write(&path, &bytes).expect("report write");
    let mut registry = load_registry(root).expect("registry");
    registry.entries.push(RegistryEntry {
        report_id: report.report_id.clone(),
        report_path: format!("validation/reports/{}/report.json", report.report_id),
        sha256: digest.clone(),
        bundle_sha256: report.bundle_ref.sha256.clone(),
        profile_sha256: report.profile_ref.profile_sha256.clone(),
        verdict: report.verdict,
        created_at: report.created_at.clone(),
    });
    write_registry(root, &registry).expect("registry write");
    digest
}

fn record_of(report: &ValidationReport, check: CheckId) -> &CheckRecord {
    report
        .checks
        .iter()
        .find(|record| record.check_id == check)
        .unwrap_or_else(|| panic!("missing record for {}", check.as_str()))
}

/// The family_static column of the applicability matrix: the checks that are
/// unconditionally Required for the kind and therefore must appear in every
/// family_static profile (VALIDATION_MATRIX §1).
fn family_static_matrix_required() -> Vec<CheckId> {
    vec![
        CheckId::PayloadIntegrity,
        CheckId::DependencyClosure,
        CheckId::PathContainment,
        CheckId::AssetDigestFull,
        CheckId::BudgetGeometry,
        CheckId::ProtectedRoles,
        CheckId::MaterialPresence,
    ]
}

/// The family_rig_compatible column of the applicability matrix, likewise.
/// `animation_clips` is conditional on the `clips_pinned` token and is not
/// part of the unconditional set.
fn family_rig_matrix_required() -> Vec<CheckId> {
    vec![
        CheckId::PayloadIntegrity,
        CheckId::DependencyClosure,
        CheckId::PathContainment,
        CheckId::AssetDigestFull,
        CheckId::BudgetGeometry,
        CheckId::ProtectedRoles,
        CheckId::MaterialPresence,
        CheckId::UvPreservation,
        CheckId::RigBindings,
    ]
}

/// F1: the attacker replaces the geometry bytes and repairs the manifest
/// (declared sha256 + asset_digest recomputed). payload_integrity passes,
/// but the pinned pre-generation expected_asset_digest refuses: a repaired
/// manifest is not legitimized by a new evaluation.
#[test]
fn f1_geometry_and_manifest_substitution_is_refused() {
    let temp = std::env::temp_dir().join("gav-adv-f1");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-f1");

    // Baseline: the honest bundle passes.
    let (baseline, _) = evaluate_fixture(&ws, &profile, "vr-f1a").expect("baseline");
    assert_eq!(baseline.verdict, Verdict::Pass);

    // Substitution: new geometry bytes, manifest fully repaired.
    let substituted = block_obj(9.0, 2);
    rewrite_artifact_and_manifest(&ws, "body-mesh", substituted.as_bytes());

    let (tampered, _) = evaluate_fixture(&ws, &profile, "vr-f1b").expect("tampered evaluation");
    let payload = record_of(&tampered, CheckId::PayloadIntegrity);
    assert_eq!(
        payload.result,
        CheckStatus::Pass,
        "the repaired manifest passes payload"
    );
    let digest_check = record_of(&tampered, CheckId::AssetDigestFull);
    assert_eq!(digest_check.result, CheckStatus::Fail);
    assert!(
        digest_check
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("SNAPSHOT_MISMATCH"),
        "detail: {:?}",
        digest_check.detail
    );
    assert_eq!(tampered.verdict, Verdict::Fail);
}

/// F2: the texture bytes change AFTER the evaluation; the registered report
/// re-reads the inputs and refuses with VALIDATION_INPUT_CHANGED.
#[test]
fn f2_input_changed_after_validation_is_detected() {
    let temp = std::env::temp_dir().join("gav-adv-f2");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-f2");

    let reservation =
        Reservation::reserve(&temp, "vr-f2", &ws.manifest_sha256, TIMESTAMP).expect("reservation");
    let (report, _) = evaluate_fixture(&ws, &profile, "vr-f2").expect("evaluation");
    write_and_register(&temp, &reservation, &report, &profile).expect("registration");
    verify_registered_report(&temp, "vr-f2", Some(&profile)).expect("verify before tamper");

    // PNG swap after validation (bytes only; the manifest is untouched).
    std::fs::write(ws.root().join("textures/wall.png"), b"tampered-png-bytes").expect("swap");

    let error = verify_registered_report(&temp, "vr-f2", Some(&profile)).expect_err("F2");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_INPUT_CHANGED
    );
    let message = error.message().to_string();
    assert!(message.contains("textures/wall.png"), "message: {message}");
}

/// F9 (parent transform): an allowed role change moves the parent; the
/// protected child mesh bytes stay identical, but its EFFECTIVE world
/// transform no longer matches the pinned expectation -> protected_roles
/// fails.
#[test]
fn f9_parent_transform_moves_protected_child() {
    use michelangelo_asset_validation::checks::parsing::LayoutDoc;

    let temp = std::env::temp_dir().join("gav-adv-f9");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-f9");

    let (baseline, _) = evaluate_fixture(&ws, &profile, "vr-f9a").expect("baseline");
    assert_eq!(
        record_of(&baseline, CheckId::ProtectedRoles).result,
        CheckStatus::Pass
    );

    // The attacker changes only the PARENT transform (an allowed role) and
    // repairs the manifest/layout digests. The protected child mesh bytes
    // are untouched.
    let layout_bytes = std::fs::read(ws.root().join("metadata/scene_layout.json")).expect("layout");
    let mut layout: LayoutDoc = serde_json::from_slice(&layout_bytes).expect("layout parse");
    for component in layout.components.iter_mut() {
        if component.component_id == "body" {
            component.transform = [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 3.0, // the allowed wall moves up by 3
                0.0, 0.0, 0.0, 1.0,
            ];
        }
    }
    rewrite_layout(&ws, &layout);

    let (tampered, _) = evaluate_fixture(&ws, &profile, "vr-f9b").expect("tampered evaluation");
    let protected = record_of(&tampered, CheckId::ProtectedRoles);
    assert_eq!(protected.result, CheckStatus::Fail);
    let detail = protected.detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("effective world transform"),
        "the refusal must name the effective transform: {detail}"
    );
    assert_eq!(tampered.verdict, Verdict::Fail);
}

/// F9 (shared material): the shared MTL dependency changes after the lock;
/// the pinned dependency aggregate refuses even though the protected mesh
/// bytes are unchanged.
#[test]
fn f9_shared_material_change_after_lock_is_refused() {
    let temp = std::env::temp_dir().join("gav-adv-f9-mtl");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-f9m");

    let (baseline, _) = evaluate_fixture(&ws, &profile, "vr-f9c").expect("baseline");
    assert_eq!(baseline.verdict, Verdict::Pass);

    // The shared material is repainted (and the manifest repaired).
    let repainted = wall_mtl("textures/wall.png").replace("0.8 0.8 0.8", "1.0 0.2 0.2");
    rewrite_dependency_and_manifest(&ws, "wall-mtl", repainted.as_bytes());

    let (tampered, _) = evaluate_fixture(&ws, &profile, "vr-f9d").expect("tampered evaluation");
    let closure = record_of(&tampered, CheckId::DependencyClosure);
    assert_eq!(closure.result, CheckStatus::Fail);
    let detail = closure.detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("DEPENDENCY_CHANGED") && detail.contains("expected_dependency_digest"),
        "detail: {detail}"
    );
    assert_eq!(tampered.verdict, Verdict::Fail);
}

/// F14 + F5 + F19: the acceptance seam refuses an unregistered report, a
/// replayed record, a foreign bundle binding, a stale profile basis, and a
/// non-pass verdict; the default seam never publishes.
#[test]
fn seam_refuses_missing_report_replay_and_stale_basis() {
    let temp = std::env::temp_dir().join("gav-adv-seam");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-seam");

    let reservation = Reservation::reserve(&temp, "vr-seam", &ws.manifest_sha256, TIMESTAMP)
        .expect("reservation");
    let (report, report_digest) = evaluate_fixture(&ws, &profile, "vr-seam").expect("evaluation");
    write_and_register(&temp, &reservation, &report, &profile).expect("registration");

    let record = |bundle_sha: &str, report_sha: &str, profile_sha: &str| AcceptanceRecord {
        schema: michelangelo_asset_validation::acceptance::ACCEPTANCE_SCHEMA.to_string(),
        record_id: "rec-seam-1".to_string(),
        created_at: TIMESTAMP.to_string(),
        decision: "accepted".to_string(),
        target: michelangelo_asset_validation::acceptance::AcceptanceTarget {
            kind: "candidate_asset_bundle".to_string(),
            candidate_id: "cand-house-seam".to_string(),
            bundle_sha256: bundle_sha.to_string(),
            asset_id: "asset-house".to_string(),
            expected_current_revision: None,
        },
        basis: michelangelo_asset_validation::acceptance::AcceptanceBasis {
            validation_report_sha256: report_sha.to_string(),
            report_registered: true,
            profile_sha256: profile_sha.to_string(),
            policy_refs: Vec::new(),
            reason: None,
        },
        actor: michelangelo_asset_validation::acceptance::Actor {
            name: "author".to_string(),
            role: "owner".to_string(),
        },
        scope_note: None,
    };

    let seam = RegistryBackedSeam { root: &temp };

    // F14: an acceptance citing an unknown report digest.
    let ghost = record(
        &ws.manifest_sha256,
        &"0".repeat(64),
        &profile.digest().expect("digest"),
    );
    let error = seam.check_prerequisites(&ghost).expect_err("F14");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_REPORT_NOT_REGISTERED
    );

    // A correct record passes all prerequisites.
    let good = record(
        &ws.manifest_sha256,
        &report_digest,
        &profile.digest().expect("digest"),
    );
    let facts = seam.check_prerequisites(&good).expect("prerequisites");
    assert!(
        facts.report_registered && facts.bundle_binding_matches && facts.profile_binding_matches
    );
    assert!(facts.verdict_sufficient);

    // Publication is refused: this module is not a publisher.
    let error = seam
        .submit_for_publication(&good)
        .expect_err("no publication");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::ACCEPT_SEAM_NOT_AVAILABLE
    );

    // F5: the same record id replayed for a different bundle digest.
    let mut index = LocalReplayIndex::load(&temp).expect("replay index");
    index.record(&temp, &good).expect("first binding");
    let foreign_bundle = "1".repeat(64);
    let replay = record(
        &foreign_bundle,
        &report_digest,
        &profile.digest().expect("digest"),
    );
    let error = index.record(&temp, &replay).expect_err("F5");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::ACCEPTANCE_REPLAY
    );

    // A sibling candidate binding: the report binds ONE bundle digest.
    let facts = seam
        .check_prerequisites(&replay)
        .expect("prerequisites computed");
    assert!(
        !facts.bundle_binding_matches,
        "the report must not bind a foreign bundle"
    );

    // F19: a changed profile (stricter threshold) invalidates the basis.
    let mut profile_v2 = profile.clone();
    profile_v2.revision = 2;
    profile_v2.thresholds = vec![threshold(
        CheckId::BudgetGeometry,
        "max_evaluated_triangles",
        Comparison::AtMost,
        1.0,
    )];
    let stale = record(
        &ws.manifest_sha256,
        &report_digest,
        &profile_v2.digest().expect("digest v2"),
    );
    let facts = seam
        .check_prerequisites(&stale)
        .expect("prerequisites computed");
    assert!(
        !facts.profile_binding_matches,
        "the changed profile must not pass the basis check"
    );
    let error =
        verify_registered_report(&temp, "vr-seam", Some(&profile_v2)).expect_err("F19 verify");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_PROFILE_CHANGED
    );

    // An acceptance of an unavailable verdict is refused (unavailable is
    // never a pass).
    let temp2 = std::env::temp_dir().join("gav-adv-seam-unavailable");
    let _ = std::fs::remove_dir_all(&temp2);
    let (ws2, profile2) = build_family_workspace(&temp2, "cand-house-seam2");
    let profile_roundtrip = declare_export_artifact(&ws2, &profile2);
    // declare_export_artifact rewrites the manifest (adds the glb artifact),
    // so the live bundle digest is the rewritten manifest's digest.
    let bundle2 = sha256_hex(&std::fs::read(ws2.root().join("manifest.json")).expect("manifest"));
    let reservation2 =
        Reservation::reserve(&temp2, "vr-seam2", &bundle2, TIMESTAMP).expect("reservation 2");
    let (report2, digest2) =
        evaluate_fixture(&ws2, &profile_roundtrip, "vr-seam2").expect("evaluation 2");
    assert_eq!(report2.verdict, Verdict::Unavailable);
    write_and_register(&temp2, &reservation2, &report2, &profile_roundtrip)
        .expect("registration 2");
    let seam2 = RegistryBackedSeam { root: &temp2 };
    let record_unavailable = record(
        &bundle2,
        &digest2,
        &profile_roundtrip.digest().expect("digest 3"),
    );
    let facts = seam2
        .check_prerequisites(&record_unavailable)
        .expect("prerequisites computed");
    assert!(
        !facts.verdict_sufficient,
        "an unavailable verdict must not authorize acceptance"
    );
}

/// F8 behavioral: a report that lacks one required check is refused both at
/// registration and by verification against the profile (forged registry
/// entry, bypassing the registration guard).
#[test]
fn f8_report_missing_required_check_is_refused() {
    let temp = std::env::temp_dir().join("gav-adv-f8");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-f8");

    // A structurally valid report assembled WITHOUT the budget check.
    let report = ValidationReport::assemble(
        "vr-f8".to_string(),
        TIMESTAMP.to_string(),
        BundleRef {
            kind: michelangelo_asset_validation::report::BundleKind::CandidateAssetBundle,
            id: ws.manifest.candidate_id.clone(),
            sha256: ws.manifest_sha256.clone(),
        },
        ProfileRef {
            profile_id: profile.profile_id.clone(),
            revision: profile.revision,
            profile_sha256: profile.digest().expect("profile digest"),
        },
        [(ws.manifest_path.clone(), ws.manifest_sha256.clone())]
            .into_iter()
            .collect(),
        vec![CheckRecord {
            check_id: CheckId::PayloadIntegrity,
            result: CheckStatus::Pass,
            value: None,
            detail: Some("hand-assembled incomplete report".to_string()),
            applicability_ref: None,
            evidence: Vec::new(),
        }],
        [("test", "0")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
    .expect("assemble");
    let reservation =
        Reservation::reserve(&temp, "vr-f8", &ws.manifest_sha256, TIMESTAMP).expect("reservation");
    let error = write_and_register(&temp, &reservation, &report, &profile)
        .expect_err("F8 registration guard");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::MISSING_REQUIRED_CHECK
    );

    // The verification guard is independent of registration: forge the entry.
    force_register(&temp, &report);
    let error = verify_registered_report(&temp, "vr-f8", Some(&profile)).expect_err("F8");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::MISSING_REQUIRED_CHECK
    );
}

/// F16: not_required without an applicability reference is refused; an
/// unavailable check never folds into a pass verdict.
#[test]
fn f16_not_required_and_unavailable_are_never_pass() {
    let temp = std::env::temp_dir().join("gav-adv-f16");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-f16");

    let bundle_ref = BundleRef {
        kind: michelangelo_asset_validation::report::BundleKind::CandidateAssetBundle,
        id: ws.manifest.candidate_id.clone(),
        sha256: ws.manifest_sha256.clone(),
    };
    let profile_ref = ProfileRef {
        profile_id: profile.profile_id.clone(),
        revision: profile.revision,
        profile_sha256: profile.digest().expect("profile digest"),
    };
    let inputs: std::collections::BTreeMap<String, String> =
        [(ws.manifest_path.clone(), ws.manifest_sha256.clone())]
            .into_iter()
            .collect();

    // not_required without applicability_ref.
    let error = ValidationReport::assemble(
        "vr-f16a".to_string(),
        TIMESTAMP.to_string(),
        bundle_ref.clone(),
        profile_ref.clone(),
        inputs.clone(),
        vec![CheckRecord {
            check_id: CheckId::FormatLossDeclared,
            result: CheckStatus::NotRequired,
            value: None,
            detail: None,
            applicability_ref: None,
            evidence: Vec::new(),
        }],
        [("test", "0")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
    .expect_err("F16 not_required");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::INVALID_NOT_REQUIRED
    );

    // A report whose stored verdict contradicts the fold (unavailable ->
    // pass tampering) is refused at parse time.
    let (report, _) = evaluate_fixture(&ws, &profile, "vr-f16b").expect("evaluation");
    let mut value = serde_json::to_value(&report).expect("value");
    value["verdict"] = serde_json::json!("pass");
    // Flip every unavailable check to pass as well: the fold must still
    // refuse inconsistent verdicts (fail > unavailable > pass).
    let bytes = serde_json::to_vec_pretty(&value).expect("bytes");
    let parsed = ValidationReport::from_json_bytes(&bytes);
    // The verdict must match the fold; with all checks pass it parses.
    if let Ok(parsed) = parsed {
        assert_eq!(parsed.verdict, Verdict::Pass);
        // But the folded verdict of the ORIGINAL report was unavailable:
        // proving that flipping unavailable->pass changes the verdict is the
        // tamper the next case refuses.
    }

    // The fold: any unavailable -> unavailable; any fail -> fail.
    let records = vec![
        CheckRecord {
            check_id: CheckId::PayloadIntegrity,
            result: CheckStatus::Pass,
            value: None,
            detail: None,
            applicability_ref: None,
            evidence: Vec::new(),
        },
        CheckRecord {
            check_id: CheckId::BudgetGeometry,
            result: CheckStatus::Unavailable,
            value: None,
            detail: None,
            applicability_ref: None,
            evidence: Vec::new(),
        },
    ];
    assert_eq!(Verdict::fold(&records).expect("fold"), Verdict::Unavailable);
    let records_fail = vec![CheckRecord {
        check_id: CheckId::PayloadIntegrity,
        result: CheckStatus::Fail,
        value: None,
        detail: None,
        applicability_ref: None,
        evidence: Vec::new(),
    }];
    assert_eq!(Verdict::fold(&records_fail).expect("fold"), Verdict::Fail);

    // A stored verdict that contradicts its fold is refused (F16 schema
    // behavioral analog).
    let mut tampered = serde_json::to_value(&report).expect("value");
    tampered["checks"] = serde_json::json!([
        {"check_id": "payload_integrity", "result": "pass"},
        {"check_id": "budget_geometry", "result": "fail"}
    ]);
    tampered["verdict"] = serde_json::json!("pass");
    let bytes = serde_json::to_vec_pretty(&tampered).expect("bytes");
    let error = ValidationReport::from_json_bytes(&bytes).expect_err("fold violation");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::INVALID_VERDICT_FOLD
    );

    // not_required is not a verdict value at the type level: parsing a
    // report with verdict "not_required" fails.
    let mut bogus = serde_json::to_value(&report).expect("value");
    bogus["verdict"] = serde_json::json!("not_required");
    let bytes = serde_json::to_vec_pretty(&bogus).expect("bytes");
    let error = ValidationReport::from_json_bytes(&bytes).expect_err("not_required verdict");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::DOCUMENT_INVALID
    );
}

/// F13-adjacent: artifact paths leaving the workspace refuse at context
/// creation; the registry refuses a report id escaping its container.
#[test]
fn f13_path_escape_is_refused() {
    let temp = std::env::temp_dir().join("gav-adv-f13");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-escape");

    // Patch the manifest with an escaping artifact path.
    let mut manifest = ws.manifest.clone();
    manifest
        .artifacts
        .push(michelangelo_asset_validation::context::ArtifactEntry {
            artifact_id: "evil".to_string(),
            path: "../outside.obj".to_string(),
            sha256: "0".repeat(64),
            size_bytes: 1,
            content_kind: "geometry".to_string(),
            roles: Vec::new(),
            protected: false,
        });
    let bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
    std::fs::write(ws.root().join("manifest.json"), bytes).expect("manifest write");

    let error = evaluate_fixture(&ws, &profile, "vr-f13").expect_err("path escape");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::PATH_ESCAPE
    );
}

/// The no-clobber registry: a repeated report (same id, different bytes) is
/// never overwritten; registration requires a matching reservation.
#[test]
fn registry_refuses_overwrite_and_unreserved_registration() {
    let temp = std::env::temp_dir().join("gav-adv-noclobber");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-noclobber");

    let wrong_reservation = Reservation::reserve(&temp, "other-id", &ws.manifest_sha256, TIMESTAMP)
        .expect("reservation");
    let (report, _) = evaluate_fixture(&ws, &profile, "vr-nc").expect("evaluation");
    let error = write_and_register(&temp, &wrong_reservation, &report, &profile).expect_err("F-id");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::RESERVATION_REQUIRED
    );

    let reservation =
        Reservation::reserve(&temp, "vr-nc", &ws.manifest_sha256, TIMESTAMP).expect("reservation");
    write_and_register(&temp, &reservation, &report, &profile).expect("registration");

    // Same id, different bytes (a different profile digest).
    let mut profile_v2 = profile.clone();
    profile_v2.revision = 7;
    let (report_v2, _) = evaluate_fixture(&ws, &profile_v2, "vr-nc").expect("evaluation v2");
    let reservation_v2 = Reservation::reserve(&temp, "vr-nc", &ws.manifest_sha256, TIMESTAMP)
        .expect("reservation v2");
    let error = write_and_register(&temp, &reservation_v2, &report_v2, &profile_v2)
        .expect_err("no-clobber");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::NO_CLOBBER_VIOLATION
    );
    // The registry still holds exactly the original report.
    let registry = load_registry(&temp).expect("registry");
    assert_eq!(registry.entries.len(), 1);
    assert_eq!(registry.entries[0].report_id, "vr-nc");
}

/// M1: registration requires a LIVE on-disk reservation. An in-memory
/// `Reservation` value (public fields) and a reservation taken for a different
/// bundle never authorize a report.
#[test]
fn m1_registration_requires_a_live_on_disk_reservation() {
    let temp = std::env::temp_dir().join("gav-adv-m1");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-m1");
    let (report, _) = evaluate_fixture(&ws, &profile, "vr-m1").expect("evaluation");

    // A fabricated in-memory reservation with matching fields.
    let phantom = Reservation {
        report_id: "vr-m1".to_string(),
        bundle_sha256: ws.manifest_sha256.clone(),
        reserved_at: TIMESTAMP.to_string(),
    };
    let error = write_and_register(&temp, &phantom, &report, &profile).expect_err("M1 phantom");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::RESERVATION_REQUIRED
    );

    // A live reservation for a DIFFERENT bundle does not authorize the report.
    let wrong = Reservation::reserve(&temp, "vr-m1", &"0".repeat(64), TIMESTAMP)
        .expect("wrong reservation");
    let error = write_and_register(&temp, &wrong, &report, &profile).expect_err("M1 bundle");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::RESERVATION_REQUIRED
    );
    let registry = load_registry(&temp).expect("registry");
    assert!(registry.entries.is_empty(), "nothing may be registered");
}

/// M2: a conditional matrix check listed as optional while the profile
/// carries its condition token is a profile conflict, so the probe cannot
/// turn it into `not_required`. Without the token the optional entry is legal.
#[test]
fn m2_conditional_check_cannot_be_optional_when_condition_is_present() {
    let conflict = make_profile(
        "m2-v0",
        ProfileKind::FamilyStatic,
        1,
        family_static_matrix_required(),
        vec![CheckId::MinimumClearance],
        Vec::new(),
        vec!["constraints:clearance".to_string()],
        ProfileReferences::default(),
    );
    let error = conflict.validate().expect_err("M2 conditional conflict");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_APPLICABILITY_CONFLICT
    );

    // An unconditionally required matrix check cannot be optional either.
    // The required set deliberately omits protected_roles: the optional-side
    // guard must refuse the entry before reverse coverage does.
    let mut required_minus_protected = family_static_matrix_required();
    required_minus_protected.retain(|check| *check != CheckId::ProtectedRoles);
    let required_optional = make_profile(
        "m2-v1",
        ProfileKind::FamilyStatic,
        1,
        required_minus_protected,
        vec![CheckId::ProtectedRoles],
        Vec::new(),
        Vec::new(),
        ProfileReferences::default(),
    );
    let error = required_optional
        .validate()
        .expect_err("M2 required conflict");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_APPLICABILITY_CONFLICT
    );

    // Without the condition token the optional entry is legitimate.
    let mut relaxed = conflict;
    relaxed.applies_to.conditions.clear();
    relaxed.validate().expect("optional without the condition");
}

/// P2 (matrix coverage): a profile cannot silently omit a check the matrix
/// marks Required (or RequiredWithCondition whose token the profile carries).
/// Omission would leave the check unmeasured while the report still folds to
/// pass — the sibling of the closed not_required masking.
#[test]
fn p2_matrix_required_checks_cannot_be_omitted_from_the_profile() {
    use michelangelo_asset_validation::error::VALIDATION_APPLICABILITY_CONFLICT;

    let temp = std::env::temp_dir().join("gav-adv-p2");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, family_profile) = build_family_workspace(&temp, "cand-rig-p2");

    // The reviewer's counterexample: family_rig_compatible carrying
    // clips_pinned but requiring only payload_integrity. rig_bindings (matrix
    // Required) and animation_clips (triggered Required) are not measured.
    let mut omitted = family_profile.clone();
    omitted.kind = ProfileKind::FamilyRigCompatible;
    omitted.required_checks = vec![CheckId::PayloadIntegrity];
    omitted.optional_checks = Vec::new();
    omitted.applies_to.conditions = vec!["clips_pinned".to_string()];
    let error = omitted
        .validate()
        .expect_err("P2: matrix-required omission");
    assert_eq!(error.code(), VALIDATION_APPLICABILITY_CONFLICT);

    // The evaluation path must refuse it too: no report can be produced and
    // therefore no registration/seam verdict can be obtained.
    let error = evaluate_fixture(&ws, &omitted, "vr-p2a").expect_err("P2 evaluation");
    assert_eq!(error.code(), VALIDATION_APPLICABILITY_CONFLICT);

    // Triggered conditional omission: every unconditional matrix Required
    // check is present, but animation_clips is missing while clips_pinned is
    // carried.
    let mut missing_triggered = omitted.clone();
    missing_triggered.required_checks = family_rig_matrix_required();
    let error = missing_triggered
        .validate()
        .expect_err("P2: triggered conditional omission");
    assert_eq!(error.code(), VALIDATION_APPLICABILITY_CONFLICT);

    // Without the token the conditional check is NOT required: the same
    // unconditional set is a valid profile.
    let mut without_token = missing_triggered.clone();
    without_token.applies_to.conditions.clear();
    without_token
        .validate()
        .expect("a conditional check without its token is not required");

    // Positive: the complete family_rig_compatible profile with every matrix
    // Required check and the triggered conditional check is valid.
    let mut complete = missing_triggered;
    complete.required_checks.push(CheckId::AnimationClips);
    complete
        .validate()
        .expect("complete family_rig_compatible profile");
}

/// M3: ghost fields in the report, a check record or the profile are rejected
/// instead of being silently dropped.
#[test]
fn m3_ghost_fields_are_rejected() {
    let temp = std::env::temp_dir().join("gav-adv-m3");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-m3");
    let (report, _) = evaluate_fixture(&ws, &profile, "vr-m3").expect("evaluation");

    let mut value = serde_json::to_value(&report).expect("report value");
    value["ghost_field"] = serde_json::json!("smuggled");
    let bytes = serde_json::to_vec_pretty(&value).expect("bytes");
    let error = ValidationReport::from_json_bytes(&bytes).expect_err("report ghost field");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::DOCUMENT_INVALID
    );

    let mut value = serde_json::to_value(&report).expect("report value");
    value["checks"][0]["ghost_field"] = serde_json::json!(true);
    let bytes = serde_json::to_vec_pretty(&value).expect("bytes");
    let error = ValidationReport::from_json_bytes(&bytes).expect_err("check ghost field");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::DOCUMENT_INVALID
    );

    let mut value = serde_json::to_value(&profile).expect("profile value");
    value["ghost_field"] = serde_json::json!(1);
    let bytes = serde_json::to_vec_pretty(&value).expect("bytes");
    let error = michelangelo_asset_validation::profile::ValidationProfile::from_json_bytes(&bytes)
        .expect_err("profile ghost field");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::DOCUMENT_INVALID
    );
}

/// M4: `uv_preservation` with measured vt lines but neither an
/// `expected_uv_counts` pin nor a `min_uv_count` threshold is `unavailable`,
/// never a pass by mere presence (it is required for family_rig_compatible).
#[test]
fn m4_uv_preservation_without_pin_or_threshold_is_unavailable() {
    let temp = std::env::temp_dir().join("gav-adv-m4");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, mut profile) = build_family_workspace(&temp, "cand-house-m4");
    profile
        .references
        .reference_digests
        .remove("expected_uv_counts");

    let (report, _) = evaluate_fixture(&ws, &profile, "vr-m4").expect("evaluation");
    let uv = record_of(&report, CheckId::UvPreservation);
    assert_eq!(uv.result, CheckStatus::Unavailable);
    let detail = uv.detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("expected_uv_counts") && detail.contains("min_uv_count"),
        "detail: {detail}"
    );
    assert_eq!(report.verdict, Verdict::Unavailable);
}

/// H2: a required check recorded as `not_required` is structurally
/// assembleable, but registration refuses it, and the forged-registration
/// verification/seam path refuses it too. An accept decision citing such a
/// report never obtains `verdict_sufficient=true`.
#[test]
fn h2_required_check_cannot_be_not_required() {
    let temp = std::env::temp_dir().join("gav-adv-h2");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-h2");

    // payload_integrity is REQUIRED for family_static, yet is recorded as
    // not_required with a plausible applicability_ref. Assembly is structural
    // only, so it accepts the document; the registration/verify guards are the
    // load-bearing refusal.
    let report = ValidationReport::assemble(
        "vr-h2".to_string(),
        TIMESTAMP.to_string(),
        BundleRef {
            kind: michelangelo_asset_validation::report::BundleKind::CandidateAssetBundle,
            id: ws.manifest.candidate_id.clone(),
            sha256: ws.manifest_sha256.clone(),
        },
        ProfileRef {
            profile_id: profile.profile_id.clone(),
            revision: profile.revision,
            profile_sha256: profile.digest().expect("profile digest"),
        },
        [(ws.manifest_path.clone(), ws.manifest_sha256.clone())]
            .into_iter()
            .collect(),
        vec![CheckRecord {
            check_id: CheckId::PayloadIntegrity,
            result: CheckStatus::NotRequired,
            value: None,
            detail: Some("mask the required measurement".to_string()),
            applicability_ref: Some("bundle_scope:looks_optional".to_string()),
            evidence: Vec::new(),
        }],
        [("test", "0")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
    .expect("structurally valid report");
    assert_eq!(
        report.verdict,
        Verdict::Pass,
        "not_required folds neutrally"
    );

    // Registration with the pinned profile refuses before writing.
    let reservation =
        Reservation::reserve(&temp, "vr-h2", &ws.manifest_sha256, TIMESTAMP).expect("reservation");
    let error =
        write_and_register(&temp, &reservation, &report, &profile).expect_err("H2 registration");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::INVALID_NOT_REQUIRED
    );

    // A forged registry entry (bypassing registration) is refused by verify
    // against the actual profile.
    let report_digest = force_register(&temp, &report);
    let error = verify_registered_report(&temp, "vr-h2", Some(&profile)).expect_err("H2 verify");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::INVALID_NOT_REQUIRED
    );
    // Without a pinned basis profile verification refuses ANY report (P3):
    // the required-check guard cannot be re-applied, so the report is
    // unprovable even before its not_required record is inspected.
    let error = verify_registered_report(&temp, "vr-h2", None).expect_err("H2 no basis");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_PROFILE_BASIS_MISSING
    );
    // Even with the pinned profile copy present the guard fires.
    store_pinned_profile(&temp, &profile).expect("pinned profile");
    let error = verify_registered_report(&temp, "vr-h2", None).expect_err("H2 basis guard");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::INVALID_NOT_REQUIRED
    );

    // The accept path (the seam calls verify with no external profile) is
    // blocked before any prerequisite fact is produced.
    let record = AcceptanceRecord {
        schema: michelangelo_asset_validation::acceptance::ACCEPTANCE_SCHEMA.to_string(),
        record_id: "rec-h2".to_string(),
        created_at: TIMESTAMP.to_string(),
        decision: "accepted".to_string(),
        target: michelangelo_asset_validation::acceptance::AcceptanceTarget {
            kind: "candidate_asset_bundle".to_string(),
            candidate_id: "cand-house-h2".to_string(),
            bundle_sha256: ws.manifest_sha256.clone(),
            asset_id: "asset-house".to_string(),
            expected_current_revision: None,
        },
        basis: michelangelo_asset_validation::acceptance::AcceptanceBasis {
            validation_report_sha256: report_digest,
            report_registered: true,
            profile_sha256: profile.digest().expect("profile digest"),
            policy_refs: Vec::new(),
            reason: None,
        },
        actor: michelangelo_asset_validation::acceptance::Actor {
            name: "author".to_string(),
            role: "owner".to_string(),
        },
        scope_note: None,
    };
    let seam = RegistryBackedSeam { root: &temp };
    let error = seam.check_prerequisites(&record).expect_err("H2 seam");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::INVALID_NOT_REQUIRED
    );
}

/// P3 (verify basis): `verify(None)` must not accept a report whose pinned
/// profile copy is missing, even when the report carries no `not_required`
/// records — the required-check guard cannot be re-applied to an unprovable
/// basis. A forged registry entry (bypassing registration) is refused; the
/// same report verifies once the pinned copy exists.
#[test]
fn p3_verify_without_a_pinned_profile_basis_is_refused() {
    let temp = std::env::temp_dir().join("gav-adv-p3-basis");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-p3b");

    let (report, _) = evaluate_fixture(&ws, &profile, "vr-p3-basis").expect("evaluation");
    assert_eq!(report.verdict, Verdict::Pass);

    // force_register writes the report bytes and the index entry but NOT the
    // digest-named profile copy registration always stores.
    force_register(&temp, &report);
    let error = verify_registered_report(&temp, "vr-p3-basis", None).expect_err("P3 no basis");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_PROFILE_BASIS_MISSING
    );

    // With the pinned basis restored the same report verifies.
    store_pinned_profile(&temp, &profile).expect("pinned profile");
    let outcome = verify_registered_report(&temp, "vr-p3-basis", None).expect("verify with basis");
    assert_eq!(outcome.verdict, Verdict::Pass);
}

/// P3 (duplicate records): registration must refuse a report where the same
/// check id appears twice ([pass, not_required]) — the first-match guard
/// would otherwise pass while verification (re-assembly) rejects the stored
/// bytes. The reservation is not consumed and nothing is written.
#[test]
fn p3_registration_refuses_duplicate_check_records() {
    let temp = std::env::temp_dir().join("gav-adv-p3-duplicate");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-p3d");

    let (report, _) = evaluate_fixture(&ws, &profile, "vr-p3-dup").expect("evaluation");
    // Direct struct construction: `assemble`/parse refuse duplicates, so a
    // bypassing writer is exactly the threat registration must stop.
    let mut duplicate = report.clone();
    duplicate.checks = vec![
        CheckRecord {
            check_id: CheckId::PayloadIntegrity,
            result: CheckStatus::Pass,
            value: None,
            detail: None,
            applicability_ref: None,
            evidence: Vec::new(),
        },
        CheckRecord {
            check_id: CheckId::PayloadIntegrity,
            result: CheckStatus::NotRequired,
            value: None,
            detail: Some("the duplicate that would mask the first record".to_string()),
            applicability_ref: Some("bundle_scope:forged".to_string()),
            evidence: Vec::new(),
        },
    ];

    let reservation = Reservation::reserve(&temp, "vr-p3-dup", &ws.manifest_sha256, TIMESTAMP)
        .expect("reservation");
    let error = write_and_register(&temp, &reservation, &duplicate, &profile)
        .expect_err("P3 duplicate registration");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::DOCUMENT_INVALID
    );
    assert!(
        load_registry(&temp).expect("registry").entries.is_empty(),
        "nothing may be registered"
    );
    assert!(
        !report_file_path(&temp, "vr-p3-dup").exists(),
        "no report bytes may be written"
    );
}

/// Declare a real export output on a finalized workspace and re-pin the
/// asset digest; the returned profile requires export_roundtrip.
fn declare_export_artifact(
    ws: &FinalizedWorkspace,
    profile: &michelangelo_asset_validation::profile::ValidationProfile,
) -> michelangelo_asset_validation::profile::ValidationProfile {
    use michelangelo_asset_validation::canonical::asset_digest;
    use michelangelo_asset_validation::context::ArtifactEntry;

    let glb: Vec<u8> = vec![b'g', b'l', b'T', b'F', 0x01];
    std::fs::create_dir_all(ws.root().join("exports")).expect("exports dir");
    std::fs::write(ws.root().join("exports/model.glb"), &glb).expect("glb write");

    let mut manifest = ws.manifest.clone();
    manifest.outputs.export_glb = true;
    manifest.artifacts.push(ArtifactEntry {
        artifact_id: "exchange".to_string(),
        path: "exports/model.glb".to_string(),
        sha256: sha256_hex(&glb),
        size_bytes: glb.len() as u64,
        content_kind: "other".to_string(),
        roles: Vec::new(),
        protected: false,
    });
    let entries: Vec<(String, String, String, String)> = manifest
        .artifacts
        .iter()
        .map(|a| {
            (
                a.artifact_id.clone(),
                a.path.clone(),
                a.content_kind.clone(),
                a.sha256.clone(),
            )
        })
        .collect();
    manifest.asset_digest = Some(asset_digest(&entries));
    let bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
    std::fs::write(ws.root().join("manifest.json"), bytes).expect("manifest write");

    let pinned = serde_json::json!({ "asset_digest": manifest.asset_digest });
    let digest_pin = write_reference(ws.root(), "expected_asset_digest", &pinned);

    let mut updated = profile.clone();
    updated.required_checks.push(CheckId::ExportRoundtrip);
    updated.applies_to.conditions.push("export:glb".to_string());
    with_reference(updated, "expected_asset_digest", &digest_pin)
}
