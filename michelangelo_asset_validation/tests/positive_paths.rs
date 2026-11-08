//! Positive paths: Profile A (passthrough preservation), Profile B
//! (family static / rig compatible), honest unavailability, the registry
//! lifecycle, and the reconstruction-kind refusal.

use michelangelo_asset_validation::check::{CheckId, CheckStatus, DocRef, Verdict};
use michelangelo_asset_validation::profile::{
    Comparison, ProfileKind, ProfileReferences, ValidationProfile,
};
use michelangelo_asset_validation::registry::{
    load_registry, verify_registered_report, write_and_register, Reservation,
};
use michelangelo_asset_validation::report::ValidationReport;
use michelangelo_asset_validation::testing::*;

fn passthrough_profile(ws: &FinalizedWorkspace) -> ValidationProfile {
    let source_digests = serde_json::json!({
        "artifacts/body.obj": ws.manifest.artifacts[0].sha256,
        "deps/wall.mtl": ws.manifest.dependencies[0].sha256,
        "textures/wall.png": ws.manifest.dependencies[1].sha256,
    });
    let expected = serde_json::json!({"asset_digest": ws.asset_digest});
    let source_pin = write_reference(ws.root(), "source_payload_digests", &source_digests);
    let digest_pin = write_reference(ws.root(), "expected_asset_digest", &expected);
    let profile = make_profile(
        "passthrough-v0",
        ProfileKind::PassthroughPreservation,
        1,
        vec![
            CheckId::PayloadIntegrity,
            CheckId::DependencyClosure,
            CheckId::PathContainment,
            CheckId::AssetDigestFull,
            CheckId::FormatLossDeclared,
            CheckId::MaterialPresence,
            CheckId::TextureIntegrity,
            CheckId::UvPreservation,
        ],
        vec![CheckId::ProtectedRoles],
        Vec::new(),
        vec![
            "materials_preserved".to_string(),
            "textures_preserved".to_string(),
            "uv_preserved".to_string(),
        ],
        ProfileReferences::default(),
    );
    let profile = with_reference(profile, "source_payload_digests", &source_pin);
    with_reference(profile, "expected_asset_digest", &digest_pin)
}

fn build_passthrough_workspace(root: &std::path::Path) -> FinalizedWorkspace {
    let mut ws = WorkspaceBuilder::new(root, "cand-model-a");
    let mtl = wall_mtl("textures/wall.png");
    ws.add_dependency("wall-mtl", "deps/wall.mtl", mtl.as_bytes(), 1);
    let texture: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 1, 2, 3, 4];
    ws.add_dependency("wall-tex", "textures/wall.png", &texture, 1);
    let obj = cube_obj();
    ws.add_obj("body", "artifacts/body.obj", &obj, &["source"], false);
    ws.declare_conversion("obj", "blend");
    ws.declare_loss("pbr_materials", "OBJ/MTL carries diffuse+spec only");
    ws.declare_outputs(false, true);
    ws.finalize()
}

#[test]
fn passthrough_profile_a_passes_honestly() {
    let temp = std::env::temp_dir().join("gav-pos-passthrough");
    let _ = std::fs::remove_dir_all(&temp);
    let ws = build_passthrough_workspace(&temp);
    let profile = passthrough_profile(&ws);

    let (report, digest) = evaluate_fixture(&ws, &profile, "vr-0001").expect("evaluation");
    assert_eq!(report.verdict, Verdict::Pass, "verdict: {:?}", report.verdict);
    for check in [
        CheckId::PayloadIntegrity,
        CheckId::DependencyClosure,
        CheckId::PathContainment,
        CheckId::AssetDigestFull,
        CheckId::FormatLossDeclared,
        CheckId::MaterialPresence,
        CheckId::TextureIntegrity,
        CheckId::UvPreservation,
    ] {
        let record = report
            .checks
            .iter()
            .find(|record| record.check_id == check)
            .unwrap_or_else(|| panic!("missing record for {}", check.as_str()));
        assert_eq!(record.result, CheckStatus::Pass, "{}: {:?}", check.as_str(), record.detail);
    }
    // The optional protected_roles probe: no scene_layout in this bundle.
    let protected = report
        .checks
        .iter()
        .find(|record| record.check_id == CheckId::ProtectedRoles)
        .expect("protected_roles record");
    assert_eq!(protected.result, CheckStatus::NotRequired);
    assert!(protected
        .applicability_ref
        .as_deref()
        .unwrap_or_default()
        .starts_with("bundle_scope:"));

    // Input digests cover the actually read bytes: manifest, artifacts,
    // dependencies and pinned references.
    for path in [
        "manifest.json",
        "artifacts/body.obj",
        "deps/wall.mtl",
        "textures/wall.png",
        "references/expected_asset_digest.json",
    ] {
        assert!(
            report.input_digests.contains_key(path),
            "the input digest of '{path}' is missing"
        );
    }

    // The report digest is deterministic over the canonical encoding.
    let recomputed = report.digest().expect("digest");
    assert_eq!(recomputed, digest);

    // Round-trip through bytes keeps the report identical.
    let bytes = report.to_json_bytes().expect("bytes");
    let parsed = ValidationReport::from_json_bytes(&bytes).expect("parse");
    assert_eq!(parsed, report);
}

#[test]
fn passthrough_with_declared_export_is_unavailable_without_backend() {
    let temp = std::env::temp_dir().join("gav-pos-roundtrip");
    let _ = std::fs::remove_dir_all(&temp);
    let mut ws = WorkspaceBuilder::new(&temp, "cand-model-a2");
    let mtl = wall_mtl("textures/wall.png");
    ws.add_dependency("wall-mtl", "deps/wall.mtl", mtl.as_bytes(), 1);
    let texture: Vec<u8> = vec![1, 2, 3];
    ws.add_dependency("wall-tex", "textures/wall.png", &texture, 1);
    let obj = cube_obj();
    ws.add_obj("body", "artifacts/body.obj", &obj, &["source"], false);
    // The declared export artifact whose roundtrip cannot be verified.
    let glb: Vec<u8> = vec![b'g', b'l', b'T', b'F', 1, 2, 3];
    ws.add_artifact("exchange", "exports/model.glb", "other", &glb, &["exchange"], false);
    ws.declare_conversion("obj", "blend");
    ws.declare_loss("pbr_materials", "OBJ/MTL carries diffuse+spec only");
    ws.declare_outputs(true, true);
    let ws = ws.finalize();

    let source_digests = serde_json::json!({
        "artifacts/body.obj": ws.manifest.artifacts[0].sha256,
        "deps/wall.mtl": ws.manifest.dependencies[0].sha256,
        "textures/wall.png": ws.manifest.dependencies[1].sha256,
    });
    let expected = serde_json::json!({"asset_digest": ws.asset_digest});
    let source_pin = write_reference(ws.root(), "source_payload_digests", &source_digests);
    let digest_pin = write_reference(ws.root(), "expected_asset_digest", &expected);
    let profile = make_profile(
        "passthrough-v1",
        ProfileKind::PassthroughPreservation,
        1,
        vec![
            CheckId::PayloadIntegrity,
            CheckId::DependencyClosure,
            CheckId::PathContainment,
            CheckId::AssetDigestFull,
            CheckId::FormatLossDeclared,
            CheckId::MaterialPresence,
            CheckId::TextureIntegrity,
            CheckId::UvPreservation,
            CheckId::ExportRoundtrip,
        ],
        Vec::new(),
        Vec::new(),
        vec![
            "materials_preserved".to_string(),
            "textures_preserved".to_string(),
            "uv_preserved".to_string(),
            "export:glb".to_string(),
        ],
        ProfileReferences::default(),
    );
    let profile = with_reference(profile, "source_payload_digests", &source_pin);
    let profile = with_reference(profile, "expected_asset_digest", &digest_pin);

    let (report, _) = evaluate_fixture(&ws, &profile, "vr-0002").expect("evaluation");
    let roundtrip = report
        .checks
        .iter()
        .find(|record| record.check_id == CheckId::ExportRoundtrip)
        .expect("roundtrip record");
    assert_eq!(roundtrip.result, CheckStatus::Unavailable);
    assert!(roundtrip.detail.as_deref().unwrap_or_default().contains("BLOCKED"));
    // An unavailable mandatory check folds to an unavailable verdict —
    // never to a pass.
    assert_eq!(report.verdict, Verdict::Unavailable);
}

#[test]
fn family_static_profile_b_passes() {
    let temp = std::env::temp_dir().join("gav-pos-family");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile) = build_family_workspace(&temp, "cand-house-v1");
    let (report, _) = evaluate_fixture(&ws, &profile, "vr-0003").expect("evaluation");
    assert_eq!(report.verdict, Verdict::Pass, "checks: {:#?}", report.checks);
    for check in [
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
        CheckId::UvPreservation,
    ] {
        let record = report
            .checks
            .iter()
            .find(|record| record.check_id == check)
            .unwrap_or_else(|| panic!("missing record for {}", check.as_str()));
        assert_eq!(record.result, CheckStatus::Pass, "{}: {:?}", check.as_str(), record.detail);
    }
    // Optional format_loss_declared with no conversions: not_required with
    // an applicability reference (F16-compliant).
    let loss = report
        .checks
        .iter()
        .find(|record| record.check_id == CheckId::FormatLossDeclared)
        .expect("format loss record");
    assert_eq!(loss.result, CheckStatus::NotRequired);
    assert!(loss.applicability_ref.as_deref().unwrap_or_default().starts_with("bundle_scope:"));
}

#[test]
fn family_rig_compatible_marks_clips_unavailable_without_backend() {
    let temp = std::env::temp_dir().join("gav-pos-rig");
    let _ = std::fs::remove_dir_all(&temp);
    let mut ws = WorkspaceBuilder::new(&temp, "cand-creature-v1");
    let obj = block_obj(1.0, 2);
    ws.add_obj("body", "artifacts/body.obj", &obj, &["body"], false);
    let bones = vec![bone("root", None)];
    let skeleton = serde_json::json!({"skeleton_id": "base-skeleton", "bones": bones});
    let skeleton_pin = write_reference(ws.root(), "skeleton_profile", &skeleton);
    let rig = rig_doc("base-skeleton", vec![bone("root", None)], vec![binding("body", "root")]);
    ws.set_rig("rig", &rig, "metadata/rig.json");
    let clip_bytes: &[u8] = b"clip-walk-bytes";
    let clip_sha = ws.add_dependency("clip-walk", "clips/walk.clip", clip_bytes, 1);
    let ws = ws.finalize();

    let expected = serde_json::json!({"asset_digest": ws.asset_digest});
    let digest_pin = write_reference(ws.root(), "expected_asset_digest", &expected);
    let profile = make_profile(
        "rig-v0",
        ProfileKind::FamilyRigCompatible,
        1,
        vec![
            CheckId::PayloadIntegrity,
            CheckId::DependencyClosure,
            CheckId::PathContainment,
            CheckId::AssetDigestFull,
            CheckId::BudgetGeometry,
            CheckId::RigBindings,
            CheckId::AnimationClips,
        ],
        Vec::new(),
        Vec::new(),
        vec!["clips_pinned".to_string()],
        ProfileReferences::default(),
    );
    let profile = with_reference(profile, "skeleton_profile", &skeleton_pin);
    let profile = with_reference(profile, "expected_asset_digest", &digest_pin);
    let profile = with_clips(
        profile,
        vec![DocRef {
            path: "clips/walk.clip".to_string(),
            sha256: clip_sha,
        }],
    );

    let (report, _) = evaluate_fixture(&ws, &profile, "vr-0004").expect("evaluation");
    let rig_record = report
        .checks
        .iter()
        .find(|record| record.check_id == CheckId::RigBindings)
        .expect("rig record");
    assert_eq!(rig_record.result, CheckStatus::Pass, "{:?}", rig_record.detail);
    let clips = report
        .checks
        .iter()
        .find(|record| record.check_id == CheckId::AnimationClips)
        .expect("clips record");
    // Digest-verified clips still do not pass without playback: honest
    // unavailable (ADR §4.2/§8.1).
    assert_eq!(clips.result, CheckStatus::Unavailable);
    assert!(clips.detail.as_deref().unwrap_or_default().contains("playback"));
    assert_eq!(report.verdict, Verdict::Unavailable);
}

#[test]
fn reconstruction_kind_is_refused_not_reimplemented() {
    let temp = std::env::temp_dir().join("gav-pos-reconstruction");
    let _ = std::fs::remove_dir_all(&temp);
    let ws = build_passthrough_workspace(&temp);
    let profile = make_profile(
        "recon-v0",
        ProfileKind::Reconstruction,
        1,
        vec![CheckId::PayloadIntegrity],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProfileReferences::default(),
    );
    let error = evaluate_fixture(&ws, &profile, "vr-0005").expect_err("must refuse");
    assert_eq!(error.code(), michelangelo_asset_validation::error::UNSUPPORTED_PROFILE_KIND);
}

#[test]
fn unknown_required_check_refuses_with_gate_code() {
    let json = r#"{
        "schema": "michelangelo.validation-profile/draft-0.1",
        "profile_id": "bad-profile",
        "revision": 1,
        "created_at": "2026-11-08T12:00:00Z",
        "kind": "family_static",
        "applies_to": {"targets": ["candidate_asset_bundle"]},
        "required_checks": ["payload_integrity", "ghost_check"],
        "evaluator_versions": {"test": "0"}
    }"#;
    let error = michelangelo_asset_validation::profile::ValidationProfile::from_json_bytes(json.as_bytes())
        .expect_err("unknown check must refuse");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_UNKNOWN_REQUIRED
    );
}

#[test]
fn registry_lifecycle_reserve_register_verify() {
    let temp = std::env::temp_dir().join("gav-pos-registry");
    let _ = std::fs::remove_dir_all(&temp);
    let ws = build_passthrough_workspace(&temp);
    let profile = passthrough_profile(&ws);

    // Reserve BEFORE the heavy evaluation.
    let reservation = Reservation::reserve(&temp, "vr-0007", &ws.manifest_sha256, TIMESTAMP)
        .expect("reservation");
    let (report, digest) = evaluate_fixture(&ws, &profile, "vr-0007").expect("evaluation");
    let entry = write_and_register(&temp, &reservation, &report).expect("registration");
    assert_eq!(entry.sha256, digest);
    assert_eq!(entry.bundle_sha256, ws.manifest_sha256);

    // Verification passes and recomputes everything.
    let outcome = verify_registered_report(&temp, "vr-0007", Some(&profile)).expect("verify");
    assert_eq!(outcome.report_sha256, digest);
    assert_eq!(outcome.verdict, Verdict::Pass);

    // A second reservation of the same id while a live one exists conflicts.
    let _ = Reservation::reserve(&temp, "vr-0008", &ws.manifest_sha256, TIMESTAMP).expect("reserve 2");
    let conflict = Reservation::reserve(&temp, "vr-0008", &ws.manifest_sha256, TIMESTAMP)
        .expect_err("reservation conflict");
    assert_eq!(conflict.code(), michelangelo_asset_validation::error::RESERVATION_CONFLICT);

    // Idempotent re-registration of the same bytes is a no-op.
    let reservation_again = Reservation::reserve(&temp, "vr-0007", &ws.manifest_sha256, TIMESTAMP)
        .expect("reserve again");
    let entry_again = write_and_register(&temp, &reservation_again, &report).expect("re-register");
    assert_eq!(entry_again.sha256, entry.sha256);
    let registry = load_registry(&temp).expect("registry");
    assert_eq!(registry.entries.len(), 1, "the registry is append-only");

    // The same id with different bytes refuses with NO_CLOBBER.
    let profile_v2 = {
        let mut profile = profile.clone();
        profile.revision = 2;
        profile
    };
    let (report_v2, _) = evaluate_fixture(&ws, &profile_v2, "vr-0007").expect("evaluation v2");
    let error = write_and_register(&temp, &reservation_again_2(&temp, &ws), &report_v2)
        .expect_err("no-clobber");
    assert_eq!(error.code(), michelangelo_asset_validation::error::NO_CLOBBER_VIOLATION);
}

fn reservation_again_2(temp: &std::path::Path, ws: &FinalizedWorkspace) -> Reservation {
    Reservation::reserve(temp, "vr-0007", &ws.manifest_sha256, TIMESTAMP).expect("reserve 3")
}

#[test]
fn changed_profile_invalidates_the_report_basis() {
    let temp = std::env::temp_dir().join("gav-pos-threshold");
    let _ = std::fs::remove_dir_all(&temp);
    let (ws, profile_v1) = build_family_workspace(&temp, "cand-house-v2");
    let reservation = Reservation::reserve(&temp, "vr-0009", &ws.manifest_sha256, TIMESTAMP)
        .expect("reservation");
    let (report, _) = evaluate_fixture(&ws, &profile_v1, "vr-0009").expect("evaluation");
    write_and_register(&temp, &reservation, &report).expect("registration");

    // Same id, stricter threshold: a different digest (a real revision).
    let mut profile_v2 = profile_v1.clone();
    profile_v2.revision = 2;
    profile_v2.thresholds = vec![threshold(
        CheckId::BudgetGeometry,
        "max_evaluated_triangles",
        Comparison::AtMost,
        3.0,
    )];
    assert_ne!(
        profile_v1.digest().expect("digest v1"),
        profile_v2.digest().expect("digest v2"),
        "a changed threshold must change the profile digest"
    );

    // Verification against the current profile refuses the stale basis.
    let error = verify_registered_report(&temp, "vr-0009", Some(&profile_v2)).expect_err("stale");
    assert_eq!(
        error.code(),
        michelangelo_asset_validation::error::VALIDATION_PROFILE_CHANGED
    );
    // Without a current profile the report itself is still verifiable.
    verify_registered_report(&temp, "vr-0009", None).expect("self-verify");
}

use michelangelo_asset_validation::testing::{binding, bone, build_family_workspace, rig_doc, threshold};

#[test]
fn report_rejects_an_imported_approval_flag() {
    // The wire DTO carries no approval field anywhere (GLM-1 draft);
    // unknown fields are rejected.
    let json = r#"{
        "schema": "michelangelo.acceptance-record/draft-0.1",
        "record_id": "rec-1",
        "created_at": "2026-11-08T12:00:00Z",
        "decision": "accepted",
        "target": {"kind": "candidate_asset_bundle", "candidate_id": "c", "bundle_sha256": "0000000000000000000000000000000000000000000000000000000000000000", "asset_id": "a", "expected_current_revision": null},
        "basis": {"validation_report_sha256": "0000000000000000000000000000000000000000000000000000000000000000", "report_registered": true, "profile_sha256": "0000000000000000000000000000000000000000000000000000000000000000"},
        "actor": {"name": "author", "role": "owner"},
        "approved": true
    }"#;
    let error = michelangelo_asset_validation::acceptance::AcceptanceRecord::from_json_bytes(
        json.as_bytes(),
    )
    .expect_err("an 'approved' field must never parse");
    assert_eq!(error.code(), michelangelo_asset_validation::error::DOCUMENT_INVALID);
}
