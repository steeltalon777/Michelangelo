//! Canonical method name constants for the Michelangelo Core Protocol.

/// `system.ping` — health check; returns pong with version info.
pub const SYSTEM_PING: &str = "system.ping";

/// `project.create` — creates a new project workspace at the given path.
pub const PROJECT_CREATE: &str = "project.create";

/// `project.open` — opens an existing project workspace.
pub const PROJECT_OPEN: &str = "project.open";

/// `project.get_snapshot` — returns current project state for UI recovery.
pub const PROJECT_GET_SNAPSHOT: &str = "project.get_snapshot";

/// `job.list` — return recent jobs for the active project.
pub const JOB_LIST: &str = "job.list";

/// `job.get` — return one job by `job_id`.
pub const JOB_GET: &str = "job.get";

/// `job.cancel` — request cancellation for a queued/running job.
pub const JOB_CANCEL: &str = "job.cancel";

/// `blender.get_capabilities` — check configured Blender executable and supported operations.
pub const BLENDER_GET_CAPABILITIES: &str = "blender.get_capabilities";

/// `blender.run_job` — schedule a safe Blender job from `BlenderJobSpec`.
pub const BLENDER_RUN_JOB: &str = "blender.run_job";

/// All known Phase 2 methods (for router registration and tests).
pub const PHASE_2_METHODS: &[&str] = &[
    JOB_LIST,
    JOB_GET,
    JOB_CANCEL,
    BLENDER_GET_CAPABILITIES,
    BLENDER_RUN_JOB,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_names_are_non_empty() {
        assert!(!SYSTEM_PING.is_empty());
        assert!(!PROJECT_CREATE.is_empty());
        assert!(!PROJECT_OPEN.is_empty());
        assert!(!PROJECT_GET_SNAPSHOT.is_empty());
        for m in PHASE_2_METHODS {
            assert!(!m.is_empty(), "empty method name in PHASE_2_METHODS");
        }
    }

    #[test]
    fn test_method_names_use_dot_notation() {
        assert!(SYSTEM_PING.contains('.'));
        assert!(PROJECT_CREATE.contains('.'));
        assert!(PROJECT_OPEN.contains('.'));
        assert!(PROJECT_GET_SNAPSHOT.contains('.'));
        for m in PHASE_2_METHODS {
            assert!(m.contains('.'), "method '{}' does not use dot notation", m);
        }
    }

    #[test]
    fn test_phase_2_methods_are_unique() {
        let mut sorted = PHASE_2_METHODS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), PHASE_2_METHODS.len());
    }
}
