//! Workspace service — create/open/validate project workspaces.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use michelangelo_protocol::snapshot::ProjectDto;
use serde::{Deserialize, Serialize};

use crate::error::WorkspaceError;
use crate::layout;

/// On-disk metadata stored in `.michelangelo/project.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub created_at: String,
}

impl ProjectMeta {
    /// Create a new metadata record.
    pub fn new(id: String, name: String, root_path: String) -> Self {
        let created_at = timestamp_iso8601();
        Self {
            id,
            name,
            root_path,
            created_at,
        }
    }
}

/// Workspace service for creating, opening, and validating project workspaces.
#[derive(Debug, Default)]
pub struct WorkspaceService;

impl WorkspaceService {
    /// Create a new project workspace at `path` with the given `name`.
    ///
    /// Returns the `ProjectDto` for the newly created project.
    ///
    /// # Errors
    ///
    /// - `WorkspaceError::AlreadyExists` if the workspace already exists.
    /// - `WorkspaceError::Io` if filesystem operations fail.
    pub fn create_project(&self, path: &Path, name: &str) -> Result<ProjectDto, WorkspaceError> {
        // Resolve to an absolute path. If the path does not exist yet, use
        // its parent directory to resolve and then append the final component.
        let resolved = resolve_path(path)?;

        if layout::is_workspace(&resolved) {
            return Err(WorkspaceError::AlreadyExists(
                resolved.display().to_string(),
            ));
        }

        // Ensure the root directory exists.
        std::fs::create_dir_all(&resolved)?;

        // Create directory layout.
        layout::create_layout(&resolved)?;

        // Generate project id from name or directory name.
        let project_name = if name.is_empty() {
            dir_name(&resolved)
        } else {
            name.to_string()
        };
        let project_id = id_from_name(&project_name);

        // Write metadata file.
        let meta = ProjectMeta::new(
            project_id.clone(),
            project_name.clone(),
            resolved.display().to_string(),
        );
        let meta_path = layout::project_file_path(&resolved);
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(&meta_path, meta_json)?;

        Ok(ProjectDto::new(
            project_id,
            project_name,
            resolved.display().to_string(),
        ))
    }

    /// Open an existing project workspace at `path`.
    ///
    /// Returns the `ProjectDto` if the workspace is valid.
    ///
    /// # Errors
    ///
    /// - `WorkspaceError::NotAWorkspace` if the path is not a valid workspace.
    /// - `WorkspaceError::Io` if filesystem operations fail.
    pub fn open_project(&self, path: &Path) -> Result<ProjectDto, WorkspaceError> {
        // Resolve the path — canonicalize if it exists, otherwise use as-is.
        let resolved = normalize_path(path);

        if !resolved.exists() {
            return Err(WorkspaceError::NotAWorkspace(format!(
                "path does not exist: {}",
                resolved.display()
            )));
        }

        if !layout::is_workspace(&resolved) {
            return Err(WorkspaceError::NotAWorkspace(
                resolved.display().to_string(),
            ));
        }

        // Read and parse metadata.
        let meta_path = layout::project_file_path(&resolved);
        let meta_json = std::fs::read_to_string(&meta_path)?;
        let meta: ProjectMeta = serde_json::from_str(&meta_json)?;

        Ok(ProjectDto::new(meta.id, meta.name, meta.root_path))
    }
}

/// Resolve a path to an absolute form, following symlinks if the path exists.
/// If the path does not exist, use its parent as a base for resolution.
fn resolve_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    // Try canonicalize first (works for existing paths).
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    // If the path doesn't exist yet, resolve relative to the current dir.
    let cwd = std::env::current_dir().map_err(WorkspaceError::Io)?;
    Ok(cwd.join(path))
}

/// Normalize a path to absolute without requiring it to exist.
fn normalize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        cwd.join(path)
    }
}

/// Generate a simple project ID from the project name.
fn id_from_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "-")
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_")
}

/// Extract the directory name from a path as a string.
fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string())
}

/// Produce a UTC timestamp string (ISO 8601 without sub-seconds, using a
/// simple calculation from the Unix epoch).
fn timestamp_iso8601() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs();

    // Simple modular calendar breakdown.
    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Approximate year/month/day from days since epoch.
    // Based on a simple leap-year-aware algorithm.
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1usize;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    let d = remaining + 1;

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_project_returns_dto() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        let dto = svc.create_project(dir.path(), "test-project").unwrap();
        assert_eq!(dto.name, "test-project");
        assert_eq!(dto.id, "test-project");
        assert!(dto
            .root_path
            .contains(dir.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn test_create_then_open() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        let created = svc.create_project(dir.path(), "my-project").unwrap();
        let opened = svc.open_project(dir.path()).unwrap();
        assert_eq!(opened.id, created.id);
        assert_eq!(opened.name, created.name);
        assert_eq!(opened.root_path, created.root_path);
    }

    #[test]
    fn test_create_idempotent_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "dup").unwrap();
        let err = svc.create_project(dir.path(), "dup").unwrap_err();
        assert!(matches!(err, WorkspaceError::AlreadyExists(_)));
    }

    #[test]
    fn test_open_non_existent() {
        let svc = WorkspaceService;
        let err = svc
            .open_project(Path::new("/tmp/nonexistent_michelangelo_xyz"))
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::NotAWorkspace(_)));
    }

    #[test]
    fn test_open_non_workspace() {
        let dir = tempfile::TempDir::new().unwrap();
        // Create an empty directory (not a workspace)
        let svc = WorkspaceService;
        let err = svc.open_project(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::NotAWorkspace(_)));
    }

    #[test]
    fn test_create_with_empty_name_uses_dir_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        let dto = svc.create_project(dir.path(), "").unwrap();
        // Name should match the directory name (last component of temp dir path)
        let expected = dir.path().file_name().unwrap().to_string_lossy();
        assert_eq!(dto.name, expected.as_ref());
    }

    #[test]
    fn test_directory_layout_created() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "layout-test").unwrap();
        for sub in crate::layout::WORKSPACE_DIRS {
            assert!(dir.path().join(sub).is_dir(), "missing: {sub}");
        }
    }
}
