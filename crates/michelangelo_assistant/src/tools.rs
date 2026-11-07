//! The four tools of the assistant surface.
//!
//! The tools are the only entry points of the surface:
//!
//! | Tool | Effect |
//! |---|---|
//! | `get_task` | writes and returns the bounded task of a run |
//! | `validate_package` | validates an input package |
//! | `submit_candidate` | validates and routes a candidate into the store |
//! | `get_report` | summarises the reconstruction report of a run |
//!
//! Every tool checks the task, the policy and the budget of the run before it
//! touches a file. No tool applies a candidate: the decision of a submission
//! is always `proposed`.

use std::path::{Path, PathBuf};

use michelangelo_edit::apply::submit_candidate as edit_submit;
use michelangelo_edit::candidate::{parse_candidate, EditCandidate, Region};
use michelangelo_edit::scene::write_bytes_atomic;
use michelangelo_edit::validate::{Decision, DecisionReport};
use michelangelo_input::{validate_package as validate_input_package, ValidateOptions};
use michelangelo_recon::report::ReconstructionReport;

use crate::budget::BudgetStore;
use crate::error::AssistantError;
use crate::policy::AssistantPolicy;
use crate::task::{generate_task, next_task_id, write_task, AssistantTask, TaskRequest};
use crate::{sanitize_paths, Clock, ASSISTANT_DIR_NAME, TMP_DIR_NAME};

/// The name of the candidate document inside a candidate directory.
pub const CANDIDATE_FILE_NAME: &str = "candidate.json";
/// The name of the reconstruction report inside a run directory.
pub const REPORT_FILE_NAME: &str = "report.json";

/// The context of a tool call.
pub struct ToolContext<'a> {
    /// The root of the workspace (used to sanitize the messages).
    pub workspace: PathBuf,
    /// The policy of the workspace.
    pub policy: &'a AssistantPolicy,
    /// The clock of the store.
    pub clock: &'a dyn Clock,
}

impl<'a> ToolContext<'a> {
    /// The context of a tool call.
    pub fn new(workspace: PathBuf, policy: &'a AssistantPolicy, clock: &'a dyn Clock) -> Self {
        ToolContext {
            workspace,
            policy,
            clock,
        }
    }

    /// Remove the absolute roots of a message.
    pub fn sanitize(&self, message: &str) -> String {
        sanitize_paths(message, &[self.workspace.as_path()])
    }
}

/// One diagnostic of a package summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDiagnostic {
    /// The validation level (1 or 2).
    pub level: u8,
    /// The stable code of the diagnostic.
    pub code: String,
    /// The severity of the diagnostic.
    pub severity: String,
    /// The address of the offending element.
    pub address: String,
    /// The human readable message.
    pub message: String,
}

/// The summary of a validated input package.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSummary {
    /// The schema of the validation report.
    pub schema: String,
    /// The identifier of the package.
    pub package_id: String,
    /// The revision of the package.
    pub revision: u32,
    /// The validation status of the package.
    pub status: String,
    /// Whether the format and safety level passed.
    pub format_safety_ok: bool,
    /// Whether the sufficiency level passed.
    pub sufficiency_ok: bool,
    /// The completeness of the package.
    pub completeness: String,
    /// The number of calibrated views of the package.
    pub calibrated_views: u32,
    /// The number of referenced resources.
    pub resources: usize,
    /// The diagnostics of the package.
    pub diagnostics: Vec<PackageDiagnostic>,
}

/// One unknown of a report summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportUnknown {
    /// The identifier of the unknown.
    pub id: String,
    /// The outcome of the unknown.
    pub status: String,
    /// A human readable detail.
    pub detail: String,
}

/// One supplement of a report summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSupplement {
    /// The identifier of the supplement.
    pub supplement_id: String,
    /// The outcome of the supplement.
    pub status: String,
    /// The rule of the supplement.
    pub rule: String,
}

/// The thresholds of a report summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportThresholds {
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
}

/// The summary of the reconstruction report of a run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSummary {
    /// The schema of the reconstruction report.
    pub schema: String,
    /// The identifier of the run.
    pub run_id: String,
    /// The identifier of the package.
    pub package_id: String,
    /// The revision of the package.
    pub package_revision: u32,
    /// The overall status of the run.
    pub status: String,
    /// The status of the quality comparison.
    pub quality_status: String,
    /// The mean IoU of the source views.
    pub source_mean_iou: f64,
    /// The smallest IoU of the source views.
    pub source_min_iou: f64,
    /// The thresholds that were applied.
    pub thresholds: ReportThresholds,
    /// The unknowns of the build.
    pub unknowns: Vec<ReportUnknown>,
    /// The supplements of the build.
    pub supplements: Vec<ReportSupplement>,
    /// The items that stayed unresolved.
    pub unresolved: Vec<String>,
}

/// The result of a tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// The bounded task of a run.
    Task {
        /// The task.
        task: Box<AssistantTask>,
        /// The run-relative path of the written task.
        path: String,
    },
    /// The summary of a validated package.
    Package(PackageSummary),
    /// The decision report of a submitted candidate.
    Decision(Box<DecisionReport>),
    /// The summary of the report of a run.
    Report(ReportSummary),
}

impl ToolOutcome {
    /// The JSON representation of the outcome.
    pub fn to_value(&self) -> Result<serde_json::Value, AssistantError> {
        let value = match self {
            ToolOutcome::Task { task, path } => serde_json::json!({
                "schema": crate::TASK_SCHEMA,
                "path": path,
                "task": task,
            }),
            ToolOutcome::Package(summary) => serde_json::to_value(summary)?,
            ToolOutcome::Decision(report) => serde_json::to_value(report)?,
            ToolOutcome::Report(summary) => serde_json::to_value(summary)?,
        };
        Ok(value)
    }
}

/// Generate and write the bounded task of a run.
///
/// The task allows every component of the scene that is not protected.
pub fn get_task(ctx: &ToolContext<'_>, run_dir: &Path) -> Result<ToolOutcome, AssistantError> {
    let task_id = next_task_id(run_dir)?;
    let task = generate_task(
        run_dir,
        &task_id,
        &TaskRequest::new(),
        ctx.policy,
        ctx.clock,
    )?;
    write_task(run_dir, &task)?;
    let path = format!("{ASSISTANT_DIR_NAME}/tasks/{task_id}.json");
    Ok(ToolOutcome::Task {
        task: Box::new(task),
        path,
    })
}

/// Validate an input package and summarise the report.
pub fn validate_package(
    ctx: &ToolContext<'_>,
    package_dir: &Path,
) -> Result<ToolOutcome, AssistantError> {
    if !package_dir.is_dir() {
        return Err(AssistantError::NotFound {
            message: "the package directory does not exist".to_string(),
        });
    }
    let report = validate_input_package(package_dir, &ValidateOptions::default());
    let summary = PackageSummary {
        schema: report.schema.clone(),
        package_id: report.package_id.clone(),
        revision: report.revision,
        status: match report.status {
            michelangelo_input::ReportStatus::ValidComplete => "valid_complete",
            michelangelo_input::ReportStatus::ValidPartial => "valid_partial",
            michelangelo_input::ReportStatus::Invalid => "invalid",
        }
        .to_string(),
        format_safety_ok: report.levels.format_safety.ok,
        sufficiency_ok: report.levels.sufficiency.ok,
        completeness: match report.completeness.status {
            michelangelo_input::CompletenessStatus::Complete => "complete",
            michelangelo_input::CompletenessStatus::Partial => "partial",
        }
        .to_string(),
        calibrated_views: report.completeness.calibrated_views,
        resources: report.resources.len(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(|diagnostic| PackageDiagnostic {
                level: diagnostic.level,
                code: diagnostic.code.clone(),
                severity: match diagnostic.severity {
                    michelangelo_input::Severity::Error => "error",
                    michelangelo_input::Severity::Warning => "warning",
                }
                .to_string(),
                address: diagnostic.address.clone(),
                message: diagnostic.message.clone(),
            })
            .collect(),
    };
    let _ = ctx;
    Ok(ToolOutcome::Package(summary))
}

/// Summarise the reconstruction report of a run.
pub fn get_report(ctx: &ToolContext<'_>, run_dir: &Path) -> Result<ToolOutcome, AssistantError> {
    let path = run_dir.join(REPORT_FILE_NAME);
    let report = ReconstructionReport::load(&path).map_err(|error| AssistantError::NotFound {
        message: format!("the reconstruction report cannot be read: {error}"),
    })?;
    let summary = ReportSummary {
        schema: report.schema.clone(),
        run_id: report.run.run_id.clone(),
        package_id: report.package.package_id.clone(),
        package_revision: report.package.revision,
        status: report.status.as_str().to_string(),
        quality_status: report.quality.status.clone(),
        source_mean_iou: report.quality.source.mean_iou,
        source_min_iou: report.quality.source.min_iou,
        thresholds: ReportThresholds {
            source_mean_iou: report.quality.thresholds.source_mean_iou,
            source_min_iou: report.quality.thresholds.source_min_iou,
            evaluation_mean_iou: report.quality.thresholds.evaluation_mean_iou,
            evaluation_min_iou: report.quality.thresholds.evaluation_min_iou,
            bbox_relative_error_max: report.quality.thresholds.bbox_relative_error_max,
        },
        unknowns: report
            .build
            .unknowns
            .iter()
            .map(|unknown| ReportUnknown {
                id: unknown.id.clone(),
                status: unknown.status.clone(),
                detail: unknown.detail.clone(),
            })
            .collect(),
        supplements: report
            .build
            .supplements
            .iter()
            .map(|supplement| ReportSupplement {
                supplement_id: supplement.supplement_id.clone(),
                status: supplement.status.clone(),
                rule: supplement.rule.clone(),
            })
            .collect(),
        unresolved: report
            .unresolved
            .iter()
            .map(|item| item.id.clone())
            .collect(),
    };
    let _ = ctx;
    Ok(ToolOutcome::Report(summary))
}

/// Validate a candidate against the task and route it into the store.
///
/// The decision of the submission is always `proposed`: the candidate waits
/// for an explicit author acceptance (`michelangelo edit accept`).
pub fn submit_candidate(
    ctx: &ToolContext<'_>,
    run_dir: &Path,
    task: &AssistantTask,
    candidate_dir: &Path,
) -> Result<ToolOutcome, AssistantError> {
    if !ctx.policy.allow_submit {
        return Err(AssistantError::PolicyDenied {
            message: "the assistant policy does not allow a submission".to_string(),
        });
    }
    if !candidate_dir.is_dir() {
        return Err(AssistantError::NotFound {
            message: "the candidate directory does not exist".to_string(),
        });
    }

    let store = BudgetStore::new(run_dir);
    let mut counters = store.load()?;
    let max_calls = ctx.policy.effective_max_calls(task.budget.max_calls);
    let max_bytes = ctx
        .policy
        .effective_max_candidate_bytes(task.budget.max_candidate_bytes);
    store.check_call(&counters, max_calls)?;
    let size = directory_size(candidate_dir)?;
    store.check_bytes(&counters, size, max_bytes)?;

    let candidate = read_candidate(candidate_dir)?;
    let region = validate_candidate_against_task(&candidate, task)?;

    let report = submit_to_store(ctx, run_dir, candidate_dir, &candidate, region)?;
    match report.decision {
        Decision::Proposed => {
            counters.candidates_submitted += 1;
        }
        Decision::Rejected => {}
        Decision::Error => {
            return Err(AssistantError::TaskInvalid {
                message: ctx.sanitize(&format!(
                    "the candidate '{}' could not be interpreted",
                    report.candidate_id
                )),
            })
        }
        Decision::Accepted => {
            return Err(AssistantError::Invalid {
                message: "the assistant must never apply a candidate".to_string(),
            })
        }
    }

    counters.calls_used += 1;
    counters.bytes_submitted = counters.bytes_submitted.saturating_add(size);
    let now = ctx.clock.now_rfc3339_utc();
    counters.last_call_at = Some(now.clone());
    counters.updated_at = Some(now);
    store.save(&counters)?;

    Ok(ToolOutcome::Decision(Box::new(report)))
}

/// Read and parse the candidate document of a candidate directory.
pub fn read_candidate(candidate_dir: &Path) -> Result<EditCandidate, AssistantError> {
    let path = candidate_dir.join(CANDIDATE_FILE_NAME);
    let text = std::fs::read_to_string(&path).map_err(|error| AssistantError::NotFound {
        message: format!("the candidate document cannot be read: {error}"),
    })?;
    parse_candidate(&text).map_err(|error| AssistantError::TaskInvalid {
        message: format!("the candidate is not usable: {error}"),
    })
}

/// Check that a candidate stays inside the scope of the task.
pub fn validate_candidate_against_task(
    candidate: &EditCandidate,
    task: &AssistantTask,
) -> Result<Region, AssistantError> {
    if candidate.base.package_id != task.base.package_id
        || candidate.base.package_revision != task.base.package_revision
    {
        return Err(AssistantError::TaskInvalid {
            message: "the candidate was built on another package than the task".to_string(),
        });
    }
    if candidate.base.run_id != task.base.run_id {
        return Err(AssistantError::TaskInvalid {
            message: "the candidate was built on another run than the task".to_string(),
        });
    }
    if candidate.base.scene_revision != task.base.scene_revision {
        return Err(AssistantError::TaskInvalid {
            message: format!(
                "the candidate was built on the revision '{}' but the task is built on '{}'",
                candidate.base.scene_revision, task.base.scene_revision
            ),
        });
    }
    if candidate.task.allowed_components.is_empty() {
        return Err(AssistantError::TaskInvalid {
            message: "the candidate allows no component".to_string(),
        });
    }
    for name in &candidate.task.allowed_components {
        if !task.allowed_components.contains(name) {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the candidate allows the component '{name}' which the task does not allow"
                ),
            });
        }
    }
    for name in &task.protected {
        if !candidate.task.protected.contains(name) {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the candidate does not protect the component '{name}' of the task"
                ),
            });
        }
    }
    for change in &candidate.changes {
        if !task.allowed_components.contains(&change.component) {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the candidate changes the component '{}' which the task does not allow",
                    change.component
                ),
            });
        }
        if task.protected.contains(&change.component) {
            return Err(AssistantError::TaskInvalid {
                message: format!(
                    "the candidate changes the protected component '{}'",
                    change.component
                ),
            });
        }
    }
    let region = match intersect_regions(&candidate.task.allowed_region, &task.allowed_region) {
        Some(region) => region,
        None => {
            return Err(AssistantError::TaskInvalid {
                message: "the region of the candidate does not intersect the region of the task"
                    .to_string(),
            })
        }
    };
    Ok(region)
}

/// The intersection of two regions (None when they do not overlap).
fn intersect_regions(left: &Region, right: &Region) -> Option<Region> {
    let mut min = [0.0_f64; 3];
    let mut max = [0.0_f64; 3];
    for axis in 0..3 {
        min[axis] = left.min[axis].max(right.min[axis]);
        max[axis] = left.max[axis].min(right.max[axis]);
        if min[axis] >= max[axis] {
            return None;
        }
    }
    Some(Region { min, max })
}

/// Stage the candidate and route it through the local edit boundary.
///
/// The staged candidate carries the origin `ai_author` and requires an
/// explicit author acceptance, so the submission can never apply it.
fn submit_to_store(
    ctx: &ToolContext<'_>,
    run_dir: &Path,
    candidate_dir: &Path,
    candidate: &EditCandidate,
    region: Region,
) -> Result<DecisionReport, AssistantError> {
    let staging_root = run_dir.join(ASSISTANT_DIR_NAME).join(TMP_DIR_NAME);
    let staging = staging_root.join(&candidate.candidate_id);
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|error| {
            AssistantError::io(
                &staging,
                format!("the staging directory cannot be cleared: {error}"),
            )
        })?;
    }
    std::fs::create_dir_all(&staging).map_err(|error| {
        AssistantError::io(
            &staging,
            format!("the staging directory cannot be created: {error}"),
        )
    })?;

    let outcome = stage_and_submit(ctx, run_dir, candidate_dir, &staging, region);
    let _ = std::fs::remove_dir_all(&staging);
    outcome
}

/// Copy the candidate into the staging directory and submit it.
fn stage_and_submit(
    ctx: &ToolContext<'_>,
    run_dir: &Path,
    candidate_dir: &Path,
    staging: &Path,
    region: Region,
) -> Result<DecisionReport, AssistantError> {
    let source = candidate_dir.join(CANDIDATE_FILE_NAME);
    let target = staging.join(CANDIDATE_FILE_NAME);
    let text = std::fs::read_to_string(&source).map_err(|error| AssistantError::NotFound {
        message: format!("the candidate document cannot be read: {error}"),
    })?;
    let mut value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| AssistantError::TaskInvalid {
            message: format!("the candidate document is not valid JSON: {error}"),
        })?;
    if let Some(provenance) = value
        .get_mut("provenance")
        .and_then(|value| value.as_object_mut())
    {
        provenance.insert(
            "origin".to_string(),
            serde_json::Value::String("ai_author".to_string()),
        );
        let detail = provenance
            .get("detail")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        provenance.insert(
            "detail".to_string(),
            serde_json::Value::String(format!(
                "{detail} (submitted through the assistant surface)"
            )),
        );
    }
    if let Some(task) = value
        .get_mut("task")
        .and_then(|value| value.as_object_mut())
    {
        task.insert(
            "require_author_acceptance".to_string(),
            serde_json::Value::Bool(true),
        );
        // The region of the task is the authority: the effective region is
        // the intersection of the region of the candidate and of the task.
        task.insert(
            "allowed_region".to_string(),
            serde_json::json!({
                "min": region.min,
                "max": region.max,
            }),
        );
    }
    let staged_text =
        serde_json::to_string_pretty(&value).map_err(|error| AssistantError::Invalid {
            message: format!("the staged candidate cannot be serialized: {error}"),
        })?;
    write_bytes_atomic(
        &target,
        format!(
            "{staged_text}
"
        )
        .as_bytes(),
    )?;

    let parsed = parse_candidate(&staged_text).map_err(|error| AssistantError::TaskInvalid {
        message: format!("the staged candidate is not usable: {error}"),
    })?;
    for change in &parsed.changes {
        let relative = Path::new(&change.obj);
        let from = candidate_dir.join(relative);
        let to = staging.join(relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AssistantError::io(
                    parent,
                    format!("the staging directory cannot be created: {error}"),
                )
            })?;
        }
        std::fs::copy(&from, &to).map_err(|error| {
            AssistantError::io(&from, format!("the geometry cannot be staged: {error}"))
        })?;
    }

    edit_submit(run_dir, staging, ctx.clock).map_err(|error| {
        let message = ctx.sanitize(&error.to_string());
        AssistantError::TaskInvalid {
            message: format!("the candidate could not be submitted: {message}"),
        }
    })
}

/// The number of bytes of a directory tree.
pub fn directory_size(directory: &Path) -> Result<u64, AssistantError> {
    let mut total = 0_u64;
    let mut stack: Vec<PathBuf> = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| {
            AssistantError::io(
                &current,
                format!("the candidate directory cannot be read: {error}"),
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                AssistantError::io(
                    &current,
                    format!("the candidate directory cannot be read: {error}"),
                )
            })?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|error| {
                AssistantError::io(&path, format!("the candidate cannot be read: {error}"))
            })?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let meta = entry.metadata().map_err(|error| {
                    AssistantError::io(&path, format!("the candidate cannot be read: {error}"))
                })?;
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}
