//! The reconstruction run store.
//!
//! A run lives in `<workspace>/recon/rNNNN/run-NNNN/` and owns:
//!
//! * `source/` — the solver artifacts (OBJ meshes, build_report.json),
//! * `renders/` — the silhouette renders of the source views,
//! * `evaluation_renders/` — the silhouette renders of the held-out views,
//! * `blockout.blend`, `blockout.glb` — the editable blockout,
//! * `build_report.json`, `quality_report.json`,
//!   `evaluation_quality_report.json`, `report.json`.
//!
//! The run sequence is the next free four-digit number: a rerun never
//! overwrites an existing run. The store is created lazily and is **not**
//! part of the workspace layout constants (no migration, no layout change).

use std::path::{Path, PathBuf};

use crate::error::ReconError;

/// Name of the reconstruction directory inside the workspace.
pub const RECON_DIR_NAME: &str = "recon";

/// Name of the solver source directory inside a run.
pub const SOURCE_DIR_NAME: &str = "source";

/// Name of the render directory inside a run.
pub const RENDERS_DIR_NAME: &str = "renders";

/// Name of the evaluation render directory inside a run.
pub const EVALUATION_RENDERS_DIR_NAME: &str = "evaluation_renders";

/// The directory name of a package revision inside the store.
pub fn revision_dir_name(revision: u32) -> String {
    format!("r{revision:04}")
}

/// The directory name of a run inside a revision directory.
pub fn run_dir_name(sequence: u32) -> String {
    format!("run-{sequence:04}")
}

/// An allocated run directory.
#[derive(Debug, Clone)]
pub struct RunStore {
    /// The workspace root.
    pub workspace: PathBuf,
    /// The package revision of the run.
    pub revision: u32,
    /// The sequence number of the run.
    pub sequence: u32,
    /// The absolute path of the run directory.
    pub run_dir: PathBuf,
    /// The workspace-relative path of the run directory (forward slashes).
    pub run_dir_rel: String,
}

impl RunStore {
    /// The absolute path of the solver source directory.
    pub fn source_dir(&self) -> PathBuf {
        self.run_dir.join(SOURCE_DIR_NAME)
    }

    /// The absolute path of the render directory.
    pub fn renders_dir(&self) -> PathBuf {
        self.run_dir.join(RENDERS_DIR_NAME)
    }

    /// The absolute path of the evaluation render directory.
    pub fn evaluation_renders_dir(&self) -> PathBuf {
        self.run_dir.join(EVALUATION_RENDERS_DIR_NAME)
    }

    /// The workspace-relative path of the solver source directory.
    pub fn source_dir_rel(&self) -> String {
        format!("{}/{SOURCE_DIR_NAME}", self.run_dir_rel)
    }

    /// The workspace-relative path of the render directory.
    pub fn renders_dir_rel(&self) -> String {
        format!("{}/{RENDERS_DIR_NAME}", self.run_dir_rel)
    }

    /// The workspace-relative path of the evaluation render directory.
    pub fn evaluation_renders_dir_rel(&self) -> String {
        format!("{}/{EVALUATION_RENDERS_DIR_NAME}", self.run_dir_rel)
    }

    /// The absolute path of the report of the run.
    pub fn report_path(&self) -> PathBuf {
        self.run_dir.join("report.json")
    }

    /// The absolute path of the build report of the run.
    pub fn build_report_path(&self) -> PathBuf {
        self.run_dir.join("build_report.json")
    }

    /// The absolute path of the quality report of the run.
    pub fn quality_report_path(&self) -> PathBuf {
        self.run_dir.join("quality_report.json")
    }

    /// The absolute path of the evaluation quality report of the run.
    pub fn evaluation_quality_report_path(&self) -> PathBuf {
        self.run_dir.join("evaluation_quality_report.json")
    }
}

/// Allocate the next free run directory of a package revision.
///
/// The directory (with its `source/` and `renders/` subdirectories) is
/// created immediately; an existing run is never overwritten.
pub fn allocate_run(workspace: &Path, revision: u32) -> Result<RunStore, ReconError> {
    std::fs::create_dir_all(workspace).map_err(|err| ReconError::Io {
        context: format!("the workspace cannot be created: '{}'", workspace.display()),
        message: err.to_string(),
    })?;

    let recon_dir = workspace.join(RECON_DIR_NAME);
    let revision_dir = recon_dir.join(revision_dir_name(revision));
    std::fs::create_dir_all(&revision_dir).map_err(|err| ReconError::Io {
        context: format!(
            "the revision directory cannot be created: '{}'",
            revision_dir.display()
        ),
        message: err.to_string(),
    })?;

    let mut sequence: u32 = 1;
    loop {
        if sequence > 9999 {
            return Err(ReconError::Io {
                context: "the run store is full".to_string(),
                message: "more than 9999 runs exist for this revision".to_string(),
            });
        }
        let candidate = revision_dir.join(run_dir_name(sequence));
        if !candidate.exists() {
            let run_dir_rel = format!(
                "{RECON_DIR_NAME}/{}/{}",
                revision_dir_name(revision),
                run_dir_name(sequence)
            );
            let run_dir = workspace.join(&run_dir_rel);
            std::fs::create_dir_all(run_dir.join(SOURCE_DIR_NAME)).map_err(|err| {
                ReconError::Io {
                    context: "the run directory cannot be created".to_string(),
                    message: err.to_string(),
                }
            })?;
            std::fs::create_dir_all(run_dir.join(RENDERS_DIR_NAME)).map_err(|err| {
                ReconError::Io {
                    context: "the render directory cannot be created".to_string(),
                    message: err.to_string(),
                }
            })?;
            return Ok(RunStore {
                workspace: workspace.to_path_buf(),
                revision,
                sequence,
                run_dir,
                run_dir_rel,
            });
        }
        sequence += 1;
    }
}

/// Write a JSON document atomically.
///
/// The document is written to a temporary file in the target directory and
/// then moved into place, so a reader never observes a half-written file.
pub fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<(), ReconError> {
    let text = serde_json::to_string_pretty(value).map_err(|err| ReconError::ReportWrite {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    write_text_atomic(
        path,
        &format!(
            "{text}
"
        ),
    )
}

/// Write a text document atomically.
pub fn write_text_atomic(path: &Path, text: &str) -> Result<(), ReconError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|err| ReconError::ReportWrite {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report.json".to_string());
    let temporary = parent.join(format!("{file_name}.tmp"));

    std::fs::write(&temporary, text).map_err(|err| ReconError::ReportWrite {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;

    let backup = parent.join(format!("{file_name}.bak"));
    let had_previous = path.exists();
    if had_previous {
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(path, &backup).map_err(|err| ReconError::ReportWrite {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
    }
    if let Err(err) = std::fs::rename(&temporary, path) {
        if had_previous {
            let _ = std::fs::rename(&backup, path);
        }
        let _ = std::fs::remove_file(&temporary);
        return Err(ReconError::ReportWrite {
            path: path.display().to_string(),
            message: err.to_string(),
        });
    }
    if had_previous {
        let _ = std::fs::remove_file(&backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_revision_directory_uses_the_r_prefix() {
        assert_eq!(revision_dir_name(1), "r0001");
        assert_eq!(revision_dir_name(42), "r0042");
    }

    #[test]
    fn the_run_directory_uses_the_run_prefix() {
        assert_eq!(run_dir_name(1), "run-0001");
        assert_eq!(run_dir_name(1234), "run-1234");
    }

    #[test]
    fn the_first_run_is_run_0001() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = allocate_run(directory.path(), 1).expect("the run is allocated");
        assert_eq!(store.sequence, 1);
        assert_eq!(store.run_dir_rel, "recon/r0001/run-0001");
        assert!(store.run_dir.is_dir());
        assert!(store.source_dir().is_dir());
        assert!(store.renders_dir().is_dir());
    }

    #[test]
    fn a_second_run_never_overwrites_the_first_one() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let first = allocate_run(directory.path(), 1).expect("the first run is allocated");
        std::fs::write(first.run_dir.join("marker.txt"), "first").expect("the marker is written");
        let second = allocate_run(directory.path(), 1).expect("the second run is allocated");
        assert_eq!(second.sequence, 2);
        assert_eq!(second.run_dir_rel, "recon/r0001/run-0002");
        assert!(first.run_dir.join("marker.txt").is_file());
    }

    #[test]
    fn the_revisions_use_separate_directories() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let first = allocate_run(directory.path(), 1).expect("the first run is allocated");
        let second = allocate_run(directory.path(), 2).expect("the second run is allocated");
        assert_eq!(first.run_dir_rel, "recon/r0001/run-0001");
        assert_eq!(second.run_dir_rel, "recon/r0002/run-0001");
    }

    #[test]
    fn a_json_document_is_written_atomically() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("report.json");
        let value = serde_json::json!({"schema": "test", "value": 1});
        write_json_atomic(&path, &value).expect("the report is written");
        let text = std::fs::read_to_string(&path).expect("the report is readable");
        assert!(text.contains("test"));
        let value = serde_json::json!({"schema": "test", "value": 2});
        write_json_atomic(&path, &value).expect("the report is replaced");
        let text = std::fs::read_to_string(&path).expect("the report is readable");
        assert!(text.contains("2"));
        assert!(!path.with_file_name("report.json.tmp").exists());
        assert!(!path.with_file_name("report.json.bak").exists());
    }
}
