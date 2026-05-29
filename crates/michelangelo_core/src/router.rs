use michelangelo_protocol::command::CommandEnvelope;
use michelangelo_protocol::error::{ErrorCode, ProtocolError};
use michelangelo_protocol::method;
use michelangelo_protocol::response::ResponseEnvelope;
use michelangelo_protocol::snapshot::ProjectSnapshotDto;
use michelangelo_protocol::PROTOCOL_VERSION;
use michelangelo_workspace::error::WorkspaceError;
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
    r
}

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
            let snapshot = ProjectSnapshotDto::with_project(project.clone());
            ResponseEnvelope::success(&cmd.id, serde_json::json!(snapshot))
        }
        None => ResponseEnvelope::error(
            Some(cmd.id.clone()),
            ProtocolError::new(ErrorCode::WorkspaceNotOpen, "no project is currently open"),
        ),
    }
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
        let mut core = CoreService::new();
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
        let mut core = CoreService::new();
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
        let mut core = CoreService::new();

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
        let mut core = CoreService::new();
        let cmd = CommandEnvelope {
            id: "e1".into(),
            method: method::PROJECT_GET_SNAPSHOT.into(),
            params: json!({}),
        };
        let resp = core.handle_command(&cmd);
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::WorkspaceNotOpen);
    }
}
