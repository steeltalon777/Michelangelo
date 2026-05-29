//! SQLite-backed project metadata storage.
//!
//! Provides deterministic create/read operations for project records
//! stored in a local SQLite database under the workspace metadata directory.

use std::path::Path;

use rusqlite::Connection;

use crate::error::WorkspaceError;
use crate::service::ProjectMeta;

/// SQLite storage handle for a single workspace.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open (or create and initialize) the metadata database at `db_path`.
    pub fn open(db_path: &Path) -> Result<Self, WorkspaceError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path).map_err(|e| {
            WorkspaceError::Io(std::io::Error::other(format!(
                "sqlite open failed for {}: {e}",
                db_path.display()
            )))
        })?;

        let storage = Self { conn };
        storage.init_tables()?;
        Ok(storage)
    }

    fn init_tables(&self) -> Result<(), WorkspaceError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS projects (
                    id         TEXT PRIMARY KEY,
                    name       TEXT NOT NULL,
                    root_path  TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );",
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite init failed: {e}")))
            })?;
        Ok(())
    }

    /// Insert or replace a project record.
    pub fn save_project(&self, meta: &ProjectMeta) -> Result<(), WorkspaceError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO projects (id, name, root_path, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![meta.id, meta.name, meta.root_path, meta.created_at],
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!(
                    "sqlite save failed: {e}"
                )))
            })?;
        Ok(())
    }

    /// Load a project record by id.
    pub fn load_project(&self, id: &str) -> Result<Option<ProjectMeta>, WorkspaceError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, root_path, created_at FROM projects WHERE id = ?1")
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite prepare failed: {e}")))
            })?;

        let result = stmt.query_row(rusqlite::params![id], |row| {
            Ok(ProjectMeta::from_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        });

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(WorkspaceError::Io(std::io::Error::other(format!(
                "sqlite query failed: {e}"
            )))),
        }
    }

    /// Load a project record by its root_path.
    pub fn load_project_by_root(
        &self,
        root_path: &str,
    ) -> Result<Option<ProjectMeta>, WorkspaceError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, root_path, created_at FROM projects WHERE root_path = ?1")
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite prepare failed: {e}")))
            })?;

        let result = stmt.query_row(rusqlite::params![root_path], |row| {
            Ok(ProjectMeta::from_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        });

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(WorkspaceError::Io(std::io::Error::other(format!(
                "sqlite query failed: {e}"
            )))),
        }
    }

    /// Return the number of project records (used in tests).
    pub fn project_count(&self) -> Result<usize, WorkspaceError> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite count failed: {e}")))
            })?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_storage_create_and_open() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("metadata.db");
        let storage = Storage::open(&db_path).unwrap();
        assert_eq!(storage.project_count().unwrap(), 0);
    }

    #[test]
    fn test_storage_save_and_load() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("test.db")).unwrap();

        let meta = ProjectMeta::from_row(
            "p1".into(),
            "test-project".into(),
            dir.path().display().to_string(),
            "2026-05-29T12:00:00Z".into(),
        );
        storage.save_project(&meta).unwrap();

        let loaded = storage.load_project("p1").unwrap().unwrap();
        assert_eq!(loaded.id, "p1");
        assert_eq!(loaded.name, "test-project");
        assert_eq!(loaded.root_path, dir.path().display().to_string());
        assert_eq!(loaded.created_at, "2026-05-29T12:00:00Z");
    }

    #[test]
    fn test_storage_load_missing() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("missing.db")).unwrap();
        assert!(storage.load_project("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_storage_replace() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("r.db")).unwrap();

        let m1 =
            ProjectMeta::from_row("p1".into(), "v1".into(), "/p1".into(), "2026-01-01Z".into());
        storage.save_project(&m1).unwrap();
        assert_eq!(storage.project_count().unwrap(), 1);

        let m2 =
            ProjectMeta::from_row("p1".into(), "v2".into(), "/p1".into(), "2026-06-01Z".into());
        storage.save_project(&m2).unwrap();
        assert_eq!(storage.project_count().unwrap(), 1);

        let loaded = storage.load_project("p1").unwrap().unwrap();
        assert_eq!(loaded.name, "v2");
        assert_eq!(loaded.created_at, "2026-06-01Z");
    }

    #[test]
    fn test_storage_load_by_root() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("br.db")).unwrap();

        let meta = ProjectMeta::from_row(
            "p1".into(),
            "root-test".into(),
            dir.path().display().to_string(),
            "2026-05-29T12:00:00Z".into(),
        );
        storage.save_project(&meta).unwrap();

        let loaded = storage
            .load_project_by_root(&dir.path().display().to_string())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.name, "root-test");
    }

    #[test]
    fn test_storage_load_by_root_missing() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("mbr.db")).unwrap();

        let result = storage.load_project_by_root("/nonexistent/path").unwrap();
        assert!(result.is_none());
    }
}
