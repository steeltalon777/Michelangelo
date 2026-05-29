//! Michelangelo Workspace — local project workspace management.
//!
//! Handles create/open/inspect operations for local Michelangelo
//! project workspaces on the filesystem.

pub mod error;
pub mod layout;
pub mod service;
pub mod storage;

pub use service::WorkspaceService;

/// Workspace crate version string.
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
