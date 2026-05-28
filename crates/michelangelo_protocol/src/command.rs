use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Incoming command envelope sent by a client or shell.
///
/// Fields follow a JSON-RPC–style shape:
/// - `id` — client-chosen identifier echoed in the response.
/// - `method` — dot-separated method name (e.g. `"system.ping"`).
/// - `params` — method-specific parameters as a JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub id: String,
    pub method: String,
    pub params: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::method;

    #[test]
    fn test_command_ping_deserializes() {
        let json = r#"{"id":"1","method":"system.ping","params":{}}"#;
        let cmd: CommandEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id, "1");
        assert_eq!(cmd.method, method::SYSTEM_PING);
    }

    #[test]
    fn test_command_create_deserializes() {
        let json =
            r#"{"id":"2","method":"project.create","params":{"path":"/tmp/proj","name":"test"}}"#;
        let cmd: CommandEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id, "2");
        assert_eq!(cmd.method, method::PROJECT_CREATE);
        assert_eq!(cmd.params["path"], "/tmp/proj");
    }

    #[test]
    fn test_command_unknown_method_roundtrip() {
        let json = r#"{"id":"99","method":"unknown.method","params":{"a":1}}"#;
        let cmd: CommandEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.method, "unknown.method");
        // re-serialize
        let out = serde_json::to_string(&cmd).unwrap();
        assert!(out.contains("unknown.method"));
    }

    #[test]
    fn test_command_rejects_invalid_json() {
        let json = r#"{"id":"1"}"#; // missing method and params
        assert!(serde_json::from_str::<CommandEnvelope>(json).is_err());
    }
}
