use serde::{Deserialize, Serialize};

/// Status of a headless job.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    #[serde(rename = "queued")]
    #[default]
    Queued,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

impl JobStatus {
    pub fn message(&self) -> &'static str {
        match self {
            JobStatus::Queued => "job is queued",
            JobStatus::Running => "job is running",
            JobStatus::Completed => "job completed successfully",
            JobStatus::Failed => "job failed",
            JobStatus::Cancelled => "job was cancelled",
        }
    }

    /// Returns `true` if this is a terminal (non-transitioning) status.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
        )
    }
}

/// Artifact produced by a job (render, blend, export, log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactDto {
    /// Relative path from workspace root.
    pub path: String,
    /// Artifact type hint (e.g. "blend", "glb", "png", "log").
    #[serde(rename = "type")]
    pub artifact_type: String,
    /// File size in bytes, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

impl ArtifactDto {
    pub fn new(path: impl Into<String>, artifact_type: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            artifact_type: artifact_type.into(),
            size_bytes: None,
        }
    }
}

/// Extended job descriptor returned in snapshots, queries, and events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDto {
    pub id: String,
    pub name: String,
    /// Machine-readable job type (e.g. "blender_smoke_scene").
    #[serde(default)]
    pub job_type: String,
    pub status: JobStatus,
    /// Progress percentage 0–100 when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u8>,
    /// Human-readable status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Alias for `finished_at` — preserved for Phase 1 backward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactDto>,
}

impl JobDto {
    /// Create a minimal job descriptor (backward-compatible with Phase 1).
    pub fn new(id: impl Into<String>, name: impl Into<String>, status: JobStatus) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            job_type: String::new(),
            status,
            progress_pct: None,
            message: None,
            created_at: None,
            started_at: None,
            completed_at: None,
            error: None,
            artifacts: Vec::new(),
        }
    }

    /// Create a job with the full Phase 2 field set.
    pub fn with_job_type(
        id: impl Into<String>,
        name: impl Into<String>,
        job_type: impl Into<String>,
        status: JobStatus,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            job_type: job_type.into(),
            status,
            progress_pct: None,
            message: None,
            created_at: None,
            started_at: None,
            completed_at: None,
            error: None,
            artifacts: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_status_serializes_as_lowercase() {
        let json = serde_json::to_string(&JobStatus::Queued).unwrap();
        assert_eq!(json, r#""queued""#);

        let json = serde_json::to_string(&JobStatus::Completed).unwrap();
        assert_eq!(json, r#""completed""#);

        let json = serde_json::to_string(&JobStatus::Failed).unwrap();
        assert_eq!(json, r#""failed""#);
    }

    #[test]
    fn test_job_status_deserializes() {
        let status: JobStatus = serde_json::from_str(r#""running""#).unwrap();
        assert_eq!(status, JobStatus::Running);

        let status: JobStatus = serde_json::from_str(r#""cancelled""#).unwrap();
        assert_eq!(status, JobStatus::Cancelled);
    }

    #[test]
    fn test_job_status_has_message() {
        assert_eq!(JobStatus::Queued.message(), "job is queued");
        assert_eq!(JobStatus::Completed.message(), "job completed successfully");
    }

    #[test]
    fn test_job_status_terminal() {
        assert!(JobStatus::Completed.is_terminal());
        assert!(JobStatus::Failed.is_terminal());
        assert!(JobStatus::Cancelled.is_terminal());
        assert!(!JobStatus::Queued.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
    }

    #[test]
    fn test_job_status_default() {
        assert_eq!(JobStatus::default(), JobStatus::Queued);
    }

    #[test]
    fn test_job_dto_serializes() {
        let job = JobDto::new("j1", "render-scene", JobStatus::Queued);
        let json = serde_json::to_string(&job).unwrap();
        assert!(json.contains(r#""id":"j1""#));
        assert!(json.contains(r#""name":"render-scene""#));
        assert!(json.contains(r#""status":"queued""#));
        // job_type defaults to empty string
        assert!(json.contains(r#""job_type":"""#));
    }

    #[test]
    fn test_job_dto_roundtrip() {
        let job = JobDto::new("j2", "export-obj", JobStatus::Running);
        let json = serde_json::to_string(&job).unwrap();
        let decoded: JobDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "j2");
        assert_eq!(decoded.status, JobStatus::Running);
    }

    #[test]
    fn test_job_dto_optional_fields() {
        let job = JobDto::new("j3", "test", JobStatus::Completed);
        let json = serde_json::to_string(&job).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("created_at").is_none());
        assert!(parsed.get("completed_at").is_none());
        assert!(parsed.get("started_at").is_none());
        assert!(parsed.get("progress_pct").is_none());
        assert!(parsed.get("message").is_none());
        assert!(parsed.get("error").is_none());
    }

    #[test]
    fn test_job_dto_with_job_type() {
        let job = JobDto::with_job_type(
            "j4",
            "blender-smoke",
            "blender_smoke_scene",
            JobStatus::Queued,
        );
        let json = serde_json::to_string(&job).unwrap();
        assert!(json.contains(r#""job_type":"blender_smoke_scene""#));
    }

    #[test]
    fn test_job_dto_with_artifacts() {
        let artifact = ArtifactDto::new("blender/scenes/smoke_v001.blend", "blend");
        let mut job = JobDto::new("j5", "smoke", JobStatus::Completed);
        job.artifacts.push(artifact);
        job.progress_pct = Some(100);
        job.message = Some("All done".into());
        let json = serde_json::to_string(&job).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["progress_pct"], 100);
        assert_eq!(parsed["message"], "All done");
        assert_eq!(
            parsed["artifacts"][0]["path"],
            "blender/scenes/smoke_v001.blend"
        );
        assert_eq!(parsed["artifacts"][0]["type"], "blend");
    }

    #[test]
    fn test_artifact_dto_roundtrip() {
        let a = ArtifactDto::new("path/to/file.glb", "glb");
        let json = serde_json::to_string(&a).unwrap();
        let decoded: ArtifactDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.path, "path/to/file.glb");
        assert_eq!(decoded.artifact_type, "glb");
        assert!(decoded.size_bytes.is_none());
    }

    #[test]
    fn test_artifact_dto_with_size() {
        let mut a = ArtifactDto::new("render.png", "png");
        a.size_bytes = Some(42_000);
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains(r#""size_bytes":42000"#));
    }
}
