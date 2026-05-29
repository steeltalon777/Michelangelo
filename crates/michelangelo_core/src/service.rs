//! Core application service — state management for the headless core.
//!
//! Command routing is delegated to [`Router`] so the registry of
//! known methods lives in one obvious place (`router.rs`).

use michelangelo_protocol::command::CommandEnvelope;
use michelangelo_protocol::error::{ErrorCode, ProtocolError};
use michelangelo_protocol::response::ResponseEnvelope;
use michelangelo_protocol::snapshot::ProjectDto;
use michelangelo_workspace::WorkspaceService;

use crate::router::{default_router, Router};

/// Core application service holding workspace and project state.
pub struct CoreService {
    workspace: WorkspaceService,
    current_project: Option<ProjectDto>,
    router: Router,
}

impl CoreService {
    pub fn new() -> Self {
        Self {
            workspace: WorkspaceService,
            current_project: None,
            router: default_router(),
        }
    }

    /// Route an incoming command to the appropriate handler via the router.
    ///
    /// Every path returns a `ResponseEnvelope`; no panic is possible
    /// from invalid input alone.
    pub fn handle_command(&mut self, cmd: &CommandEnvelope) -> ResponseEnvelope {
        match self.router.find(&cmd.method) {
            Some(handler) => handler(self, cmd),
            None => ResponseEnvelope::error(
                Some(cmd.id.clone()),
                ProtocolError::new(
                    ErrorCode::MethodNotFound,
                    format!("unknown method: '{}'", cmd.method),
                ),
            ),
        }
    }

    pub fn workspace_ref(&self) -> &WorkspaceService {
        &self.workspace
    }

    pub fn current_project(&self) -> Option<&ProjectDto> {
        self.current_project.as_ref()
    }

    pub fn set_current_project(&mut self, project: Option<ProjectDto>) {
        self.current_project = project;
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
    use michelangelo_protocol::method;
    use serde_json::json;

    fn cmd(method: &str, params: serde_json::Value) -> CommandEnvelope {
        CommandEnvelope {
            id: "test".into(),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn test_service_ping() {
        let mut svc = CoreService::new();
        let resp = svc.handle_command(&cmd(method::SYSTEM_PING, json!({})));
        assert_eq!(resp.result.unwrap()["pong"], true);
    }

    #[test]
    fn test_service_create_and_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut svc = CoreService::new();

        let create_resp = svc.handle_command(&cmd(
            method::PROJECT_CREATE,
            json!({"path": dir.path(), "name": "svc-test"}),
        ));
        assert!(create_resp.error.is_none());

        let snap_resp = svc.handle_command(&cmd(method::PROJECT_GET_SNAPSHOT, json!({})));
        assert!(snap_resp.error.is_none());
        assert_eq!(snap_resp.result.unwrap()["workspace_status"], "active");
    }

    #[test]
    fn test_service_open_and_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut svc = CoreService::new();

        svc.handle_command(&cmd(
            method::PROJECT_CREATE,
            json!({"path": dir.path(), "name": "open-test"}),
        ));

        // Create a new CoreService to simulate process boundary
        let mut svc2 = CoreService::new();
        let open_resp =
            svc2.handle_command(&cmd(method::PROJECT_OPEN, json!({"path": dir.path()})));
        assert!(open_resp.error.is_none());

        let snap_resp = svc2.handle_command(&cmd(method::PROJECT_GET_SNAPSHOT, json!({})));
        assert!(snap_resp.error.is_none());
        assert_eq!(snap_resp.result.unwrap()["project"]["name"], "open-test");
    }

    #[test]
    fn test_service_accessors() {
        let mut svc = CoreService::new();
        assert!(svc.current_project().is_none());

        let dto = ProjectDto::new("p1", "accessor-test", "/tmp/p1");
        svc.set_current_project(Some(dto));
        assert_eq!(svc.current_project().unwrap().name, "accessor-test");
    }

    #[test]
    fn test_service_default() {
        let svc = CoreService::default();
        assert!(svc.current_project().is_none());
    }
}
