use serde::{Deserialize, Serialize};

/// Status of a headless job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    #[serde(rename = "queued")]
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
}

/// Minimal job descriptor returned in project snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDto {
    pub id: String,
    pub name: String,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

impl JobDto {
    pub fn new(id: impl Into<String>, name: impl Into<String>, status: JobStatus) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            status,
            created_at: None,
            completed_at: None,
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
    fn test_job_dto_serializes() {
        let job = JobDto::new("j1", "render-scene", JobStatus::Queued);
        let json = serde_json::to_string(&job).unwrap();
        assert!(json.contains(r#""id":"j1""#));
        assert!(json.contains(r#""name":"render-scene""#));
        assert!(json.contains(r#""status":"queued""#));
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
    }
}
