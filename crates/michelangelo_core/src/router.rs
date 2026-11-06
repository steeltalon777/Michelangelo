use michelangelo_protocol::command::CommandEnvelope;
use michelangelo_protocol::error::{ErrorCode, ProtocolError};
use michelangelo_protocol::event::{
    EventEnvelope, JOB_CANCELLED, JOB_COMPLETED, JOB_FAILED, JOB_QUEUED, JOB_STARTED,
};
use michelangelo_protocol::method;
use michelangelo_protocol::response::ResponseEnvelope;
use michelangelo_protocol::snapshot::ProjectSnapshotDto;
use michelangelo_protocol::{
    BlenderJobResult, BlenderJobSpec, BlenderOperation, JobDto, JobStatus, PROTOCOL_VERSION,
    SAFE_WORKER_OPS,
};
use michelangelo_workspace::error::WorkspaceError;
use michelangelo_workspace::storage::Storage;
use std::path::{Component, Path};

use crate::service::CoreService;

pub type Handler = fn(&mut CoreService, &CommandEnvelope) -> ResponseEnvelope;

pub struct Router {
    handlers: Vec<(&'static str, Handler)>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn register(&mut self, method: &'static str, handler: Handler) {
        self.handlers.push((method, handler));
    }

    pub fn find(&self, method: &str) -> Option<Handler> {
        for (name, handler) in &self.handlers {
            if *name == method {
                return Some(*handler);
            }
        }
        None
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

pub fn default_router() -> Router {
    let mut r = Router::new();
    r.register(method::SYSTEM_PING, handle_ping);
    r.register(method::PROJECT_CREATE, handle_project_create);
    r.register(method::PROJECT_OPEN, handle_project_open);
    r.register(method::PROJECT_GET_SNAPSHOT, handle_project_get_snapshot);
    r.register(method::JOB_LIST, handle_job_list);
    r.register(method::JOB_GET, handle_job_get);
    r.register(method::JOB_CANCEL, handle_job_cancel);
    r.register(
        method::BLENDER_GET_CAPABILITIES,
        handle_blender_get_capabilities,
    );
    r.register(method::BLENDER_RUN_JOB, handle_blender_run_job);
    r
}

// ---------------------------------------------------------------------------
// Phase 1 handlers
// ---------------------------------------------------------------------------

fn handle_ping(_core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({
            "pong": true,
            "protocol_version": PROTOCOL_VERSION,
        }),
    )
}

fn handle_project_create(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let path_str = match cmd.params.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return ResponseEnvelope::error(
                Some(cmd.id.clone()),
                ProtocolError::new(ErrorCode::InvalidParams, "missing required param: 'path'"),
            );
        }
    };
    let name = cmd
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let path = Path::new(path_str);
    match core.workspace_ref().create_project(path, name) {
        Ok(dto) => {
            core.set_current_project(Some(dto.clone()));
            core.init_storage();
            ResponseEnvelope::success(&cmd.id, serde_json::json!(dto))
        }
        Err(e) => {
            let (code, msg) = map_workspace_error(&e);
            ResponseEnvelope::error(Some(cmd.id.clone()), ProtocolError::new(code, msg))
        }
    }
}

fn handle_project_open(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let path_str = match cmd.params.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return ResponseEnvelope::error(
                Some(cmd.id.clone()),
                ProtocolError::new(ErrorCode::InvalidParams, "missing required param: 'path'"),
            );
        }
    };

    let path = Path::new(path_str);
    match core.workspace_ref().open_project(path) {
        Ok(dto) => {
            core.set_current_project(Some(dto.clone()));
            core.init_storage();
            ResponseEnvelope::success(&cmd.id, serde_json::json!(dto))
        }
        Err(e) => {
            let (code, msg) = map_workspace_error(&e);
            ResponseEnvelope::error(Some(cmd.id.clone()), ProtocolError::new(code, msg))
        }
    }
}

fn handle_project_get_snapshot(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    match core.current_project() {
        Some(project) => {
            let mut snapshot = ProjectSnapshotDto::with_project(project.clone());
            if let Some(storage) = core.storage() {
                if let Ok(jobs) = storage.list_jobs(&project.id) {
                    snapshot.jobs = jobs.into_iter().filter_map(storage_job_to_dto).collect();
                }
            }
            ResponseEnvelope::success(&cmd.id, serde_json::json!(snapshot))
        }
        None => ResponseEnvelope::error(
            Some(cmd.id.clone()),
            ProtocolError::new(ErrorCode::WorkspaceNotOpen, "no project is currently open"),
        ),
    }
}

// ---------------------------------------------------------------------------
// Phase 2 — Job handlers
// ---------------------------------------------------------------------------

fn handle_job_list(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let project = match core.current_project() {
        Some(p) => p,
        None => return no_project_error(cmd),
    };
    let storage = match core.storage() {
        Some(s) => s,
        None => return storage_not_open_error(cmd),
    };
    match storage.list_jobs(&project.id) {
        Ok(jobs) => ResponseEnvelope::success(&cmd.id, serde_json::json!({"jobs": jobs})),
        Err(e) => internal_error(cmd, &format!("failed to list jobs: {e}")),
    }
}

fn handle_job_get(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let job_id = match cmd.params.get("job_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return invalid_params(cmd, "missing required param: 'job_id'"),
    };
    let storage = match core.storage() {
        Some(s) => s,
        None => return storage_not_open_error(cmd),
    };
    match storage.get_job(job_id) {
        Ok(Some(job)) => ResponseEnvelope::success(&cmd.id, job),
        Ok(None) => ResponseEnvelope::error(
            Some(cmd.id.clone()),
            ProtocolError::new(ErrorCode::InvalidParams, format!("job not found: {job_id}")),
        ),
        Err(e) => internal_error(cmd, &format!("failed to get job: {e}")),
    }
}

fn handle_job_cancel(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let job_id = match cmd.params.get("job_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return invalid_params(cmd, "missing required param: 'job_id'"),
    };

    if let Err(e) = core.scheduler_mut().cancel(job_id) {
        return ResponseEnvelope::error(
            Some(cmd.id.clone()),
            ProtocolError::new(ErrorCode::InvalidParams, format!("cancel failed: {e}")),
        );
    }

    if let Some(storage) = core.storage_mut() {
        if let Err(e) = storage.update_job_status(job_id, "cancelled", None, None) {
            return internal_error(cmd, &format!("failed to update job status: {e}"));
        }
    }

    core.emit_event(EventEnvelope::new(
        JOB_CANCELLED,
        serde_json::json!({
            "job_id": job_id,
        }),
    ));

    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({"job_id": job_id, "status": "cancelled"}),
    )
}

// ---------------------------------------------------------------------------
// Phase 2 — Blender handlers
// ---------------------------------------------------------------------------

fn handle_blender_get_capabilities(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
) -> ResponseEnvelope {
    // Probing is bounded (binary --version + worker --capabilities with a 30 s
    // timeout) and never hangs the request: a missing binary is a normal
    // capabilities response with static fallback operations.
    let capabilities = core.blender().probe_capabilities();
    match serde_json::to_value(&capabilities) {
        Ok(value) => ResponseEnvelope::success(&cmd.id, value),
        Err(e) => internal_error(cmd, &format!("failed to serialize capabilities: {e}")),
    }
}

fn handle_blender_run_job(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let project = match core.current_project() {
        Some(project) => project.clone(),
        None => return no_project_error(cmd),
    };

    let spec: BlenderJobSpec = match serde_json::from_value(cmd.params.clone()) {
        Ok(spec) => spec,
        Err(e) => return invalid_params(cmd, &format!("invalid spec: {e}")),
    };

    // Validate before creating any job record: invalid specs must not leave
    // orphan queued/stored jobs behind (TZ 5.8).
    if let Err(message) = validate_blender_spec(&spec) {
        return invalid_params(cmd, &message);
    }

    let job_id = Storage::generate_job_id();

    // Step 1: create the persisted job record.
    {
        let storage = match core.storage_mut() {
            Some(storage) => storage,
            None => return storage_not_open_error(cmd),
        };
        if let Err(e) = storage.create_job_with_id(&job_id, &project.id, &spec.job_type) {
            return internal_error(cmd, &format!("failed to create job: {e}"));
        }
    }

    // Step 2: enqueue in the scheduler (persist failure if this fails).
    if let Err(e) = core
        .scheduler_mut()
        .enqueue_with_id(&job_id, &project.id, &spec.job_type)
    {
        let message = format!("failed to enqueue job: {e}");
        return persistence_error_response(core, cmd, &job_id, &spec.job_type, &message);
    }

    // Step 3: emit the queued event.
    core.emit_event(EventEnvelope::new(
        JOB_QUEUED,
        serde_json::json!({
            "job_id": job_id,
            "job_type": spec.job_type,
        }),
    ));

    if !spec.wait {
        // `wait=false` is queue-only: the job stays queued and is never
        // started by this stdio process.
        return ResponseEnvelope::success(
            &cmd.id,
            serde_json::json!({
                "id": job_id,
                "job_type": spec.job_type,
                "status": "queued",
            }),
        );
    }

    // Step 4: start exactly *this* job (never the FIFO head, which may be an
    // earlier wait=false job — TZ 5.2).
    if let Err(e) = core.scheduler_mut().dequeue_job(&job_id) {
        let message = format!("failed to start job: {e}");
        return persistence_error_response(core, cmd, &job_id, &spec.job_type, &message);
    }

    core.emit_event(EventEnvelope::new(
        JOB_STARTED,
        serde_json::json!({
            "job_id": job_id,
            "job_type": spec.job_type,
        }),
    ));

    {
        let storage = match core.storage_mut() {
            Some(storage) => storage,
            None => return storage_not_open_error(cmd),
        };
        if let Err(e) = storage.update_job_status(&job_id, "running", None, None) {
            let message = format!("failed to update job status: {e}");
            return persistence_error_response(core, cmd, &job_id, &spec.job_type, &message);
        }
    }

    // Step 5: execute via the Blender adapter. Timeout resolution
    // (explicit > env > default) happens inside the adapter.
    let workspace_root = Path::new(&project.root_path).to_path_buf();
    let output_dir = workspace_root.join("jobs");

    let result = core.blender().run_job(
        &job_id,
        &spec,
        &workspace_root,
        &output_dir,
        spec.timeout_ms,
    );

    // Step 6: map the adapter outcome onto scheduler/storage/event state.
    match result {
        Ok(blender_result) => finalize_blender_result(core, cmd, &spec, &job_id, blender_result),
        Err(error) => finalize_adapter_error(core, cmd, &spec, &job_id, &format!("{error}")),
    }
}

// ---------------------------------------------------------------------------
// Blender job finalization helpers
// ---------------------------------------------------------------------------

/// Persist a failure and return an `InternalError` response.
///
/// Used when a scheduler/storage transition itself fails; the failure state is
/// persisted where legal and exactly one `job.failed` event is emitted for the
/// same job id when persistence succeeds.
fn persistence_error_response(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    job_id: &str,
    job_type: &str,
    message: &str,
) -> ResponseEnvelope {
    let transition_problems = persist_failure(core, job_id, job_type, message);
    if transition_problems.is_empty() {
        internal_error(cmd, message)
    } else {
        internal_error(
            cmd,
            &format!(
                "{message}; additionally failed to persist failure state: {}",
                transition_problems.join("; ")
            ),
        )
    }
}

/// Terminal-fail a job whose record exists but which never reached a normal
/// terminal transition (enqueue/start/storage failure).
///
/// Order is storage first, then scheduler (`running → failed`, or
/// `queued → failed` via `Scheduler::fail_queued`), then exactly one core
/// `job.failed` event — only if both transitions succeeded. Terminal identity is
/// therefore identical in scheduler, storage and events (`failed`, same id).
/// Returns transition errors (empty on success).
fn persist_failure(
    core: &mut CoreService,
    job_id: &str,
    job_type: &str,
    message: &str,
) -> Vec<String> {
    let mut problems = mark_storage_failed(core, job_id, message, None);
    if problems.is_empty() {
        match core.scheduler().get_job(job_id).map(|job| job.status) {
            Some(JobStatus::Running) => {
                if let Err(e) = core.scheduler_mut().fail(job_id, message) {
                    problems.push(format!("scheduler.fail: {e}"));
                }
            }
            Some(JobStatus::Queued) => {
                if let Err(e) = core.scheduler_mut().fail_queued(job_id, message) {
                    problems.push(format!("scheduler.fail_queued: {e}"));
                }
            }
            _ => {}
        }
    }

    if problems.is_empty() {
        core.emit_event(EventEnvelope::new(
            JOB_FAILED,
            serde_json::json!({
                "job_id": job_id,
                "job_type": job_type,
                "status": "failed",
                "error": message,
            }),
        ));
    }

    problems
}

fn finalize_blender_result(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    spec: &BlenderJobSpec,
    job_id: &str,
    blender_result: BlenderJobResult,
) -> ResponseEnvelope {
    let output_json = serde_json::to_string(&blender_result).ok();

    match blender_result.status.as_str() {
        "completed" => finalize_completed(core, cmd, spec, job_id, &blender_result, output_json),
        "cancelled" => finalize_cancelled(core, cmd, spec, job_id, output_json),
        // "failed" and any unexpected status are treated as job failures; the
        // adapter validates the status enum, so this is defensive.
        _ => {
            let error_message = blender_result
                .error
                .clone()
                .or_else(|| blender_result.message.clone())
                .unwrap_or_else(|| "Blender job failed".to_string());
            finalize_failed(core, cmd, spec, job_id, &error_message, output_json)
        }
    }
}

fn finalize_completed(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    spec: &BlenderJobSpec,
    job_id: &str,
    blender_result: &BlenderJobResult,
    output_json: Option<String>,
) -> ResponseEnvelope {
    let artifacts: Vec<michelangelo_protocol::ArtifactDto> = blender_result
        .artifacts
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|artifact| michelangelo_protocol::ArtifactDto {
            path: artifact.path,
            artifact_type: artifact.artifact_type,
            size_bytes: artifact.size_bytes,
        })
        .collect();

    // P2-1: persist storage first, then the scheduler transition; the terminal
    // protocol event is emitted only when both transitions succeeded.
    let storage_problems = persist_terminal_storage(
        core,
        job_id,
        "completed",
        Some(100),
        Some("job completed"),
        output_json.as_deref(),
        None,
    );
    let problems = if storage_problems.is_empty() {
        match core.scheduler_mut().complete(job_id, artifacts.clone()) {
            Ok(()) => Vec::new(),
            Err(e) => vec![format!("scheduler.complete: {e}")],
        }
    } else {
        storage_problems
    };

    if !problems.is_empty() {
        return terminal_transition_error(core, cmd, job_id, "completed", &problems);
    }

    core.emit_event(EventEnvelope::new(
        JOB_COMPLETED,
        serde_json::json!({
            "job_id": job_id,
            "job_type": spec.job_type,
            "status": "completed",
            "artifacts": artifacts,
        }),
    ));

    let final_job = core
        .storage()
        .and_then(|storage| storage.get_job(job_id).ok().flatten());
    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({
            "job": final_job,
            "status": "completed",
        }),
    )
}

fn finalize_failed(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    spec: &BlenderJobSpec,
    job_id: &str,
    error_message: &str,
    output_json: Option<String>,
) -> ResponseEnvelope {
    // P2-1: storage first, scheduler second, event only if both succeeded.
    let storage_problems = persist_terminal_storage(
        core,
        job_id,
        "failed",
        None,
        Some(error_message),
        output_json.as_deref(),
        Some(error_message),
    );
    let problems = if storage_problems.is_empty() {
        match core.scheduler_mut().fail(job_id, error_message) {
            Ok(()) => Vec::new(),
            Err(e) => vec![format!("scheduler.fail: {e}")],
        }
    } else {
        storage_problems
    };

    if !problems.is_empty() {
        return terminal_transition_error(core, cmd, job_id, "failed", &problems);
    }

    core.emit_event(EventEnvelope::new(
        JOB_FAILED,
        serde_json::json!({
            "job_id": job_id,
            "job_type": spec.job_type,
            "status": "failed",
            "error": error_message,
        }),
    ));

    let final_job = core
        .storage()
        .and_then(|storage| storage.get_job(job_id).ok().flatten());
    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({
            "job": final_job,
            "status": "failed",
            "error": error_message,
        }),
    )
}

fn finalize_cancelled(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    spec: &BlenderJobSpec,
    job_id: &str,
    output_json: Option<String>,
) -> ResponseEnvelope {
    // P2-1: storage first, scheduler second, event only if both succeeded.
    let storage_problems = persist_terminal_storage(
        core,
        job_id,
        "cancelled",
        None,
        Some("job cancelled"),
        output_json.as_deref(),
        None,
    );
    let problems = if storage_problems.is_empty() {
        match core.scheduler_mut().cancel(job_id) {
            Ok(()) => Vec::new(),
            Err(e) => vec![format!("scheduler.cancel: {e}")],
        }
    } else {
        storage_problems
    };

    if !problems.is_empty() {
        return terminal_transition_error(core, cmd, job_id, "cancelled", &problems);
    }

    core.emit_event(EventEnvelope::new(
        JOB_CANCELLED,
        serde_json::json!({
            "job_id": job_id,
            "job_type": spec.job_type,
        }),
    ));

    let final_job = core
        .storage()
        .and_then(|storage| storage.get_job(job_id).ok().flatten());
    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({
            "job": final_job,
            "status": "cancelled",
        }),
    )
}

fn finalize_adapter_error(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    spec: &BlenderJobSpec,
    job_id: &str,
    error_message: &str,
) -> ResponseEnvelope {
    // P2-1: storage first, scheduler second, event only if both succeeded.
    let storage_problems = persist_terminal_storage(
        core,
        job_id,
        "failed",
        None,
        Some(error_message),
        None,
        Some(error_message),
    );
    let problems = if storage_problems.is_empty() {
        match core.scheduler_mut().fail(job_id, error_message) {
            Ok(()) => Vec::new(),
            Err(e) => vec![format!("scheduler.fail: {e}")],
        }
    } else {
        storage_problems
    };

    if !problems.is_empty() {
        return terminal_transition_error(core, cmd, job_id, "failed", &problems);
    }

    core.emit_event(EventEnvelope::new(
        JOB_FAILED,
        serde_json::json!({
            "job_id": job_id,
            "job_type": spec.job_type,
            "status": "failed",
            "error": error_message,
        }),
    ));

    let final_job = core
        .storage()
        .and_then(|storage| storage.get_job(job_id).ok().flatten());
    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({
            "job": final_job,
            "status": "failed",
            "error": error_message,
        }),
    )
}

/// Persist a terminal status (and optional output JSON / error) in storage.
///
/// Returns transition errors; the caller only performs the scheduler transition
/// and emits the terminal event when this is empty.
fn persist_terminal_storage(
    core: &mut CoreService,
    job_id: &str,
    status: &str,
    progress_pct: Option<u8>,
    message: Option<&str>,
    output_json: Option<&str>,
    error: Option<&str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    match core.storage_mut() {
        Some(storage) => {
            if let Err(e) = storage.update_job_status(job_id, status, progress_pct, message) {
                problems.push(format!("storage.status: {e}"));
            }
            if let Err(e) = storage.update_job_output(job_id, output_json, error, None) {
                problems.push(format!("storage.output: {e}"));
            }
        }
        None => problems.push("storage not initialized".to_string()),
    }
    problems
}

/// Handle a failed terminal transition.
///
/// Recovers to `failed` where legal (scheduler running/queued job, storage),
/// emits **no** terminal protocol event that could contradict the persisted
/// state, and returns `InternalError` naming the inconsistency.
fn terminal_transition_error(
    core: &mut CoreService,
    cmd: &CommandEnvelope,
    job_id: &str,
    intended_status: &str,
    problems: &[String],
) -> ResponseEnvelope {
    let message = format!(
        "could not persist terminal state '{intended_status}' for job {job_id}: {}",
        problems.join("; ")
    );

    let mut recovery = Vec::new();
    match core.scheduler().get_job(job_id).map(|job| job.status) {
        Some(JobStatus::Running) => {
            if let Err(e) = core.scheduler_mut().fail(job_id, &message) {
                recovery.push(format!("scheduler.fail: {e}"));
            }
        }
        Some(JobStatus::Queued) => {
            if let Err(e) = core.scheduler_mut().fail_queued(job_id, &message) {
                recovery.push(format!("scheduler.fail_queued: {e}"));
            }
        }
        _ => {}
    }
    recovery.extend(mark_storage_failed(core, job_id, &message, None));

    if recovery.is_empty() {
        internal_error(cmd, &message)
    } else {
        internal_error(
            cmd,
            &format!("{message}; recovery errors: {}", recovery.join("; ")),
        )
    }
}

/// Persist `failed` status/error (and optional output JSON) in storage.
///
/// Returns transition errors; the caller decides whether they are fatal.
fn mark_storage_failed(
    core: &mut CoreService,
    job_id: &str,
    error_message: &str,
    output_json: Option<&str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    match core.storage_mut() {
        Some(storage) => {
            if let Err(e) = storage.update_job_status(job_id, "failed", None, Some(error_message)) {
                problems.push(format!("storage.status: {e}"));
            }
            if let Err(e) =
                storage.update_job_output(job_id, output_json, Some(error_message), None)
            {
                problems.push(format!("storage.output: {e}"));
            }
        }
        None => problems.push("storage not initialized".to_string()),
    }
    problems
}

// ---------------------------------------------------------------------------
// Blender spec validation
// ---------------------------------------------------------------------------

const MAX_BLENDER_OPERATIONS: usize = 256;
const MAX_JOB_TYPE_CHARS: usize = 128;
const MAX_SPEC_PATH_CHARS: usize = 1024;
const MAX_VIEWS: usize = 64;

/// Basic `BlenderJobSpec` validation performed before a job record is created.
///
/// Rejects unknown operations, unsafe paths and non-finite numeric parameters
/// with a `-32602` (`InvalidParams`) response and no persisted job.
fn validate_blender_spec(spec: &BlenderJobSpec) -> Result<(), String> {
    if spec.job_type.trim().is_empty() {
        return Err("spec.job_type must not be empty".to_string());
    }
    if spec.job_type.chars().count() > MAX_JOB_TYPE_CHARS {
        return Err(format!(
            "spec.job_type is too long (max {MAX_JOB_TYPE_CHARS} chars)"
        ));
    }
    if spec.job_type.chars().any(char::is_control) {
        return Err("spec.job_type must not contain control characters".to_string());
    }
    if spec.spec.operations.len() > MAX_BLENDER_OPERATIONS {
        return Err(format!(
            "too many operations: {} (max {MAX_BLENDER_OPERATIONS})",
            spec.spec.operations.len()
        ));
    }

    for (index, operation) in spec.spec.operations.iter().enumerate() {
        let label = format!("operation[{index}] ('{}')", operation.op);
        if operation.op.trim().is_empty() {
            return Err(format!(
                "operation[{index}]: operation name must not be empty"
            ));
        }
        if !SAFE_WORKER_OPS.contains(&operation.op.as_str()) {
            return Err(format!("{label}: unsupported operation"));
        }
        validate_operation_path(operation, &label)?;
        validate_operation_numbers(operation, &label)?;
    }

    Ok(())
}

fn validate_operation_path(operation: &BlenderOperation, label: &str) -> Result<(), String> {
    match operation.op.as_str() {
        "save_blend" | "export_glb" => match operation.path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                validate_workspace_relative_path(path).map_err(|e| format!("{label}: {e}"))
            }
            _ => Err(format!("{label}: missing or empty 'path'")),
        },
        "render_views" => match operation.path.as_deref() {
            Some(path) => {
                validate_workspace_relative_path(path).map_err(|e| format!("{label}: {e}"))
            }
            None => Ok(()),
        },
        _ => match operation.path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                validate_workspace_relative_path(path).map_err(|e| format!("{label}: {e}"))
            }
            _ => Ok(()),
        },
    }
}

fn validate_operation_numbers(operation: &BlenderOperation, label: &str) -> Result<(), String> {
    if let Some(size) = &operation.size {
        validate_numeric_vector(size, 3, "size", label)?;
    }
    if let Some(location) = &operation.location {
        validate_numeric_vector(location, 3, "location", label)?;
        if location.iter().any(|value| !value.is_finite()) {
            return Err(format!("{label}: 'location' contains non-finite values"));
        }
    }
    if let Some(color) = &operation.color {
        if color.len() != 3 && color.len() != 4 {
            return Err(format!("{label}: 'color' must have 3 or 4 components"));
        }
        if color.iter().any(|value| !value.is_finite()) {
            return Err(format!("{label}: 'color' contains non-finite values"));
        }
    }
    if let Some(views) = &operation.views {
        if views.len() > MAX_VIEWS {
            return Err(format!("{label}: too many 'views' (max {MAX_VIEWS})"));
        }
    }
    Ok(())
}

fn validate_numeric_vector(
    values: &[f64],
    expected_len: usize,
    field: &str,
    label: &str,
) -> Result<(), String> {
    if values.len() != expected_len {
        return Err(format!(
            "{label}: '{field}' must have exactly {expected_len} components"
        ));
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(format!("{label}: '{field}' contains non-finite values"));
    }
    Ok(())
}

fn validate_workspace_relative_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("path must not be empty".to_string());
    }
    if path.chars().count() > MAX_SPEC_PATH_CHARS {
        return Err(format!(
            "path is too long (max {MAX_SPEC_PATH_CHARS} chars)"
        ));
    }
    if path.contains('\0') {
        return Err("path must not contain NUL".to_string());
    }

    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(format!("path must be workspace-relative: '{path}'"));
    }
    for component in parsed.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(format!(
                    "path must not contain parent or root components: '{path}'"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Job DTO conversion
// ---------------------------------------------------------------------------

fn storage_job_to_dto(job: serde_json::Value) -> Option<JobDto> {
    let id = job.get("id")?.as_str()?.to_string();
    let job_type = job
        .get("job_type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status_str = job
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("queued");
    let status = match status_str {
        "running" => JobStatus::Running,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "cancelled" => JobStatus::Cancelled,
        _ => JobStatus::Queued,
    };
    let progress_pct = job
        .get("progress_pct")
        .and_then(|v| v.as_i64())
        .map(|v| v as u8);
    let message = job
        .get("message")
        .and_then(|v| v.as_str())
        .map(String::from);
    let created_at = job
        .get("created_at")
        .and_then(|v| v.as_str())
        .map(String::from);
    let started_at = job
        .get("started_at")
        .and_then(|v| v.as_str())
        .map(String::from);
    let finished_at = job
        .get("finished_at")
        .and_then(|v| v.as_str())
        .map(String::from);
    let error = job.get("error").and_then(|v| v.as_str()).map(String::from);

    let artifacts = job
        .get("output_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("artifacts").cloned())
        .and_then(|a| serde_json::from_value(a).ok())
        .unwrap_or_default();

    Some(JobDto {
        id,
        name: job_type.clone(),
        job_type,
        status,
        progress_pct,
        message,
        created_at,
        started_at,
        completed_at: finished_at,
        error,
        artifacts,
    })
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn no_project_error(cmd: &CommandEnvelope) -> ResponseEnvelope {
    ResponseEnvelope::error(
        Some(cmd.id.clone()),
        ProtocolError::new(ErrorCode::WorkspaceNotOpen, "no project is currently open"),
    )
}

fn storage_not_open_error(cmd: &CommandEnvelope) -> ResponseEnvelope {
    ResponseEnvelope::error(
        Some(cmd.id.clone()),
        ProtocolError::new(ErrorCode::InternalError, "storage not initialized"),
    )
}

fn invalid_params(cmd: &CommandEnvelope, msg: &str) -> ResponseEnvelope {
    ResponseEnvelope::error(
        Some(cmd.id.clone()),
        ProtocolError::new(ErrorCode::InvalidParams, msg),
    )
}

fn internal_error(cmd: &CommandEnvelope, msg: &str) -> ResponseEnvelope {
    ResponseEnvelope::error(
        Some(cmd.id.clone()),
        ProtocolError::new(ErrorCode::InternalError, msg),
    )
}

fn map_workspace_error(err: &WorkspaceError) -> (ErrorCode, String) {
    match err {
        WorkspaceError::NotAWorkspace(msg) => (ErrorCode::WorkspaceNotOpen, msg.clone()),
        WorkspaceError::Io(e) => (ErrorCode::InternalError, format!("filesystem error: {e}")),
        WorkspaceError::Json(e) => (ErrorCode::InternalError, format!("metadata error: {e}")),
        WorkspaceError::AlreadyExists(msg) => (ErrorCode::AlreadyExists, msg.clone()),
        WorkspaceError::Corrupted(msg) => (ErrorCode::InternalError, msg.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::CoreService;
    use michelangelo_blender::HeadlessBlenderAdapter;
    use serde_json::json;
    use std::sync::mpsc::Receiver;
    use tempfile::TempDir;

    fn handler_for(router: &Router, method: &str) -> Option<Handler> {
        router.find(method)
    }

    fn core() -> CoreService {
        let (svc, _rx) = CoreService::new();
        svc
    }

    /// Install a deterministic fake Blender adapter through the test seam
    /// (no process-wide `BLENDER_BIN` mutation). The fake script's TempDir is
    /// returned because it must stay alive while jobs run.
    fn install_fake_adapter(core: &mut CoreService, mode: &str) -> TempDir {
        let fake_dir = TempDir::new().unwrap();
        let fake = michelangelo_blender::create_fake_blender(fake_dir.path(), mode);
        let worker = HeadlessBlenderAdapter::new().worker_script().to_path_buf();
        core.set_blender_adapter(HeadlessBlenderAdapter::with_config(
            fake.to_string_lossy().to_string(),
            worker,
        ));
        fake_dir
    }

    /// Build a CoreService whose Blender adapter points at a deterministic
    /// fake executable via the test seam.
    fn fake_core(mode: &str) -> (CoreService, Receiver<EventEnvelope>, TempDir) {
        let (mut core, rx) = CoreService::new();
        let fake_dir = install_fake_adapter(&mut core, mode);
        (core, rx, fake_dir)
    }

    fn create_project(core: &mut CoreService, dir: &TempDir, name: &str) {
        let create = core.handle_command(&CommandEnvelope {
            id: "c1".into(),
            method: method::PROJECT_CREATE.into(),
            params: json!({"path": dir.path(), "name": name}),
        });
        assert!(
            create.error.is_none(),
            "project.create failed: {:?}",
            create.error
        );
    }

    fn run_blender_job(core: &mut CoreService, params: serde_json::Value) -> ResponseEnvelope {
        core.handle_command(&CommandEnvelope {
            id: "j1".into(),
            method: method::BLENDER_RUN_JOB.into(),
            params,
        })
    }

    #[test]
    fn test_router_find_ping() {
        let router = default_router();
        assert!(handler_for(&router, method::SYSTEM_PING).is_some());
    }

    #[test]
    fn test_router_find_unknown() {
        let router = default_router();
        assert!(handler_for(&router, "foo.bar").is_none());
    }

    #[test]
    fn test_router_find_all_known() {
        let router = default_router();
        assert!(handler_for(&router, method::SYSTEM_PING).is_some());
        assert!(handler_for(&router, method::PROJECT_CREATE).is_some());
        assert!(handler_for(&router, method::PROJECT_OPEN).is_some());
        assert!(handler_for(&router, method::PROJECT_GET_SNAPSHOT).is_some());
        assert!(handler_for(&router, method::JOB_LIST).is_some());
        assert!(handler_for(&router, method::JOB_GET).is_some());
        assert!(handler_for(&router, method::JOB_CANCEL).is_some());
        assert!(handler_for(&router, method::BLENDER_GET_CAPABILITIES).is_some());
        assert!(handler_for(&router, method::BLENDER_RUN_JOB).is_some());
    }

    #[test]
    fn test_router_empty_returns_none() {
        let router = Router::new();
        assert!(router.find(method::SYSTEM_PING).is_none());
    }

    #[test]
    fn test_router_register_and_find() {
        let mut router = Router::new();
        router.register(method::SYSTEM_PING, handle_ping);
        assert!(router.find(method::SYSTEM_PING).is_some());
        assert!(router.find(method::PROJECT_CREATE).is_none());
    }

    #[test]
    fn test_handler_ping_via_core_service() {
        let mut core = core();
        let cmd = CommandEnvelope {
            id: "ping".into(),
            method: method::SYSTEM_PING.into(),
            params: json!({}),
        };
        let resp = core.handle_command(&cmd);
        assert_eq!(resp.result.unwrap()["pong"], true);
    }

    #[test]
    fn test_handler_create_missing_path_via_core() {
        let mut core = core();
        let cmd = CommandEnvelope {
            id: "c1".into(),
            method: method::PROJECT_CREATE.into(),
            params: json!({}),
        };
        let resp = core.handle_command(&cmd);
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn test_handler_create_and_snapshot_via_core() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut core = core();

        let create = core.handle_command(&CommandEnvelope {
            id: "c1".into(),
            method: method::PROJECT_CREATE.into(),
            params: json!({"path": dir.path(), "name": "router-e2e"}),
        });
        assert!(create.error.is_none());

        let snap = core.handle_command(&CommandEnvelope {
            id: "s1".into(),
            method: method::PROJECT_GET_SNAPSHOT.into(),
            params: json!({}),
        });
        assert!(snap.error.is_none());
        assert_eq!(snap.result.unwrap()["project"]["name"], "router-e2e");
    }

    #[test]
    fn test_handler_get_snapshot_no_project() {
        let mut core = core();
        let cmd = CommandEnvelope {
            id: "e1".into(),
            method: method::PROJECT_GET_SNAPSHOT.into(),
            params: json!({}),
        };
        let resp = core.handle_command(&cmd);
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::WorkspaceNotOpen);
    }

    #[test]
    fn test_handler_job_list_no_project() {
        let mut core = core();
        let resp = core.handle_command(&CommandEnvelope {
            id: "j1".into(),
            method: method::JOB_LIST.into(),
            params: json!({}),
        });
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ErrorCode::WorkspaceNotOpen);
    }

    #[test]
    fn test_handler_job_get_missing_id() {
        let mut core = core();
        let resp = core.handle_command(&CommandEnvelope {
            id: "j1".into(),
            method: method::JOB_GET.into(),
            params: json!({}),
        });
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_handler_blender_capabilities() {
        // Deterministic adapter via the test seam (no process-wide BLENDER_BIN).
        let (mut core, _rx, _fake_dir) = fake_core("success");
        let resp = core.handle_command(&CommandEnvelope {
            id: "b1".into(),
            method: method::BLENDER_GET_CAPABILITIES.into(),
            params: json!({}),
        });
        assert!(
            resp.error.is_none(),
            "capabilities failed: {:?}",
            resp.error
        );

        let caps = resp.result.unwrap();
        assert_eq!(caps["binary_found"], json!(true));
        assert_eq!(caps["bpy_available"], json!(true));
        assert_eq!(caps["blender_version"], json!("Blender fake 0.0.0"));
        assert_eq!(caps["python_version"], json!("3.13.0"));
        assert_eq!(caps["gltf_export_available"], json!(true));
        assert!(
            caps["render_engines"]
                .as_array()
                .unwrap()
                .contains(&json!("BLENDER_WORKBENCH")),
            "render_engines: {}",
            caps["render_engines"]
        );
        let ops = caps["supported_operations"]
            .as_array()
            .expect("supported_operations must be an array");
        assert!(ops.iter().any(|op| op == "create_box"));
        assert!(caps.get("error").is_none());
    }

    #[test]
    fn test_handler_blender_run_job_no_project() {
        let mut core = core();
        let resp = core.handle_command(&CommandEnvelope {
            id: "b1".into(),
            method: method::BLENDER_RUN_JOB.into(),
            params: json!({"job_type": "test", "spec": {"operations": []}}),
        });
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ErrorCode::WorkspaceNotOpen);
    }

    #[test]
    fn test_handler_blender_run_job_wait_false_queue_only() {
        use std::sync::mpsc::Receiver;

        let dir = tempfile::TempDir::new().unwrap();
        let (mut core, rx): (CoreService, Receiver<EventEnvelope>) = CoreService::new();

        let create = core.handle_command(&CommandEnvelope {
            id: "c1".into(),
            method: method::PROJECT_CREATE.into(),
            params: json!({"path": dir.path(), "name": "wait-false-test"}),
        });
        assert!(
            create.error.is_none(),
            "project.create failed: {:?}",
            create.error
        );

        let resp = core.handle_command(&CommandEnvelope {
            id: "j1".into(),
            method: method::BLENDER_RUN_JOB.into(),
            params: json!({
                "job_type": "test_wait_false",
                "wait": false,
                "spec": {
                    "operations": [{"op": "create_box", "name": "box", "size": [2.0, 1.0, 0.5]}]
                }
            }),
        });

        assert!(
            resp.error.is_none(),
            "blender.run_job wait=false failed: {:?}",
            resp.error
        );
        let result = resp.result.unwrap();
        assert_eq!(
            result["status"], "queued",
            "expected status=queued, got {:?}",
            result
        );
        let job_id = result["id"].as_str().unwrap().to_string();
        assert!(!job_id.is_empty());

        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        let event_names: Vec<&str> = events.iter().map(|e| e.event.as_str()).collect();
        assert!(
            event_names.contains(&"job.queued"),
            "expected job.queued event, got: {:?}",
            event_names
        );
        assert!(
            !event_names.contains(&"job.started"),
            "wait=false must not emit job.started"
        );
        assert!(
            !event_names.contains(&"job.completed"),
            "wait=false must not emit job.completed"
        );

        let get_resp = core.handle_command(&CommandEnvelope {
            id: "g1".into(),
            method: method::JOB_GET.into(),
            params: json!({"job_id": &job_id}),
        });
        assert!(
            get_resp.error.is_none(),
            "job.get failed: {:?}",
            get_resp.error
        );
        assert_eq!(
            get_resp.result.unwrap()["status"],
            "queued",
            "persisted job must be queued"
        );

        let list_resp = core.handle_command(&CommandEnvelope {
            id: "l1".into(),
            method: method::JOB_LIST.into(),
            params: json!({}),
        });
        let list_result = list_resp.result.unwrap();
        let jobs = list_result["jobs"].as_array().unwrap();
        assert!(
            jobs.iter().any(|j| j["id"] == job_id),
            "job.list must contain queued job"
        );
    }

    // -- IT-1: job identity, status mapping, spec validation -----------------

    #[test]
    fn test_handler_wait_false_job_is_not_hijacked_by_later_wait_true() {
        let (mut core, rx, _fake_dir) = fake_core("success");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "wait-true-identity");

        // Job A: queue-only.
        let resp_a = run_blender_job(
            &mut core,
            json!({
                "job_type": "wait_false_type",
                "wait": false,
                "spec": {"operations": [{"op": "create_box", "name": "a", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        assert!(resp_a.error.is_none(), "A failed: {:?}", resp_a.error);
        let result_a = resp_a.result.unwrap();
        assert_eq!(result_a["status"], "queued");
        let job_a = result_a["id"].as_str().unwrap().to_string();

        // Job B: wait=true must run B itself, not the earlier queued A.
        let resp_b = run_blender_job(
            &mut core,
            json!({
                "job_type": "wait_true_type",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "b", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        assert!(resp_b.error.is_none(), "B failed: {:?}", resp_b.error);
        let result_b = resp_b.result.unwrap();
        let job_b = result_b["job"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert_eq!(result_b["status"], "completed", "B result: {result_b}");
        assert_ne!(job_a, job_b, "B must get its own job id");
        assert_eq!(result_b["job"]["status"], "completed");

        // Artifacts survive in the persisted output_json.
        let output_json = result_b["job"]["output_json"]
            .as_str()
            .expect("completed job must persist output_json");
        let persisted: serde_json::Value =
            serde_json::from_str(output_json).expect("output_json must be valid JSON");
        assert!(
            !persisted["artifacts"]
                .as_array()
                .unwrap_or(&Vec::new())
                .is_empty(),
            "completed job must persist artifacts: {persisted}"
        );
        let artifact_path = persisted["artifacts"][0]["path"]
            .as_str()
            .expect("artifact path");
        assert!(
            project_dir.path().join(artifact_path).is_file(),
            "persisted artifact must exist on disk: {artifact_path}"
        );

        // A stays queued in storage and scheduler; B is completed; slot free.
        let get_a = core.handle_command(&CommandEnvelope {
            id: "g1".into(),
            method: method::JOB_GET.into(),
            params: json!({"job_id": &job_a}),
        });
        assert_eq!(get_a.result.unwrap()["status"], "queued");
        let get_b = core.handle_command(&CommandEnvelope {
            id: "g2".into(),
            method: method::JOB_GET.into(),
            params: json!({"job_id": &job_b}),
        });
        assert_eq!(get_b.result.unwrap()["status"], "completed");

        assert!(core.scheduler().can_run());
        assert_eq!(core.scheduler().queued_count(), 1);
        assert_eq!(
            core.scheduler().get_job(&job_a).unwrap().status,
            JobStatus::Queued
        );
        assert_eq!(
            core.scheduler().get_job(&job_b).unwrap().status,
            JobStatus::Completed
        );

        // Events: started/completed carry B's id; A only ever queued.
        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        let ids_for = |name: &str| -> Vec<String> {
            events
                .iter()
                .filter(|event| event.event == name)
                .map(|event| event.data["job_id"].as_str().unwrap_or("").to_string())
                .collect()
        };
        assert_eq!(ids_for("job.started"), vec![job_b.clone()]);
        assert_eq!(ids_for("job.completed"), vec![job_b.clone()]);
        let queued_ids = ids_for("job.queued");
        assert!(queued_ids.contains(&job_a));
        assert!(queued_ids.contains(&job_b));
        assert!(events
            .iter()
            .all(|event| event.event != "job.failed" && event.event != "job.cancelled"));
    }

    #[test]
    fn test_handler_blender_failed_status_maps_to_failed_job() {
        let (mut core, rx, _fake_dir) = fake_core("failed_result");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "failed-status");

        let resp = run_blender_job(
            &mut core,
            json!({
                "job_type": "failing_job",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        assert!(resp.error.is_none(), "response: {:?}", resp.error);
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["error"], "Fake worker failure");
        assert_eq!(result["job"]["status"], "failed");
        assert_eq!(result["job"]["error"], "Fake worker failure");

        assert!(core.scheduler().can_run());
        assert_eq!(core.scheduler().queued_count(), 0);

        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        let failed: Vec<&EventEnvelope> = events
            .iter()
            .filter(|event| event.event == "job.failed")
            .collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].data["status"], "failed");
        assert_eq!(failed[0].data["job_id"], result["job"]["id"]);
        assert!(events.iter().all(|event| event.event != "job.completed"));
    }

    #[test]
    fn test_handler_blender_cancelled_status_maps_to_cancelled_job() {
        let (mut core, rx, _fake_dir) = fake_core("cancelled_result");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "cancelled-status");

        let resp = run_blender_job(
            &mut core,
            json!({
                "job_type": "cancelled_job",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        assert!(resp.error.is_none(), "response: {:?}", resp.error);
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["job"]["status"], "cancelled");

        assert!(core.scheduler().can_run());
        assert_eq!(core.scheduler().queued_count(), 0);

        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        let cancelled: Vec<&EventEnvelope> = events
            .iter()
            .filter(|event| event.event == "job.cancelled")
            .collect();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].data["job_id"], result["job"]["id"]);
        assert!(events.iter().all(|event| event.event != "job.failed"));
    }

    #[test]
    fn test_handler_blender_invalid_spec_creates_no_job_record() {
        let (mut core, rx) = CoreService::new();
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "invalid-spec");

        let invalid_specs = vec![
            json!({"job_type": "x", "wait": true, "spec": {"operations": [{"op": "explode"}]}}),
            json!({"job_type": "", "wait": true, "spec": {"operations": []}}),
            json!({"job_type": "x", "wait": true, "spec": {"operations": [{"op": "save_blend", "path": "../evil.blend"}]}}),
            json!({"job_type": "x", "wait": true, "spec": {"operations": [{"op": "save_blend"}]}}),
            json!({"job_type": "x", "wait": true, "spec": {"operations": [{"op": "create_box", "size": [1.0, 2.0]}]}}),
        ];

        for params in invalid_specs {
            let resp = run_blender_job(&mut core, params);
            let error = resp.error.expect("invalid spec must be rejected");
            assert_eq!(error.code, ErrorCode::InvalidParams);
        }

        // No orphan job records/events must be produced by rejected specs.
        assert_eq!(core.storage().unwrap().job_count().unwrap(), 0);
        assert_eq!(core.scheduler().queued_count(), 0);
        assert!(
            rx.try_iter().next().is_none(),
            "rejected specs must not emit job events"
        );
    }

    #[test]
    fn test_validate_blender_spec_numeric_and_limits() {
        let mut spec: BlenderJobSpec = serde_json::from_value(json!({
            "job_type": "spec",
            "wait": true,
            "timeout_ms": 1000,
            "spec": {
                "operations": [{
                    "op": "create_box",
                    "name": "box",
                    "size": [1.0, 2.0, 3.0],
                    "location": [0.0, 0.0, 0.0]
                }]
            }
        }))
        .unwrap();
        assert!(validate_blender_spec(&spec).is_ok());

        // Non-finite numeric parameters are rejected.
        spec.spec.operations[0].size = Some(vec![1.0, f64::NAN, 3.0]);
        assert!(validate_blender_spec(&spec)
            .unwrap_err()
            .contains("non-finite"));
        spec.spec.operations[0].size = Some(vec![1.0, 2.0]);
        assert!(validate_blender_spec(&spec)
            .unwrap_err()
            .contains("3 components"));
        spec.spec.operations[0].size = Some(vec![1.0, 2.0, 3.0]);

        // Too many operations.
        let operation = spec.spec.operations[0].clone();
        spec.spec.operations = vec![operation; MAX_BLENDER_OPERATIONS + 1];
        assert!(validate_blender_spec(&spec)
            .unwrap_err()
            .contains("too many operations"));

        // Long job type.
        spec.spec.operations.clear();
        spec.job_type = "x".repeat(MAX_JOB_TYPE_CHARS + 1);
        assert!(validate_blender_spec(&spec)
            .unwrap_err()
            .contains("too long"));

        // Absolute and parent-traversal paths (platform-aware absolute path).
        let absolute = std::env::current_dir().unwrap().join("evil.blend");
        let mut path_spec: BlenderJobSpec = serde_json::from_value(json!({
            "job_type": "spec",
            "spec": {"operations": [{"op": "save_blend", "path": absolute.to_string_lossy()}]}
        }))
        .unwrap();
        assert!(validate_blender_spec(&path_spec)
            .unwrap_err()
            .contains("workspace-relative"));
        path_spec.spec.operations[0].path = Some("../evil.blend".into());
        assert!(validate_blender_spec(&path_spec)
            .unwrap_err()
            .contains("parent or root"));
    }

    #[test]
    fn test_handler_blender_capabilities_missing_binary_does_not_hang() {
        let worker = HeadlessBlenderAdapter::new().worker_script().to_path_buf();
        let (mut core, _rx) = CoreService::new();
        core.set_blender_adapter(HeadlessBlenderAdapter::with_config(
            "nonexistent_blender_it1_xyzzy",
            worker,
        ));

        let started = std::time::Instant::now();
        let resp = core.handle_command(&CommandEnvelope {
            id: "caps-missing".into(),
            method: method::BLENDER_GET_CAPABILITIES.into(),
            params: json!({}),
        });
        assert!(
            started.elapsed() < std::time::Duration::from_secs(40),
            "capabilities request must be bounded"
        );
        assert!(resp.error.is_none(), "response: {:?}", resp.error);

        let caps = resp.result.unwrap();
        assert_eq!(caps["binary_found"], json!(false));
        assert_eq!(caps["bpy_available"], json!(false));
        assert!(caps["blender_version"].is_null());
        assert!(caps["error"].is_string());
        assert_eq!(
            caps["supported_operations"].as_array().unwrap().len(),
            michelangelo_protocol::SAFE_WORKER_OPS.len()
        );
    }

    #[test]
    fn test_handler_blender_capabilities_version_hang_is_bounded() {
        let fake_dir = TempDir::new().unwrap();
        let fake = michelangelo_blender::create_fake_blender(fake_dir.path(), "version_hang");
        let worker = HeadlessBlenderAdapter::new().worker_script().to_path_buf();
        let (mut core, _rx) = CoreService::new();
        core.set_blender_adapter(
            HeadlessBlenderAdapter::with_config(fake.to_string_lossy().to_string(), worker)
                .with_version_timeout(200),
        );

        let started = std::time::Instant::now();
        let resp = core.handle_command(&CommandEnvelope {
            id: "caps-version-hang".into(),
            method: method::BLENDER_GET_CAPABILITIES.into(),
            params: json!({}),
        });
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "version-hang capabilities request must be bounded, took {elapsed:?}"
        );
        assert!(resp.error.is_none(), "response: {:?}", resp.error);

        let caps = resp.result.unwrap();
        // The binary exists (it just hung); the probe reports degraded caps.
        assert_eq!(caps["binary_found"], json!(true));
        assert_eq!(caps["bpy_available"], json!(false));
        assert!(caps["error"].is_string());
        assert_eq!(
            caps["supported_operations"].as_array().unwrap().len(),
            michelangelo_protocol::SAFE_WORKER_OPS.len()
        );
    }

    // -- IT-1 rework: internal failure identity + adapter-error mapping -------

    #[test]
    fn test_handler_queued_job_failure_keeps_terminal_identity() {
        let (mut core, rx, _fake_dir) = fake_core("success");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "queued-failure-identity");
        let project_id = core.current_project().unwrap().id.clone();

        // Occupy the single running slot with a foreign job type.
        core.scheduler_mut()
            .enqueue_with_id("blocker-job", &project_id, "blocker_type")
            .unwrap();
        core.scheduler_mut().dequeue_job("blocker-job").unwrap();

        // wait=true job of a different type: enqueue succeeds, dequeue fails
        // because the slot is busy -> internal queued-failure path.
        let resp = run_blender_job(
            &mut core,
            json!({
                "job_type": "victim_type",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        let error = resp
            .error
            .expect("busy slot must yield an InternalError response");
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(
            error.message.contains("another job is already running"),
            "unexpected error: {}",
            error.message
        );

        // Storage and scheduler terminal status must both be `failed` for the
        // same job id (never scheduler=cancelled vs storage=failed).
        let failed_job = {
            let jobs = core.storage().unwrap().list_jobs(&project_id).unwrap();
            jobs.into_iter()
                .find(|job| job["status"] == json!("failed"))
                .expect("storage must persist the failed job")
        };
        let failed_id = failed_job["id"]
            .as_str()
            .expect("failed job id")
            .to_string();
        assert!(
            failed_job["error"]
                .as_str()
                .unwrap_or_default()
                .contains("another job is already running"),
            "storage error: {}",
            failed_job["error"]
        );
        let scheduler_job = core
            .scheduler()
            .get_job(&failed_id)
            .expect("scheduler must track the failed job");
        assert_eq!(
            scheduler_job.status,
            JobStatus::Failed,
            "queued job must terminal-fail, not cancel"
        );
        assert_eq!(core.scheduler().queued_count(), 0);

        // Exactly one terminal event, for the failed job id, status failed.
        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        let terminal: Vec<&EventEnvelope> = events
            .iter()
            .filter(|event| {
                matches!(
                    event.event.as_str(),
                    "job.failed" | "job.completed" | "job.cancelled"
                )
            })
            .collect();
        assert_eq!(terminal.len(), 1, "terminal events: {terminal:?}");
        assert_eq!(terminal[0].event, "job.failed");
        assert_eq!(terminal[0].data["job_id"], failed_id);
        assert_eq!(terminal[0].data["status"], "failed");
    }

    #[test]
    fn test_handler_adapter_timeout_maps_to_failed_job() {
        let (mut core, rx, _fake_dir) = fake_core("timeout");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "adapter-timeout");

        let resp = run_blender_job(
            &mut core,
            json!({
                "job_type": "timeout_job",
                "wait": true,
                "timeout_ms": 500,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        let result = resp
            .result
            .expect("adapter errors map to a job-failure response, not a protocol error");
        assert_eq!(result["status"], "failed");
        let message = result["error"].as_str().expect("error text");
        assert!(
            message.contains("timed out after 500 ms"),
            "unexpected error: {message}"
        );
        assert_eq!(result["job"]["status"], "failed");

        let job_id = result["job"]["id"].as_str().unwrap().to_string();
        assert!(core.scheduler().can_run(), "scheduler slot must be free");
        assert_eq!(core.scheduler().queued_count(), 0);
        assert_eq!(
            core.scheduler().get_job(&job_id).unwrap().status,
            JobStatus::Failed
        );

        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        let failed: Vec<&EventEnvelope> = events
            .iter()
            .filter(|event| event.event == "job.failed")
            .collect();
        assert_eq!(failed.len(), 1, "events: {events:?}");
        assert_eq!(failed[0].data["job_id"], job_id);
        assert!(events
            .iter()
            .all(|event| { !matches!(event.event.as_str(), "job.completed" | "job.cancelled") }));
    }

    #[test]
    fn test_handler_adapter_wrong_job_id_maps_to_failed_job() {
        let (mut core, rx, _fake_dir) = fake_core("wrong_job_id");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "adapter-wrong-id");

        let resp = run_blender_job(
            &mut core,
            json!({
                "job_type": "wrong_id_job",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        );
        let result = resp
            .result
            .expect("adapter errors map to a job-failure response");
        assert_eq!(result["status"], "failed");
        let message = result["error"].as_str().expect("error text");
        assert!(
            message.contains("job_id mismatch"),
            "unexpected error: {message}"
        );
        assert_eq!(result["job"]["status"], "failed");

        let job_id = result["job"]["id"].as_str().unwrap().to_string();
        assert!(core.scheduler().can_run());
        assert_eq!(
            core.scheduler().get_job(&job_id).unwrap().status,
            JobStatus::Failed
        );
        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event == "job.failed")
                .count(),
            1,
            "events: {events:?}"
        );
    }

    #[test]
    fn test_validate_workspace_relative_path_allows_dotdot_in_file_name() {
        assert!(validate_workspace_relative_path("blender/scenes/model.v1..2.blend").is_ok());
        assert!(validate_workspace_relative_path("../evil.blend").is_err());

        let spec: BlenderJobSpec = serde_json::from_value(json!({
            "job_type": "spec",
            "spec": {"operations": [
                {"op": "save_blend", "path": "blender/scenes/model.v1..2.blend"}
            ]}
        }))
        .unwrap();
        assert!(validate_blender_spec(&spec).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_workspace_relative_path_rejects_windows_forms() {
        // Drive-relative (`C:foo.blend`) and UNC (`\\server\share\...`).
        for path in [r"C:foo.blend", r"\\server\share\evil.blend"] {
            assert!(
                validate_workspace_relative_path(path).is_err(),
                "path {path:?} must be rejected"
            );
        }
    }

    #[test]
    fn test_terminal_transition_failure_emits_no_contradicting_event() {
        let (mut core, rx, _fake_dir) = fake_core("success");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "terminal-transition-failure");
        let project_id = core.current_project().unwrap().id.clone();

        // A foreign job occupies the single running slot.
        core.scheduler_mut()
            .enqueue_with_id("blocker-job", &project_id, "blocker_type")
            .unwrap();
        core.scheduler_mut().dequeue_job("blocker-job").unwrap();

        // Queued job B: storage is persisted `completed` first, then the
        // scheduler transition must fail because the slot is busy.
        let job_b = "job-terminal-transition".to_string();
        core.storage_mut()
            .unwrap()
            .create_job_with_id(&job_b, &project_id, "target_type")
            .unwrap();
        core.scheduler_mut()
            .enqueue_with_id(&job_b, &project_id, "target_type")
            .unwrap();

        let spec: BlenderJobSpec = serde_json::from_value(json!({
            "job_type": "target_type",
            "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
        }))
        .unwrap();
        let result = BlenderJobResult {
            job_id: job_b.clone(),
            status: "completed".into(),
            message: None,
            artifacts: None,
            scene_summary: None,
            error: None,
        };
        let cmd = CommandEnvelope {
            id: "finalize".into(),
            method: method::BLENDER_RUN_JOB.into(),
            params: json!({}),
        };

        let resp = finalize_completed(&mut core, &cmd, &spec, &job_b, &result, None);
        let error = resp
            .error
            .expect("inconsistent transitions must return InternalError");
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(
            error.message.contains("could not persist terminal state"),
            "unexpected error: {}",
            error.message
        );

        // No terminal event may contradict the persisted failed state.
        let events: Vec<EventEnvelope> = rx.try_iter().collect();
        assert!(
            events.is_empty(),
            "transition failure must not emit any job event: {events:?}"
        );

        // Recovery: scheduler and storage both `failed` for the same id.
        assert_eq!(
            core.scheduler().get_job(&job_b).unwrap().status,
            JobStatus::Failed
        );
        let stored = core
            .storage()
            .unwrap()
            .get_job(&job_b)
            .unwrap()
            .expect("stored job");
        assert_eq!(stored["status"], json!("failed"));
        // The blocker is untouched and still running.
        assert_eq!(
            core.scheduler().get_job("blocker-job").unwrap().status,
            JobStatus::Running
        );
    }

    #[test]
    fn test_handler_terminal_jobs_persist_finished_at() {
        let (mut core, _rx, _fake_dir) = fake_core("success");
        let project_dir = TempDir::new().unwrap();
        create_project(&mut core, &project_dir, "finished-at");

        // Completed job: response and job.get expose non-null finished_at.
        let completed = run_blender_job(
            &mut core,
            json!({
                "job_type": "completed_job",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        )
        .result
        .unwrap();
        assert_eq!(completed["status"], "completed");
        let completed_id = completed["job"]["id"].as_str().unwrap().to_string();
        let completed_finished = completed["job"]["finished_at"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            !completed_finished.is_empty(),
            "completed job must persist finished_at: {completed}"
        );
        let get_completed = core.handle_command(&CommandEnvelope {
            id: "g-completed".into(),
            method: method::JOB_GET.into(),
            params: json!({"job_id": &completed_id}),
        });
        assert_eq!(
            get_completed.result.unwrap()["finished_at"].as_str(),
            Some(completed_finished.as_str())
        );

        // Failed job (worker failed status) also persists finished_at.
        let _failed_fake = install_fake_adapter(&mut core, "failed_result");
        let failed = run_blender_job(
            &mut core,
            json!({
                "job_type": "failed_job",
                "wait": true,
                "spec": {"operations": [{"op": "create_box", "name": "x", "size": [1.0, 1.0, 1.0]}]}
            }),
        )
        .result
        .unwrap();
        assert_eq!(failed["status"], "failed");
        let failed_id = failed["job"]["id"].as_str().unwrap().to_string();
        let failed_finished = failed["job"]["finished_at"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            !failed_finished.is_empty(),
            "failed job must persist finished_at: {failed}"
        );
        let get_failed = core.handle_command(&CommandEnvelope {
            id: "g-failed".into(),
            method: method::JOB_GET.into(),
            params: json!({"job_id": &failed_id}),
        });
        assert_eq!(
            get_failed.result.unwrap()["finished_at"].as_str(),
            Some(failed_finished.as_str())
        );

        // job.list exposes the same terminal finish times.
        let list = core.handle_command(&CommandEnvelope {
            id: "l1".into(),
            method: method::JOB_LIST.into(),
            params: json!({}),
        });
        let list = list.result.unwrap();
        let jobs = list["jobs"].as_array().unwrap();
        for (id, finished) in [
            (&completed_id, &completed_finished),
            (&failed_id, &failed_finished),
        ] {
            let job = jobs
                .iter()
                .find(|job| job["id"].as_str() == Some(id.as_str()))
                .expect("terminal job in job.list");
            assert_eq!(
                job["finished_at"].as_str(),
                Some(finished.as_str()),
                "job.list finished_at for {id}"
            );
        }
    }
}
