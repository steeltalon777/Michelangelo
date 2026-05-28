//! Michelangelo Workspace — local project workspace management.
//!
//! Handles create/open/inspect operations for local Michelangelo
//! project workspaces on the filesystem.

/// Placeholder: workspace logic will be implemented in T-0005+.
pub fn workspace_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_version_is_not_empty() {
        assert!(!workspace_version().is_empty());
    }
}
