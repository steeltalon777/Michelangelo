//! The reconstruction report (`michelangelo.reconstruction-report/0`).
//!
//! The report is the stable machine-readable result of a reconstruction run
//! or of a scoring pass. The top-level keys are fixed:
//!
//! `schema`, `run`, `package`, `solver`, `build`, `worker`,
//! `quality`, `status`, `unresolved`, `diagnostics`.
//!
//! The report never contains an absolute path: the run directory, the
//! artifacts and the renders are all workspace-relative.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::ReconError;

/// Schema identifier of the reconstruction report.
pub const REPORT_SCHEMA: &str = "michelangelo.reconstruction-report/0";

/// The overall status of a reconstruction run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run completed and every quality threshold was met.
    Pass,
    /// The run completed but at least one threshold was not met.
    Fail,
    /// The run did not complete.
    Error,
}

impl RunStatus {
    /// The status as the text used in the report.
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Pass => "pass",
            RunStatus::Fail => "fail",
            RunStatus::Error => "error",
        }
    }

    /// The CLI exit code of the status.
    pub fn exit_code(&self) -> u8 {
        match self {
            RunStatus::Pass => 0,
            RunStatus::Fail => 1,
            RunStatus::Error => 2,
        }
    }
}

/// Identity and location of a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo {
    /// The job identifier of the run.
    pub run_id: String,
    /// The workspace-relative path of the run directory.
    pub run_dir: String,
    /// The creation time of the run (RFC 3339, UTC).
    pub created_at: String,
}

/// Identity of the reconstructed package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// The package identifier.
    pub package_id: String,
    /// The package revision.
    pub revision: u32,
    /// The validation status of the package (`valid_complete`,
    /// `valid_partial` or `invalid`).
    pub validation_status: String,
    /// The identifiers of the calibrated views.
    pub calibrated_views: Vec<String>,
}

/// The solver that produced the blockout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverInfo {
    /// The solver name (`voxel-carve`).
    pub name: String,
    /// The solver version.
    pub version: u32,
    /// The requested voxel resolution.
    pub resolution: u32,
    /// The voxel size in metres.
    pub voxel_size: f64,
    /// The grid dimensions.
    pub grid: [u32; 3],
    /// The up axis of the produced OBJ files (`y`).
    pub obj_up: String,
}

/// One mesh of the build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildComponent {
    /// The component name.
    pub name: String,
    /// The OBJ file of the component.
    pub obj: String,
    /// The number of voxels of the component.
    pub voxels: u64,
    /// The number of triangles of the component.
    pub triangles: u64,
}

/// One authored supplement and its outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSupplement {
    /// The supplement identifier.
    pub supplement_id: String,
    /// The outcome (`applied`, `unsupported` or `rejected`).
    pub status: String,
    /// The supplement rule.
    pub rule: String,
    /// A human readable detail.
    pub detail: String,
}

/// One unknown and its outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildUnknown {
    /// The unknown identifier.
    pub id: String,
    /// The outcome (`stub`, `unresolved` or `resolved`).
    pub status: String,
    /// A human readable detail.
    pub detail: String,
}

/// One declared measurement and its measured value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildMeasurement {
    /// The measurement identifier.
    pub id: String,
    /// The declared value in metres.
    pub declared: f64,
    /// The measured value in metres.
    pub measured: f64,
    /// The absolute error in metres.
    pub absolute_error: f64,
    /// The relative error (None when it cannot be computed).
    pub relative_error: Option<f64>,
    /// Whether the measurement is within its tolerance.
    pub within_tolerance: Option<bool>,
}

/// The build result of the solver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildInfo {
    /// The produced meshes.
    pub components: Vec<BuildComponent>,
    /// The authored supplements.
    pub supplements: Vec<BuildSupplement>,
    /// The unknowns of the package.
    pub unknowns: Vec<BuildUnknown>,
    /// The declared measurements.
    pub measurements: Vec<BuildMeasurement>,
}

/// The Blender job of a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    /// The job identifier.
    pub job_id: String,
    /// The worker status (`completed`, `failed` or `cancelled`).
    pub status: String,
    /// The workspace-relative artifact paths.
    pub artifacts: Vec<String>,
    /// The workspace-relative path of the blend file.
    pub blend: Option<String>,
    /// The workspace-relative path of the glb file.
    pub glb: Option<String>,
}

/// One measured view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewMetric {
    /// The view identifier.
    #[serde(default)]
    pub view_id: String,
    /// The intersection over union of the mask and the render.
    #[serde(default)]
    pub iou: f64,
    /// The number of mask pixels.
    #[serde(default)]
    pub mask_px: u64,
    /// The number of render pixels.
    #[serde(default)]
    pub render_px: u64,
    /// The number of intersecting pixels.
    #[serde(default)]
    pub intersection_px: u64,
    /// The number of pixels in the union.
    #[serde(default)]
    pub union_px: u64,
    /// The mask coverage of the view.
    #[serde(default)]
    pub mask_coverage: f64,
    /// The render coverage of the view.
    #[serde(default)]
    pub render_coverage: f64,
}

/// One measured view set (the source views or the held-out views).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualitySection {
    /// The measured views, sorted by view identifier.
    #[serde(default)]
    pub views: Vec<ViewMetric>,
    /// The mean IoU of the views.
    #[serde(default)]
    pub mean_iou: f64,
    /// The smallest IoU of the views.
    #[serde(default)]
    pub min_iou: f64,
}

/// The thresholds of the quality comparison.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Thresholds {
    /// The required mean IoU of the source views.
    #[serde(default)]
    pub source_mean_iou: f64,
    /// The required smallest IoU of the source views.
    #[serde(default)]
    pub source_min_iou: f64,
    /// The required mean IoU of the held-out views.
    #[serde(default)]
    pub evaluation_mean_iou: f64,
    /// The required smallest IoU of the held-out views.
    #[serde(default)]
    pub evaluation_min_iou: f64,
    /// The largest accepted relative error of a declared measurement.
    #[serde(default)]
    pub bbox_relative_error_max: f64,
}

/// The quality result of a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityInfo {
    /// The measured source views.
    pub source: QualitySection,
    /// The measured held-out views (None when they were not scored).
    pub evaluation: Option<QualitySection>,
    /// The thresholds that were applied.
    pub thresholds: Thresholds,
    /// The threshold status (`pass` or `fail`).
    pub status: String,
}

/// One unresolved item of the package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unresolved {
    /// The identifier of the unknown.
    pub id: String,
    /// Why the item stayed unresolved.
    pub reason: String,
}

/// One diagnostic of the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconDiagnostic {
    /// The stable diagnostic code.
    pub code: String,
    /// The severity (`error`, `warning` or `info`).
    pub severity: String,
    /// The human readable message.
    pub message: String,
    /// The address of the offending element (None when it has none).
    pub address: Option<String>,
}

/// The reconstruction report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconstructionReport {
    /// The schema identifier.
    pub schema: String,
    /// The identity of the run.
    pub run: RunInfo,
    /// The identity of the package.
    pub package: PackageInfo,
    /// The solver that produced the blockout.
    pub solver: SolverInfo,
    /// The build result.
    pub build: BuildInfo,
    /// The Blender job.
    pub worker: WorkerInfo,
    /// The quality result.
    pub quality: QualityInfo,
    /// The overall status.
    pub status: RunStatus,
    /// The items that stayed unresolved.
    pub unresolved: Vec<Unresolved>,
    /// The diagnostics of the run.
    pub diagnostics: Vec<ReconDiagnostic>,
}

impl ReconstructionReport {
    /// The JSON representation of the report (pretty, deterministic).
    pub fn to_json(&self) -> Result<String, ReconError> {
        serde_json::to_string_pretty(self).map_err(|err| ReconError::ReportWrite {
            path: "report.json".to_string(),
            message: err.to_string(),
        })
    }

    /// Read a report from a file.
    pub fn load(path: &Path) -> Result<Self, ReconError> {
        let text = std::fs::read_to_string(path).map_err(|err| ReconError::ReportWrite {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        serde_json::from_str(&text).map_err(|err| ReconError::ReportWrite {
            path: path.display().to_string(),
            message: err.to_string(),
        })
    }

    /// The unresolved items of the build, sorted by identifier.
    pub fn unresolved_from(unknowns: &[BuildUnknown]) -> Vec<Unresolved> {
        let mut unresolved: Vec<Unresolved> = unknowns
            .iter()
            .filter(|unknown| unknown.status == "unresolved")
            .map(|unknown| Unresolved {
                id: unknown.id.clone(),
                reason: unknown.detail.clone(),
            })
            .collect();
        unresolved.sort_by(|left, right| left.id.cmp(&right.id));
        unresolved
    }
}

/// The current time as an RFC 3339 timestamp in UTC.
pub fn now_rfc3339_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Convert a day count since the Unix epoch into a civil date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// Replace absolute paths in a text with placeholders.
///
/// The report must never leak the absolute location of the workspace, the
/// package or the tools, so captured tool output is sanitized before it
/// enters the report.
pub fn sanitize_text(text: &str, roots: &[&Path]) -> String {
    let mut sanitized = text.to_string();
    for root in roots {
        let rendered = root.display().to_string();
        if rendered.is_empty() {
            continue;
        }
        sanitized = sanitized.replace(&rendered, "<path>");
        sanitized = sanitized.replace(&rendered.replace('\\', "/"), "<path>");
    }
    let mut result = String::with_capacity(sanitized.len());
    for token in sanitized.split_whitespace() {
        let trimmed = token.trim_matches(|character: char| {
            character.is_whitespace()
                || character == '"'
                || character == '\''
                || character == '('
                || character == ')'
                || character == ','
                || character == ':'
                || character == ';'
        });
        if looks_absolute(trimmed) {
            result.push_str("<path>");
        } else {
            result.push_str(token);
        }
        result.push(' ');
    }
    result.trim_end().to_string()
}

/// True when a token looks like an absolute path.
fn looks_absolute(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let bytes = token.as_bytes();
    if bytes[0] == b'/' {
        return true;
    }
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// The path of the report inside a run directory.
pub fn report_path(run_dir: &Path) -> PathBuf {
    run_dir.join("report.json")
}

/// Read a string field of a JSON object.
fn get_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|field| field.as_str())
        .map(|text| text.to_string())
}

/// Read a floating point field of a JSON object.
fn get_f64(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|field| field.as_f64())
}

/// Read an unsigned field of a JSON object.
fn get_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|field| field.as_u64())
}

/// The solver section of a build report.
pub fn solver_info(build_report: &serde_json::Value) -> SolverInfo {
    let solver = build_report.get("solver");
    let grid = solver
        .and_then(|value| value.get("grid"))
        .and_then(|value| value.as_array())
        .map(|values| {
            let mut grid = [0_u32; 3];
            for (index, value) in values.iter().take(3).enumerate() {
                grid[index] = value.as_u64().unwrap_or(0) as u32;
            }
            grid
        })
        .unwrap_or([0, 0, 0]);

    SolverInfo {
        name: solver
            .and_then(|value| get_str(value, "name"))
            .unwrap_or_default(),
        version: solver
            .and_then(|value| get_u64(value, "version"))
            .unwrap_or(0) as u32,
        resolution: solver
            .and_then(|value| get_u64(value, "resolution"))
            .unwrap_or(0) as u32,
        voxel_size: solver
            .and_then(|value| get_f64(value, "voxel_size"))
            .unwrap_or(0.0),
        grid,
        obj_up: solver
            .and_then(|value| get_str(value, "obj_up"))
            .unwrap_or_else(|| "y".to_string()),
    }
}

/// The build section of a build report.
pub fn build_info(build_report: &serde_json::Value) -> BuildInfo {
    let components = build_report
        .get("components")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| BuildComponent {
                    name: get_str(entry, "name").unwrap_or_default(),
                    obj: get_str(entry, "obj").unwrap_or_default(),
                    voxels: get_u64(entry, "voxels").unwrap_or(0),
                    triangles: get_u64(entry, "triangles").unwrap_or(0),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let supplements = build_report
        .get("supplements")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| BuildSupplement {
                    supplement_id: get_str(entry, "supplement_id").unwrap_or_default(),
                    status: get_str(entry, "status").unwrap_or_default(),
                    rule: get_str(entry, "rule").unwrap_or_default(),
                    detail: get_str(entry, "detail").unwrap_or_default(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let unknowns = build_report
        .get("unknowns")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| BuildUnknown {
                    id: get_str(entry, "id").unwrap_or_default(),
                    status: get_str(entry, "status").unwrap_or_default(),
                    detail: get_str(entry, "detail").unwrap_or_default(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let measurements = build_report
        .get("measurements")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| BuildMeasurement {
                    id: get_str(entry, "id").unwrap_or_default(),
                    declared: get_f64(entry, "declared").unwrap_or(0.0),
                    measured: get_f64(entry, "measured").unwrap_or(0.0),
                    absolute_error: get_f64(entry, "absolute_error").unwrap_or(0.0),
                    relative_error: get_f64(entry, "relative_error"),
                    within_tolerance: entry
                        .get("within_tolerance")
                        .and_then(|value| value.as_bool()),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    BuildInfo {
        components,
        supplements,
        unknowns,
        measurements,
    }
}

/// The diagnostics of a build report.
pub fn build_diagnostics(build_report: &serde_json::Value) -> Vec<ReconDiagnostic> {
    build_report
        .get("diagnostics")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| ReconDiagnostic {
                    code: get_str(entry, "code").unwrap_or_default(),
                    severity: get_str(entry, "severity").unwrap_or_default(),
                    message: get_str(entry, "message").unwrap_or_default(),
                    address: entry
                        .get("address")
                        .and_then(|value| value.as_str())
                        .map(|text| text.to_string()),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// The quality section of a quality report.
pub fn quality_info(quality_report: &serde_json::Value) -> QualityInfo {
    let source: QualitySection = quality_report
        .get("source")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let evaluation: Option<QualitySection> = quality_report
        .get("evaluation")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    let thresholds: Thresholds = quality_report
        .get("thresholds")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let status = quality_report
        .get("thresholds")
        .and_then(|value| get_str(value, "status"))
        .unwrap_or_else(|| "fail".to_string());

    let mut source = source;
    source
        .views
        .sort_by(|left, right| left.view_id.cmp(&right.view_id));
    let evaluation = evaluation.map(|mut section| {
        section
            .views
            .sort_by(|left, right| left.view_id.cmp(&right.view_id));
        section
    });

    QualityInfo {
        source,
        evaluation,
        thresholds,
        status,
    }
}

/// The diagnostics of a quality report.
pub fn quality_diagnostics(quality_report: &serde_json::Value) -> Vec<ReconDiagnostic> {
    quality_report
        .get("diagnostics")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| ReconDiagnostic {
                    code: get_str(entry, "code").unwrap_or_default(),
                    severity: get_str(entry, "severity").unwrap_or_default(),
                    message: get_str(entry, "message").unwrap_or_default(),
                    address: entry
                        .get("address")
                        .and_then(|value| value.as_str())
                        .map(|text| text.to_string()),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Sort the diagnostics of a report deterministically.
pub fn sort_diagnostics(diagnostics: &mut [ReconDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.message.cmp(&right.message))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> ReconstructionReport {
        ReconstructionReport {
            schema: REPORT_SCHEMA.to_string(),
            run: RunInfo {
                run_id: "recon_0001_0001".to_string(),
                run_dir: "recon/r0001/run-0001".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
            package: PackageInfo {
                package_id: "demo".to_string(),
                revision: 1,
                validation_status: "valid_complete".to_string(),
                calibrated_views: vec!["front".to_string(), "right".to_string()],
            },
            solver: SolverInfo {
                name: "voxel-carve".to_string(),
                version: 1,
                resolution: 64,
                voxel_size: 0.01,
                grid: [8, 8, 8],
                obj_up: "y".to_string(),
            },
            build: BuildInfo {
                components: vec![BuildComponent {
                    name: "blockout".to_string(),
                    obj: "blockout.obj".to_string(),
                    voxels: 512,
                    triangles: 1024,
                }],
                supplements: Vec::new(),
                unknowns: vec![BuildUnknown {
                    id: "u_inner".to_string(),
                    status: "unresolved".to_string(),
                    detail: "no geometry in the package".to_string(),
                }],
                measurements: Vec::new(),
            },
            worker: WorkerInfo {
                job_id: "recon_0001_0001".to_string(),
                status: "completed".to_string(),
                artifacts: vec!["recon/r0001/run-0001/blockout.blend".to_string()],
                blend: Some("recon/r0001/run-0001/blockout.blend".to_string()),
                glb: Some("recon/r0001/run-0001/blockout.glb".to_string()),
            },
            quality: QualityInfo {
                source: QualitySection {
                    views: vec![ViewMetric {
                        view_id: "front".to_string(),
                        iou: 0.95,
                        mask_px: 100,
                        render_px: 100,
                        intersection_px: 95,
                        union_px: 100,
                        mask_coverage: 0.5,
                        render_coverage: 0.5,
                    }],
                    mean_iou: 0.95,
                    min_iou: 0.95,
                },
                evaluation: None,
                thresholds: Thresholds {
                    source_mean_iou: 0.9,
                    source_min_iou: 0.85,
                    evaluation_mean_iou: 0.8,
                    evaluation_min_iou: 0.7,
                    bbox_relative_error_max: 0.05,
                },
                status: "pass".to_string(),
            },
            status: RunStatus::Pass,
            unresolved: vec![Unresolved {
                id: "u_inner".to_string(),
                reason: "no geometry in the package".to_string(),
            }],
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn the_report_has_the_exact_top_level_keys() {
        let value = serde_json::to_value(sample_report()).expect("the report is serializable");
        let object = value.as_object().expect("the report is an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "build",
                "diagnostics",
                "package",
                "quality",
                "run",
                "schema",
                "solver",
                "status",
                "unresolved",
                "worker",
            ]
        );
    }

    #[test]
    fn the_status_maps_to_the_exit_codes() {
        assert_eq!(RunStatus::Pass.exit_code(), 0);
        assert_eq!(RunStatus::Fail.exit_code(), 1);
        assert_eq!(RunStatus::Error.exit_code(), 2);
        assert_eq!(RunStatus::Pass.as_str(), "pass");
        assert_eq!(RunStatus::Fail.as_str(), "fail");
        assert_eq!(RunStatus::Error.as_str(), "error");
    }

    #[test]
    fn the_report_contains_no_absolute_path() {
        let report = sample_report();
        let json = report.to_json().expect("the report is serializable");
        assert!(!json.contains("C:\\"));
        assert!(!json.contains("/home/"));
        assert!(json.contains("recon/r0001/run-0001"));
    }

    #[test]
    fn the_report_is_deterministic() {
        let first = sample_report()
            .to_json()
            .expect("the report is serializable");
        let second = sample_report()
            .to_json()
            .expect("the report is serializable");
        assert_eq!(first, second);
    }

    #[test]
    fn the_report_round_trips() {
        let report = sample_report();
        let json = report.to_json().expect("the report is serializable");
        let parsed: ReconstructionReport =
            serde_json::from_str(&json).expect("the report is readable");
        assert_eq!(parsed.schema, REPORT_SCHEMA);
        assert_eq!(parsed.status, RunStatus::Pass);
        assert_eq!(parsed.run.run_dir, "recon/r0001/run-0001");
    }

    #[test]
    fn the_unresolved_items_are_sorted() {
        let unknowns = vec![
            BuildUnknown {
                id: "u_b".to_string(),
                status: "unresolved".to_string(),
                detail: "b".to_string(),
            },
            BuildUnknown {
                id: "u_a".to_string(),
                status: "unresolved".to_string(),
                detail: "a".to_string(),
            },
            BuildUnknown {
                id: "u_c".to_string(),
                status: "stub".to_string(),
                detail: "c".to_string(),
            },
        ];
        let unresolved = ReconstructionReport::unresolved_from(&unknowns);
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0].id, "u_a");
        assert_eq!(unresolved[1].id, "u_b");
    }

    #[test]
    fn the_timestamp_is_rfc3339_utc() {
        let stamp = now_rfc3339_utc();
        assert!(stamp.ends_with('Z'));
        assert_eq!(stamp.len(), 20);
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
    }

    #[test]
    fn absolute_paths_are_sanitized() {
        let text = "failed to open C:\\temp\\package\\package.json and /home/user/x";
        let sanitized = sanitize_text(text, &[]);
        assert!(!sanitized.contains("C:\\temp"));
        assert!(!sanitized.contains("/home/user"));
        assert!(sanitized.contains("<path>"));
    }

    #[test]
    fn the_quality_section_is_read_from_a_comparator_report() {
        let quality = serde_json::json!({
            "schema": "michelangelo.quality-report/0",
            "source": {
                "views": [{
                    "view_id": "right",
                    "iou": 0.9,
                    "mask_px": 10,
                    "render_px": 11,
                    "intersection_px": 9,
                    "union_px": 12,
                    "mask_coverage": 0.5,
                    "render_coverage": 0.55
                }],
                "mean_iou": 0.9,
                "min_iou": 0.9
            },
            "evaluation": null,
            "thresholds": {
                "source_mean_iou": 0.9,
                "source_min_iou": 0.85,
                "evaluation_mean_iou": 0.8,
                "evaluation_min_iou": 0.7,
                "bbox_relative_error_max": 0.05,
                "status": "pass"
            }
        });
        let info = quality_info(&quality);
        assert_eq!(info.status, "pass");
        assert_eq!(info.source.mean_iou, 0.9);
        assert_eq!(info.source.views.len(), 1);
        assert_eq!(info.source.views[0].view_id, "right");
        assert!(info.evaluation.is_none());
        assert_eq!(info.thresholds.source_mean_iou, 0.9);
    }

    #[test]
    fn the_solver_section_is_read_from_a_build_report() {
        let build = serde_json::json!({
            "schema": "michelangelo.build-report/0",
            "solver": {
                "name": "voxel-carve",
                "version": 1,
                "resolution": 8,
                "voxel_size": 0.09375,
                "grid": [8, 8, 8],
                "origin": [0.0, 0.0, 0.0],
                "obj_units": "m",
                "obj_up": "y"
            },
            "components": [{
                "name": "blockout",
                "obj": "blockout.obj",
                "voxels": 456,
                "triangles": 736
            }],
            "supplements": [],
            "unknowns": [],
            "measurements": []
        });
        let info = solver_info(&build);
        assert_eq!(info.name, "voxel-carve");
        assert_eq!(info.resolution, 8);
        assert_eq!(info.grid, [8, 8, 8]);
        assert_eq!(info.obj_up, "y");
        let build_info = build_info(&build);
        assert_eq!(build_info.components.len(), 1);
        assert_eq!(build_info.components[0].obj, "blockout.obj");
    }
}
