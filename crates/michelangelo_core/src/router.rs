use michelangelo_protocol::command::CommandEnvelope;
use michelangelo_protocol::error::{ErrorCode, ProtocolError};
use michelangelo_protocol::event::{
    EventEnvelope, JOB_CANCELLED, JOB_COMPLETED, JOB_FAILED, JOB_QUEUED, JOB_STARTED,
};
use michelangelo_protocol::method;
use michelangelo_protocol::response::ResponseEnvelope;
use michelangelo_protocol::snapshot::ProjectSnapshotDto;
use michelangelo_protocol::{BlenderJobSpec, JobDto, JobStatus, PROTOCOL_VERSION};
use michelangelo_workspace::error::WorkspaceError;
use michelangelo_workspace::storage::Storage;
use std::path::Path;

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
    let adapter = core.blender();
    let version = adapter.check_binary();
    let supported: Vec<&str> = michelangelo_protocol::SAFE_WORKER_OPS.to_vec();

    match version {
        Ok(v) => ResponseEnvelope::success(
            &cmd.id,
            serde_json::json!({
                "blender_version": v,
                "bpy_available": true,
                "supported_operations": supported,
                "binary_found": true,
            }),
        ),
        Err(e) => ResponseEnvelope::success(
            &cmd.id,
            serde_json::json!({
                "blender_version": null,
                "bpy_available": false,
                "supported_operations": supported,
                "binary_found": false,
                "error": format!("{e}"),
            }),
        ),
    }
}

fn handle_blender_run_job(core: &mut CoreService, cmd: &CommandEnvelope) -> ResponseEnvelope {
    let project = match core.current_project() {
        Some(p) => p.clone(),
        None => return no_project_error(cmd),
    };

    let spec: BlenderJobSpec = match serde_json::from_value(cmd.params.clone()) {
        Ok(s) => s,
        Err(e) => return invalid_params(cmd, &format!("invalid spec: {e}")),
    };

    let job_id = Storage::generate_job_id();

    // Step 1: Create job in storage (scoped mutable borrow)
    {
        let storage = match core.storage_mut() {
            Some(s) => s,
            None => return storage_not_open_error(cmd),
        };
        if let Err(e) = storage.create_job_with_id(&job_id, &project.id, &spec.job_type) {
            return internal_error(cmd, &format!("failed to create job: {e}"));
        }
    }

    // Step 2: Enqueue in scheduler
    let scheduler_job_id =
        match core
            .scheduler_mut()
            .enqueue_with_id(&job_id, &project.id, &spec.job_type)
        {
            Ok(id) => id,
            Err(e) => return internal_error(cmd, &e),
        };

    // Step 3: Emit queued event
    core.emit_event(EventEnvelope::new(
        JOB_QUEUED,
        serde_json::json!({
            "job_id": scheduler_job_id.0,
            "job_type": spec.job_type,
        }),
    ));

    if !spec.wait {
        return ResponseEnvelope::success(
            &cmd.id,
            serde_json::json!({
                "id": scheduler_job_id.0,
                "job_type": spec.job_type,
                "status": "queued",
            }),
        );
    }

    // Step 4: Dequeue and start running
    if core.scheduler_mut().dequeue().is_none() {
        return internal_error(cmd, "failed to dequeue job");
    }

    core.emit_event(EventEnvelope::new(
        JOB_STARTED,
        serde_json::json!({
            "job_id": scheduler_job_id.0,
            "job_type": spec.job_type,
        }),
    ));

    {
        let storage = match core.storage_mut() {
            Some(s) => s,
            None => return storage_not_open_error(cmd),
        };
        if let Err(e) = storage.update_job_status(&job_id, "running", None, None) {
            return internal_error(cmd, &format!("failed to update job status: {e}"));
        }
    }

    // Step 5: Execute via Blender adapter
    let workspace_root = Path::new(&project.root_path).to_path_buf();
    let output_dir = workspace_root.join("jobs");

    let timeout = spec.timeout_ms.or_else(|| {
        std::env::var("MICHELANGELO_BLENDER_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
    });

    let result = core.blender().run_job(
        &scheduler_job_id.0,
        &spec,
        &workspace_root,
        &output_dir,
        timeout,
    );

    // Step 6: Handle result
    match result {
        Ok(blender_result) => {
            let artifacts: Vec<michelangelo_protocol::ArtifactDto> = blender_result
                .artifacts
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|a| michelangelo_protocol::ArtifactDto {
                    path: a.path,
                    artifact_type: a.artifact_type,
                    size_bytes: a.size_bytes,
                })
                .collect();

            let _ = core
                .scheduler_mut()
                .complete(&scheduler_job_id.0, artifacts.clone());

            {
                let storage = match core.storage_mut() {
                    Some(s) => s,
                    None => return storage_not_open_error(cmd),
                };
                let _ = storage.update_job_status(
                    &job_id,
                    "completed",
                    Some(100),
                    Some("job completed"),
                );
                let output_json = serde_json::to_string(&blender_result).ok();
                if let Some(ref json) = output_json {
                    let _ = storage.update_job_output(&job_id, Some(json), None, None);
                }
            }

            core.emit_event(EventEnvelope::new(
                JOB_COMPLETED,
                serde_json::json!({
                    "job_id": scheduler_job_id.0,
                    "job_type": spec.job_type,
                    "status": "completed",
                    "artifacts": artifacts,
                }),
            ));

            let final_job = core
                .storage()
                .and_then(|s| s.get_job(&job_id).ok().flatten());
            ResponseEnvelope::success(
                &cmd.id,
                serde_json::json!({
                    "job": final_job,
                    "status": "completed",
                }),
            )
        }
        Err(e) => {
            let error_msg = format!("{e}");
            let _ = core.scheduler_mut().fail(&scheduler_job_id.0, &error_msg);

            {
                let storage = match core.storage_mut() {
                    Some(s) => s,
                    None => return storage_not_open_error(cmd),
                };
                let _ = storage.update_job_status(&job_id, "failed", None, Some(&error_msg));
                let _ = storage.update_job_output(&job_id, None, Some(&error_msg), None);
            }

            core.emit_event(EventEnvelope::new(
                JOB_FAILED,
                serde_json::json!({
                    "job_id": scheduler_job_id.0,
                    "job_type": spec.job_type,
                    "status": "failed",
                    "error": error_msg,
                }),
            ));

            let final_job = core
                .storage()
                .and_then(|s| s.get_job(&job_id).ok().flatten());
            ResponseEnvelope::success(
                &cmd.id,
                serde_json::json!({
                    "job": final_job,
                    "status": "failed",
                    "error": error_msg,
                }),
            )
        }
    }
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
    use serde_json::json;

    fn handler_for(router: &Router, method: &str) -> Option<Handler> {
        router.find(method)
    }

    fn core() -> CoreService {
        let (svc, _rx) = CoreService::new();
        svc
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
        let mut core = core();
        let resp = core.handle_command(&CommandEnvelope {
            id: "b1".into(),
            method: method::BLENDER_GET_CAPABILITIES.into(),
            params: json!({}),
        });
        assert!(resp.error.is_none());
        let caps = resp.result.unwrap();
        assert!(caps["supported_operations"].is_array());
        let ops = caps["supported_operations"].as_array().unwrap();
        assert!(!ops.is_empty(), "should list at least create_box");
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
}
