//! The bounded assistant task (`michelangelo.assistant-task/0`).
//!
//! The task is generated deterministically from a reconstruction run: it names
//! the base (the package and the current scene revision), the components the
//! assistant may change, the region it may touch, the components it must not
//! touch, the references it may look at, the acceptance criteria, the budget
//! and the data policy.
//!
//! The task never carries an absolute path, never names the evaluation data
//! and never names the ground truth.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use michelangelo_edit::candidate::Region;
use michelangelo_edit::scene::{load_current, load_scene, Scene, SceneStore};
use michelangelo_input::load_manifest;
use michelangelo_input::manifest::{InputPackage, UnknownPolicy, ViewKind};
use michelangelo_recon::report::ReconstructionReport;

use crate::error::AssistantError;
use crate::policy::AssistantPolicy;
use crate::{Clock, ASSISTANT_DIR_NAME, TASKS_DIR_NAME, TASK_SCHEMA};

/// The relative margin that the default region adds to the bounding box of
/// the allowed components (5 %).
pub const REGION_MARGIN_RATIO: f64 = 0.05;
/// The default number of seconds a task may take.
pub const DEFAULT_MAX_SECONDS: u64 = 600;
/// The default cost of a task (no paid call is ever made).
pub const DEFAULT_MAX_COST_USD: f64 = 0.0;
/// The reference kinds a task may carry.
pub const ALLOWED_REFERENCE_KINDS: [&str; 3] = ["mask", "silhouette_render", "report"];

/// The base a task is built on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskBase {
    /// The identifier of the input package.
    pub package_id: String,
    /// The revision of the input package.
    pub package_revision: u32,
    /// The identifier of the run (the name of the run directory).
    pub run_id: String,
    /// The scene revision the task is built on.
    pub scene_revision: String,
}

/// One unknown of the package, as far as the task is concerned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskUnknown {
    /// The identifier of the unknown.
    pub id: String,
    /// A human readable description.
    pub description: String,
    /// The policy of the unknown (`author_required`, `rule_allowed` or
    /// `stub_allowed`).
    pub policy: String,
}

/// One supplement of the build, as far as the task is concerned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSupplement {
    /// The identifier of the supplement.
    pub supplement_id: String,
    /// The rule of the supplement.
    pub kind: String,
    /// The outcome of the supplement.
    pub status: String,
}

/// One reference view of the task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskView {
    /// The identifier of the view.
    pub view_id: String,
    /// The run-relative path of the mask of the view.
    pub mask: Option<String>,
    /// The run-relative path of the image of the view.
    pub image: Option<String>,
}

/// One silhouette render of the task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRender {
    /// The identifier of the render (the stem of the file name).
    pub view_id: String,
    /// The run-relative path of the render.
    pub path: String,
}

/// The references a task may look at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReferences {
    /// The calibrated views of the package.
    pub views: Vec<TaskView>,
    /// The silhouette renders of the run.
    pub renders: Vec<TaskRender>,
    /// The run-relative path of the quality report.
    pub quality_report: Option<String>,
    /// The run-relative path of the build report.
    pub build_report: Option<String>,
}

/// The measurable acceptance criteria of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCriteria {
    /// The smallest IoU of the source views the result must reach.
    pub min_source_iou: f64,
    /// The smallest IoU of the held-out views the result must reach.
    pub min_evaluation_iou: f64,
    /// The largest accepted relative error of a declared measurement.
    pub measurement_relative_error_max: f64,
    /// Whether a change of a protected component refuses the result.
    pub reject_if_protected_changed: bool,
}

/// The budget of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskBudget {
    /// The maximum number of assistant calls.
    pub max_calls: u64,
    /// The maximum number of seconds.
    pub max_seconds: u64,
    /// The maximum cost in US dollars.
    pub max_cost_usd: f64,
    /// The maximum number of bytes a candidate may carry.
    pub max_candidate_bytes: u64,
}

/// The data policy of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataPolicy {
    /// Whether the assistant may use the network (never).
    pub allow_network: bool,
    /// Whether the assistant may call an external service (never).
    pub allow_external_service: bool,
    /// The kinds of reference the task carries.
    pub allowed_reference_kinds: Vec<String>,
}

/// The acceptance criteria of a task that is narrowed to one unknown.
///
/// The criteria are the conditions of the resolution of the unknown
/// (contract §7): the change must touch the binding, the evaluation must
/// pass and the author must confirm the claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownCriteria {
    /// The identifier of the unknown.
    pub unknown_id: String,
    /// The components the task may change (a subset of the components of the
    /// binding of the unknown).
    pub components: Vec<String>,
    /// The region the task may touch (inside the region of the binding of the
    /// unknown).
    pub region: Region,
    /// The version of the binding document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_version: Option<String>,
    /// The claim of the author about the unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<String>,
    /// Whether an author confirmation is required to close the unknown.
    pub requires_confirmation: bool,
    /// Whether a registered evaluation is required to close the unknown.
    pub requires_evaluation: bool,
    /// Whether the package of the run requires an author acceptance.
    pub package_requires_author_acceptance: bool,
    /// The measurable conditions the result must satisfy.
    pub conditions: Vec<String>,
}

/// The provenance of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProvenance {
    /// The origin of the task.
    pub origin: String,
    /// A human readable detail.
    pub detail: String,
}

/// The bounded assistant task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTask {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the task.
    pub task_id: String,
    /// The creation time of the task (RFC 3339, UTC).
    pub created_at: String,
    /// The base of the task.
    pub base: TaskBase,
    /// The components the task may change (sorted).
    pub allowed_components: Vec<String>,
    /// The region the task may touch.
    pub allowed_region: Region,
    /// The components the task must not touch (sorted).
    pub protected: Vec<String>,
    /// The unknowns of the package (sorted by identifier).
    pub unknowns: Vec<TaskUnknown>,
    /// The supplements of the build (sorted by identifier).
    pub supplements: Vec<TaskSupplement>,
    /// The references the task may look at.
    pub references: TaskReferences,
    /// The measurable acceptance criteria.
    pub acceptance_criteria: AcceptanceCriteria,
    /// The budget of the task.
    pub budget: TaskBudget,
    /// The data policy of the task.
    pub data_policy: DataPolicy,
    /// The provenance of the task.
    pub provenance: TaskProvenance,
    /// The unknown the task is narrowed to (None for a general task).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_id: Option<String>,
    /// The acceptance criteria of the unknown (None for a general task).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_criteria: Option<UnknownCriteria>,
}

impl AssistantTask {
    /// The JSON representation of the task (pretty, deterministic).
    pub fn to_json(&self) -> Result<String, AssistantError> {
        serde_json::to_string_pretty(self).map_err(|error| AssistantError::Invalid {
            message: format!("the assistant task cannot be serialized: {error}"),
        })
    }

    /// Parse a task document.
    pub fn parse(text: &str) -> Result<AssistantTask, AssistantError> {
        let task: AssistantTask =
            serde_json::from_str(text).map_err(|error| AssistantError::TaskInvalid {
                message: format!("the assistant task is not usable: {error}"),
            })?;
        if task.schema != TASK_SCHEMA {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the assistant task declares the schema '{}' which is not supported",
                    task.schema
                ),
            });
        }
        if task.allowed_components.is_empty() {
            return Err(AssistantError::TaskInvalid {
                message: "the assistant task allows no component".to_string(),
            });
        }
        if !task.allowed_region.is_valid() {
            return Err(AssistantError::TaskInvalid {
                message: "the allowed region of the task is empty or inverted".to_string(),
            });
        }
        Ok(task)
    }
}

/// The request of a task generation.
#[derive(Debug, Clone, Default)]
pub struct TaskRequest {
    /// The components the task may change (None: every component that is not
    /// protected).
    pub allowed: Option<Vec<String>>,
    /// The region the task may touch (None: the bounding box of the allowed
    /// components with a margin of 5 %).
    pub region: Option<Region>,
    /// The unknown the task is narrowed to (None: a general task).
    ///
    /// The scope of the task is then the binding of the unknown: its
    /// components and its region.
    pub unknown: Option<String>,
}

impl TaskRequest {
    /// A request without an explicit scope.
    pub fn new() -> Self {
        TaskRequest::default()
    }

    /// A request for an explicit set of components.
    pub fn with_allowed(allowed: &[String]) -> Self {
        TaskRequest {
            allowed: Some(allowed.to_vec()),
            region: None,
            unknown: None,
        }
    }

    /// A request that is narrowed to one unknown.
    pub fn with_unknown(unknown_id: &str) -> Self {
        TaskRequest {
            allowed: None,
            region: None,
            unknown: Some(unknown_id.to_string()),
        }
    }
}

/// The directory of the tasks of a run.
pub fn tasks_dir(run_dir: &Path) -> PathBuf {
    run_dir.join(ASSISTANT_DIR_NAME).join(TASKS_DIR_NAME)
}

/// The path of a task of a run.
pub fn task_path(run_dir: &Path, task_id: &str) -> PathBuf {
    tasks_dir(run_dir).join(format!("{task_id}.json"))
}

/// The next free identifier of a task of a run.
///
/// The identifier is `asst-NNNN`; the numbers of the existing tasks are
/// never reused.
pub fn next_task_id(run_dir: &Path) -> Result<String, AssistantError> {
    let directory = tasks_dir(run_dir);
    let mut highest = 0_u32;
    if directory.is_dir() {
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            AssistantError::io(&directory, format!("the tasks cannot be read: {error}"))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                AssistantError::io(&directory, format!("the tasks cannot be read: {error}"))
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(number) = name
                .strip_prefix("asst-")
                .and_then(|rest| rest.strip_suffix(".json"))
                .and_then(|rest| rest.parse::<u32>().ok())
            {
                if number > highest {
                    highest = number;
                }
            }
        }
    }
    Ok(format!("asst-{:04}", highest + 1))
}

/// Write a task into the store of a run (atomic).
pub fn write_task(run_dir: &Path, task: &AssistantTask) -> Result<PathBuf, AssistantError> {
    let path = task_path(run_dir, &task.task_id);
    let text = task.to_json()?;
    michelangelo_edit::scene::write_bytes_atomic(
        &path,
        format!(
            "{text}
"
        )
        .as_bytes(),
    )?;
    Ok(path)
}

/// Read a task from a file.
pub fn load_task(path: &Path) -> Result<AssistantTask, AssistantError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        AssistantError::io(path, format!("the assistant task cannot be read: {error}"))
    })?;
    AssistantTask::parse(&text)
}

/// Generate the bounded task of a run.
///
/// The generation is deterministic: the same run, the same request and the
/// same policy always produce the same task (the creation time comes from
/// the injected clock).
pub fn generate_task(
    run_dir: &Path,
    task_id: &str,
    request: &TaskRequest,
    policy: &AssistantPolicy,
    clock: &dyn Clock,
) -> Result<AssistantTask, AssistantError> {
    if !run_dir.is_dir() {
        return Err(AssistantError::NotFound {
            message: format!("the run directory '{}' does not exist", run_dir.display()),
        });
    }
    if task_id.trim().is_empty() {
        return Err(AssistantError::TaskInvalid {
            message: "the task id must not be empty".to_string(),
        });
    }

    let store = SceneStore::new(run_dir);
    let pointer = load_current(&store)?.ok_or_else(|| AssistantError::NotFound {
        message: "the scene store of the run is not initialized: run 'michelangelo edit init'                   first"
            .to_string(),
    })?;
    let scene = load_scene(&store, &pointer.scene_revision)?;

    let report_path = run_dir.join("report.json");
    let report =
        ReconstructionReport::load(&report_path).map_err(|error| AssistantError::NotFound {
            message: format!("the reconstruction report of the run cannot be read: {error}"),
        })?;

    let package = load_package(run_dir);
    // The binding of the unknown comes from the bundle of the revision the
    // store points to: the globally latest binding document is never used
    // instead (a rolled back binding is never reactivated).
    let binding = match &request.unknown {
        Some(unknown_id) => Some(binding_of(&store, &pointer.scene_revision, unknown_id)?),
        None => None,
    };
    let (allowed, protected) = resolve_scope(&scene, request, binding.as_ref().map(|(b, _)| b))?;
    let allowed_region = resolve_region(
        &scene,
        &allowed,
        request.region,
        binding.as_ref().map(|(b, _)| b),
    )?;

    let references = collect_references(run_dir, package.as_ref());
    let mut unknowns = collect_unknowns(&report, package.as_ref());
    let supplements = collect_supplements(&report);
    if let Some((binding, _version)) = &binding {
        // The references of a narrowed task describe the unknown only.
        unknowns.retain(|unknown| unknown.id == binding.unknown_id);
    }

    let thresholds = &report.quality.thresholds;
    let acceptance_criteria = AcceptanceCriteria {
        min_source_iou: thresholds.source_min_iou,
        min_evaluation_iou: thresholds.evaluation_min_iou,
        measurement_relative_error_max: thresholds.bbox_relative_error_max,
        reject_if_protected_changed: true,
    };

    let budget = TaskBudget {
        max_calls: policy.max_calls_per_run,
        max_seconds: DEFAULT_MAX_SECONDS,
        max_cost_usd: DEFAULT_MAX_COST_USD,
        max_candidate_bytes: policy.max_candidate_bytes,
    };

    let data_policy = DataPolicy {
        allow_network: false,
        allow_external_service: false,
        allowed_reference_kinds: ALLOWED_REFERENCE_KINDS
            .iter()
            .map(|kind| kind.to_string())
            .collect(),
    };

    // The criteria of a narrowed unknown carry the effective scope of the
    // task: the components and the region it may touch.
    let criteria = binding.as_ref().map(|(binding, version)| {
        unknown_criteria(binding, version, &store, &allowed, &allowed_region)
    });

    Ok(AssistantTask {
        schema: TASK_SCHEMA.to_string(),
        task_id: task_id.to_string(),
        created_at: clock.now_rfc3339_utc(),
        base: TaskBase {
            package_id: scene.source.package_id.clone(),
            package_revision: scene.source.package_revision,
            run_id: scene.run_id.clone(),
            scene_revision: pointer.scene_revision.clone(),
        },
        allowed_components: allowed,
        allowed_region,
        protected,
        unknowns,
        supplements,
        references,
        acceptance_criteria,
        budget,
        data_policy,
        provenance: TaskProvenance {
            origin: "core".to_string(),
            detail: format!("bounded task generated from run '{}'", scene.run_id),
        },
        unknown_id: request.unknown.clone(),
        unknown_criteria: criteria,
    })
}

/// The binding of an unknown in the bundle of a revision.
///
/// The bundle of the revision is the authority (the same authority as
/// `unknown show`): the globally latest binding document is never used
/// instead, so a binding that a rollback discarded is never reactivated.
fn binding_of(
    store: &michelangelo_edit::scene::SceneStore,
    scene_revision: &str,
    unknown_id: &str,
) -> Result<(michelangelo_edit::unknowns::UnknownBinding, String), crate::error::AssistantError> {
    let binding = michelangelo_edit::unknowns::revision_binding(store, scene_revision, unknown_id)
        .map_err(crate::error::AssistantError::from)?;
    match binding {
        Some(binding) => Ok(binding),
        None => Err(crate::error::AssistantError::NotFound {
            message: format!(
                "the unknown '{unknown_id}' carries no binding in the bundle of the revision '{scene_revision}': run 'michelangelo unknown bind' first"
            ),
        }),
    }
}

/// The allowed and the protected components of a task.
///
/// The allowed set is a non-empty subset of the components of the scene; the
/// protected set is every other component of the scene, so the two sets are
/// always disjoint. A task that is narrowed to an unknown is bound to the
/// components of its binding: an explicit set of components may only
/// **narrow** the binding, never widen it.
fn resolve_scope(
    scene: &Scene,
    request: &TaskRequest,
    binding: Option<&michelangelo_edit::unknowns::UnknownBinding>,
) -> Result<(Vec<String>, Vec<String>), AssistantError> {
    let all: Vec<String> = scene
        .components
        .iter()
        .map(|component| component.name.clone())
        .collect();
    let protected_flags: Vec<String> = scene
        .components
        .iter()
        .filter(|component| component.protected)
        .map(|component| component.name.clone())
        .collect();

    let mut allowed: Vec<String> = match (&request.allowed, binding) {
        (Some(names), Some(binding)) => {
            // The scope of a narrowed task may only be narrowed: a component
            // that the binding does not cover is refused.
            for name in names {
                if !binding.components.contains(name) {
                    return Err(AssistantError::TaskInvalid {
                        message: format!(
                            "SCOPE_DENIED: the component '{name}' is not covered by the binding of the unknown '{}' (the binding covers {}): the scope of a narrowed task may only be narrowed",
                            binding.unknown_id,
                            binding.components.join(", ")
                        ),
                    });
                }
            }
            names.clone()
        }
        (Some(names), None) => names.clone(),
        (None, Some(binding)) => binding.components.clone(),
        (None, None) => all
            .iter()
            .filter(|name| !protected_flags.contains(name))
            .cloned()
            .collect(),
    };
    allowed.sort();
    allowed.dedup();

    if allowed.is_empty() {
        return Err(AssistantError::TaskInvalid {
            message: "the task allows no component: the scene has no editable component"
                .to_string(),
        });
    }
    for name in &allowed {
        if !all.contains(name) {
            return Err(AssistantError::TaskInvalid {
                message: format!("the component '{name}' is not part of the scene"),
            });
        }
        if protected_flags.contains(name) {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the component '{name}' is protected by the run and cannot be allowed"
                ),
            });
        }
    }

    let protected: Vec<String> = all
        .iter()
        .filter(|name| !allowed.contains(name))
        .cloned()
        .collect();

    Ok((allowed, protected))
}

/// The region of a task.
///
/// The default is the bounding box of the allowed components with a margin of
/// 5 % on every axis. A region must intersect every allowed component. The
/// region of a task that is narrowed to an unknown is the region of its
/// binding: an explicit region may only **narrow** it, never widen it.
fn resolve_region(
    scene: &Scene,
    allowed: &[String],
    requested: Option<Region>,
    binding: Option<&michelangelo_edit::unknowns::UnknownBinding>,
) -> Result<Region, AssistantError> {
    let region = match requested {
        Some(region) => {
            // The scope of a narrowed task may only be narrowed: the region
            // must lie inside the region of the binding.
            if let Some(binding) = binding {
                if !region_is_inside(&region, &binding.region) {
                    return Err(AssistantError::TaskInvalid {
                        message: format!(
                            "SCOPE_DENIED: the region {} is not inside the region of the binding of the unknown '{}' ({}): the scope of a narrowed task may only be narrowed",
                            region_text(&region),
                            binding.unknown_id,
                            region_text(&binding.region),
                        ),
                    });
                }
            }
            region
        }
        None => match binding {
            Some(binding) => binding.region,
            None => default_region(scene, allowed)?,
        },
    };
    if !region.is_valid() {
        return Err(AssistantError::TaskInvalid {
            message: "the allowed region is empty or inverted".to_string(),
        });
    }
    for name in allowed {
        let component = match scene.component(name) {
            Some(component) => component,
            None => continue,
        };
        if !intersects(&region, &component.bbox_min, &component.bbox_max) {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the allowed region does not intersect the component '{name}' (TASK_EMPTY)"
                ),
            });
        }
    }
    Ok(region)
}

/// The default region of a task (the bounding box of the allowed components
/// with a margin of 5 %).
fn default_region(scene: &Scene, allowed: &[String]) -> Result<Region, AssistantError> {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut found = false;
    for name in allowed {
        let component = match scene.component(name) {
            Some(component) => component,
            None => continue,
        };
        found = true;
        for axis in 0..3 {
            if component.bbox_min[axis] < min[axis] {
                min[axis] = component.bbox_min[axis];
            }
            if component.bbox_max[axis] > max[axis] {
                max[axis] = component.bbox_max[axis];
            }
        }
    }
    if !found {
        return Err(AssistantError::TaskInvalid {
            message: "the task allows no component with a bounding box".to_string(),
        });
    }
    for axis in 0..3 {
        let extent = max[axis] - min[axis];
        let margin = if extent > 0.0 {
            extent * REGION_MARGIN_RATIO
        } else {
            0.001
        };
        min[axis] = round(min[axis] - margin);
        max[axis] = round(max[axis] + margin);
    }
    Ok(Region { min, max })
}

/// Round a coordinate to nine decimals.
fn round(value: f64) -> f64 {
    (value * 1_000_000_000.0).round() / 1_000_000_000.0
}

/// True when a region and a bounding box overlap.
fn intersects(region: &Region, bbox_min: &[f64; 3], bbox_max: &[f64; 3]) -> bool {
    (0..3).all(|axis| region.min[axis] <= bbox_max[axis] && bbox_min[axis] <= region.max[axis])
}

/// True when a region lies inside another one.
fn region_is_inside(region: &Region, outer: &Region) -> bool {
    (0..3).all(|axis| outer.min[axis] <= region.min[axis] && region.max[axis] <= outer.max[axis])
}

/// The text of a region (`[x, y, z]..[x, y, z]`).
fn region_text(region: &Region) -> String {
    format!(
        "[{}, {}, {}]..[{}, {}, {}]",
        region.min[0], region.min[1], region.min[2], region.max[0], region.max[1], region.max[2],
    )
}

/// The manifest of the package of a run (None when the run carries none).
fn load_package(run_dir: &Path) -> Option<InputPackage> {
    let package_dir = run_dir.join("package");
    if !package_dir.join("package.json").is_file() {
        return None;
    }
    load_manifest(&package_dir).ok()
}

/// The references of a task.
///
/// Only the calibrated views of the package, the silhouette renders of the
/// run and the build and quality reports of the run are referenced. The
/// evaluation data and the ground truth are never referenced.
fn collect_references(run_dir: &Path, package: Option<&InputPackage>) -> TaskReferences {
    let mut references = TaskReferences::default();

    if let Some(package) = package {
        for view in &package.views {
            if view.kind != ViewKind::Calibrated {
                continue;
            }
            let mask = existing_reference(run_dir, &format!("package/{}", view.mask.path));
            let image = existing_reference(run_dir, &format!("package/{}", view.image.path));
            if mask.is_none() && image.is_none() {
                continue;
            }
            references.views.push(TaskView {
                view_id: view.view_id.clone(),
                mask,
                image,
            });
        }
        references
            .views
            .sort_by(|left, right| left.view_id.cmp(&right.view_id));
    }

    let renders_dir = run_dir.join("renders");
    if renders_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&renders_dir) {
            let mut renders: Vec<TaskRender> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.to_ascii_lowercase().ends_with(".png") {
                    continue;
                }
                let path = format!("renders/{name}");
                if !entry.path().is_file() {
                    continue;
                }
                let view_id = name
                    .strip_suffix(".png")
                    .unwrap_or(name.as_str())
                    .to_string();
                renders.push(TaskRender { view_id, path });
            }
            renders.sort_by(|left, right| left.view_id.cmp(&right.view_id));
            references.renders = renders;
        }
    }

    if run_dir.join("quality_report.json").is_file() {
        references.quality_report = Some("quality_report.json".to_string());
    }
    if run_dir.join("build_report.json").is_file() {
        references.build_report = Some("build_report.json".to_string());
    }

    references
}

/// The run-relative path of a reference, when the file exists and the path is
/// safe.
fn existing_reference(run_dir: &Path, relative: &str) -> Option<String> {
    if !crate::is_safe_relative_path(relative) || crate::is_forbidden_reference(relative) {
        return None;
    }
    let path = run_dir.join(relative);
    if path.is_file() {
        Some(relative.to_string())
    } else {
        None
    }
}

/// The unknowns of a task.
///
/// The manifest of the package describes the unknown; the report of the run
/// is the fallback.
fn collect_unknowns(
    report: &ReconstructionReport,
    package: Option<&InputPackage>,
) -> Vec<TaskUnknown> {
    let mut unknowns: Vec<TaskUnknown> = Vec::new();
    if let Some(package) = package {
        for unknown in &package.unknowns {
            unknowns.push(TaskUnknown {
                id: unknown.id.clone(),
                description: unknown.description.clone(),
                policy: policy_text(unknown.policy).to_string(),
            });
        }
    }
    if unknowns.is_empty() {
        for unknown in &report.build.unknowns {
            unknowns.push(TaskUnknown {
                id: unknown.id.clone(),
                description: unknown.detail.clone(),
                policy: policy_from_status(&unknown.status).to_string(),
            });
        }
    }
    unknowns.sort_by(|left, right| left.id.cmp(&right.id));
    unknowns
}

/// The text of a policy of an unknown.
fn policy_text(policy: Option<UnknownPolicy>) -> &'static str {
    match policy {
        Some(UnknownPolicy::AuthorRequired) => "author_required",
        Some(UnknownPolicy::RuleAllowed) => "rule_allowed",
        Some(UnknownPolicy::StubAllowed) => "stub_allowed",
        None => "author_required",
    }
}

/// The policy of an unknown that is derived from the status of the build.
fn policy_from_status(status: &str) -> &'static str {
    match status {
        "stub" => "stub_allowed",
        "resolved" => "rule_allowed",
        _ => "author_required",
    }
}

/// The supplements of a task.
fn collect_supplements(report: &ReconstructionReport) -> Vec<TaskSupplement> {
    let mut supplements: Vec<TaskSupplement> = report
        .build
        .supplements
        .iter()
        .map(|supplement| TaskSupplement {
            supplement_id: supplement.supplement_id.clone(),
            kind: supplement.rule.clone(),
            status: supplement.status.clone(),
        })
        .collect();
    supplements.sort_by(|left, right| left.supplement_id.cmp(&right.supplement_id));
    supplements
}

/// The acceptance criteria of a task that is narrowed to one unknown.
///
/// The criteria carry the effective scope of the task: the components and
/// the region the task may touch. The scope is always a subset of the
/// binding of the unknown.
fn unknown_criteria(
    binding: &michelangelo_edit::unknowns::UnknownBinding,
    binding_version: &str,
    store: &michelangelo_edit::scene::SceneStore,
    components: &[String],
    region: &Region,
) -> UnknownCriteria {
    let policy = michelangelo_edit::unknowns::package_policy(store);
    UnknownCriteria {
        unknown_id: binding.unknown_id.clone(),
        components: components.to_vec(),
        region: *region,
        binding_version: Some(binding_version.to_string()),
        claim: binding.claim.clone(),
        requires_confirmation: true,
        requires_evaluation: true,
        package_requires_author_acceptance: policy.requires_author_acceptance(),
        conditions: vec![
            String::from(
                "the change touches the components of the binding and stays inside its region",
            ),
            String::from(
                "a registered evaluation of the new geometry passes the projection conformance and every required author geometric condition",
            ),
            String::from(
                "the author confirms the claim for this geometry and this evaluation before the unknown is closed",
            ),
            String::from(
                "the unknown is closed only in a revision that references the resolution",
            ),
        ],
    }
}
