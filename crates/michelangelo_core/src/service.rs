//! Core application service — state management for the headless core.
//!
//! Command routing is delegated to [`Router`] so the registry of
//! known methods lives in one obvious place (`router.rs`).

use std::sync::mpsc::{self, Receiver, Sender};

use michelangelo_blender::HeadlessBlenderAdapter;
use michelangelo_jobs::Scheduler;
use michelangelo_protocol::command::CommandEnvelope;
use michelangelo_protocol::error::{ErrorCode, ProtocolError};
use michelangelo_protocol::event::EventEnvelope;
use michelangelo_protocol::response::ResponseEnvelope;
use michelangelo_protocol::snapshot::ProjectDto;
use michelangelo_workspace::storage::Storage;
use michelangelo_workspace::WorkspaceService;

use crate::router::{default_router, Router};

/// Core application service holding workspace, project, job, and Blender state.
pub struct CoreService {
    workspace: WorkspaceService,
    current_project: Option<ProjectDto>,
    router: Router,
    storage: Option<Storage>,
    scheduler: Scheduler,
    blender: HeadlessBlenderAdapter,
    event_tx: Sender<EventEnvelope>,
    _job_event_rx: Receiver<michelangelo_jobs::JobEvent>,
}

impl CoreService {
    /// Create a new CoreService with an event channel.
    ///
    /// Returns the service and a `Receiver` for consuming protocol `EventEnvelope`s.
    pub fn new() -> (Self, Receiver<EventEnvelope>) {
        let (scheduler, job_event_rx) = Scheduler::new();
        let (event_tx, event_rx) = mpsc::channel();
        let service = Self {
            workspace: WorkspaceService,
            current_project: None,
            router: default_router(),
            storage: None,
            scheduler,
            blender: HeadlessBlenderAdapter::new(),
            event_tx,
            _job_event_rx: job_event_rx,
        };
        (service, event_rx)
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

    // -- Accessors -----------------------------------------------------------

    pub fn workspace_ref(&self) -> &WorkspaceService {
        &self.workspace
    }

    pub fn current_project(&self) -> Option<&ProjectDto> {
        self.current_project.as_ref()
    }

    pub fn set_current_project(&mut self, project: Option<ProjectDto>) {
        self.current_project = project;
    }

    pub fn storage(&self) -> Option<&Storage> {
        self.storage.as_ref()
    }

    pub fn storage_mut(&mut self) -> Option<&mut Storage> {
        self.storage.as_mut()
    }

    pub fn set_storage(&mut self, storage: Option<Storage>) {
        self.storage = storage;
    }

    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    pub fn scheduler_mut(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }

    pub fn blender(&self) -> &HeadlessBlenderAdapter {
        &self.blender
    }

    /// Send a protocol event through the event channel.
    pub fn emit_event(&self, event: EventEnvelope) {
        let _ = self.event_tx.send(event);
    }

    /// Return a clone of the event sender (for testing or forwarding).
    pub fn event_sender(&self) -> Sender<EventEnvelope> {
        self.event_tx.clone()
    }

    /// Initialize storage from the current project's SQLite database.
    pub fn init_storage(&mut self) {
        if let Some(project) = &self.current_project {
            let db_path =
                michelangelo_workspace::layout::db_path(std::path::Path::new(&project.root_path));
            if let Ok(storage) = Storage::open(&db_path) {
                self.storage = Some(storage);
            }
        }
    }
}

impl Default for CoreService {
    fn default() -> Self {
        let (svc, _rx) = Self::new();
        svc
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

    fn new_core() -> (CoreService, Receiver<EventEnvelope>) {
        CoreService::new()
    }

    #[test]
    fn test_service_ping() {
        let (mut svc, _rx) = new_core();
        let resp = svc.handle_command(&cmd(method::SYSTEM_PING, json!({})));
        assert_eq!(resp.result.unwrap()["pong"], true);
    }

    #[test]
    fn test_service_create_and_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let (mut svc, _rx) = new_core();

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
        let (mut svc, _rx) = new_core();

        svc.handle_command(&cmd(
            method::PROJECT_CREATE,
            json!({"path": dir.path(), "name": "open-test"}),
        ));

        let (mut svc2, _rx) = new_core();
        let open_resp =
            svc2.handle_command(&cmd(method::PROJECT_OPEN, json!({"path": dir.path()})));
        assert!(open_resp.error.is_none());

        let snap_resp = svc2.handle_command(&cmd(method::PROJECT_GET_SNAPSHOT, json!({})));
        assert!(snap_resp.error.is_none());
        assert_eq!(snap_resp.result.unwrap()["project"]["name"], "open-test");
    }

    #[test]
    fn test_service_accessors() {
        let (mut svc, _rx) = new_core();
        assert!(svc.current_project().is_none());

        let dto = ProjectDto::new("p1", "accessor-test", "/tmp/p1");
        svc.set_current_project(Some(dto));
        assert_eq!(svc.current_project().unwrap().name, "accessor-test");
    }

    #[test]
    fn test_service_default() {
        let (svc, _rx) = CoreService::new();
        assert!(svc.current_project().is_none());
    }

    #[test]
    fn test_emit_event() {
        let (svc, rx) = CoreService::new();
        let event = EventEnvelope::new("test.event", json!({"key": "value"}));
        svc.emit_event(event);

        let received: Vec<EventEnvelope> = rx.try_iter().collect();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].event, "test.event");
        assert_eq!(received[0].data["key"], "value");
    }
}
