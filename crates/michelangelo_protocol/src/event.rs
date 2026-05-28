use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
