//! The data transfer objects of the evaluation report (contract §6).
//!
//! The report is the immutable evidence of one evaluation. It never
//! self-certifies: the Core registry (`verification/evaluations/index.json`)
//! holds the digest of the file, and the accept gate re-computes it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::digest::camera_digest;
use crate::error::EvidenceError;

/// The schema of an evaluation report.
pub const EVALUATION_REPORT_SCHEMA: &str = "michelangelo.evaluation-report/0";

/// The kind of the geometry an evaluation measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// The full post-change result of a candidate (its snapshot).
    Candidate,
    /// A published scene revision.
    Revision,
}

impl TargetKind {
    /// The kind as the text used in the report.
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetKind::Candidate => "candidate",
            TargetKind::Revision => "revision",
        }
    }
}

/// A result with three states: measured and good, measured and bad, or not
/// measurable at all.
///
/// A missing measurement is never a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriState {
    /// The check was made and it succeeded.
    Pass,
    /// The check was made and it failed.
    Fail,
    /// The check could not be made (the data is missing).
    Unavailable,
}

impl TriState {
    /// The state as the text used in the report.
    pub fn as_str(&self) -> &'static str {
        match self {
            TriState::Pass => "pass",
            TriState::Fail => "fail",
            TriState::Unavailable => "unavailable",
        }
    }
}

/// The decision of the author about a hidden shape.
///
/// A decision is not a measurement: it never turns an unavailable measurement
/// into a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorDecision {
    /// The author accepted the hidden shape.
    Accepted,
    /// The author rejected the hidden shape.
    Rejected,
    /// The author has not decided yet.
    Pending,
}

impl AuthorDecision {
    /// The decision as the text used in the report.
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthorDecision::Accepted => "accepted",
            AuthorDecision::Rejected => "rejected",
            AuthorDecision::Pending => "pending",
        }
    }
}

/// Whether an independent ground truth was supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundTruthState {
    /// No independent ground truth was supplied.
    Absent,
    /// An independent ground truth was supplied.
    Present,
}

/// The status of a claim about a hidden shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    /// The claim was verified with evidence.
    Verified,
    /// The claim has no resolvable evidence yet.
    Unverified,
    /// The claim was checked and it failed.
    Failed,
}

/// The calibration of a view (the fields of the camera digest).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraDigest {
    /// The identifier of the view.
    pub view_id: String,
    /// The location of the camera, in package units.
    pub location_m: [f64; 3],
    /// The point the camera looks at, in package units.
    pub look_at_m: [f64; 3],
    /// The world space up vector of the camera.
    pub up_world: [f64; 3],
    /// The visible world width of the image.
    pub ortho_scale_m: f64,
    /// The horizontal lens shift.
    pub shift_x: f64,
    /// The vertical lens shift.
    pub shift_y: f64,
    /// The near clip distance.
    pub clip_start_m: f64,
    /// The far clip distance.
    pub clip_end_m: f64,
    /// The width of the image, in pixels.
    pub width_px: u32,
    /// The height of the image, in pixels.
    pub height_px: u32,
}

/// The pinned calibration of a view of an evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewPin {
    /// The identifier of the view.
    pub view_id: String,
    /// The digest of the calibration of the view.
    pub camera_digest: String,
    /// The digest of the mask of the view.
    pub mask_sha256: String,
    /// The resolution of the view, in pixels.
    pub resolution: [u32; 2],
    /// Whether the view belongs to the held-out set.
    pub held_out: bool,
}

/// The geometry an evaluation measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationTarget {
    /// The kind of the target.
    pub kind: TargetKind,
    /// The identifier of the target (a candidate id or a scene revision).
    pub id: String,
    /// The aggregate digest of the component set of the target.
    pub aggregate_sha256: String,
    /// The scene revision the target was built on (None when it is unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_scene_revision: Option<String>,
}

/// The pinned inputs of an evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationInputs {
    /// The digest of the copy of the input package of the run.
    pub package_sha256: Option<String>,
    /// The pinned views, sorted by the identifier of the view.
    pub views: Vec<ViewPin>,
    /// The digest of the held-out evaluation package (None when there is none).
    pub evaluation_package_sha256: Option<String>,
}

/// The thresholds an evaluation applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationThresholds {
    /// The required mean IoU of the source views.
    pub source_mean_iou: f64,
    /// The required smallest IoU of the source views.
    pub source_min_iou: f64,
    /// The required mean IoU of the held-out views.
    pub evaluation_mean_iou: f64,
    /// The required smallest IoU of the held-out views.
    pub evaluation_min_iou: f64,
    /// The largest accepted relative error of a declared measurement.
    pub bbox_relative_error_max: f64,
    /// The largest accepted drop of the IoU of a source view against the base.
    pub per_view_regression_max: f64,
}

impl Default for EvaluationThresholds {
    fn default() -> Self {
        EvaluationThresholds {
            source_mean_iou: 0.90,
            source_min_iou: 0.85,
            evaluation_mean_iou: 0.80,
            evaluation_min_iou: 0.70,
            bbox_relative_error_max: 0.05,
            per_view_regression_max: 0.01,
        }
    }
}

/// The four independent results of an evaluation.
///
/// The results are never collapsed into one boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationResults {
    /// The conformance of the projections against the pinned views.
    pub projection_conformance: TriState,
    /// The measurable conditions the author declared.
    pub author_geometric_conditions: TriState,
    /// The comparison against an independent ground truth.
    pub independent_ground_truth: TriState,
    /// The decision of the author about a hidden shape.
    pub author_hidden_decision: AuthorDecision,
}

/// The measured IoU of one view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibleView {
    /// The identifier of the view.
    pub view_id: String,
    /// The IoU of the target against the mask of the view.
    pub iou: f64,
    /// The error of the bounding box of the render, in pixels (None when the
    /// pinned comparator does not report it).
    pub bbox_error_px: Option<f64>,
    /// The IoU of the base revision against the same mask (None when the base
    /// could not be measured).
    pub base_iou: Option<f64>,
    /// The drop of the IoU against the base revision.
    pub regression: Option<f64>,
}

/// The measured result of a set of views.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibleSet {
    /// The measured views, sorted by the identifier of the view.
    pub views: Vec<VisibleView>,
    /// The mean IoU of the views.
    pub mean_iou: f64,
    /// The smallest IoU of the views.
    pub min_iou: f64,
}

/// The visible (projected) part of an evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibleSection {
    /// The measured source views.
    pub views: Vec<VisibleView>,
    /// The mean IoU of the source views.
    pub mean_iou: f64,
    /// The smallest IoU of the source views.
    pub min_iou: f64,
    /// The mean IoU of the base revision against the same masks.
    pub base_mean_iou: Option<f64>,
    /// The verdict of the visible part (the absolute thresholds and the
    /// per-view regression).
    pub verdict: TriState,
    /// The measured held-out views (None when there is no held-out set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_out: Option<VisibleSet>,
}

/// One claim about a hidden shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiddenClaim {
    /// The identifier of the claim (the identifier of the unknown).
    pub id: String,
    /// The kind of the claim (an unknown, a supplement, ...).
    pub kind: String,
    /// The status of the claim.
    pub status: ClaimStatus,
    /// The references to the evidence of the claim.
    pub evidence_refs: Vec<String>,
    /// Whether the claim needs an independent ground truth to be verified.
    #[serde(default)]
    pub requires_ground_truth: bool,
}

/// The hidden part of an evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiddenSection {
    /// Whether an independent ground truth was supplied.
    pub ground_truth: GroundTruthState,
    /// The claims about the hidden shapes.
    pub claims: Vec<HiddenClaim>,
    /// The verdict of the hidden part.
    pub verdict: AuthorDecision,
}

/// One check of an evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationCheck {
    /// The identifier of the check (the identifier of the condition).
    pub id: String,
    /// The kind of the check (for example `author_geometric_condition`).
    pub kind: String,
    /// Whether the check is required for the acceptance.
    pub required: bool,
    /// The status of the check.
    pub status: TriState,
    /// The human readable detail of the check.
    pub detail: String,
}

/// The integrity block of a report.
///
/// The report never self-certifies: the Core registry holds its digest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportIntegrity {
    /// The digest of the schema of the report (None when it is not pinned).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_digest: Option<String>,
    /// The note that states that the digest lives in the Core registry.
    pub note: String,
}

impl Default for ReportIntegrity {
    fn default() -> Self {
        ReportIntegrity {
            schema_digest: None,
            note: String::from(
                "the report never self-certifies: the Core registry holds the digest of this file",
            ),
        }
    }
}

/// The immutable report of one evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationReport {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the evaluation.
    pub evaluation_id: String,
    /// The creation time of the report (RFC 3339, UTC).
    pub created_at: String,
    /// The identifier of the run the evaluation belongs to.
    pub run_id: String,
    /// The geometry the evaluation measured.
    pub target: EvaluationTarget,
    /// The pinned inputs of the evaluation.
    pub inputs: EvaluationInputs,
    /// The thresholds the evaluation applied.
    pub thresholds: EvaluationThresholds,
    /// The four independent results.
    pub results: EvaluationResults,
    /// The visible (projected) part of the evaluation.
    pub visible: VisibleSection,
    /// The hidden part of the evaluation.
    pub hidden: HiddenSection,
    /// The checks of the evaluation, sorted by the identifier of the check.
    pub checks: Vec<EvaluationCheck>,
    /// The bases of the acceptance (the reasons the evaluation may be used).
    pub acceptance_basis: Vec<String>,
    /// The versions of the tools that produced the evaluation.
    pub tool_versions: BTreeMap<String, String>,
    /// The integrity block of the report.
    pub integrity: ReportIntegrity,
}

impl EvaluationReport {
    /// The JSON representation of the report (pretty, deterministic).
    pub fn to_json(&self) -> Result<String, EvidenceError> {
        serde_json::to_string_pretty(self).map_err(|err| EvidenceError::Invalid {
            message: format!("the evaluation report cannot be serialized: {err}"),
        })
    }

    /// Read a report from a file.
    pub fn load(path: &std::path::Path) -> Result<EvaluationReport, EvidenceError> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| EvidenceError::io(path, format!("the report cannot be read: {err}")))?;
        serde_json::from_str(&text).map_err(|err| EvidenceError::Invalid {
            message: format!(
                "the evaluation report '{}' is not usable: {err}",
                path.display()
            ),
        })
    }

    /// The verdict of the whole evaluation.
    ///
    /// The verdict is a summary: the independent results stay independent in
    /// the report.
    pub fn verdict(&self) -> TriState {
        if self.results.projection_conformance != TriState::Pass {
            return self.results.projection_conformance;
        }
        if self.results.author_geometric_conditions != TriState::Pass {
            return self.results.author_geometric_conditions;
        }
        let limit = self.thresholds.per_view_regression_max;
        for view in &self.visible.views {
            match view.regression {
                Some(regression) if regression > limit => return TriState::Fail,
                Some(_) => {}
                None => return TriState::Unavailable,
            }
        }
        for claim in &self.hidden.claims {
            if claim.requires_ground_truth && claim.status != ClaimStatus::Verified {
                return TriState::Unavailable;
            }
        }
        TriState::Pass
    }

    /// The check of an identifier.
    pub fn check(&self, id: &str) -> Option<&EvaluationCheck> {
        self.checks.iter().find(|check| check.id == id)
    }
}

/// One calibrated view of an input package.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageView {
    /// The identifier of the view.
    pub view_id: String,
    /// The calibration of the view.
    pub camera: CameraDigest,
    /// The digest of the mask of the view.
    pub mask_sha256: String,
}

impl PackageView {
    /// The pinned view of an evaluation.
    pub fn pin(&self, held_out: bool) -> ViewPin {
        ViewPin {
            view_id: self.camera.view_id.clone(),
            camera_digest: camera_digest(&self.camera),
            mask_sha256: self.mask_sha256.clone(),
            resolution: [self.camera.width_px, self.camera.height_px],
            held_out,
        }
    }
}

/// The calibrated views of an input package, sorted by the identifier.
pub fn package_views(package: &serde_json::Value) -> Vec<PackageView> {
    let mut views: Vec<PackageView> = Vec::new();
    let entries = match package.get("views").and_then(|value| value.as_array()) {
        Some(entries) => entries,
        None => return views,
    };
    for entry in entries {
        let kind = entry
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("calibrated");
        if kind != "calibrated" {
            continue;
        }
        let view_id = entry
            .get("view_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let projection = match entry.get("projection") {
            Some(projection) => projection,
            None => continue,
        };
        let image = entry.get("image");
        let width_px = image
            .and_then(|image| image.get("width_px"))
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32;
        let height_px = image
            .and_then(|image| image.get("height_px"))
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32;
        let camera = CameraDigest {
            view_id: view_id.clone(),
            location_m: number_triple(projection.get("location_m")),
            look_at_m: number_triple(projection.get("look_at_m")),
            up_world: number_triple(projection.get("up_world")),
            ortho_scale_m: projection
                .get("ortho_scale_m")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
            shift_x: projection
                .get("shift_x")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
            shift_y: projection
                .get("shift_y")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
            clip_start_m: projection
                .get("clip_start_m")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.1),
            clip_end_m: projection
                .get("clip_end_m")
                .and_then(|value| value.as_f64())
                .unwrap_or(1000.0),
            width_px,
            height_px,
        };
        let mask_sha256 = entry
            .get("mask")
            .and_then(|mask| mask.get("sha256"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        views.push(PackageView {
            view_id,
            camera,
            mask_sha256,
        });
    }
    views.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    views
}

/// The identifiers of the declared measurements of an input package.
///
/// The measurements are the author-accepted geometric data of the package:
/// they are the source of the required author geometric conditions.
pub fn package_measurement_ids(package: &serde_json::Value) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let entries = match package
        .get("measurements")
        .and_then(|value| value.as_array())
    {
        Some(entries) => entries,
        None => return ids,
    };
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if !id.is_empty() {
            ids.push(id);
        }
    }
    ids.sort();
    ids
}

/// The identifiers of the unknowns of an input package.
pub fn package_unknown_ids(package: &serde_json::Value) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let entries = match package.get("unknowns").and_then(|value| value.as_array()) {
        Some(entries) => entries,
        None => return ids,
    };
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if !id.is_empty() {
            ids.push(id);
        }
    }
    ids.sort();
    ids
}

/// Read three numbers of a JSON value (zeros when the value is missing).
fn number_triple(value: Option<&serde_json::Value>) -> [f64; 3] {
    let mut result = [0.0_f64; 3];
    if let Some(entries) = value.and_then(|value| value.as_array()) {
        for (index, entry) in entries.iter().take(3).enumerate() {
            if let Some(number) = entry.as_f64() {
                result[index] = number;
            }
        }
    }
    result
}
