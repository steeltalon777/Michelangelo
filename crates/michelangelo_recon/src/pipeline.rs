//! The reconstruction pipelines.
//!
//! # `run`
//!
//! 1. validate the input package (level 1 and level 2),
//! 2. allocate the next free run directory in the workspace store,
//! 3. run the Python solver into `source/`,
//! 4. build the Blender job specification,
//! 5. run the job through the headless Blender adapter,
//! 6. validate the produced artifacts,
//! 7. run the comparator and aggregate the report,
//! 8. write `report.json`.
//!
//! # `score`
//!
//! 1. read the stored report of a run,
//! 2. re-import the meshes of the run and create the cameras of a held-out
//!    evaluation package,
//! 3. render the held-out views into `evaluation_renders/`,
//! 4. run the comparator with the source and the evaluation view set,
//! 5. update the quality section of the report and recompute the status.

use std::path::{Path, PathBuf};

use michelangelo_blender::HeadlessBlenderAdapter;
use michelangelo_input::manifest::{InputPackage, ViewKind};
use michelangelo_input::{load_manifest, validate_package, ValidateOptions};

use crate::error::ReconError;
use crate::plan::{build_job, PlannedMesh};
use crate::report::{
    build_diagnostics, build_info, now_rfc3339_utc, quality_diagnostics, quality_info,
    sanitize_text, solver_info, PackageInfo, ReconDiagnostic, ReconstructionReport, RunInfo,
    RunStatus, WorkerInfo, REPORT_SCHEMA,
};
use crate::solver::{self, ToolPaths};
use crate::store::{self, RunStore};

/// The options of a reconstruction run.
#[derive(Debug, Clone, Default)]
pub struct ReconOptions {
    /// The voxel resolution handed to the solver.
    pub resolution: Option<u32>,
    /// The solver script (default: the repository solver).
    pub solver: Option<PathBuf>,
    /// The Python interpreter (default: `python`).
    pub python: Option<PathBuf>,
    /// The Blender executable (default: `BLENDER_BIN` or `blender`).
    pub blender: Option<PathBuf>,
    /// The timeout of the Blender job in milliseconds.
    pub timeout_ms: Option<u64>,
    /// The timeout of the solver and the comparator in milliseconds.
    pub solver_timeout_ms: Option<u64>,
    /// A fake Blender executable (test hook, never used in production).
    pub fake_blender: Option<PathBuf>,
}

/// The options of a scoring pass.
#[derive(Debug, Clone, Default)]
pub struct ScoreOptions {
    /// The Blender executable (default: `BLENDER_BIN` or `blender`).
    pub blender: Option<PathBuf>,
    /// The timeout of the Blender job in milliseconds.
    pub timeout_ms: Option<u64>,
    /// The timeout of the comparator in milliseconds.
    pub solver_timeout_ms: Option<u64>,
    /// A fake Blender executable (test hook, never used in production).
    pub fake_blender: Option<PathBuf>,
}

/// The name of the package copy inside a run directory.
pub const PACKAGE_COPY_DIR: &str = "package";

/// The name of the previous report copy inside a run directory.
pub const PREVIOUS_REPORT_NAME: &str = "report.before_score.json";

/// The name of the evaluation quality report inside a run directory.
pub const EVALUATION_QUALITY_REPORT_NAME: &str = "evaluation_quality_report.json";

/// Run the full reconstruction pipeline.
pub fn run(
    package_dir: &Path,
    workspace: &Path,
    options: &ReconOptions,
) -> Result<ReconstructionReport, ReconError> {
    if !package_dir.is_dir() {
        return Err(ReconError::MissingPath {
            what: "the package directory".to_string(),
            path: package_dir.to_path_buf(),
        });
    }

    let validation = validate_package(
        package_dir,
        &ValidateOptions {
            require_complete: false,
            verify_hashes: true,
        },
    );
    if validation.status == michelangelo_input::ReportStatus::Invalid {
        let message = validation
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == michelangelo_input::Severity::Error)
            .map(|diagnostic| {
                format!(
                    "{} at {}: {}",
                    diagnostic.code, diagnostic.address, diagnostic.message
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ReconError::InvalidPackage {
            package_id: validation.package_id.clone(),
            message: if message.is_empty() {
                "the package did not pass validation".to_string()
            } else {
                message
            },
        });
    }

    let manifest = load_manifest(package_dir).map_err(|err| ReconError::InvalidPackage {
        package_id: validation.package_id.clone(),
        message: err.to_string(),
    })?;

    let store = store::allocate_run(workspace, manifest.revision)?;
    let tools = solver::resolve_tool_paths(options.solver.as_deref(), options.python.as_deref())?;
    let validation_status = status_text(validation.status);

    match run_inner(
        package_dir,
        workspace,
        &manifest,
        &store,
        &tools,
        options,
        &validation.package_id,
        validation_status,
    ) {
        Ok(report) => Ok(report),
        Err(error) => {
            let report = error_report(&store, &manifest, &validation.package_id, &error);
            let _ = write_report(&store, &report);
            Err(error)
        }
    }
}

/// The inner pipeline, after the run directory was allocated.
#[allow(clippy::too_many_arguments)]
fn run_inner(
    package_dir: &Path,
    workspace: &Path,
    manifest: &InputPackage,
    store: &RunStore,
    tools: &ToolPaths,
    options: &ReconOptions,
    validation_package_id: &str,
    validation_status: &str,
) -> Result<ReconstructionReport, ReconError> {
    copy_package(package_dir, &store.run_dir.join(PACKAGE_COPY_DIR))?;

    let solver_run = solver::run_solver(
        tools,
        package_dir,
        &store.source_dir(),
        options.resolution,
        options.solver_timeout_ms,
    )?;

    let meshes = meshes_from_build_report(&solver_run.build_report)?;
    let planned = build_job(
        manifest,
        &meshes,
        &store.run_dir_rel,
        &store.source_dir_rel(),
        &store.renders_dir_rel(),
        true,
    )?;

    let job_id = format!("recon_{:04}_{:04}", store.revision, store.sequence);
    let result = run_blender_job(
        options.blender.as_deref(),
        options.fake_blender.as_deref(),
        &job_id,
        &planned.spec,
        workspace,
        &store.run_dir.join("jobs"),
        options.timeout_ms,
    )?;

    if result.status != "completed" {
        return Err(ReconError::Blender {
            message: format!(
                "the job '{}' finished with the status '{}'",
                job_id, result.status
            ),
        });
    }

    let artifact_paths: Vec<String> = result
        .artifacts
        .as_ref()
        .map(|artifacts| {
            let mut paths: Vec<String> = artifacts
                .iter()
                .map(|artifact| artifact.path.clone())
                .collect();
            paths.sort();
            paths.dedup();
            paths
        })
        .unwrap_or_default();

    let mut expected_paths: Vec<&str> = planned.render_paths.iter().map(String::as_str).collect();
    expected_paths.push(planned.blend_path.as_str());
    expected_paths.push(planned.glb_path.as_str());
    for expected in expected_paths {
        let absolute = workspace.join(expected);
        let metadata = std::fs::metadata(&absolute).map_err(|_| ReconError::MissingArtifact {
            path: expected.to_string(),
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(ReconError::MissingArtifact {
                path: expected.to_string(),
            });
        }
    }

    let compare = solver::run_compare(
        tools,
        package_dir,
        &store.renders_dir(),
        &store.run_dir,
        None,
        options.solver_timeout_ms,
    )?;

    let mut diagnostics = build_diagnostics(&solver_run.build_report);
    diagnostics.extend(quality_diagnostics(&compare.report));
    diagnostics.dedup_by(|left, right| {
        left.code == right.code && left.address == right.address && left.message == right.message
    });
    if compare.status != "pass" {
        diagnostics.push(ReconDiagnostic {
            code: "QUALITY_THRESHOLD_NOT_MET".to_string(),
            severity: "warning".to_string(),
            message: "the quality thresholds of the source views were not met".to_string(),
            address: None,
        });
    }
    crate::report::sort_diagnostics(&mut diagnostics);

    let build = build_info(&solver_run.build_report);
    let unresolved = ReconstructionReport::unresolved_from(&build.unknowns);
    let quality = quality_info(&compare.report);
    let status = if compare.status == "pass" {
        RunStatus::Pass
    } else {
        RunStatus::Fail
    };

    let calibrated_views: Vec<String> = manifest
        .views
        .iter()
        .filter(|view| view.kind == ViewKind::Calibrated)
        .map(|view| view.view_id.clone())
        .collect();

    let report = ReconstructionReport {
        schema: REPORT_SCHEMA.to_string(),
        run: RunInfo {
            run_id: job_id.clone(),
            run_dir: store.run_dir_rel.clone(),
            created_at: now_rfc3339_utc(),
        },
        package: PackageInfo {
            package_id: if validation_package_id.is_empty() {
                manifest.package_id.clone()
            } else {
                validation_package_id.to_string()
            },
            revision: manifest.revision,
            validation_status: validation_status.to_string(),
            calibrated_views,
        },
        solver: solver_info(&solver_run.build_report),
        build,
        worker: WorkerInfo {
            job_id,
            status: result.status.clone(),
            artifacts: artifact_paths.clone(),
            blend: Some(planned.blend_path.clone()),
            glb: Some(planned.glb_path.clone()),
        },
        quality,
        status,
        unresolved,
        diagnostics,
    };

    let build_report_value = &solver_run.build_report;
    store::write_json_atomic(&store.build_report_path(), build_report_value)?;
    write_report(store, &report)?;
    Ok(report)
}

/// Score a finished run against a held-out evaluation package.
pub fn score(
    run_dir: &Path,
    evaluation_package: &Path,
    options: &ScoreOptions,
) -> Result<ReconstructionReport, ReconError> {
    if !run_dir.is_dir() {
        return Err(ReconError::MissingPath {
            what: "the run directory".to_string(),
            path: run_dir.to_path_buf(),
        });
    }
    if !evaluation_package.is_dir() {
        return Err(ReconError::MissingPath {
            what: "the evaluation package directory".to_string(),
            path: evaluation_package.to_path_buf(),
        });
    }

    let report_path = run_dir.join("report.json");
    if !report_path.is_file() {
        return Err(ReconError::MissingPath {
            what: "the report of the run".to_string(),
            path: report_path,
        });
    }
    let previous = ReconstructionReport::load(&report_path)?;

    let (workspace, run_dir_rel) = derive_workspace(run_dir)?;
    let stored_package = run_dir.join(PACKAGE_COPY_DIR);
    if !stored_package.join("package.json").is_file() {
        return Err(ReconError::MissingPath {
            what: "the package copy of the run".to_string(),
            path: stored_package,
        });
    }

    let validation = validate_package(
        evaluation_package,
        &ValidateOptions {
            require_complete: false,
            verify_hashes: true,
        },
    );
    if validation.status == michelangelo_input::ReportStatus::Invalid {
        return Err(ReconError::InvalidPackage {
            package_id: validation.package_id.clone(),
            message: "the evaluation package did not pass validation".to_string(),
        });
    }
    let evaluation_manifest =
        load_manifest(evaluation_package).map_err(|err| ReconError::InvalidPackage {
            package_id: validation.package_id.clone(),
            message: err.to_string(),
        })?;

    let tools = solver::resolve_tool_paths(None, None)?;
    let meshes = meshes_from_report(&previous);
    let planned = build_job(
        &evaluation_manifest,
        &meshes,
        &run_dir_rel,
        &format!("{run_dir_rel}/{}", store::SOURCE_DIR_NAME),
        &format!("{run_dir_rel}/{}", store::EVALUATION_RENDERS_DIR_NAME),
        false,
    )?;

    let job_id = format!("recon_eval_{}", previous.run.run_id.replace("recon_", ""));
    let result = run_blender_job(
        options.blender.as_deref(),
        options.fake_blender.as_deref(),
        &job_id,
        &planned.spec,
        &workspace,
        &run_dir.join("jobs"),
        options.timeout_ms,
    )?;
    if result.status != "completed" {
        return Err(ReconError::Blender {
            message: format!(
                "the evaluation job '{}' finished with the status '{}'",
                job_id, result.status
            ),
        });
    }

    for expected in &planned.render_paths {
        let absolute = workspace.join(expected);
        let metadata = std::fs::metadata(&absolute).map_err(|_| ReconError::MissingArtifact {
            path: expected.clone(),
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(ReconError::MissingArtifact {
                path: expected.clone(),
            });
        }
    }

    let evaluation_out = run_dir.join("evaluation");
    let compare = solver::run_compare(
        &tools,
        &stored_package,
        &run_dir.join(store::RENDERS_DIR_NAME),
        &evaluation_out,
        Some((
            evaluation_package,
            &run_dir.join(store::EVALUATION_RENDERS_DIR_NAME),
        )),
        options.solver_timeout_ms,
    )?;

    let quality_report_path = evaluation_out.join("quality_report.json");
    if quality_report_path.is_file() {
        let text = std::fs::read_to_string(&quality_report_path).map_err(|err| {
            ReconError::CompareFailed {
                message: err.to_string(),
            }
        })?;
        store::write_text_atomic(&run_dir.join(EVALUATION_QUALITY_REPORT_NAME), &text)?;
    }

    let mut updated = previous.clone();
    let new_quality = quality_info(&compare.report);
    updated.quality.thresholds = new_quality.thresholds.clone();
    updated.quality.evaluation = new_quality.evaluation.clone();
    updated.quality.status = new_quality.status.clone();
    updated.status = if new_quality.status == "pass" {
        RunStatus::Pass
    } else {
        RunStatus::Fail
    };

    let mut diagnostics = updated.diagnostics.clone();
    diagnostics.extend(quality_diagnostics(&compare.report));
    diagnostics.push(ReconDiagnostic {
        code: "EVALUATION_SCORED".to_string(),
        severity: "info".to_string(),
        message: format!("the held-out views were scored with the job '{}'", job_id),
        address: None,
    });
    crate::report::sort_diagnostics(&mut diagnostics);
    updated.diagnostics = diagnostics;

    let mut artifacts = updated.worker.artifacts.clone();
    if let Some(list) = &result.artifacts {
        for artifact in list {
            if !artifacts.contains(&artifact.path) {
                artifacts.push(artifact.path.clone());
            }
        }
    }
    artifacts.sort();
    updated.worker.artifacts = artifacts;

    store::write_text_atomic(&run_dir.join(PREVIOUS_REPORT_NAME), &previous.to_json()?)?;
    let report_path = run_dir.join("report.json");
    store::write_text_atomic(&report_path, &updated.to_json()?)?;
    Ok(updated)
}

/// Write the report of a run into the run directory.
fn write_report(store: &RunStore, report: &ReconstructionReport) -> Result<(), ReconError> {
    store::write_text_atomic(&store.report_path(), &report.to_json()?)
}

/// The report of a failed run.
fn error_report(
    store: &RunStore,
    manifest: &InputPackage,
    package_id: &str,
    error: &ReconError,
) -> ReconstructionReport {
    let mut diagnostics = vec![ReconDiagnostic {
        code: "RECON_RUN_FAILED".to_string(),
        severity: "error".to_string(),
        message: sanitize_text(&error.to_string(), &[&store.workspace]),
        address: None,
    }];
    for (code, severity, tail) in solver_tails(error) {
        let text = sanitize_text(&tail, &[&store.workspace]);
        if text.trim().is_empty() {
            continue;
        }
        diagnostics.push(ReconDiagnostic {
            code: code.to_string(),
            severity: severity.to_string(),
            message: text,
            address: None,
        });
    }
    crate::report::sort_diagnostics(&mut diagnostics);
    ReconstructionReport {
        schema: REPORT_SCHEMA.to_string(),
        run: RunInfo {
            run_id: format!("recon_{:04}_{:04}", store.revision, store.sequence),
            run_dir: store.run_dir_rel.clone(),
            created_at: now_rfc3339_utc(),
        },
        package: PackageInfo {
            package_id: if package_id.is_empty() {
                manifest.package_id.clone()
            } else {
                package_id.to_string()
            },
            revision: manifest.revision,
            validation_status: "unknown".to_string(),
            calibrated_views: Vec::new(),
        },
        solver: crate::report::SolverInfo {
            name: String::new(),
            version: 0,
            resolution: 0,
            voxel_size: 0.0,
            grid: [0, 0, 0],
            obj_up: String::new(),
        },
        build: crate::report::BuildInfo {
            components: Vec::new(),
            supplements: Vec::new(),
            unknowns: Vec::new(),
            measurements: Vec::new(),
        },
        worker: WorkerInfo {
            job_id: String::new(),
            status: "error".to_string(),
            artifacts: Vec::new(),
            blend: None,
            glb: None,
        },
        quality: crate::report::QualityInfo {
            source: crate::report::QualitySection::default(),
            evaluation: None,
            thresholds: crate::report::Thresholds::default(),
            status: "fail".to_string(),
        },
        status: RunStatus::Error,
        unresolved: Vec::new(),
        diagnostics,
    }
}

/// The captured output tails of a solver failure.
fn solver_tails(error: &ReconError) -> Vec<(&'static str, &'static str, String)> {
    match error {
        ReconError::SolverRejected { message, stdout } => vec![
            ("SOLVER_STDERR", "error", message.clone()),
            ("SOLVER_STDOUT", "info", stdout.clone()),
        ],
        ReconError::SolverFailed { stderr, stdout, .. } => vec![
            ("SOLVER_STDERR", "error", stderr.clone()),
            ("SOLVER_STDOUT", "info", stdout.clone()),
        ],
        ReconError::SolverTimeout { stderr, stdout, .. } => vec![
            ("SOLVER_STDERR", "error", stderr.clone()),
            ("SOLVER_STDOUT", "info", stdout.clone()),
        ],
        _ => Vec::new(),
    }
}

/// The meshes of a solver build report.
fn meshes_from_build_report(
    build_report: &serde_json::Value,
) -> Result<Vec<PlannedMesh>, ReconError> {
    let entries = build_report
        .get("components")
        .and_then(|value| value.as_array())
        .ok_or_else(|| ReconError::SolverOutput {
            message: "the build report has no component list".to_string(),
        })?;
    let mut meshes = Vec::new();
    for entry in entries {
        let obj = entry
            .get("obj")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ReconError::SolverOutput {
                message: "a solver component has no 'obj' file name".to_string(),
            })?
            .to_string();
        let stem = obj.strip_suffix(".obj").unwrap_or(obj.as_str()).to_string();
        meshes.push(PlannedMesh {
            name: stem,
            obj,
            path: String::new(),
        });
    }
    meshes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(meshes)
}

/// The meshes of a stored report (used by the scoring pass).
fn meshes_from_report(report: &ReconstructionReport) -> Vec<PlannedMesh> {
    let mut meshes: Vec<PlannedMesh> = report
        .build
        .components
        .iter()
        .map(|component| PlannedMesh {
            name: component.name.clone(),
            obj: component.obj.clone(),
            path: String::new(),
        })
        .collect();
    meshes.sort_by(|left, right| left.name.cmp(&right.name));
    meshes
}

/// Copy the manifest and the masks of a package into the run directory.
///
/// The copy makes a run self-contained: the scoring pass can compare the
/// source views without access to the original package directory.
fn copy_package(package_dir: &Path, destination: &Path) -> Result<(), ReconError> {
    let manifest_path = package_dir.join("package.json");
    if !manifest_path.is_file() {
        return Err(ReconError::MissingPath {
            what: "the package manifest".to_string(),
            path: manifest_path,
        });
    }
    std::fs::create_dir_all(destination).map_err(|err| ReconError::Io {
        context: "the package copy cannot be created".to_string(),
        message: err.to_string(),
    })?;
    std::fs::copy(&manifest_path, destination.join("package.json")).map_err(|err| {
        ReconError::Io {
            context: "the package manifest cannot be copied".to_string(),
            message: err.to_string(),
        }
    })?;

    let manifest = load_manifest(package_dir).map_err(|err| ReconError::InvalidPackage {
        package_id: String::new(),
        message: err.to_string(),
    })?;
    for view in &manifest.views {
        let separator = std::path::MAIN_SEPARATOR.to_string();
        let relative = view.mask.path.replace('/', &separator);
        let source = package_dir.join(&relative);
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| ReconError::Io {
                context: "the package copy cannot be created".to_string(),
                message: err.to_string(),
            })?;
        }
        std::fs::copy(&source, &target).map_err(|err| ReconError::Io {
            context: "a mask of the package cannot be copied".to_string(),
            message: err.to_string(),
        })?;
    }
    Ok(())
}

/// Derive the workspace root and the workspace-relative run path.
fn derive_workspace(run_dir: &Path) -> Result<(PathBuf, String), ReconError> {
    let canonical = std::fs::canonicalize(run_dir).map_err(|err| ReconError::Io {
        context: "the run directory cannot be resolved".to_string(),
        message: err.to_string(),
    })?;
    let revision_dir = canonical.parent().ok_or_else(|| ReconError::MissingPath {
        what: "the revision directory of the run".to_string(),
        path: canonical.clone(),
    })?;
    let recon_dir = revision_dir
        .parent()
        .ok_or_else(|| ReconError::MissingPath {
            what: "the reconstruction directory of the run".to_string(),
            path: revision_dir.to_path_buf(),
        })?;
    let workspace = recon_dir.parent().ok_or_else(|| ReconError::MissingPath {
        what: "the workspace of the run".to_string(),
        path: recon_dir.to_path_buf(),
    })?;
    let revision_name = revision_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let run_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((
        workspace.to_path_buf(),
        format!("{}/{}/{}", store::RECON_DIR_NAME, revision_name, run_name),
    ))
}

/// Run the Blender job through the headless adapter.
#[allow(clippy::too_many_arguments)]
fn run_blender_job(
    blender: Option<&Path>,
    fake_blender: Option<&Path>,
    job_id: &str,
    spec: &michelangelo_protocol::BlenderJobSpec,
    workspace: &Path,
    output_dir: &Path,
    timeout_ms: Option<u64>,
) -> Result<michelangelo_protocol::BlenderJobResult, ReconError> {
    let blender_bin = blender
        .map(|path| path.display().to_string())
        .or_else(|| std::env::var("BLENDER_BIN").ok())
        .unwrap_or_else(|| "blender".to_string());
    let adapter = HeadlessBlenderAdapter::with_config(blender_bin, worker_script_path());
    let result = match fake_blender {
        Some(fake) => {
            adapter.run_job_with_fake(fake, job_id, spec, workspace, output_dir, timeout_ms)
        }
        None => adapter.run_job(job_id, spec, workspace, output_dir, timeout_ms),
    };
    result.map_err(|err| ReconError::Blender {
        message: err.to_string(),
    })
}

/// The path of the Blender worker script.
fn worker_script_path() -> PathBuf {
    let candidate = solver::repo_root().join("blender").join("worker.py");
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from("blender").join("worker.py")
    }
}

/// The text of a package validation status.
fn status_text(status: michelangelo_input::ReportStatus) -> &'static str {
    match status {
        michelangelo_input::ReportStatus::ValidComplete => "valid_complete",
        michelangelo_input::ReportStatus::ValidPartial => "valid_partial",
        michelangelo_input::ReportStatus::Invalid => "invalid",
    }
}
