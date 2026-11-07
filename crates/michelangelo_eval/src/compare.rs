//! The comparator of the rendered geometry.
//!
//! The comparator is injected so that the evaluation can be tested without
//! Blender and without Python: the production implementation runs the pinned
//! comparator of the repository (tools/solver/compare.py), the tests use a
//! deterministic fake.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use michelangelo_recon::solver::{self, ToolPaths};
use serde_json::Value;

use crate::error::EvalError;

/// One measured view of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareView {
    /// The identifier of the view.
    pub view_id: String,
    /// The measured IoU of the view.
    pub iou: f64,
    /// The error of the bounding box of the render, in pixels (None when it
    /// could not be measured).
    pub bbox_error_px: Option<f64>,
}

/// The measured result of a set of views.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareSet {
    /// The measured views, sorted by the identifier of the view.
    pub views: Vec<CompareView>,
    /// The mean IoU of the views.
    pub mean_iou: f64,
    /// The smallest IoU of the views.
    pub min_iou: f64,
}

/// One declared measurement and its measured value.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareMeasurement {
    /// The identifier of the measurement.
    pub id: String,
    /// The declared value, in metres.
    pub declared: Option<f64>,
    /// The measured value, in metres.
    pub measured: Option<f64>,
    /// The absolute error, in metres.
    pub absolute_error: Option<f64>,
    /// The relative error.
    pub relative_error: Option<f64>,
    /// Whether the measured value is inside the declared tolerance.
    pub within_tolerance: Option<bool>,
}

/// The outcome of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareOutcome {
    /// The measured source views.
    pub source: CompareSet,
    /// The measured held-out views (None when they were not compared).
    pub evaluation: Option<CompareSet>,
    /// The declared measurements and their measured values.
    pub measurements: Vec<CompareMeasurement>,
    /// The diagnostics of the comparator.
    pub diagnostics: Vec<String>,
}

/// A request to compare the renders of a geometry with the masks.
pub struct CompareRequest<'a> {
    /// The directory of the package that carries the masks.
    pub package_dir: &'a Path,
    /// The directory of the renders.
    pub renders_dir: &'a Path,
    /// The directory of the quality report of the comparison.
    pub out_dir: &'a Path,
    /// The held-out package and the directory of its renders.
    pub evaluation: Option<(&'a Path, &'a Path)>,
    /// The masks of the views, with the path of the mask file.
    pub mask_paths: &'a [(String, PathBuf)],
}

/// The comparator of the renders.
pub trait Comparator {
    /// Compare the renders of the request with the pinned masks.
    fn compare(&self, request: &CompareRequest<'_>) -> Result<CompareOutcome, EvalError>;
}

/// The comparator that runs the pinned comparator of the repository.
pub struct ScriptComparator {
    paths: ToolPaths,
    timeout_ms: Option<u64>,
    bbox_errors: bool,
}

impl ScriptComparator {
    /// The comparator that runs tools/solver/compare.py.
    pub fn new(
        solver: Option<&Path>,
        python: Option<&Path>,
        timeout_ms: Option<u64>,
    ) -> Result<Self, EvalError> {
        let paths =
            solver::resolve_tool_paths(solver, python).map_err(|err| EvalError::Missing {
                message: format!("the comparator of the repository is not available: {err}"),
            })?;
        Ok(ScriptComparator {
            paths,
            timeout_ms,
            bbox_errors: true,
        })
    }

    /// The comparator that does not measure the error of the bounding boxes.
    pub fn without_bbox_errors(mut self) -> Self {
        self.bbox_errors = false;
        self
    }
}

impl Comparator for ScriptComparator {
    fn compare(&self, request: &CompareRequest<'_>) -> Result<CompareOutcome, EvalError> {
        let run = solver::run_compare(
            &self.paths,
            request.package_dir,
            request.renders_dir,
            request.out_dir,
            request.evaluation,
            self.timeout_ms,
        )
        .map_err(|err| EvalError::Compare {
            message: format!("the comparator failed: {err}"),
        })?;

        let source = compare_set(run.report.get("source"));
        let evaluation = match run.report.get("evaluation") {
            Some(value) if !value.is_null() => Some(compare_set(Some(value))),
            _ => None,
        };
        let measurements = compare_measurements(run.report.get("measurements"));
        let mut diagnostics: Vec<String> = Vec::new();
        if let Some(entries) = run
            .report
            .get("diagnostics")
            .and_then(|value| value.as_array())
        {
            for entry in entries {
                let code = entry
                    .get("code")
                    .and_then(|value| value.as_str())
                    .unwrap_or("DIAGNOSTIC");
                let message = entry
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                diagnostics.push(format!("{code}: {message}"));
            }
        }

        let mut outcome = CompareOutcome {
            source,
            evaluation,
            measurements,
            diagnostics,
        };
        if self.bbox_errors {
            let errors = self.measure_bbox_errors(request);
            for view in outcome.source.views.iter_mut() {
                view.bbox_error_px = errors
                    .iter()
                    .find(|(id, _)| *id == view.view_id)
                    .and_then(|(_, value)| *value);
            }
        }
        Ok(outcome)
    }
}

impl ScriptComparator {
    /// Measure the error of the bounding box of every render.
    ///
    /// The measurement uses the PNG reader of the repository: the pinned
    /// comparator does not report the bounding box of a render.
    fn measure_bbox_errors(&self, request: &CompareRequest<'_>) -> Vec<(String, Option<f64>)> {
        let tools = self
            .paths
            .compare_script
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut spec: Vec<Value> = Vec::new();
        for (view_id, mask_path) in request.mask_paths {
            let render = request.renders_dir.join(format!("{view_id}.png"));
            spec.push(serde_json::json!({
                "view_id": view_id,
                "mask": mask_path,
                "render": render,
            }));
        }
        if spec.is_empty() {
            return Vec::new();
        }
        let payload = serde_json::to_string(&spec).unwrap_or_else(|_| "[]".to_string());
        let mut child = match Command::new(&self.paths.python)
            .arg("-c")
            .arg(BBOX_SCRIPT)
            .arg(&tools)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return Vec::new(),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.as_bytes());
        }
        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(_) => return Vec::new(),
        };
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let parsed: Vec<Value> = match serde_json::from_str(text.trim()) {
            Ok(parsed) => parsed,
            Err(_) => return Vec::new(),
        };
        parsed
            .iter()
            .map(|entry| {
                let view_id = entry
                    .get("view_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let error = entry.get("bbox_error_px").and_then(|value| value.as_f64());
                (view_id, error)
            })
            .collect()
    }
}

/// The Python snippet that measures the error of the bounding boxes.
const BBOX_SCRIPT: &str = r#"
import json
import sys

sys.path.insert(0, sys.argv[1])
import pngread

spec = json.load(sys.stdin)
result = []
for item in spec:
    entry = {"view_id": item["view_id"], "bbox_error_px": None}
    try:
        mask = pngread.mask_from_png(item["mask"])
        render = pngread.mask_from_png(item["render"])
        mask_box = mask.bbox()
        render_box = render.bbox()
        if mask_box is not None and render_box is not None:
            error = max(abs(mask_box[i] - render_box[i]) for i in range(4))
            entry["bbox_error_px"] = float(error)
    except Exception as exc:  # noqa: BLE001 - the error is reported as None
        entry["error"] = str(exc)
    result.append(entry)
print(json.dumps(result))
"#;

/// Read a measured set of views of a quality report.
fn compare_set(value: Option<&Value>) -> CompareSet {
    let mut views: Vec<CompareView> = Vec::new();
    if let Some(entries) = value
        .and_then(|value| value.get("views"))
        .and_then(|value| value.as_array())
    {
        for entry in entries {
            let view_id = entry
                .get("view_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            let iou = entry
                .get("iou")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            views.push(CompareView {
                view_id,
                iou,
                bbox_error_px: None,
            });
        }
    }
    views.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    let mean_iou = value
        .and_then(|value| value.get("mean_iou"))
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let min_iou = value
        .and_then(|value| value.get("min_iou"))
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    CompareSet {
        views,
        mean_iou,
        min_iou,
    }
}

/// Read the measured measurements of a quality report.
fn compare_measurements(value: Option<&Value>) -> Vec<CompareMeasurement> {
    let mut measurements: Vec<CompareMeasurement> = Vec::new();
    let entries = match value.and_then(|value| value.as_array()) {
        Some(entries) => entries,
        None => return measurements,
    };
    for entry in entries {
        measurements.push(CompareMeasurement {
            id: entry
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            declared: entry.get("declared").and_then(|value| value.as_f64()),
            measured: entry.get("measured").and_then(|value| value.as_f64()),
            absolute_error: entry.get("absolute_error").and_then(|value| value.as_f64()),
            relative_error: entry.get("relative_error").and_then(|value| value.as_f64()),
            within_tolerance: entry
                .get("within_tolerance")
                .and_then(|value| value.as_bool()),
        });
    }
    measurements.sort_by(|left, right| left.id.cmp(&right.id));
    measurements
}
