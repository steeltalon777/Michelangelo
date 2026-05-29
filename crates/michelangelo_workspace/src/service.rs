//! Workspace service — create/open/validate project workspaces.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use michelangelo_protocol::snapshot::ProjectDto;
use serde::{Deserialize, Serialize};

use crate::error::WorkspaceError;
use crate::layout;
use crate::storage::Storage;

/// On-disk metadata stored in `.michelangelo/project.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub created_at: String,
}

impl ProjectMeta {
    /// Create a new metadata record with auto-generated timestamp.
    pub fn new(id: String, name: String, root_path: String) -> Self {
        let created_at = timestamp_iso8601();
        Self {
            id,
            name,
            root_path,
            created_at,
        }
    }

    /// Create a metadata record from explicit parts (used when loading from storage).
    pub fn from_row(id: String, name: String, root_path: String, created_at: String) -> Self {
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

        // Write metadata file (JSON — backward compatible marker).
        let meta = ProjectMeta::new(
            project_id.clone(),
            project_name.clone(),
            resolved.display().to_string(),
        );
        let meta_path = layout::project_file_path(&resolved);
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(&meta_path, meta_json)?;

        // Also persist to SQLite.
        let db_path = layout::db_path(&resolved);
        if let Ok(storage) = Storage::open(&db_path) {
            let _ = storage.save_project(&meta);
        }

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

        // Validate that required sub-directories exist.
        layout::validate_layout(&resolved)?;

        // Prefer SQLite storage; fall back to JSON project file.
        let root_path_str = resolved.display().to_string();
        let meta = match load_meta_from_sqlite(&resolved, &root_path_str) {
            Ok(Some(m)) => m,
            _ => load_meta_from_json(&resolved)?,
        };

        Ok(ProjectDto::new(meta.id, meta.name, meta.root_path))
    }
}

/// Try to load project metadata from SQLite storage.
fn load_meta_from_sqlite(
    root: &Path,
    root_path: &str,
) -> Result<Option<ProjectMeta>, WorkspaceError> {
    let db_path = layout::db_path(root);
    let storage = Storage::open(&db_path)?;
    storage.load_project_by_root(root_path)
}

/// Load project metadata from the JSON project file (backward compat).
fn load_meta_from_json(root: &Path) -> Result<ProjectMeta, WorkspaceError> {
    let meta_path = layout::project_file_path(root);
    let meta_json = std::fs::read_to_string(&meta_path)?;
    let meta: ProjectMeta = serde_json::from_str(&meta_json)?;
    Ok(meta)
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

    #[test]
    fn test_open_corrupted_missing_subdir() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "corrupt-test").unwrap();

        // Remove a required subdirectory
        std::fs::remove_dir(dir.path().join("refs")).unwrap();

        let err = svc.open_project(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::Corrupted(_)));
        assert!(err.to_string().contains("refs"));
    }

    #[test]
    fn test_open_corrupted_project_json_without_sqlite() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "json-test").unwrap();

        // Remove SQLite DB so open_project falls back to JSON
        let db_path = crate::layout::db_path(dir.path());
        if db_path.exists() {
            std::fs::remove_file(&db_path).unwrap();
        }

        // Overwrite project.json with invalid JSON
        let meta_path = crate::layout::project_file_path(dir.path());
        std::fs::write(&meta_path, r#"{invalid json content}"#).unwrap();

        let err = svc.open_project(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::Json(_)));
    }

    #[test]
    fn test_open_corrupted_empty_project_json_without_sqlite() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "empty-json").unwrap();

        // Remove SQLite DB so open_project falls back to JSON
        let db_path = crate::layout::db_path(dir.path());
        if db_path.exists() {
            std::fs::remove_file(&db_path).unwrap();
        }

        // Overwrite project.json with empty content
        let meta_path = crate::layout::project_file_path(dir.path());
        std::fs::write(&meta_path, "").unwrap();

        let err = svc.open_project(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::Json(_)));
    }

    #[test]
    fn test_open_prefers_sqlite_over_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "sqlite-first").unwrap();

        // Corrupt the JSON but keep SQLite — open should succeed from SQLite
        let meta_path = crate::layout::project_file_path(dir.path());
        std::fs::write(&meta_path, r#"{garbage}"#).unwrap();

        let result = svc.open_project(dir.path());
        assert!(
            result.is_ok(),
            "should open from SQLite: {:?}",
            result.err()
        );
        let dto = result.unwrap();
        assert_eq!(dto.name, "sqlite-first");
    }

    #[test]
    fn test_open_falls_back_to_json_without_sqlite() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;

        // Create a workspace, then delete the SQLite DB
        svc.create_project(dir.path(), "json-fallback").unwrap();
        let db_path = crate::layout::db_path(dir.path());
        assert!(db_path.exists());
        std::fs::remove_file(&db_path).unwrap();

        // Open should still work via JSON
        let result = svc.open_project(dir.path());
        assert!(
            result.is_ok(),
            "should fall back to JSON: {:?}",
            result.err()
        );
        let dto = result.unwrap();
        assert_eq!(dto.name, "json-fallback");
    }

    #[test]
    fn test_open_after_subdir_repair() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = WorkspaceService;
        svc.create_project(dir.path(), "repair-test").unwrap();

        // Remove and restore a directory
        std::fs::remove_dir(dir.path().join("masks")).unwrap();
        assert!(svc.open_project(dir.path()).is_err());

        std::fs::create_dir(dir.path().join("masks")).unwrap();
        assert!(svc.open_project(dir.path()).is_ok());
    }
}
