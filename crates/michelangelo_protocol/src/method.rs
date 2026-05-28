//! Canonical method name constants for the Michelangelo Core Protocol.

/// `system.ping` — health check; returns pong with version info.
pub const SYSTEM_PING: &str = "system.ping";

/// `project.create` — creates a new project workspace at the given path.
pub const PROJECT_CREATE: &str = "project.create";

/// `project.open` — opens an existing project workspace.
pub const PROJECT_OPEN: &str = "project.open";

/// `project.get_snapshot` — returns current project state for UI recovery.
pub const PROJECT_GET_SNAPSHOT: &str = "project.get_snapshot";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_names_are_non_empty() {
        assert!(!SYSTEM_PING.is_empty());
        assert!(!PROJECT_CREATE.is_empty());
        assert!(!PROJECT_OPEN.is_empty());
        assert!(!PROJECT_GET_SNAPSHOT.is_empty());
    }

    #[test]
    fn test_method_names_use_dot_notation() {
        assert!(SYSTEM_PING.contains('.'));
        assert!(PROJECT_CREATE.contains('.'));
        assert!(PROJECT_OPEN.contains('.'));
        assert!(PROJECT_GET_SNAPSHOT.contains('.'));
    }
}
