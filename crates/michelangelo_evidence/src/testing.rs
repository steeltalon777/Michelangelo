//! Test support: a deterministic synthetic evaluation report.
//!
//! The helper builds the report of an evaluation that **passes** the accept
//! gate for a candidate of a store: the digests, the pins and the results
//! are taken from the store itself, so the report is consistent with the
//! state of the run.
//!
//! The helper is public so that the integration tests of the CLI crate can
//! reuse it; it is never used by the production code paths.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::digest::file_sha256;
use crate::error::EvidenceError;
use crate::report::{
    package_mask_paths, package_measurement_ids, package_views, AuthorDecision, EvaluationCheck,
    EvaluationInputs, EvaluationReport, EvaluationResults, EvaluationTarget, EvaluationThresholds,
    GroundTruthState, HiddenSection, ReportIntegrity, TargetKind, TriState, ViewPin,
    VisibleSection, VisibleView, EVALUATION_REPORT_SCHEMA,
};

/// The IoU the synthetic report claims for every view.
pub const SYNTHETIC_IOU: f64 = 0.99;

/// Build a synthetic evaluation report of a candidate that passes the gate.
///
/// The report is pinned to the snapshot of the candidate, to the copy of the
/// input package of the run and to the base revision of the candidate.
pub fn synthetic_evaluation(
    run_dir: &Path,
    candidate_id: &str,
    evaluation_id: &str,
    created_at: &str,
) -> Result<EvaluationReport, EvidenceError> {
    let manifest_path = run_dir
        .join("verification")
        .join("snapshots")
        .join(candidate_id)
        .join("manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path).map_err(|err| {
        EvidenceError::io(
            &manifest_path,
            format!("the snapshot manifest cannot be read: {err}"),
        )
    })?;
    let manifest: Value =
        serde_json::from_str(&manifest_text).map_err(|err| EvidenceError::Invalid {
            message: format!("the snapshot manifest is not usable: {err}"),
        })?;
    let aggregate_sha256 = manifest
        .get("aggregate_sha256")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let base_scene_revision = manifest
        .get("base")
        .and_then(|base| base.get("scene_revision"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    let package_path = run_dir.join("package").join("package.json");
    let package_dir = run_dir.join("package");
    let (package_sha256, views, measurements) = if package_path.is_file() {
        let digest = file_sha256(&package_path)?;
        let text = std::fs::read_to_string(&package_path).map_err(|err| {
            EvidenceError::io(
                &package_path,
                format!("the package copy cannot be read: {err}"),
            )
        })?;
        let package: Value = serde_json::from_str(&text).map_err(|err| EvidenceError::Invalid {
            message: format!("the package copy is not usable: {err}"),
        })?;
        let mut views = package_views(&package)
            .iter()
            .map(|view| view.pin(false))
            .collect::<Vec<ViewPin>>();
        // The pin is the digest of the bytes of the mask file the run really
        // carries: a mask that changes after the evaluation is detected.
        for (view_id, relative) in package_mask_paths(&package) {
            let path = package_dir.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            let actual = match file_sha256(&path) {
                Ok(actual) => actual,
                Err(_) => continue,
            };
            for view in views.iter_mut() {
                if view.view_id == view_id {
                    view.mask_sha256 = actual.clone();
                }
            }
        }
        let measurements = package_measurement_ids(&package);
        (Some(digest), views, measurements)
    } else {
        (None, Vec::new(), Vec::new())
    };

    let visible_views: Vec<VisibleView> = views
        .iter()
        .map(|view| VisibleView {
            view_id: view.view_id.clone(),
            iou: SYNTHETIC_IOU,
            bbox_error_px: Some(0.0),
            base_iou: Some(SYNTHETIC_IOU),
            regression: Some(0.0),
        })
        .collect();

    let mut checks: Vec<EvaluationCheck> = measurements
        .iter()
        .map(|id| EvaluationCheck {
            id: id.clone(),
            kind: crate::gate::AUTHOR_CONDITION_KIND.to_string(),
            required: true,
            status: TriState::Pass,
            detail: String::from("the synthetic report verifies the declared measurement"),
        })
        .collect();
    checks.push(EvaluationCheck {
        id: String::from("ground_truth"),
        kind: String::from("ground_truth"),
        required: false,
        status: TriState::Unavailable,
        detail: String::from(
            "the synthetic report carries no independent ground truth: the absence is never a pass",
        ),
    });

    let mut tool_versions: BTreeMap<String, String> = BTreeMap::new();
    tool_versions.insert(
        String::from("michelangelo"),
        String::from(env!("CARGO_PKG_VERSION")),
    );
    tool_versions.insert(
        String::from("evaluation_schema"),
        EVALUATION_REPORT_SCHEMA.to_string(),
    );

    Ok(EvaluationReport {
        schema: EVALUATION_REPORT_SCHEMA.to_string(),
        evaluation_id: evaluation_id.to_string(),
        created_at: created_at.to_string(),
        run_id: run_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("run")),
        target: EvaluationTarget {
            kind: TargetKind::Candidate,
            id: candidate_id.to_string(),
            aggregate_sha256,
            base_scene_revision,
        },
        inputs: EvaluationInputs {
            package_sha256,
            views,
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
            views: visible_views,
            mean_iou: SYNTHETIC_IOU,
            min_iou: SYNTHETIC_IOU,
            base_mean_iou: Some(SYNTHETIC_IOU),
            verdict: TriState::Pass,
            held_out: None,
        },
        hidden: HiddenSection {
            ground_truth: GroundTruthState::Absent,
            claims: Vec::new(),
            verdict: AuthorDecision::Pending,
        },
        checks,
        acceptance_basis: vec![
            String::from("projection_conformance=pass"),
            String::from("author_geometric_conditions=pass"),
        ],
        tool_versions,
        integrity: ReportIntegrity::default(),
    })
}
