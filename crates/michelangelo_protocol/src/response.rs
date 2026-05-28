use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProtocolError;

/// Outgoing response envelope for a command.
///
/// Exactly one of `result` or `error` is present.
/// `id` mirrors the request `id`, or is `null` if the id could not be parsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl ResponseEnvelope {
    /// Build a success response with the given id and result value.
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: Some(id.into()),
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response preserving the request id.
    pub fn error(id: Option<String>, err: ProtocolError) -> Self {
        Self {
            id,
            result: None,
            error: Some(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn test_success_response_serializes() {
        let resp = ResponseEnvelope::success("1", serde_json::json!({"pong": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""id":"1""#));
        assert!(json.contains(r#""result""#));
        assert!(!json.contains(r#""error""#));
    }

    #[test]
    fn test_error_response_serializes() {
        let err = ProtocolError::new(ErrorCode::MethodNotFound, "unknown method");
        let resp = ResponseEnvelope::error(Some("42".into()), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""id":"42""#));
        assert!(json.contains(r#""error""#));
        assert!(!json.contains(r#""result""#));
    }

    #[test]
    fn test_error_response_with_null_id() {
        let err = ProtocolError::new(ErrorCode::ParseError, "malformed json");
        let resp = ResponseEnvelope::error(None, err);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""id":null"#));
    }

    #[test]
    fn test_response_roundtrip() {
        let original = ResponseEnvelope::success("x1", serde_json::json!({"status":"ok"}));
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ResponseEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, Some("x1".into()));
        assert!(decoded.result.is_some());
        assert!(decoded.error.is_none());
    }
}
