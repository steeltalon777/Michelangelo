//! JSONL stdin/stdout transport loop for the Michelangelo Core Protocol.
//!
//! Reads one JSON message per line from stdin, dispatches it to the
//! appropriate handler, and writes one JSON response per line to stdout.
//! All human-readable diagnostics go to stderr.

use std::io::{self, BufRead, Write};

use michelangelo_protocol::{
    command::CommandEnvelope,
    error::{ErrorCode, ProtocolError},
    method,
    response::ResponseEnvelope,
};

/// Run the JSONL stdin/stdout loop until EOF on stdin.
///
/// Returns `Ok(())` after a clean EOF, or `Err` on an unrecoverable I/O error.
pub fn run_loop() -> Result<(), io::Error> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;

        if line.trim().is_empty() {
            // Skip blank lines silently.
            continue;
        }

        let response = process_line(&line);
        let json = serde_json::to_string(&response).unwrap_or_else(|e| {
            // This should never happen — all our types are serializable.
            // If it does, write a last-resort error to stderr and return a
            // minimal JSON error to stdout.
            eprintln!("[michelangelo] fatal serialization error: {e}");
            r#"{"id":null,"error":{"code":"InternalError","message":"internal serialization error"}}"#.to_string()
        });

        writeln!(stdout_lock, "{json}")?;
        stdout_lock.flush()?;
    }

    Ok(())
}

/// Parse a single input line and produce a JSONL response.
fn process_line(line: &str) -> ResponseEnvelope {
    let cmd: CommandEnvelope = match serde_json::from_str(line) {
        Ok(cmd) => cmd,
        Err(e) => {
            // Try to extract an id even from malformed JSON
            let id = extract_id_from_partial(line);
            return ResponseEnvelope::error(
                id,
                ProtocolError::new(ErrorCode::ParseError, format!("invalid JSON: {e}")),
            );
        }
    };

    match cmd.method.as_str() {
        method::SYSTEM_PING => handle_ping(&cmd),
        _ => ResponseEnvelope::error(
            Some(cmd.id),
            ProtocolError::new(
                ErrorCode::MethodNotFound,
                format!("unknown method: '{}'", cmd.method),
            ),
        ),
    }
}

/// Handle `system.ping` — return a pong with version info.
fn handle_ping(cmd: &CommandEnvelope) -> ResponseEnvelope {
    ResponseEnvelope::success(
        &cmd.id,
        serde_json::json!({
            "pong": true,
            "protocol_version": michelangelo_protocol::PROTOCOL_VERSION,
        }),
    )
}

/// Attempt to extract an `id` field from a string that may not be valid JSON.
/// Returns `None` if no id can be extracted.
fn extract_id_from_partial(text: &str) -> Option<String> {
    // Quick heuristic: look for `"id":"..."` or `"id":"..."` patterns
    if let Some(start) = text.find(r#""id":""#) {
        let value_start = start + 6; // skip `"id":`
        if let Some(value_end) = text[value_start..].find('"') {
            let extracted = &text[value_start..value_start + value_end];
            if !extracted.is_empty() {
                return Some(extracted.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_valid_ping() {
        let resp = process_line(r#"{"id":"1","method":"system.ping","params":{}}"#);
        assert_eq!(resp.id, Some("1".into()));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        // Verify the pong field
        let result = resp.result.unwrap();
        assert_eq!(result["pong"], true);
    }

    #[test]
    fn test_process_unknown_method() {
        let resp = process_line(r#"{"id":"5","method":"foo.bar","params":{}}"#);
        assert_eq!(resp.id, Some("5".into()));
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::MethodNotFound);
    }

    #[test]
    fn test_process_malformed_json() {
        let resp = process_line(r#"this is not json"#);
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::ParseError);
    }

    #[test]
    fn test_process_malformed_with_id_heuristic() {
        let resp = process_line(r#"{"id":"42","method":"system.ping"params:{}}"#);
        // Malformed but id should be extractable
        assert_eq!(resp.id, Some("42".into()));
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_process_empty_line_becomes_parse_error() {
        // process_line is only called for non-empty trimmed lines by run_loop
        let resp = process_line("");
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_extract_id_from_partial() {
        assert_eq!(
            extract_id_from_partial(r#"{"id":"abc"}"#),
            Some("abc".into())
        );
        assert_eq!(
            extract_id_from_partial(r#"{"id":"123","method":"x"}"#),
            Some("123".into())
        );
        assert_eq!(extract_id_from_partial(r#"no id here"#), None);
        assert_eq!(extract_id_from_partial(r#"{"id":null}"#), None); // not a string id
    }

    #[test]
    fn test_response_is_one_line() {
        let resp = process_line(r#"{"id":"1","method":"system.ping","params":{}}"#);
        let json = serde_json::to_string(&resp).unwrap();
        // Ensure no newlines in the serialized output
        assert!(
            !json.contains('\n'),
            "response must be a single line: {json}"
        );
    }
}
