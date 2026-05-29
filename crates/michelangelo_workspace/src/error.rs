use std::fmt;

/// Errors that can occur during workspace operations.
#[derive(Debug)]
pub enum WorkspaceError {
    /// The path is not a valid Michelangelo workspace.
    NotAWorkspace(String),
    /// I/O error during filesystem operation.
    Io(std::io::Error),
    /// JSON (de)serialization error.
    Json(serde_json::Error),
    /// Project with the given path already exists and is valid.
    AlreadyExists(String),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceError::NotAWorkspace(p) => write!(f, "not a Michelangelo workspace: {p}"),
            WorkspaceError::Io(e) => write!(f, "I/O error: {e}"),
            WorkspaceError::Json(e) => write!(f, "metadata error: {e}"),
            WorkspaceError::AlreadyExists(p) => write!(f, "workspace already exists: {p}"),
        }
    }
}

impl std::error::Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WorkspaceError::Io(e) => Some(e),
            WorkspaceError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for WorkspaceError {
    fn from(e: std::io::Error) -> Self {
        WorkspaceError::Io(e)
    }
}

impl From<serde_json::Error> for WorkspaceError {
    fn from(e: serde_json::Error) -> Self {
        WorkspaceError::Json(e)
    }
}

/// Map a `WorkspaceError` to a protocol `ErrorCode` and message.
pub fn workspace_error_to_protocol(err: &WorkspaceError) -> (i32, String) {
    match err {
        WorkspaceError::NotAWorkspace(_) => (-32000, err.to_string()),
        WorkspaceError::Io(e) => (-32603, format!("filesystem error: {e}")),
        WorkspaceError::Json(e) => (-32603, format!("metadata error: {e}")),
        WorkspaceError::AlreadyExists(_) => (-32001, err.to_string()),
    }
}
