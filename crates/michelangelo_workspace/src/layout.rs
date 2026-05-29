//! Workspace directory layout definition and validation.

use std::path::{Path, PathBuf};

/// Metadata directory (contains project.json).
pub const META_DIR: &str = ".michelangelo";

/// Metadata file containing project descriptor.
pub const PROJECT_FILE: &str = ".michelangelo/project.json";

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
}
