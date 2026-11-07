//! The tests of the evidence store: the digests, the registry and the
//! accept gate (IT-8B).

use std::collections::BTreeMap;

use michelangelo_evidence::digest::{
    camera_digest, canonical_json, file_sha256, geometry_aggregate, inputs_digest,
};
use michelangelo_evidence::gate::{verify_report, GateContext};
use michelangelo_evidence::registry::{
    load_registry, report_path, write_and_register, write_registry, EvaluationRegistry,
    RegistryEntry,
};
use michelangelo_evidence::report::{
    AuthorDecision, CameraDigest, ClaimStatus, EvaluationCheck, EvaluationInputs, EvaluationReport,
    EvaluationResults, EvaluationTarget, EvaluationThresholds, GroundTruthState, HiddenClaim,
    HiddenSection, ReportIntegrity, TargetKind, TriState, ViewPin, VisibleSection, VisibleView,
    EVALUATION_REPORT_SCHEMA,
};

/// The camera of the test views.
fn camera(view_id: &str) -> CameraDigest {
    CameraDigest {
        view_id: view_id.to_string(),
        location_m: [0.0, -4.0, 0.0],
        look_at_m: [0.0, 0.0, 0.0],
        up_world: [0.0, 0.0, 1.0],
        ortho_scale_m: 1.0,
        shift_x: 0.0,
        shift_y: 0.0,
        clip_start_m: 0.02,
        clip_end_m: 100.0,
        width_px: 8,
        height_px: 8,
    }
}

/// The pinned view of the test.
fn view_pin() -> ViewPin {
    ViewPin {
        view_id: "front".to_string(),
        camera_digest: camera_digest(&camera("front")),
        mask_sha256: "mask".to_string(),
        resolution: [8, 8],
        held_out: false,
    }
}

/// A report that passes every check of the gate.
fn passing_report() -> EvaluationReport {
    EvaluationReport {
        schema: EVALUATION_REPORT_SCHEMA.to_string(),
        evaluation_id: "ev-0001".to_string(),
        created_at: "2026-01-03T04:05:06Z".to_string(),
        run_id: "run-0001".to_string(),
        target: EvaluationTarget {
            kind: TargetKind::Candidate,
            id: "cand-0001".to_string(),
            aggregate_sha256: "aggregate".to_string(),
            base_scene_revision: Some("s0001".to_string()),
        },
        inputs: EvaluationInputs {
            package_sha256: Some("package".to_string()),
            views: vec![view_pin()],
            evaluation_package_sha256: None,
        },
        thresholds: EvaluationThresholds::default(),
        results: EvaluationResults {
            projection_conformance: TriState::Pass,
            author_geometric_conditions: TriState::Pass,
            independent_ground_truth: TriState::Unavailable,
            author_hidden_decision: AuthorDecision::Pending,
        },
        visible: VisibleSection {
            views: vec![VisibleView {
                view_id: "front".to_string(),
                iou: 0.95,
                bbox_error_px: Some(0.0),
                base_iou: Some(0.95),
                regression: Some(0.0),
            }],
            mean_iou: 0.95,
            min_iou: 0.95,
            base_mean_iou: Some(0.95),
            verdict: TriState::Pass,
            held_out: None,
        },
        hidden: HiddenSection {
            ground_truth: GroundTruthState::Absent,
            claims: Vec::new(),
            verdict: AuthorDecision::Pending,
        },
        checks: vec![EvaluationCheck {
            id: "height".to_string(),
            kind: "author_geometric_condition".to_string(),
            required: true,
            status: TriState::Pass,
            detail: "the measured height is inside the tolerance".to_string(),
        }],
        acceptance_basis: vec!["projection_conformance=pass".to_string()],
        tool_versions: BTreeMap::new(),
        integrity: ReportIntegrity::default(),
    }
}

/// The context of the gate for the passing report.
fn context() -> GateContext<'static> {
    GateContext {
        run_id: "run-0001",
        target_kind: TargetKind::Candidate,
        target_id: "cand-0001",
        target_aggregate_sha256: "aggregate",
        base_scene_revision: Some("s0001"),
        current_package_sha256: Some("package"),
        current_source_views: Box::leak(Box::new(vec![view_pin()])),
        required_condition_ids: Box::leak(Box::new(vec!["height".to_string()])),
        decision_evaluation_sha256: None,
    }
}

/// The registry that carries the passing report.
fn registry_with(report: &EvaluationReport, sha256: &str) -> EvaluationRegistry {
    let mut registry = EvaluationRegistry::empty();
    registry
        .register(RegistryEntry {
            evaluation_id: report.evaluation_id.clone(),
            report_path: format!(
                "verification/evaluations/{}/report.json",
                report.evaluation_id
            ),
            sha256: sha256.to_string(),
            target_digest: report.target.aggregate_sha256.clone(),
            created_at: report.created_at.clone(),
        })
        .expect("the entry is registered");
    registry
}

#[test]
fn the_geometry_aggregate_sorts_the_components() {
    let first = geometry_aggregate(&[
        ("lid".to_string(), "b".to_string()),
        ("base".to_string(), "a".to_string()),
    ]);
    let second = geometry_aggregate(&[
        ("base".to_string(), "a".to_string()),
        ("lid".to_string(), "b".to_string()),
    ]);
    assert_eq!(first, second, "the digest does not depend on the order");
    let expected = michelangelo_evidence::digest::sha256_hex(b"base\0a\nlid\0b\n");
    assert_eq!(first, expected);
}

#[test]
fn the_camera_digest_pins_the_calibration() {
    let first = camera_digest(&camera("front"));
    let mut changed = camera("front");
    changed.ortho_scale_m = 2.0;
    let second = camera_digest(&changed);
    assert_ne!(first, second, "the calibration is pinned");
    assert_eq!(first.len(), 64);
}

#[test]
fn the_inputs_digest_pins_the_views() {
    let inputs = EvaluationInputs {
        package_sha256: Some("package".to_string()),
        views: vec![view_pin()],
        evaluation_package_sha256: None,
    };
    let digest = inputs_digest(&inputs);
    assert_eq!(digest.len(), 64);
    let mut other = inputs.clone();
    other.views[0].mask_sha256 = "other".to_string();
    assert_ne!(digest, inputs_digest(&other));
}

#[test]
fn the_canonical_json_sorts_the_keys() {
    let value = serde_json::json!({"b": 1, "a": {"d": 2, "c": 3}});
    assert_eq!(canonical_json(&value), "{\"a\":{\"c\":3,\"d\":2},\"b\":1}");
}

#[test]
fn the_registry_is_append_only_and_deterministic() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path();
    let report = passing_report();
    let entry = write_and_register(run_dir, &report).expect("the report is registered");
    assert_eq!(entry.evaluation_id, "ev-0001");
    assert!(report_path(run_dir, "ev-0001").is_file());

    // A second registration of the same report is a no-op.
    let again = write_and_register(run_dir, &report).expect("the report is registered");
    assert_eq!(again.sha256, entry.sha256);

    // A different digest under the same identifier is refused.
    let registry = load_registry(run_dir).expect("the registry is read");
    assert_eq!(registry.evaluations.len(), 1);
    let mut other = report.clone();
    other.created_at = "2026-02-02T00:00:00Z".to_string();
    let error = write_and_register(run_dir, &other).expect_err("the entry is never overwritten");
    assert_eq!(error.code(), "EVALUATION_ALREADY_REGISTERED");
}

#[test]
fn the_registry_orders_the_entries_by_the_identifier() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path();
    let mut registry = EvaluationRegistry::empty();
    for id in ["ev-0002", "ev-0001"] {
        registry
            .register(RegistryEntry {
                evaluation_id: id.to_string(),
                report_path: format!("verification/evaluations/{id}/report.json"),
                sha256: "digest".to_string(),
                target_digest: "aggregate".to_string(),
                created_at: "2026-01-03T04:05:06Z".to_string(),
            })
            .expect("the entry is registered");
    }
    write_registry(run_dir, &registry).expect("the registry is written");
    let text = std::fs::read_to_string(michelangelo_evidence::registry::registry_path(run_dir))
        .expect("the registry is readable");
    let first = text.find("ev-0001").expect("the first entry is listed");
    let second = text.find("ev-0002").expect("the second entry is listed");
    assert!(first < second, "the entries are ordered");
}

#[test]
fn a_registered_report_passes_the_gate() {
    let report = passing_report();
    let registry = registry_with(&report, "digest");
    verify_report(&registry, &report, "digest", &context()).expect("the report passes");
}

#[test]
fn an_unregistered_report_is_refused() {
    let report = passing_report();
    let registry = EvaluationRegistry::empty();
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "EVALUATION_UNREGISTERED");
}

#[test]
fn a_tampered_report_is_refused() {
    let report = passing_report();
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "other", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "EVALUATION_STALE");
}

#[test]
fn a_report_of_another_geometry_is_refused() {
    let mut report = passing_report();
    report.target.aggregate_sha256 = "other".to_string();
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "EVALUATION_STALE");
}

#[test]
fn a_report_of_another_base_is_refused() {
    let mut report = passing_report();
    report.target.base_scene_revision = Some("s0000".to_string());
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "EVALUATION_STALE");
}

#[test]
fn a_report_of_another_package_is_refused() {
    let mut report = passing_report();
    report.inputs.package_sha256 = Some("other".to_string());
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "INPUT_CHANGED");
}

#[test]
fn a_report_of_another_camera_is_refused() {
    let mut report = passing_report();
    report.inputs.views[0].camera_digest = "other".to_string();
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "INPUT_CHANGED");
}

#[test]
fn a_failed_projection_is_refused() {
    let mut report = passing_report();
    report.results.projection_conformance = TriState::Fail;
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "VISIBLE_FAILED");
}

#[test]
fn a_failed_author_condition_is_refused() {
    let mut report = passing_report();
    report.checks[0].status = TriState::Fail;
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "AUTHOR_CONDITION_FAILED");
}

#[test]
fn a_missing_author_condition_is_refused() {
    let mut report = passing_report();
    report.checks.clear();
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "AUTHOR_CONDITION_FAILED");
}

#[test]
fn a_regression_is_refused() {
    let mut report = passing_report();
    report.visible.views[0].regression = Some(0.5);
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "REGRESSION");
}

#[test]
fn a_report_cannot_widen_the_per_view_regression_limit() {
    // The owner fixed the per-view limit at 0.01 IoU; a report that raises its
    // own threshold must not make a 0.02 drop acceptable.
    let mut report = passing_report();
    report.thresholds.per_view_regression_max = 0.5;
    report.visible.views[0].regression = Some(0.02);
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "REGRESSION");
}

#[test]
fn the_gate_tolerates_an_unsorted_current_view_list() {
    fn view_pin_of(view_id: &str) -> ViewPin {
        let mut pin = view_pin();
        pin.view_id = view_id.to_string();
        pin
    }
    let mut report = passing_report();
    report.inputs.views = vec![view_pin_of("front"), view_pin_of("left")];
    let registry = registry_with(&report, "digest");
    let mut gate_context = context();
    gate_context.current_source_views =
        Box::leak(Box::new(vec![view_pin_of("left"), view_pin_of("front")]));
    verify_report(&registry, &report, "digest", &gate_context)
        .expect("the view order does not matter");
}

#[test]
fn an_unmeasured_base_is_refused() {
    let mut report = passing_report();
    report.visible.views[0].regression = None;
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "REGRESSION");
}

#[test]
fn a_claim_without_a_ground_truth_is_refused() {
    let mut report = passing_report();
    report.hidden.claims.push(HiddenClaim {
        id: "u_interior".to_string(),
        kind: "unknown".to_string(),
        status: ClaimStatus::Unverified,
        evidence_refs: Vec::new(),
        requires_ground_truth: true,
    });
    let registry = registry_with(&report, "digest");
    let error =
        verify_report(&registry, &report, "digest", &context()).expect_err("the report is refused");
    assert_eq!(error.code(), "NO_GROUND_TRUTH");
}

#[test]
fn an_unverified_hidden_claim_does_not_block_the_acceptance() {
    let mut report = passing_report();
    report.hidden.claims.push(HiddenClaim {
        id: "u_interior".to_string(),
        kind: "unknown".to_string(),
        status: ClaimStatus::Unverified,
        evidence_refs: Vec::new(),
        requires_ground_truth: false,
    });
    let registry = registry_with(&report, "digest");
    verify_report(&registry, &report, "digest", &context())
        .expect("the hidden claim does not block the acceptance");
}

#[test]
fn the_file_digest_pins_the_bytes() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("file.txt");
    std::fs::write(&path, b"hello").expect("the file is written");
    let digest = file_sha256(&path).expect("the digest is computed");
    assert_eq!(digest.len(), 64);
    std::fs::write(&path, b"other").expect("the file is written");
    assert_ne!(digest, file_sha256(&path).expect("the digest is computed"));
}
