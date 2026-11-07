//! The tests of the evaluator (IT-8B).
//!
//! The tests use a deterministic fake renderer and a deterministic fake
//! comparator: the evaluation assembly, the pins, the regression, the
//! hidden defaults and the gate checks are proved without Blender and
//! without Python.

use std::path::{Path, PathBuf};

use michelangelo_edit::scene::init_scene;
use michelangelo_edit::FixedClock;
use michelangelo_eval::compare::{
    Comparator, CompareMeasurement, CompareOutcome, CompareRequest, CompareSet, CompareView,
};
use michelangelo_eval::error::EvalError;
use michelangelo_eval::evaluate::{
    evaluate_candidate_with, evaluate_revision_with, EvaluationOptions,
};
use michelangelo_eval::render::{RenderRequest, Renderer};
use michelangelo_evidence::report::{ClaimStatus, TriState};

/// A box of 10 cm.
const BASE_OBJ: &str = "o base
v -0.05 -0.05 -0.05
v 0.05 -0.05 -0.05
v 0.05 0.05 -0.05
v -0.05 0.05 -0.05
v -0.05 -0.05 0.05
v 0.05 -0.05 0.05
v 0.05 0.05 0.05
v -0.05 0.05 0.05
f 1 4 3 2
f 5 6 7 8
f 1 2 6 5
f 4 8 7 3
f 1 5 8 4
f 2 3 7 6
";

/// The lid of the box.
const LID_OBJ: &str = "o lid
v -0.04 0.05 -0.04
v 0.04 0.05 -0.04
v 0.04 0.09 -0.04
v -0.04 0.09 -0.04
v -0.04 0.05 0.04
v 0.04 0.05 0.04
v 0.04 0.09 0.04
v -0.04 0.09 0.04
f 1 4 3 2
f 5 6 7 8
f 1 2 6 5
f 4 8 7 3
f 1 5 8 4
f 2 3 7 6
";

/// The package of the fixture: one calibrated view and one measurement.
fn package_json(view_id: &str, location: [f64; 3]) -> String {
    serde_json::json!({
        "schema": "michelangelo.input-package/0",
        "package_id": "demo-lantern",
        "object": {"name": "demo-lantern"},
        "revision": 1,
        "units": "m",
        "coordinate_system": {
            "handedness": "right",
            "up": "+Z",
            "right": "+X",
            "forward": "-Y"
        },
        "views": [
            {
                "view_id": view_id,
                "kind": "calibrated",
                "image": {
                    "path": format!("images/{view_id}.png"),
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000001",
                    "width_px": 8,
                    "height_px": 8
                },
                "mask": {
                    "path": format!("masks/{view_id}.png"),
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000002",
                    "format": "grayscale8",
                    "foreground": "white"
                },
                "projection": {
                    "type": "orthographic",
                    "location_m": location,
                    "look_at_m": [0.0, 0.0, 0.0],
                    "up_world": [0.0, 0.0, 1.0],
                    "ortho_scale_m": 1.0
                }
            }
        ],
        "measurements": [
            {
                "id": "height",
                "label": "overall height",
                "kind": "height",
                "value_m": 0.25,
                "tolerance_m": 0.002,
                "origin": "human"
            }
        ],
        "unknowns": [
            {
                "id": "u_interior",
                "description": "the interior of the body is unknown",
                "affected_parts": [],
                "policy": "author_required"
            }
        ],
        "provenance": {"origin": "human", "detail": "fixture"}
    })
    .to_string()
}

/// The candidate of the fixture.
fn candidate_json() -> String {
    serde_json::json!({
        "schema": "michelangelo.edit-candidate/0",
        "candidate_id": "cand-0001",
        "base": {
            "package_id": "demo-lantern",
            "package_revision": 1,
            "scene_revision": "s0001",
            "run_id": "run-0001"
        },
        "task": {
            "task_id": "fix-lid",
            "allowed_components": ["lid"],
            "allowed_region": {"min": [-0.2, -0.2, -0.2], "max": [0.2, 0.2, 0.2]},
            "protected": ["base"],
            "allowed_intersections": [],
            "require_author_acceptance": true,
            "max_changed_vertices": 100000
        },
        "changes": [{"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}],
        "provenance": {"origin": "human", "detail": "a fixture candidate"},
        "budget": {"max_calls": 1, "max_seconds": 120, "max_cost_usd": 0.0}
    })
    .to_string()
}

/// A run directory with a package copy, a scene and a submitted candidate.
struct Fixture {
    _directory: tempfile::TempDir,
    run_dir: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let run_dir = directory.path().join("run-0001");
        write(&run_dir.join("source/base.obj"), BASE_OBJ);
        write(&run_dir.join("source/lid.obj"), LID_OBJ);
        write(
            &run_dir.join("package/package.json"),
            &package_json("front", [0.0, -4.0, 0.0]),
        );
        write(&run_dir.join("package/masks/front.png"), "mask");
        write(
            &run_dir.join("report.json"),
            &serde_json::json!({
                "schema": "michelangelo.reconstruction-report/0",
                "package": {"package_id": "demo-lantern", "revision": 1},
                "quality": {
                    "thresholds": {
                        "source_mean_iou": 0.9,
                        "source_min_iou": 0.85,
                        "evaluation_mean_iou": 0.8,
                        "evaluation_min_iou": 0.7,
                        "bbox_relative_error_max": 0.05
                    }
                }
            })
            .to_string(),
        );
        init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
            .expect("the scene is created");

        let candidate_dir = directory.path().join("cand-0001");
        write(&candidate_dir.join("candidate.json"), &candidate_json());
        write(&candidate_dir.join("geometry/lid.obj"), LID_OBJ);
        michelangelo_edit::apply::submit_candidate(
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
}

/// Write a file and create its parent directories.
fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent directory is created");
    }
    std::fs::write(path, content).expect("the file is written");
}

/// The renderer that writes a placeholder for every render.
#[derive(Default)]
struct FakeRenderer {
    renders: std::sync::Mutex<usize>,
}

impl FakeRenderer {
    /// The number of the rendered geometries.
    fn renders(&self) -> usize {
        *self.renders.lock().expect("the counter is not poisoned")
    }
}

impl Renderer for FakeRenderer {
    fn render(&self, request: &RenderRequest<'_>) -> Result<(), EvalError> {
        for view in &request.views.views {
            let path = request.renders_dir.join(format!("{}.png", view.view_id));
            std::fs::write(&path, b"render").map_err(|err| {
                EvalError::io(&path, format!("the render cannot be written: {err}"))
            })?;
        }
        let mut counter = self.renders.lock().expect("the counter is not poisoned");
        *counter += 1;
        Ok(())
    }
}

/// The comparator that measures a fixed IoU.
struct FakeComparator {
    target_iou: f64,
    base_iou: f64,
    within_tolerance: Option<bool>,
}

impl Comparator for FakeComparator {
    fn compare(&self, request: &CompareRequest<'_>) -> Result<CompareOutcome, EvalError> {
        let is_target = request.renders_dir.to_string_lossy().contains("target");
        let iou = if is_target {
            self.target_iou
        } else {
            self.base_iou
        };
        let views: Vec<CompareView> = request
            .mask_paths
            .iter()
            .map(|(view_id, _)| CompareView {
                view_id: view_id.clone(),
                iou,
                bbox_error_px: Some(0.0),
            })
            .collect();
        let source = CompareSet {
            views,
            mean_iou: iou,
            min_iou: iou,
        };
        let evaluation = request.evaluation.map(|_| CompareSet {
            views: vec![CompareView {
                view_id: "held_out".to_string(),
                iou: self.target_iou,
                bbox_error_px: Some(0.0),
            }],
            mean_iou: self.target_iou,
            min_iou: self.target_iou,
        });
        let measurements = vec![CompareMeasurement {
            id: "height".to_string(),
            declared: Some(0.25),
            measured: Some(0.25),
            absolute_error: Some(0.0),
            relative_error: Some(0.0),
            within_tolerance: self.within_tolerance,
        }];
        Ok(CompareOutcome {
            source,
            evaluation,
            measurements,
            diagnostics: Vec::new(),
        })
    }
}

/// The options of the tests.
fn options() -> EvaluationOptions {
    EvaluationOptions::default()
}

#[test]
fn an_evaluation_of_a_candidate_passes_and_is_registered() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    assert_eq!(report.schema, "michelangelo.evaluation-report/0");
    assert_eq!(report.evaluation_id, "ev-0001");
    assert_eq!(report.target.id, "cand-0001");
    assert_eq!(report.results.projection_conformance, TriState::Pass);
    assert_eq!(report.results.author_geometric_conditions, TriState::Pass);
    assert_eq!(
        report.results.independent_ground_truth,
        TriState::Unavailable,
        "the absence of a ground truth is never a pass"
    );
    assert_eq!(report.verdict(), TriState::Pass);
    assert_eq!(
        renderer.renders(),
        2,
        "the target and the base are rendered"
    );
    let text = report.to_json().expect("the report is serialized");
    assert!(!text.contains(&fixture.run_dir.display().to_string()));

    // The report is registered with the digest of the written bytes.
    let registry =
        michelangelo_evidence::registry::load_registry(&fixture.run_dir).expect("the registry");
    let entry = registry
        .entry("ev-0001")
        .expect("the evaluation is registered");
    let path = fixture.run_dir.join(&entry.report_path);
    let bytes = std::fs::read(&path).expect("the report is readable");
    assert_eq!(
        entry.sha256,
        michelangelo_evidence::digest::sha256_hex(&bytes)
    );
}

#[test]
fn the_evaluation_pins_the_inputs_of_the_run() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    let package = fixture.run_dir.join("package/package.json");
    let digest =
        michelangelo_evidence::digest::file_sha256(&package).expect("the digest is computed");
    assert_eq!(report.inputs.package_sha256, Some(digest));
    assert_eq!(report.inputs.views.len(), 1);
    assert_eq!(report.inputs.views[0].view_id, "front");
    assert!(!report.inputs.views[0].held_out);
    assert_eq!(report.inputs.evaluation_package_sha256, None);
}

#[test]
fn a_regression_of_a_source_view_fails_the_evaluation() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.80,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    assert_eq!(report.visible.views.len(), 1);
    let regression = report.visible.views[0]
        .regression
        .expect("the regression is measured");
    assert!((regression - 0.15).abs() < 1e-9);
    assert_eq!(report.verdict(), TriState::Fail);
}

#[test]
fn a_mean_below_the_threshold_fails_the_evaluation() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.50,
        base_iou: 0.50,
        within_tolerance: Some(true),
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    assert_eq!(report.results.projection_conformance, TriState::Fail);
    assert_eq!(report.visible.verdict, TriState::Fail);
    assert_eq!(report.verdict(), TriState::Fail);
}

#[test]
fn the_hidden_claims_stay_unverified() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    assert_eq!(report.hidden.claims.len(), 1);
    assert_eq!(report.hidden.claims[0].id, "u_interior");
    assert_eq!(report.hidden.claims[0].status, ClaimStatus::Unverified);
    assert!(!report.hidden.claims[0].requires_ground_truth);
}

#[test]
fn the_author_conditions_come_from_the_package() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    let condition = report
        .check("height")
        .expect("the declared measurement is checked");
    assert_eq!(condition.kind, "author_geometric_condition");
    assert!(condition.required);
    assert_eq!(condition.status, TriState::Pass);
}

#[test]
fn an_unmeasurable_author_condition_is_never_a_pass() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: None,
    };
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    let condition = report
        .check("height")
        .expect("the declared measurement is checked");
    assert_eq!(condition.status, TriState::Unavailable);
    assert_eq!(
        report.results.author_geometric_conditions,
        TriState::Unavailable
    );
    assert_eq!(report.verdict(), TriState::Unavailable);
}

#[test]
fn a_run_without_a_package_copy_cannot_be_evaluated() {
    let fixture = Fixture::new();
    std::fs::remove_dir_all(fixture.run_dir.join("package")).expect("the copy is removed");
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let error = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect_err("the evaluation is refused");
    assert_eq!(error.code(), "EVALUATION_INPUT_MISSING");
}

#[test]
fn a_candidate_without_a_snapshot_cannot_be_evaluated() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let error = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-9999",
        &options(),
        &renderer,
        &comparator,
    )
    .expect_err("the evaluation is refused");
    assert_eq!(error.code(), "EVALUATION_INPUT_MISSING");
}

#[test]
fn the_held_out_cameras_must_be_distinct_from_the_source_cameras() {
    let fixture = Fixture::new();
    let directory = tempfile::tempdir().expect("a temporary directory");
    let evaluation_package = directory.path().join("evaluation");
    write(
        &evaluation_package.join("package.json"),
        &package_json("front", [0.0, -4.0, 0.0]),
    );
    write(&evaluation_package.join("masks/front.png"), "mask");

    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let mut options = options();
    options.evaluation_package = Some(evaluation_package);
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options,
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    let check = report
        .check("held_out")
        .expect("the held-out set is reported");
    assert_eq!(check.status, TriState::Unavailable);
    assert!(check.detail.contains("not distinct"));
    assert_eq!(report.inputs.evaluation_package_sha256, None);
    assert_eq!(renderer.renders(), 2, "the held-out views are not rendered");
}

#[test]
fn opposite_orthographic_held_out_cameras_are_allowed() {
    // A view looking in the opposite direction (0, +4, 0) is the mirrored
    // silhouette of the front source view: it is an independent constraint and
    // the held-out controls of the proof datasets use exactly this form.
    let fixture = Fixture::new();
    let directory = tempfile::tempdir().expect("a temporary directory");
    let evaluation_package = directory.path().join("evaluation");
    write(
        &evaluation_package.join("package.json"),
        &package_json("back", [0.0, 4.0, 0.0]),
    );
    write(&evaluation_package.join("masks/back.png"), "mask");

    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let mut options = options();
    options.evaluation_package = Some(evaluation_package);
    let report = evaluate_candidate_with(
        &fixture.run_dir,
        "cand-0001",
        &options,
        &renderer,
        &comparator,
    )
    .expect("the candidate is evaluated");

    let held_out = report
        .visible
        .held_out
        .as_ref()
        .expect("the held-out set is evaluated");
    assert_eq!(held_out.views[0].view_id, "held_out");
    assert!(report.inputs.evaluation_package_sha256.is_some());
    let check = report.check("held_out").expect("the held-out check exists");
    assert_eq!(check.status, TriState::Pass);
}

#[test]
fn a_revision_is_evaluated_against_its_base() {
    let fixture = Fixture::new();
    let renderer = FakeRenderer::default();
    let comparator = FakeComparator {
        target_iou: 0.95,
        base_iou: 0.95,
        within_tolerance: Some(true),
    };
    let report = evaluate_revision_with(
        &fixture.run_dir,
        "s0001",
        &options(),
        &renderer,
        &comparator,
    )
    .expect("the revision is evaluated");
    assert_eq!(report.target.id, "s0001");
    assert_eq!(report.verdict(), TriState::Pass);
}
