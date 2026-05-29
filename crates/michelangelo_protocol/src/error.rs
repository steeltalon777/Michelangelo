use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Numeric error codes for the Michelangelo Core Protocol.
///
/// Serialized as a JSON-RPC–style integer on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Request JSON could not be parsed.
    ParseError = -32700,
    /// Request envelope is structurally invalid.
    InvalidRequest = -32600,
    /// No handler for the given method.
    MethodNotFound = -32601,
    /// Method parameters are invalid.
    InvalidParams = -32602,
    /// Internal server error.
    InternalError = -32603,
    /// No project is currently open.
    WorkspaceNotOpen = -32000,
    /// Project workspace already exists at the requested path.
    AlreadyExists = -32001,
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i32(*self as i32)
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ErrorCodeVisitor;
        impl Visitor<'_> for ErrorCodeVisitor {
            type Value = ErrorCode;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON-RPC error code integer")
            }
            fn visit_i32<E: de::Error>(self, value: i32) -> Result<ErrorCode, E> {
                Ok(ErrorCode::from_code(value))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<ErrorCode, E> {
                let code = i32::try_from(value)
                    .map_err(|_| de::Error::custom(format!("error code out of range: {value}")))?;
                Ok(ErrorCode::from_code(code))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<ErrorCode, E> {
                let code = i32::try_from(value)
                    .map_err(|_| de::Error::custom(format!("error code out of range: {value}")))?;
                Ok(ErrorCode::from_code(code))
            }
        }
        deserializer.deserialize_i32(ErrorCodeVisitor)
    }
}

impl ErrorCode {
    /// Create an `ErrorCode` from a numeric code.
    ///
    /// Returns `InternalError` for unknown codes.
    pub fn from_code(code: i32) -> Self {
        match code {
            -32700 => ErrorCode::ParseError,
            -32600 => ErrorCode::InvalidRequest,
            -32601 => ErrorCode::MethodNotFound,
            -32602 => ErrorCode::InvalidParams,
            -32603 => ErrorCode::InternalError,
            -32000 => ErrorCode::WorkspaceNotOpen,
            -32001 => ErrorCode::AlreadyExists,
            _ => ErrorCode::InternalError,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            ErrorCode::ParseError => "Parse error",
            ErrorCode::InvalidRequest => "Invalid request",
            ErrorCode::MethodNotFound => "Method not found",
            ErrorCode::InvalidParams => "Invalid params",
            ErrorCode::InternalError => "Internal error",
            ErrorCode::WorkspaceNotOpen => "Workspace not open",
            ErrorCode::AlreadyExists => "Already exists",
        }
    }
}

/// Structured protocol error body returned in `ResponseEnvelope`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: ErrorCode, message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_has_message() {
        assert_eq!(ErrorCode::ParseError.message(), "Parse error");
        assert_eq!(ErrorCode::WorkspaceNotOpen.message(), "Workspace not open");
    }

    #[test]
    fn test_error_code_serializes_as_number() {
        let json = serde_json::to_string(&ErrorCode::ParseError).unwrap();
        assert_eq!(json, "-32700");

        let json = serde_json::to_string(&ErrorCode::MethodNotFound).unwrap();
        assert_eq!(json, "-32601");

        let json = serde_json::to_string(&ErrorCode::AlreadyExists).unwrap();
        assert_eq!(json, "-32001");
    }

    #[test]
    fn test_error_code_deserializes_from_number() {
        let code: ErrorCode = serde_json::from_str("-32700").unwrap();
        assert_eq!(code, ErrorCode::ParseError);

        let code: ErrorCode = serde_json::from_str("-32001").unwrap();
        assert_eq!(code, ErrorCode::AlreadyExists);
    }

    #[test]
    fn test_error_code_deserializes_unknown_as_internal() {
        let code: ErrorCode = serde_json::from_str("-99999").unwrap();
        assert_eq!(code, ErrorCode::InternalError);
    }

    #[test]
    fn test_protocol_error_serializes_with_numeric_code() {
        let err = ProtocolError::new(ErrorCode::MethodNotFound, "no handler for 'foo.bar'");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""code":-32601"#));
        assert!(json.contains("no handler"));
    }

    #[test]
    fn test_protocol_error_with_data() {
        let err = ProtocolError::with_data(
            ErrorCode::InvalidParams,
            "missing 'path'",
            serde_json::json!({"param": "path"}),
        );
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""data""#));
        assert!(json.contains(r#""param":"path""#));
        assert!(json.contains(r#""code":-32602"#));
    }

    #[test]
    fn test_protocol_error_roundtrip() {
        let original = ProtocolError::new(ErrorCode::InternalError, "something broke");
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ProtocolError = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.code, ErrorCode::InternalError);
        assert_eq!(decoded.message, "something broke");
        assert!(decoded.data.is_none());
    }

    // ── Golden example tests ──────────────────────────────────────────
    // These test exact JSON shapes so clients and future UI shells
    // can depend on the wire format.

    #[test]
    fn golden_parse_error_response() {
        let err = ProtocolError::new(
            ErrorCode::ParseError,
            "invalid JSON: expected ident at line 1",
        );
        let resp = crate::ResponseEnvelope::error(None, err);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"id":null,"error":{"code":-32700,"message":"invalid JSON: expected ident at line 1"}}"#
        );
    }

    #[test]
    fn golden_method_not_found_response() {
        let err = ProtocolError::new(ErrorCode::MethodNotFound, "unknown method: 'foo.bar'");
        let resp = crate::ResponseEnvelope::error(Some("req-1".into()), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"id":"req-1","error":{"code":-32601,"message":"unknown method: 'foo.bar'"}}"#
        );
    }

    #[test]
    fn golden_invalid_params_response() {
        let err = ProtocolError::new(ErrorCode::InvalidParams, "missing required param: 'path'");
        let resp = crate::ResponseEnvelope::error(Some("req-2".into()), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"id":"req-2","error":{"code":-32602,"message":"missing required param: 'path'"}}"#
        );
    }

    #[test]
    fn golden_success_ping_response() {
        let resp = crate::ResponseEnvelope::success("req-3", serde_json::json!({"pong": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"id":"req-3","result":{"pong":true}}"#);
    }

    #[test]
    fn golden_internal_error_response() {
        let err = ProtocolError::new(
            ErrorCode::InternalError,
            "filesystem error: permission denied",
        );
        let resp = crate::ResponseEnvelope::error(Some("req-4".into()), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"id":"req-4","error":{"code":-32603,"message":"filesystem error: permission denied"}}"#
        );
    }

    #[test]
    fn golden_workspace_not_open_response() {
        let err = ProtocolError::new(ErrorCode::WorkspaceNotOpen, "no project is currently open");
        let resp = crate::ResponseEnvelope::error(Some("req-5".into()), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"id":"req-5","error":{"code":-32000,"message":"no project is currently open"}}"#
        );
    }

    #[test]
    fn golden_already_exists_response() {
        let err = ProtocolError::new(
            ErrorCode::AlreadyExists,
            "workspace already exists: /tmp/proj",
        );
        let resp = crate::ResponseEnvelope::error(Some("req-6".into()), err);
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"id":"req-6","error":{"code":-32001,"message":"workspace already exists: /tmp/proj"}}"#
        );
    }
}
