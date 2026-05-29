use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `job.queued` — a job has been added to the queue.
pub const JOB_QUEUED: &str = "job.queued";

/// `job.started` — a job has begun execution.
pub const JOB_STARTED: &str = "job.started";

/// `job.progress` — progress update for a running job.
pub const JOB_PROGRESS: &str = "job.progress";

/// `job.completed` — a job completed successfully.
pub const JOB_COMPLETED: &str = "job.completed";

/// `job.failed` — a job terminated with an error.
pub const JOB_FAILED: &str = "job.failed";

/// `job.cancelled` — a job was cancelled before completion.
pub const JOB_CANCELLED: &str = "job.cancelled";

/// All known Phase 2 job event names.
pub const JOB_EVENTS: &[&str] = &[
    JOB_QUEUED,
    JOB_STARTED,
    JOB_PROGRESS,
    JOB_COMPLETED,
    JOB_FAILED,
    JOB_CANCELLED,
];

/// Server-sent event envelope.
///
/// Events do not carry an `id`; they are fire-and-forget notifications
/// for UI shells and subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    pub data: Value,
}

impl EventEnvelope {
    pub fn new(event: impl Into<String>, data: Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_serializes() {
        let ev = EventEnvelope::new("project.opened", serde_json::json!({"project_id": "demo"}));
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""event":"project.opened""#));
        assert!(json.contains(r#""data""#));
    }

    #[test]
    fn test_event_has_no_id_field() {
        let ev = EventEnvelope::new("test", serde_json::json!({}));
        let json = serde_json::to_string(&ev).unwrap();
        // no "id" top-level key
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("id").is_none());
    }

    #[test]
    fn test_event_roundtrip() {
        let original = EventEnvelope::new("job.progress", serde_json::json!({"pct": 50}));
        let json = serde_json::to_string(&original).unwrap();
        let decoded: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.event, "job.progress");
        assert_eq!(decoded.data["pct"], 50);
    }

    #[test]
    fn test_job_event_constants_are_non_empty() {
        for ev in JOB_EVENTS {
            assert!(!ev.is_empty(), "empty event constant");
        }
    }

    #[test]
    fn test_job_event_constants_use_dot_notation() {
        for ev in JOB_EVENTS {
            assert!(ev.contains('.'), "event '{}' does not use dot notation", ev);
        }
    }

    #[test]
    fn test_job_event_constants_prefix() {
        for ev in JOB_EVENTS {
            assert!(
                ev.starts_with("job."),
                "event '{}' does not start with 'job.'",
                ev
            );
        }
    }
}
