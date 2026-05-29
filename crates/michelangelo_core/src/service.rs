//! Core application service — command routing and state management.

use std::path::Path;

use michelangelo_protocol::{
    command::CommandEnvelope,
    error::{ErrorCode, ProtocolError},
    method,
    response::ResponseEnvelope,
    snapshot::{ProjectDto, ProjectSnapshotDto},
};
use michelangelo_workspace::{error::WorkspaceError, WorkspaceService};

/// Core application service holding workspace state.
pub struct CoreService {
    workspace: WorkspaceService,
    current_project: Option<ProjectDto>,
}

impl CoreService {
    pub fn new() -> Self {
        Self {
            workspace: WorkspaceService,
            current_project: None,
        }
    }

    /// Route an incoming command to the appropriate handler.
    ///
    /// Every path returns a `ResponseEnvelope`; no panic is possible
    /// from invalid input alone.
    pub fn handle_command(&mut self, cmd: &CommandEnvelope) -> ResponseEnvelope {
        let id = Some(cmd.id.clone());

        match cmd.method.as_str() {
            method::SYSTEM_PING => ResponseEnvelope::success(
                &cmd.id,
                serde_json::json!({
                    "pong": true,
                    "protocol_version": michelangelo_protocol::PROTOCOL_VERSION,
                }),
            ),
            method::PROJECT_CREATE => self.handle_project_create(cmd),
            method::PROJECT_OPEN => self.handle_project_open(cmd),
            method::PROJECT_GET_SNAPSHOT => self.handle_project_get_snapshot(cmd),
            _ => ResponseEnvelope::error(
                id,
                ProtocolError::new(
                    ErrorCode::MethodNotFound,
                    format!("unknown method: '{}'", cmd.method),
                ),
            ),
        }
    }

    /// Handle `project.create` — create a new project workspace.
    fn handle_project_create(&mut self, cmd: &CommandEnvelope) -> ResponseEnvelope {
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
        match self.workspace.create_project(path, name) {
            Ok(dto) => {
                self.current_project = Some(dto.clone());
                ResponseEnvelope::success(&cmd.id, serde_json::json!(dto))
            }
            Err(e) => {
                let (code, msg) = map_workspace_error(&e);
                ResponseEnvelope::error(Some(cmd.id.clone()), ProtocolError::new(code, msg))
            }
        }
    }

    /// Handle `project.open` — open an existing project workspace.
    fn handle_project_open(&mut self, cmd: &CommandEnvelope) -> ResponseEnvelope {
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
        match self.workspace.open_project(path) {
            Ok(dto) => {
                self.current_project = Some(dto.clone());
                ResponseEnvelope::success(&cmd.id, serde_json::json!(dto))
            }
            Err(e) => {
                let (code, msg) = map_workspace_error(&e);
                ResponseEnvelope::error(Some(cmd.id.clone()), ProtocolError::new(code, msg))
            }
        }
    }

    /// Handle `project.get_snapshot` — return current project state.
    fn handle_project_get_snapshot(&self, cmd: &CommandEnvelope) -> ResponseEnvelope {
        match &self.current_project {
            Some(project) => {
                let snapshot = ProjectSnapshotDto::with_project(project.clone());
                ResponseEnvelope::success(&cmd.id, serde_json::json!(snapshot))
            }
            None => ResponseEnvelope::error(
                Some(cmd.id.clone()),
                ProtocolError::new(ErrorCode::WorkspaceNotOpen, "no project is currently open"),
            ),
        }
    }
}

/// Map a WorkspaceError to a protocol ErrorCode and message string.
fn map_workspace_error(err: &WorkspaceError) -> (ErrorCode, String) {
    match err {
        WorkspaceError::NotAWorkspace(msg) => (ErrorCode::WorkspaceNotOpen, msg.clone()),
        WorkspaceError::Io(e) => (ErrorCode::InternalError, format!("filesystem error: {e}")),
        WorkspaceError::Json(e) => (ErrorCode::InternalError, format!("metadata error: {e}")),
        WorkspaceError::AlreadyExists(msg) => (ErrorCode::AlreadyExists, msg.clone()),
    }
}

impl Default for CoreService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(method: &str, params: serde_json::Value) -> CommandEnvelope {
        CommandEnvelope {
            id: "test".into(),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn test_system_ping() {
        let mut svc = CoreService::new();
        let resp = svc.handle_command(&cmd(method::SYSTEM_PING, json!({})));
        assert_eq!(resp.id, Some("test".into()));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["pong"], true);
    }

    #[test]
    fn test_unknown_method() {
        let mut svc = CoreService::new();
        let resp = svc.handle_command(&cmd("foo.bar", json!({})));
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_project_create_missing_path() {
        let mut svc = CoreService::new();
        let resp = svc.handle_command(&cmd(method::PROJECT_CREATE, json!({})));
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn test_project_create_and_open_integration() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut svc = CoreService::new();

        // Create
        let create_resp = svc.handle_command(&cmd(
            method::PROJECT_CREATE,
            json!({"path": dir.path(), "name": "integration-test"}),
        ));
        assert!(
            create_resp.error.is_none(),
            "create failed: {:?}",
            create_resp.error
        );
        let dto = create_resp.result.unwrap();
        assert_eq!(dto["name"], "integration-test");

        // Open
        let open_resp = svc.handle_command(&cmd(method::PROJECT_OPEN, json!({"path": dir.path()})));
        assert!(
            open_resp.error.is_none(),
            "open failed: {:?}",
            open_resp.error
        );
        let opened = open_resp.result.unwrap();
        assert_eq!(opened["name"], "integration-test");
    }

    #[test]
    fn test_project_open_non_existent() {
        let mut svc = CoreService::new();
        let resp = svc.handle_command(&cmd(
            method::PROJECT_OPEN,
            json!({"path": "/tmp/__nonexistent_michelangelo_test__"}),
        ));
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_project_get_snapshot_after_create() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut svc = CoreService::new();

        let create_resp = svc.handle_command(&cmd(
            method::PROJECT_CREATE,
            json!({"path": dir.path(), "name": "snapshot-test"}),
        ));
        assert!(create_resp.error.is_none());

        let snap_resp = svc.handle_command(&cmd(method::PROJECT_GET_SNAPSHOT, json!({})));
        assert!(
            snap_resp.error.is_none(),
            "snapshot failed: {:?}",
            snap_resp.error
        );
        let snap = snap_resp.result.unwrap();
        assert_eq!(snap["workspace_status"], "active");
        assert_eq!(snap["project"]["name"], "snapshot-test");
    }

    #[test]
    fn test_project_get_snapshot_empty() {
        let mut svc = CoreService::new();
        let resp = svc.handle_command(&cmd(method::PROJECT_GET_SNAPSHOT, json!({})));
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::WorkspaceNotOpen);
    }
}
