use serde::{Deserialize, Serialize};

/// Numeric error codes for the Michelangelo Core Protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl ErrorCode {
    pub fn message(&self) -> &'static str {
        match self {
            ErrorCode::ParseError => "Parse error",
            ErrorCode::InvalidRequest => "Invalid request",
            ErrorCode::MethodNotFound => "Method not found",
            ErrorCode::InvalidParams => "Invalid params",
            ErrorCode::InternalError => "Internal error",
            ErrorCode::WorkspaceNotOpen => "Workspace not open",
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
    fn test_protocol_error_serializes() {
        let err = ProtocolError::new(ErrorCode::MethodNotFound, "no handler for 'foo.bar'");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""code":"MethodNotFound""#));
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
}
