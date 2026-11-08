//! The evaluator of a candidate or of a scene revision (IT-8B).
//!
//! The evaluator renders the **actual** geometry of the target (the snapshot
//! of a candidate or the components of a revision) and of the base revision
//! with identical inputs, cameras and settings, compares both with the
//! pinned masks of the package copy of the run and writes an immutable
//! evaluation report into the store.
//!
//! The evaluation never imports the old blockout: the geometry of the
//! snapshot (or of the revision) is staged and rendered through the existing
//! worker operations.

use std::path::{Path, PathBuf};

use michelangelo_edit::scene::{load_scene, run_id_of, SceneStore};
use michelangelo_edit::verification::{load_snapshot, tool_versions, verify_snapshot, StoreLock};
use michelangelo_evidence::digest::{file_sha256, geometry_aggregate};
use michelangelo_evidence::registry::{load_registry, write_and_register};
use michelangelo_evidence::report::{
    EvaluationReport, EvaluationThresholds, PackageView, TargetKind, TriState,
    EVALUATION_REPORT_SCHEMA,
};
use serde_json::Value;

use crate::assemble::{assemble, Assembly};
use crate::compare::{Comparator, CompareRequest, ScriptComparator};
use crate::error::EvalError;
use crate::helpers::{direction_of, dot, mask_paths_of, units_of, view_spec};
use crate::render::{BlenderRenderer, GeometrySet, Renderer, ViewSet};

/// The largest accepted drop of the IoU of a source view against the base.
pub const PER_VIEW_REGRESSION_MAX: f64 = 0.01;

/// The largest accepted scalar product of the directions of two views.
pub const HELD_OUT_MAX_DOT: f64 = 0.9;

/// The run directory as an absolute path.
///
/// The Blender worker rejects a relative `workspace_root`, so every evaluation
/// resolves the run directory before any path is written into a job.
fn absolute_run(run_dir: &Path) -> Result<PathBuf, EvalError> {
    std::path::absolute(run_dir).map_err(|err| EvalError::Invalid {
        message: format!(
            "the run directory '{}' cannot be resolved to an absolute path: {err}",
            run_dir.display()
        ),
    })
}

/// The options of an evaluation.
#[derive(Debug, Clone, Default)]
pub struct EvaluationOptions {
    /// The held-out evaluation package (optional).
    pub evaluation_package: Option<PathBuf>,
    /// The Blender executable (default: BLENDER_BIN or blender).
    pub blender: Option<PathBuf>,
    /// The Python interpreter (default: python).
    pub python: Option<PathBuf>,
    /// The solver script (default: the solver of the repository).
    pub solver: Option<PathBuf>,
    /// The timeout of the Blender job, in milliseconds.
    pub timeout_ms: Option<u64>,
    /// The timeout of the comparator, in milliseconds.
    pub solver_timeout_ms: Option<u64>,
    /// A fake Blender executable (the test hook).
    pub fake_blender: Option<PathBuf>,
}

/// One component of the geometry an evaluation measures.
#[derive(Debug, Clone)]
struct Component {
    name: String,
    path: PathBuf,
    sha256: String,
}

/// The geometry an evaluation measures.
#[derive(Debug, Clone)]
struct Target {
    kind: TargetKind,
    id: String,
    aggregate_sha256: String,
    base_scene_revision: Option<String>,
    components: Vec<Component>,
    base_components: Vec<Component>,
}

/// The held-out set of an evaluation.
struct HoldOut {
    views: Vec<PackageView>,
    view_set: ViewSet,
    package_dir: PathBuf,
    package_sha256: String,
}

/// Evaluate the snapshot of a candidate.
pub fn evaluate_candidate(
    run_dir: &Path,
    candidate_id: &str,
    options: &EvaluationOptions,
) -> Result<EvaluationReport, EvalError> {
    let renderer = BlenderRenderer::new(options.blender.as_deref(), options.timeout_ms)
        .with_fake(options.fake_blender.clone());
    let comparator = ScriptComparator::new(
        options.solver.as_deref(),
        options.python.as_deref(),
        options.solver_timeout_ms,
    )?;
    evaluate_candidate_with(run_dir, candidate_id, options, &renderer, &comparator)
}

/// Evaluate the snapshot of a candidate with the given renderer and comparator.
pub fn evaluate_candidate_with(
    run_dir: &Path,
    candidate_id: &str,
    options: &EvaluationOptions,
    renderer: &dyn Renderer,
    comparator: &dyn Comparator,
) -> Result<EvaluationReport, EvalError> {
    let absolute = absolute_run(run_dir)?;
    let run_dir = absolute.as_path();
    let store = SceneStore::new(run_dir);
    let manifest = match load_snapshot(&store, candidate_id)? {
        Some(manifest) => manifest,
        None => {
            return Err(EvalError::Missing {
                message: format!(
                    "the candidate '{candidate_id}' carries no snapshot: it cannot be evaluated"
                ),
            })
        }
    };
    verify_snapshot(&store, &manifest).map_err(|err| EvalError::Evidence {
        message: err.message().to_string(),
    })?;

    let base = load_scene(&store, &manifest.base.scene_revision)?;
    let base_components: Vec<Component> = base
        .components
        .iter()
        .map(|component| Component {
            name: component.name.clone(),
            path: store
                .components_dir(&manifest.base.scene_revision)
                .join(format!("{}.obj", component.name)),
            sha256: component.sha256.clone(),
        })
        .collect();
    let target_components: Vec<Component> = manifest
        .files
        .iter()
        .map(|file| Component {
            name: file.component.clone(),
            path: store.snapshot_dir(candidate_id).join(&file.path),
            sha256: file.sha256.clone(),
        })
        .collect();
    let target = Target {
        kind: TargetKind::Candidate,
        id: candidate_id.to_string(),
        aggregate_sha256: manifest.aggregate_sha256.clone(),
        base_scene_revision: Some(manifest.base.scene_revision.clone()),
        components: target_components,
        base_components,
    };
    evaluate(run_dir, target, options, renderer, comparator)
}

/// Evaluate a published scene revision.
pub fn evaluate_revision(
    run_dir: &Path,
    revision: &str,
    options: &EvaluationOptions,
) -> Result<EvaluationReport, EvalError> {
    let renderer = BlenderRenderer::new(options.blender.as_deref(), options.timeout_ms)
        .with_fake(options.fake_blender.clone());
    let comparator = ScriptComparator::new(
        options.solver.as_deref(),
        options.python.as_deref(),
        options.solver_timeout_ms,
    )?;
    evaluate_revision_with(run_dir, revision, options, &renderer, &comparator)
}

/// Evaluate a published scene revision with the given renderer and comparator.
pub fn evaluate_revision_with(
    run_dir: &Path,
    revision: &str,
    options: &EvaluationOptions,
    renderer: &dyn Renderer,
    comparator: &dyn Comparator,
) -> Result<EvaluationReport, EvalError> {
    let absolute = absolute_run(run_dir)?;
    let run_dir = absolute.as_path();
    let store = SceneStore::new(run_dir);
    let scene = load_scene(&store, revision)?;
    let components: Vec<Component> = scene
        .components
        .iter()
        .map(|component| Component {
            name: component.name.clone(),
            path: store
                .components_dir(revision)
                .join(format!("{}.obj", component.name)),
            sha256: component.sha256.clone(),
        })
        .collect();
    let base_revision = scene.base_scene_revision.clone();
    let base_components: Vec<Component> = match &base_revision {
        Some(base_revision) => {
            let base = load_scene(&store, base_revision)?;
            base.components
                .iter()
                .map(|component| Component {
                    name: component.name.clone(),
                    path: store
                        .components_dir(base_revision)
                        .join(format!("{}.obj", component.name)),
                    sha256: component.sha256.clone(),
                })
                .collect()
        }
        None => components.clone(),
    };
    let aggregate = geometry_aggregate(
        &components
            .iter()
            .map(|component| (component.name.clone(), component.sha256.clone()))
            .collect::<Vec<(String, String)>>(),
    );
    let target = Target {
        kind: TargetKind::Revision,
        id: revision.to_string(),
        aggregate_sha256: aggregate,
        base_scene_revision: base_revision.clone(),
        components,
        base_components,
    };
    evaluate(run_dir, target, options, renderer, comparator)
}

/// The evaluation of a geometry.
fn evaluate(
    run_dir: &Path,
    target: Target,
    options: &EvaluationOptions,
    renderer: &dyn Renderer,
    comparator: &dyn Comparator,
) -> Result<EvaluationReport, EvalError> {
    let store = SceneStore::new(run_dir);
    let package_path = store.package_copy_path();
    if !package_path.is_file() {
        return Err(EvalError::Missing {
            message: String::from(
                "the run carries no copy of the input package: the masks cannot be pinned",
            ),
        });
    }
    let package_dir = package_path
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| run_dir.to_path_buf());
    let package_sha256 = file_sha256(&package_path).map_err(|err| EvalError::Invalid {
        message: err.message().to_string(),
    })?;
    let package_text = std::fs::read_to_string(&package_path).map_err(|err| {
        EvalError::io(
            &package_path,
            format!("the package copy cannot be read: {err}"),
        )
    })?;
    let package: Value = serde_json::from_str(&package_text).map_err(|err| EvalError::Invalid {
        message: format!("the package copy of the run is not usable: {err}"),
    })?;

    let mut source_views = michelangelo_evidence::report::package_views(&package);
    if source_views.is_empty() {
        return Err(EvalError::Missing {
            message: String::from("the package copy of the run has no calibrated view"),
        });
    }
    // The evaluation pins the bytes it really reads: the digest of every mask
    // file is computed and compared with the digest the package declares. A
    // package that does not match its own files is refused.
    pin_actual_masks(&package_dir, &package, &mut source_views)?;
    let view_set = ViewSet {
        units: units_of(&package),
        views: source_views.iter().map(view_spec).collect(),
    };
    let mask_paths = mask_paths_of(&package_dir, &package);

    // The held-out set: the directions must be distinct from the source
    // directions, otherwise the held-out result is unavailable.
    let mut held_out: Option<HoldOut> = None;
    let mut held_out_diagnostic: Option<String> = None;
    if let Some(evaluation_package) = &options.evaluation_package {
        match held_out_set(evaluation_package, &view_set) {
            Ok(set) => held_out = Some(set),
            Err(message) => held_out_diagnostic = Some(message),
        }
    }

    // The work area of the evaluation: the identifier is reserved with an
    // atomic create (no overwrite) under the short lock of the store, so two
    // concurrent evaluations never share a directory.
    let (evaluation_id, work) = reserve_evaluation(run_dir, &store)?;
    let target_dir = work.join("target");
    let base_dir = work.join("base");
    stage_geometry(&target.components, &target_dir)?;
    stage_geometry(&target.base_components, &base_dir)?;

    let work_rel = format!("verification/evaluations/{evaluation_id}/work");
    let target_geometry = GeometrySet {
        dir: target_dir.clone(),
        components: target
            .components
            .iter()
            .map(|component| (component.name.clone(), component.sha256.clone()))
            .collect(),
    };
    let base_geometry = GeometrySet {
        dir: base_dir.clone(),
        components: target
            .base_components
            .iter()
            .map(|component| (component.name.clone(), component.sha256.clone()))
            .collect(),
    };

    // The renders of the target and of the base: identical inputs, cameras
    // and settings.
    let target_renders = work.join("renders/target");
    let base_renders = work.join("renders/base");
    render_geometry(
        renderer,
        &target_geometry,
        &view_set,
        run_dir,
        &format!("{work_rel}/target"),
        &format!("{work_rel}/renders/target"),
        &work_rel,
        &target_renders,
        &format!("eval_{evaluation_id}_target"),
        &work.join("jobs"),
    )?;
    render_geometry(
        renderer,
        &base_geometry,
        &view_set,
        run_dir,
        &format!("{work_rel}/base"),
        &format!("{work_rel}/renders/base"),
        &work_rel,
        &base_renders,
        &format!("eval_{evaluation_id}_base"),
        &work.join("jobs"),
    )?;

    // The held-out views are rendered with the same geometry and the same
    // settings, only the cameras of the held-out package differ.
    let held_out_renders = work.join("renders/held-out");
    if let Some(held_out) = &held_out {
        render_geometry(
            renderer,
            &target_geometry,
            &held_out.view_set,
            run_dir,
            &format!("{work_rel}/target"),
            &format!("{work_rel}/renders/held-out"),
            &work_rel,
            &held_out_renders,
            &format!("eval_{evaluation_id}_held_out"),
            &work.join("jobs"),
        )?;
    }

    // The comparisons.
    let evaluation_argument = held_out
        .as_ref()
        .map(|held_out| (held_out.package_dir.as_path(), held_out_renders.as_path()));
    let target_outcome = comparator.compare(&CompareRequest {
        package_dir: &package_dir,
        renders_dir: &target_renders,
        out_dir: &work.join("compare/target"),
        evaluation: evaluation_argument,
        mask_paths: &mask_paths,
    })?;
    let base_outcome = comparator.compare(&CompareRequest {
        package_dir: &package_dir,
        renders_dir: &base_renders,
        out_dir: &work.join("compare/base"),
        evaluation: None,
        mask_paths: &mask_paths,
    })?;

    let thresholds = thresholds_of(run_dir);
    let run_report = read_run_report(run_dir);
    let mut versions = tool_versions(&store);
    versions.insert(
        String::from("comparator"),
        String::from("tools/solver/compare.py"),
    );
    versions.insert(
        String::from("evaluation_schema"),
        EVALUATION_REPORT_SCHEMA.to_string(),
    );

    let report = assemble(&Assembly {
        evaluation_id: &evaluation_id,
        created_at: michelangelo_recon::report::now_rfc3339_utc(),
        run_id: &run_id_of(run_dir),
        target_kind: target.kind,
        target_id: &target.id,
        target_aggregate_sha256: &target.aggregate_sha256,
        base_scene_revision: target.base_scene_revision.as_deref(),
        package_sha256: Some(package_sha256),
        source_views: &source_views,
        held_out_views: held_out.as_ref().map(|held_out| held_out.views.as_slice()),
        evaluation_package_sha256: held_out
            .as_ref()
            .map(|held_out| held_out.package_sha256.clone()),
        held_out_diagnostic,
        thresholds,
        target_outcome: &target_outcome,
        base_outcome: &base_outcome,
        package: &package,
        run_report: run_report.as_ref(),
        tool_versions: versions,
    });

    // The report is registered under the lock of the store: the registry
    // never carries the digest of bytes that are not on the disk.
    let _lock = lock_store(&store, "evaluate")?;
    write_and_register(run_dir, &report).map_err(|err| EvalError::Invalid {
        message: err.message().to_string(),
    })?;
    Ok(report)
}

/// Take the lock of the store for a short section of an evaluation.
///
/// The lock of the store is exclusive and is never stolen: the evaluator waits
/// a short moment for it (the sections are short) instead of refusing the
/// evaluation. Two concurrent evaluations therefore both succeed, with
/// distinct identifiers.
fn lock_store(store: &SceneStore, operation: &str) -> Result<StoreLock, EvalError> {
    let mut waited = 0_u32;
    loop {
        match StoreLock::acquire(store, operation) {
            Ok(lock) => return Ok(lock),
            Err(error) => {
                if error.code() != michelangelo_edit::error::STORE_BUSY {
                    return Err(EvalError::Evidence {
                        message: error.message().to_string(),
                    });
                }
                if waited >= 100 {
                    return Err(EvalError::Evidence {
                        message: error.message().to_string(),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                waited += 1;
            }
        }
    }
}

/// Reserve the directory of an evaluation.
///
/// The reservation is an atomic create (no overwrite) under the short lock of
/// the store: two concurrent evaluations get distinct identifiers and both
/// reports stay intact. The render happens outside of the lock.
fn reserve_evaluation(run_dir: &Path, store: &SceneStore) -> Result<(String, PathBuf), EvalError> {
    let _lock = lock_store(store, "evaluate-reserve")?;
    let registry = load_registry(run_dir).map_err(|err| EvalError::Invalid {
        message: err.message().to_string(),
    })?;
    let mut number = registry
        .next_evaluation_id()
        .strip_prefix("ev-")
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(1);
    let evaluations_dir = run_dir
        .join(michelangelo_evidence::registry::VERIFICATION_DIR_NAME)
        .join(michelangelo_evidence::registry::EVALUATIONS_DIR_NAME);
    std::fs::create_dir_all(&evaluations_dir).map_err(|err| {
        EvalError::io(
            &evaluations_dir,
            format!("the evaluations directory cannot be created: {err}"),
        )
    })?;
    loop {
        let evaluation_id = format!("ev-{number:04}");
        let directory = michelangelo_evidence::registry::evaluation_dir(run_dir, &evaluation_id);
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                let work = directory.join("work");
                std::fs::create_dir_all(&work).map_err(|err| {
                    EvalError::io(
                        &work,
                        format!("the work directory of the evaluation cannot be created: {err}"),
                    )
                })?;
                return Ok((evaluation_id, work));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                number += 1;
                if number > 1_000_000 {
                    return Err(EvalError::Invalid {
                        message: String::from(
                            "the run carries too many evaluations: no identifier is free",
                        ),
                    });
                }
            }
            Err(err) => {
                return Err(EvalError::io(
                    &directory,
                    format!("the evaluation cannot be reserved: {err}"),
                ))
            }
        }
    }
}

/// Pin the bytes of the mask files a package really carries.
///
/// The digests of the mask files are computed and compared with the digests
/// the package declares: the evaluation measures against the masks of the
/// disk, and a package that does not match its own files is refused.
fn pin_actual_masks(
    package_dir: &Path,
    package: &Value,
    views: &mut [PackageView],
) -> Result<(), EvalError> {
    let separator = std::path::MAIN_SEPARATOR.to_string();
    for (view_id, relative) in michelangelo_evidence::report::package_mask_paths(package) {
        let path = package_dir.join(relative.replace('/', &separator));
        if !path.is_file() {
            return Err(EvalError::Missing {
                message: format!(
                    "the mask file of the view '{view_id}' is missing: '{}'",
                    path.display()
                ),
            });
        }
        let actual = michelangelo_evidence::digest::file_sha256(&path)
            .map_err(|err| EvalError::io(&path, format!("the mask file cannot be read: {err}")))?;
        let declared = views
            .iter()
            .find(|view| view.view_id == view_id)
            .map(|view| view.mask_sha256.clone())
            .unwrap_or_default();
        if !declared.is_empty() && declared != actual {
            return Err(EvalError::Invalid {
                message: format!(
                    "the package declares the mask {declared} of the view '{view_id}' but the file carries {actual}: the package does not match its own files"
                ),
            });
        }
        for view in views.iter_mut() {
            if view.view_id == view_id {
                view.mask_sha256 = actual.clone();
            }
        }
    }
    Ok(())
}

/// Stage the components of a geometry into the work area of the evaluation.
fn stage_geometry(components: &[Component], destination: &Path) -> Result<(), EvalError> {
    std::fs::create_dir_all(destination).map_err(|err| {
        EvalError::io(
            destination,
            format!("the work directory cannot be created: {err}"),
        )
    })?;
    for component in components {
        let bytes = std::fs::read(&component.path).map_err(|err| {
            EvalError::io(
                &component.path,
                format!("the component cannot be read: {err}"),
            )
        })?;
        let digest = michelangelo_evidence::digest::sha256_hex(&bytes);
        if digest != component.sha256 {
            return Err(EvalError::Evidence {
                message: format!(
                    "the component '{}' carries the digest {digest} but the store pins {}",
                    component.name, component.sha256
                ),
            });
        }
        let target = destination.join(format!("{}.obj", component.name));
        std::fs::write(&target, &bytes).map_err(|err| {
            EvalError::io(&target, format!("the component cannot be written: {err}"))
        })?;
    }
    Ok(())
}

/// Render a geometry with a set of views.
#[allow(clippy::too_many_arguments)]
fn render_geometry(
    renderer: &dyn Renderer,
    geometry: &GeometrySet,
    views: &ViewSet,
    workspace_root: &Path,
    source_dir_rel: &str,
    renders_dir_rel: &str,
    work_dir_rel: &str,
    renders_dir: &Path,
    job_id: &str,
    output_dir: &Path,
) -> Result<(), EvalError> {
    std::fs::create_dir_all(renders_dir).map_err(|err| {
        EvalError::io(
            renders_dir,
            format!("the render directory cannot be created: {err}"),
        )
    })?;
    renderer.render(&crate::render::RenderRequest {
        geometry,
        views,
        workspace_root,
        source_dir_rel,
        renders_dir_rel,
        work_dir_rel,
        renders_dir,
        job_id,
        output_dir,
    })
}

/// The held-out set of an evaluation package.
///
/// The directions of the held-out views must be distinct from the directions
/// of the source views, otherwise the held-out result is unavailable.
fn held_out_set(evaluation_package: &Path, source: &ViewSet) -> Result<HoldOut, String> {
    let package_path = evaluation_package.join("package.json");
    let text = std::fs::read_to_string(&package_path).map_err(|err| {
        format!(
            "the evaluation package '{}' cannot be read: {err}",
            package_path.display()
        )
    })?;
    let package: Value = serde_json::from_str(&text).map_err(|err| {
        format!(
            "the evaluation package '{}' is not usable: {err}",
            package_path.display()
        )
    })?;
    let mut views = michelangelo_evidence::report::package_views(&package);
    if views.is_empty() {
        return Err(String::from(
            "the evaluation package has no calibrated view: the held-out result is unavailable",
        ));
    }
    // The held-out set is pinned against the bytes it really reads: a package
    // that does not match its own mask files cannot be used.
    if let Err(error) = pin_actual_masks(evaluation_package, &package, &mut views) {
        return Err(format!(
            "the held-out package does not match its own files: {}",
            error.message()
        ));
    }
    let package_sha256 = michelangelo_evidence::digest::file_sha256(&package_path)
        .map_err(|err| err.message().to_string())?;
    let view_set = ViewSet {
        units: units_of(&package),
        views: views.iter().map(view_spec).collect(),
    };
    for held_out_view in &view_set.views {
        let direction = direction_of(held_out_view);
        for source_view in &source.views {
            let dot = dot(direction, direction_of(source_view));
            // A view that looks in the same direction as a source view adds no
            // silhouette constraint. An opposite orthographic view (dot ~ -1)
            // is not independent either: for a full opaque silhouette it is
            // the mirrored occupancy, not a new shape constraint. Only a view
            // whose direction is far from every source direction (|dot| <=
            // 0.9) adds an independent constraint.
            if dot.abs() > HELD_OUT_MAX_DOT {
                return Err(format!(
                    "the held-out view '{}' is not independent of the source view '{}': the scalar product of the directions is {dot:.4} (the limit is {HELD_OUT_MAX_DOT:.2} in absolute value): an opposite orthographic view is the mirrored silhouette and not an independent shape constraint, so the held-out result is unavailable",
                    held_out_view.view_id, source_view.view_id
                ));
            }
        }
    }
    Ok(HoldOut {
        views,
        view_set,
        package_dir: evaluation_package.to_path_buf(),
        package_sha256,
    })
}

/// The thresholds of an evaluation.
///
/// The thresholds of the report of the run are used when they are set; the
/// absolute defaults of the contract are used otherwise. The per-view
/// regression is fixed by the decision of the owner.
fn thresholds_of(run_dir: &Path) -> EvaluationThresholds {
    let mut thresholds = EvaluationThresholds {
        per_view_regression_max: PER_VIEW_REGRESSION_MAX,
        ..EvaluationThresholds::default()
    };
    let path = run_dir.join("report.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return thresholds,
    };
    let report: Value = match serde_json::from_str(&text) {
        Ok(report) => report,
        Err(_) => return thresholds,
    };
    let values = match report
        .get("quality")
        .and_then(|quality| quality.get("thresholds"))
    {
        Some(values) => values,
        None => return thresholds,
    };
    let pick = |name: &str, fallback: f64| -> f64 {
        match values.get(name).and_then(|value| value.as_f64()) {
            Some(value) if value > 0.0 => value,
            _ => fallback,
        }
    };
    thresholds.source_mean_iou = pick("source_mean_iou", thresholds.source_mean_iou);
    thresholds.source_min_iou = pick("source_min_iou", thresholds.source_min_iou);
    thresholds.evaluation_mean_iou = pick("evaluation_mean_iou", thresholds.evaluation_mean_iou);
    thresholds.evaluation_min_iou = pick("evaluation_min_iou", thresholds.evaluation_min_iou);
    thresholds.bbox_relative_error_max = pick(
        "bbox_relative_error_max",
        thresholds.bbox_relative_error_max,
    );
    thresholds
}

/// The immutable report of the run (None when the run carries none).
fn read_run_report(run_dir: &Path) -> Option<Value> {
    let path = run_dir.join("report.json");
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The state of the visible result of an evaluation (used by the CLI).
pub fn verdict_of(report: &EvaluationReport) -> TriState {
    report.verdict()
}

#[cfg(test)]
mod tests {
    use super::absolute_run;
    use std::path::Path;

    #[test]
    fn a_relative_run_directory_is_resolved_before_the_render() {
        // The Blender worker rejects a relative workspace_root; the evaluator
        // must resolve the run directory even when the CLI was given a relative
        // path (regression: RENDER_FAILED "workspace_root must be an absolute
        // path").
        let absolute = absolute_run(Path::new("examples/edit/run-0001"))
            .expect("the relative path is resolved");
        assert!(absolute.is_absolute());
    }
}
