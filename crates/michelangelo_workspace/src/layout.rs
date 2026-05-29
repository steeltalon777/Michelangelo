//! Workspace directory layout definition and validation.

use std::path::{Path, PathBuf};

use crate::error::WorkspaceError;

/// Metadata directory (contains project.json).
pub const META_DIR: &str = ".michelangelo";

/// Metadata file containing project descriptor.
pub const PROJECT_FILE: &str = ".michelangelo/project.json";

/// SQLite metadata database path (relative to workspace root).
pub const DB_FILE: &str = ".michelangelo/metadata.db";

/// Sub-directories created under the workspace root.
pub const WORKSPACE_DIRS: &[&str] = &[
    ".michelangelo",
    "assets",
    "refs",
    "layers",
    "thumbnails",
    "masks",
    "contours",
    "jobs",
    "blender/scenes",
    "blender/renders",
    "blender/exports",
];

/// Return the path to the project metadata file.
pub fn project_file_path(root: &Path) -> PathBuf {
    root.join(PROJECT_FILE)
}

/// Return the path to the metadata directory.
pub fn meta_dir_path(root: &Path) -> PathBuf {
    root.join(META_DIR)
}

/// Return the path to the SQLite metadata database.
pub fn db_path(root: &Path) -> PathBuf {
    root.join(DB_FILE)
}

/// Check whether a path looks like a valid Michelangelo workspace.
///
/// Returns `true` if the metadata file exists and is a file.
pub fn is_workspace(root: &Path) -> bool {
    project_file_path(root).is_file()
}

/// Create the standard workspace sub-directory layout under `root`.
pub fn create_layout(root: &Path) -> std::io::Result<()> {
    for sub in WORKSPACE_DIRS {
        let dir = root.join(sub);
        std::fs::create_dir_all(&dir)?;
    }
    Ok(())
}

/// Validate that a workspace has the expected directory layout.
///
/// Checks that every entry in `WORKSPACE_DIRS` exists as a directory under `root`.
/// Returns `Ok(())` if all directories are present, or a `Corrupted` error with
/// a list of the missing paths.
pub fn validate_layout(root: &Path) -> Result<(), WorkspaceError> {
    let missing: Vec<&str> = WORKSPACE_DIRS
        .iter()
        .filter(|sub| !root.join(sub).is_dir())
        .copied()
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(WorkspaceError::Corrupted(format!(
            "missing workspace directories: {}",
            missing.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_file_path() {
        let p = Path::new("/tmp/test");
        assert_eq!(
            project_file_path(p),
            Path::new("/tmp/test/.michelangelo/project.json")
        );
    }

    #[test]
    fn test_is_workspace_returns_false_for_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!is_workspace(dir.path()));
    }

    #[test]
    fn test_create_layout_creates_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        create_layout(dir.path()).unwrap();
        for sub in WORKSPACE_DIRS {
            assert!(dir.path().join(sub).is_dir(), "missing: {sub}");
        }
    }

    #[test]
    fn test_validate_layout_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        create_layout(dir.path()).unwrap();
        assert!(validate_layout(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_layout_missing_all_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = validate_layout(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::Corrupted(_)));
        let msg = err.to_string();
        // Should list several missing dirs
        for sub in WORKSPACE_DIRS.iter().take(3) {
            assert!(msg.contains(sub), "missing '{}' from error: {msg}", sub);
        }
    }

    #[test]
    fn test_validate_layout_missing_one_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        create_layout(dir.path()).unwrap();

        // Remove one directory
        let target = dir.path().join("assets");
        std::fs::remove_dir(&target).unwrap();

        let err = validate_layout(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::Corrupted(_)));
        assert!(err.to_string().contains("assets"));
    }

    #[test]
    fn test_validate_layout_clean_after_repair() {
        let dir = tempfile::TempDir::new().unwrap();
        create_layout(dir.path()).unwrap();

        // Remove and re-create a directory
        std::fs::remove_dir(dir.path().join("layers")).unwrap();
        assert!(validate_layout(dir.path()).is_err());

        std::fs::create_dir(dir.path().join("layers")).unwrap();
        assert!(validate_layout(dir.path()).is_ok());
    }

    #[test]
    fn test_is_workspace_requires_project_json() {
        let dir = tempfile::TempDir::new().unwrap();
        create_layout(dir.path()).unwrap();
        // Layout exists, but no project.json
        assert!(!is_workspace(dir.path()));
    }
}
