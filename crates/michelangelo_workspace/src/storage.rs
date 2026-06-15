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
                );
                CREATE TABLE IF NOT EXISTS jobs (
                    id            TEXT PRIMARY KEY,
                    project_id    TEXT NOT NULL,
                    job_type      TEXT NOT NULL,
                    status        TEXT NOT NULL,
                    progress_pct  INTEGER,
                    message       TEXT,
                    input_json    TEXT,
                    output_json   TEXT,
                    error         TEXT,
                    logs          TEXT,
                    created_at    TEXT NOT NULL,
                    started_at    TEXT,
                    finished_at   TEXT
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

    // ------------------------------------------------------------------
    //  Job operations
    // ------------------------------------------------------------------

    /// Generate a unique job id with a `job_` prefix.
    pub fn generate_job_id() -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        format!("job_{}", now.as_nanos())
    }

    /// ISO 8601 UTC timestamp (same algorithm as `service.rs`).
    fn iso_timestamp() -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs();

        let days = total_secs / 86400;
        let time_secs = total_secs % 86400;
        let hours = time_secs / 3600;
        let minutes = (time_secs % 3600) / 60;
        let seconds = time_secs % 60;

        let mut y = 1970i64;
        let mut remaining = days as i64;
        loop {
            let days_in_year = if Self::is_leap(y) { 366 } else { 365 };
            if remaining < days_in_year {
                break;
            }
            remaining -= days_in_year;
            y += 1;
        }
        let month_days = if Self::is_leap(y) {
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

    /// Insert a new job with status `"queued"`. Returns the auto-generated job id.
    pub fn create_job(&self, project_id: &str, job_type: &str) -> Result<String, WorkspaceError> {
        let id = Self::generate_job_id();
        self.create_job_with_id(&id, project_id, job_type)?;
        Ok(id)
    }

    /// Insert a new job with an explicit job id.
    pub fn create_job_with_id(
        &self,
        id: &str,
        project_id: &str,
        job_type: &str,
    ) -> Result<(), WorkspaceError> {
        let created_at = Self::iso_timestamp();
        self.conn
            .execute(
                "INSERT INTO jobs (id, project_id, job_type, status, progress_pct, message, input_json, output_json, error, logs, created_at, started_at, finished_at)
                 VALUES (?1, ?2, ?3, 'queued', NULL, NULL, NULL, NULL, NULL, NULL, ?4, NULL, NULL)",
                rusqlite::params![id, project_id, job_type, created_at],
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite create_job failed: {e}")))
            })?;
        Ok(())
    }

    /// Update a job's status, and optionally its progress percentage and message.
    ///
    /// If the new status is `"running"` and `started_at` is still NULL, it is set
    /// to the current timestamp.
    pub fn update_job_status(
        &self,
        job_id: &str,
        status: &str,
        progress_pct: Option<u8>,
        message: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        let now = Self::iso_timestamp();
        let pct: Option<i32> = progress_pct.map(|v| v as i32);
        self.conn
            .execute(
                "UPDATE jobs SET
                    status        = ?1,
                    progress_pct  = COALESCE(?2, progress_pct),
                    message       = COALESCE(?3, message),
                    started_at    = CASE WHEN ?1 = 'running' AND started_at IS NULL THEN ?4 ELSE started_at END
                 WHERE id = ?5",
                rusqlite::params![status, pct, message, now, job_id],
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!(
                    "sqlite update_job_status failed: {e}"
                )))
            })?;
        Ok(())
    }

    /// Save output data for a finished or failed job.
    ///
    /// Each field is optional — pass `None` to leave the existing value unchanged.
    pub fn update_job_output(
        &self,
        job_id: &str,
        output_json: Option<&str>,
        error: Option<&str>,
        logs: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        self.conn
            .execute(
                "UPDATE jobs SET
                    output_json = COALESCE(?1, output_json),
                    error       = COALESCE(?2, error),
                    logs        = COALESCE(?3, logs)
                 WHERE id = ?4",
                rusqlite::params![output_json, error, logs, job_id],
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!(
                    "sqlite update_job_output failed: {e}"
                )))
            })?;
        Ok(())
    }

    /// Return all jobs for a project, ordered by `created_at DESC`.
    pub fn list_jobs(&self, project_id: &str) -> Result<Vec<serde_json::Value>, WorkspaceError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, job_type, status, progress_pct, message,
                        input_json, output_json, error, logs, created_at,
                        started_at, finished_at
                 FROM jobs
                 WHERE project_id = ?1
                 ORDER BY created_at DESC, id DESC",
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite prepare failed: {e}")))
            })?;

        let rows = stmt
            .query_map(rusqlite::params![project_id], |row| {
                Ok(serde_json::json!({
                    "id":          row.get::<_, String>(0)?,
                    "project_id":  row.get::<_, String>(1)?,
                    "job_type":    row.get::<_, String>(2)?,
                    "status":      row.get::<_, String>(3)?,
                    "progress_pct": row.get::<_, Option<i32>>(4)?,
                    "message":     row.get::<_, Option<String>>(5)?,
                    "input_json":  row.get::<_, Option<String>>(6)?,
                    "output_json": row.get::<_, Option<String>>(7)?,
                    "error":       row.get::<_, Option<String>>(8)?,
                    "logs":        row.get::<_, Option<String>>(9)?,
                    "created_at":  row.get::<_, String>(10)?,
                    "started_at":  row.get::<_, Option<String>>(11)?,
                    "finished_at": row.get::<_, Option<String>>(12)?,
                }))
            })
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite query failed: {e}")))
            })?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row.map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite row failed: {e}")))
            })?);
        }
        Ok(jobs)
    }

    /// Return a single job by its id, or `None` if it does not exist.
    pub fn get_job(&self, job_id: &str) -> Result<Option<serde_json::Value>, WorkspaceError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, job_type, status, progress_pct, message,
                        input_json, output_json, error, logs, created_at,
                        started_at, finished_at
                 FROM jobs
                 WHERE id = ?1",
            )
            .map_err(|e| {
                WorkspaceError::Io(std::io::Error::other(format!("sqlite prepare failed: {e}")))
            })?;

        let result = stmt.query_row(rusqlite::params![job_id], |row| {
            Ok(serde_json::json!({
                "id":          row.get::<_, String>(0)?,
                "project_id":  row.get::<_, String>(1)?,
                "job_type":    row.get::<_, String>(2)?,
                "status":      row.get::<_, String>(3)?,
                "progress_pct": row.get::<_, Option<i32>>(4)?,
                "message":     row.get::<_, Option<String>>(5)?,
                "input_json":  row.get::<_, Option<String>>(6)?,
                "output_json": row.get::<_, Option<String>>(7)?,
                "error":       row.get::<_, Option<String>>(8)?,
                "logs":        row.get::<_, Option<String>>(9)?,
                "created_at":  row.get::<_, String>(10)?,
                "started_at":  row.get::<_, Option<String>>(11)?,
                "finished_at": row.get::<_, Option<String>>(12)?,
            }))
        });

        match result {
            Ok(job) => Ok(Some(job)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(WorkspaceError::Io(std::io::Error::other(format!(
                "sqlite query failed: {e}"
            )))),
        }
    }

    /// Return the number of job records (used in tests).
    pub fn job_count(&self) -> Result<usize, WorkspaceError> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
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

    // ------------------------------------------------------------------
    //  Job tests
    // ------------------------------------------------------------------

    #[test]
    fn test_jobs_table_created() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();
        assert_eq!(storage.job_count().unwrap(), 0);
    }

    #[test]
    fn test_create_and_list_jobs() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let j1 = storage.create_job("proj1", "render").unwrap();
        let j2 = storage.create_job("proj1", "export").unwrap();

        let jobs = storage.list_jobs("proj1").unwrap();
        assert_eq!(jobs.len(), 2);

        let ids: Vec<&str> = jobs.iter().map(|j| j["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&j1.as_str()));
        assert!(ids.contains(&j2.as_str()));
    }

    #[test]
    fn test_get_job_by_id() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let job_id = storage.create_job("proj1", "render").unwrap();
        let job = storage.get_job(&job_id).unwrap().unwrap();

        assert_eq!(job["id"].as_str().unwrap(), job_id);
        assert_eq!(job["project_id"].as_str().unwrap(), "proj1");
        assert_eq!(job["job_type"].as_str().unwrap(), "render");
    }

    #[test]
    fn test_get_job_missing() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();
        assert!(storage.get_job("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_update_job_status() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let job_id = storage.create_job("proj1", "render").unwrap();

        // transition to running — sets started_at
        storage
            .update_job_status(&job_id, "running", None, None)
            .unwrap();
        let job = storage.get_job(&job_id).unwrap().unwrap();
        assert_eq!(job["status"].as_str().unwrap(), "running");
        assert!(!job["started_at"].as_str().unwrap_or("").is_empty());

        // second running update — started_at stays set
        storage
            .update_job_status(&job_id, "running", None, None)
            .unwrap();
        let job = storage.get_job(&job_id).unwrap().unwrap();
        assert_eq!(job["status"].as_str().unwrap(), "running");
        assert!(!job["started_at"].as_str().unwrap_or("").is_empty());
    }

    #[test]
    fn test_update_job_progress_and_message() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let job_id = storage.create_job("proj1", "render").unwrap();
        storage
            .update_job_status(&job_id, "processing", Some(50), Some("halfway"))
            .unwrap();

        let job = storage.get_job(&job_id).unwrap().unwrap();
        assert_eq!(job["status"].as_str().unwrap(), "processing");
        assert_eq!(job["progress_pct"].as_i64().unwrap(), 50);
        assert_eq!(job["message"].as_str().unwrap(), "halfway");
    }

    #[test]
    fn test_update_job_output() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let job_id = storage.create_job("proj1", "render").unwrap();
        storage
            .update_job_output(
                &job_id,
                Some(r#"{"result":"ok"}"#),
                Some("no error"),
                Some("log line 1\nlog line 2"),
            )
            .unwrap();

        let job = storage.get_job(&job_id).unwrap().unwrap();
        assert_eq!(job["output_json"].as_str().unwrap(), r#"{"result":"ok"}"#);
        assert_eq!(job["error"].as_str().unwrap(), "no error");
        assert_eq!(job["logs"].as_str().unwrap(), "log line 1\nlog line 2");
    }

    #[test]
    fn test_job_survives_storage_reopen() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("j.db");
        let job_id = {
            let storage = Storage::open(&db_path).unwrap();
            storage.create_job("proj1", "render").unwrap()
            // storage dropped here
        };

        let storage = Storage::open(&db_path).unwrap();
        let job = storage.get_job(&job_id).unwrap().unwrap();
        assert_eq!(job["project_id"].as_str().unwrap(), "proj1");
        assert_eq!(job["job_type"].as_str().unwrap(), "render");
        assert_eq!(job["status"].as_str().unwrap(), "queued");
    }

    #[test]
    fn test_list_jobs_ordered() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let j1 = storage.create_job("proj1", "render").unwrap();
        // Small delay so created_at differs (second‑precision timestamps)
        std::thread::sleep(std::time::Duration::from_secs(1));
        let j2 = storage.create_job("proj1", "export").unwrap();

        let jobs = storage.list_jobs("proj1").unwrap();
        assert_eq!(jobs.len(), 2);
        // most recent first
        assert_eq!(jobs[0]["id"].as_str().unwrap(), j2);
        assert_eq!(jobs[1]["id"].as_str().unwrap(), j1);
    }

    #[test]
    fn test_job_default_fields() {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(&dir.path().join("j.db")).unwrap();

        let job_id = storage.create_job("proj1", "render").unwrap();
        let job = storage.get_job(&job_id).unwrap().unwrap();

        assert_eq!(job["status"].as_str().unwrap(), "queued");
        assert!(!job["created_at"].as_str().unwrap_or("").is_empty());
        assert!(job["started_at"].is_null());
        assert!(job["finished_at"].is_null());
        assert!(job["progress_pct"].is_null());
        assert!(job["message"].is_null());
    }
}
