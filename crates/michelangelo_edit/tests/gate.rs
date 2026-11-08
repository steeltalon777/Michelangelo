//! The failure tests of the accept gate (IT-8B).
//!
//! The tests prove the invariant of the contract §7: an acceptance needs a
//! registered evaluation whose digest, target aggregate, base revision and
//! pinned inputs match the state of the store, whose projection conformance
//! passes, whose required author geometric conditions pass and whose source
//! views do not regress against the base revision.

mod common;

use std::path::{Path, PathBuf};

use common::{candidate, lid_obj, make_run, write_candidate};
use michelangelo_edit::apply::{accept_candidate, accept_candidate_with_options, submit_candidate};
use michelangelo_edit::error::{
    AUTHOR_CONDITION_FAILED, EVALUATION_MISSING, EVALUATION_STALE, EVALUATION_UNREGISTERED,
    INPUT_CHANGED, NO_GROUND_TRUTH, REGRESSION, VISIBLE_FAILED,
};
use michelangelo_edit::gate::AcceptOptions;
use michelangelo_edit::FixedClock;
use michelangelo_evidence::registry::{load_registry, write_and_register};
use michelangelo_evidence::report::{
    ClaimStatus, EvaluationReport, GroundTruthState, HiddenClaim, TriState,
};
use michelangelo_evidence::testing::synthetic_evaluation;

/// A scratch run with a submitted candidate.
struct Fixture {
    _directory: tempfile::TempDir,
    run_dir: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let run_dir = directory.path().join("run-0001");
        make_run(&run_dir);
        michelangelo_edit::scene::init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
            .expect("the scene is created");
        std::fs::create_dir_all(run_dir.join("package")).expect("the package directory is created");
        std::fs::write(run_dir.join("package/package.json"), package_json())
            .expect("the package copy is written");
        // The copy of the package carries the mask of the view: the pins of
        // an evaluation are the digests of the bytes that are on the disk.
        std::fs::create_dir_all(run_dir.join("package/masks"))
            .expect("the mask directory is created");
        std::fs::write(run_dir.join("package/masks/front.png"), b"mask")
            .expect("the mask is written");
        let candidate_dir = directory.path().join("cand-0001");
        write_candidate(
            &candidate_dir,
            &candidate(&serde_json::json!({})),
            &[("lid.obj", lid_obj(0.02))],
        );
        submit_candidate(
            &run_dir,
            &candidate_dir,
            &FixedClock::new("2026-01-03T04:05:06Z"),
        )
        .expect("the candidate is submitted");
        Fixture {
            _directory: directory,
            run_dir,
        }
    }

    fn clock(&self) -> FixedClock {
        FixedClock::new("2026-01-03T04:05:06Z")
    }
}

/// The package copy of the run: one calibrated view and one measurement.
fn package_json() -> String {
    let mut text = String::new();
    text.push_str("{\n");
    text.push_str("  \"schema\": \"michelangelo.input-package/0\",\n");
    text.push_str("  \"package_id\": \"demo-package\",\n");
    text.push_str("  \"revision\": 1,\n");
    text.push_str("  \"units\": \"m\",\n");
    text.push_str("  \"coordinate_system\": {\"handedness\": \"right\", \"up\": \"+Z\", \"right\": \"+X\", \"forward\": \"-Y\"},\n");
    text.push_str("  \"views\": [{\n");
    text.push_str("    \"view_id\": \"front\",\n");
    text.push_str("    \"kind\": \"calibrated\",\n");
    text.push_str("    \"image\": {\"path\": \"images/front.png\", \"sha256\": \"0000000000000000000000000000000000000000000000000000000000000001\", \"width_px\": 8, \"height_px\": 8},\n");
    text.push_str("    \"mask\": {\"path\": \"masks/front.png\", \"sha256\": \"0000000000000000000000000000000000000000000000000000000000000002\", \"format\": \"grayscale8\", \"foreground\": \"white\"},\n");
    text.push_str("    \"projection\": {\"type\": \"orthographic\", \"location_m\": [0.0, -4.0, 0.0], \"look_at_m\": [0.0, 0.0, 0.0], \"up_world\": [0.0, 0.0, 1.0], \"ortho_scale_m\": 1.0}\n");
    text.push_str("  }],\n");
    text.push_str("  \"measurements\": [{\"id\": \"height\", \"kind\": \"height\", \"value_m\": 0.25, \"tolerance_m\": 0.002, \"origin\": \"human\"}],\n");
    text.push_str("  \"provenance\": {\"origin\": \"human\", \"detail\": \"fixture\"}\n");
    text.push_str("}\n");
    text
}

/// Register an evaluation of the candidate, optionally modified.
fn register(
    run_dir: &Path,
    candidate_id: &str,
    modify: impl FnOnce(&mut EvaluationReport),
) -> PathBuf {
    let registry = load_registry(run_dir).expect("the registry is read");
    let evaluation_id = registry.next_evaluation_id();
    let mut report = synthetic_evaluation(
        run_dir,
        candidate_id,
        &evaluation_id,
        "2026-01-03T04:05:06Z",
    )
    .expect("the synthetic evaluation is built");
    modify(&mut report);
    let entry = write_and_register(run_dir, &report).expect("the evaluation is registered");
    run_dir.join(entry.report_path)
}

/// The names of the records of the decision history of the candidate.
fn history(run_dir: &Path) -> Vec<String> {
    let directory = run_dir.join("candidates/cand-0001/decisions");
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

#[test]
fn an_acceptance_without_an_evaluation_is_refused() {
    let fixture = Fixture::new();
    let error = accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVALUATION_MISSING);
    assert_eq!(error.exit_code(), 1);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
    assert!(
        history(&fixture.run_dir)
            .iter()
            .any(|name| name.contains("accept-refused")),
        "the refusal is auditable"
    );
}

#[test]
fn an_unregistered_evaluation_is_refused() {
    let fixture = Fixture::new();
    let report = synthetic_evaluation(
        &fixture.run_dir,
        "cand-0001",
        "ev-9999",
        "2026-01-03T04:05:06Z",
    )
    .expect("the synthetic evaluation is built");
    let path = fixture
        .run_dir
        .join("verification/evaluations/ev-9999/report.json");
    std::fs::create_dir_all(path.parent().expect("the parent exists"))
        .expect("the directory is created");
    std::fs::write(&path, report.to_json().expect("the report is serialized"))
        .expect("the report is written");

    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVALUATION_UNREGISTERED);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_tampered_evaluation_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |_| {});
    let mut text = std::fs::read_to_string(&path).expect("the report is readable");
    text.push('\n');
    std::fs::write(&path, text).expect("the report is written");

    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVALUATION_STALE);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn an_evaluation_of_another_geometry_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |report| {
        report.target.aggregate_sha256 = "another".to_string();
    });
    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVALUATION_STALE);
}

#[test]
fn changed_inputs_are_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |_| {});

    // The package of the run changes after the evaluation.
    let package = fixture.run_dir.join("package/package.json");
    std::fs::write(
        &package,
        package_json().replace("\"revision\": 1", "\"revision\": 2"),
    )
    .expect("the package copy is written");

    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), INPUT_CHANGED);
}

#[test]
fn a_failed_visible_result_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |report| {
        report.results.projection_conformance = TriState::Fail;
    });
    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), VISIBLE_FAILED);
}

#[test]
fn a_failed_author_condition_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |report| {
        for check in report.checks.iter_mut() {
            if check.kind == "author_geometric_condition" {
                check.status = TriState::Fail;
            }
        }
    });
    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), AUTHOR_CONDITION_FAILED);
}

#[test]
fn a_regression_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |report| {
        for view in report.visible.views.iter_mut() {
            view.regression = Some(0.5);
        }
    });
    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), REGRESSION);
}

#[test]
fn a_claim_without_a_ground_truth_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |report| {
        report.hidden.ground_truth = GroundTruthState::Absent;
        report.hidden.claims.push(HiddenClaim {
            id: "u_interior".to_string(),
            kind: "unknown".to_string(),
            status: ClaimStatus::Unverified,
            evidence_refs: Vec::new(),
            requires_ground_truth: true,
        });
    });
    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), NO_GROUND_TRUTH);
}

#[test]
fn a_changed_mask_after_the_evaluation_is_refused() {
    // The pin of a view is the digest of the bytes of the mask file: a mask
    // that changes after the evaluation is an input that changed.
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |_| {});

    let mask = fixture.run_dir.join("package/masks/front.png");
    std::fs::write(&mask, b"another mask").expect("the mask is written");

    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), INPUT_CHANGED);
    assert!(
        !fixture.run_dir.join("scene/s0002").exists(),
        "no revision is published"
    );
}

#[test]
fn a_missing_mask_after_the_evaluation_is_refused() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |_| {});

    let mask = fixture.run_dir.join("package/masks/front.png");
    std::fs::remove_file(&mask).expect("the mask is removed");

    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), INPUT_CHANGED);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_registered_evaluation_publishes_the_revision_and_references_it() {
    let fixture = Fixture::new();
    let path = register(&fixture.run_dir, "cand-0001", |_| {});
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            ..Default::default()
        },
    )
    .expect("the candidate is accepted");
    assert_eq!(report.resulting_scene_revision, Some("s0002".to_string()));
    assert_eq!(report.evaluation_id, Some("ev-0001".to_string()));
    assert!(report.evaluation_sha256.is_some());

    let scene: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.run_dir.join("scene/s0002/scene.json"))
            .expect("the scene is readable"),
    )
    .expect("the scene is JSON");
    let evaluations = scene["verification"]["evaluations"]
        .as_array()
        .expect("the evaluations are listed");
    assert_eq!(evaluations.len(), 1);
    assert_eq!(evaluations[0]["evaluation_id"], "ev-0001");
    assert_eq!(
        evaluations[0]["path"],
        "verification/evaluations/ev-0001/report.json"
    );
}
