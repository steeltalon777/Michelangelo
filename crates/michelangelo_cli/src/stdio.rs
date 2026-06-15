//! JSONL stdin/stdout transport loop for the Michelangelo Core Protocol.
//!
//! Reads one JSON message per line from stdin, dispatches it to the
//! core service, and writes JSON messages per line to stdout.
//! Events (from job execution) are emitted before the final response.
//! All human-readable diagnostics go to stderr.

use std::io::{self, BufRead, Write};
use std::sync::mpsc::Receiver;

use michelangelo_core::CoreService;
use michelangelo_protocol::error::{ErrorCode, ProtocolError};
use michelangelo_protocol::event::EventEnvelope;
use michelangelo_protocol::response::ResponseEnvelope;

/// Run the JSONL stdin/stdout loop until EOF on stdin.
///
/// Returns `Ok(())` after a clean EOF, or `Err` on an unrecoverable I/O error.
pub fn run_loop() -> Result<(), io::Error> {
    let (mut core, event_rx) = CoreService::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;

        if line.trim().is_empty() {
            continue;
        }

        let response = process_line(&mut core, &line);

        // Drain pending events BEFORE the response (events must precede the
        // terminal response per protocol contract).
        drain_events(&event_rx, &mut stdout_lock)?;

        let json = serde_json::to_string(&response).unwrap_or_else(|e| {
            eprintln!("[michelangelo] fatal serialization error: {e}");
            r#"{"id":null,"error":{"code":"InternalError","message":"internal serialization error"}}"#.to_string()
        });

        writeln!(stdout_lock, "{json}")?;
        stdout_lock.flush()?;
    }

    Ok(())
}

/// Drain all available events from the channel and write them as JSONL to stdout.
fn drain_events(rx: &Receiver<EventEnvelope>, writer: &mut impl Write) -> Result<(), io::Error> {
    for event in rx.try_iter() {
        let json = serde_json::to_string(&event).unwrap_or_else(|e| {
            eprintln!("[michelangelo] event serialization error: {e}");
            String::new()
        });
        if !json.is_empty() {
            writeln!(writer, "{json}")?;
        }
    }
    Ok(())
}

/// Parse a single input line and dispatch it through the core service.
fn process_line(core: &mut CoreService, line: &str) -> ResponseEnvelope {
    use serde_json::error::Category;

    let cmd = match serde_json::from_str(line) {
        Ok(cmd) => cmd,
        Err(e) => {
            let id = extract_id_from_partial(line);
            let code = match e.classify() {
                Category::Data => ErrorCode::InvalidRequest,
                _ => ErrorCode::ParseError,
            };
            return ResponseEnvelope::error(id, ProtocolError::new(code, format!("{e}")));
        }
    };

    core.handle_command(&cmd)
}

/// Attempt to extract an `id` field from a string that may not be valid JSON.
fn extract_id_from_partial(text: &str) -> Option<String> {
    // First, try structural extraction from valid JSON
    // This handles whitespace around the colon, numeric ids, etc.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        return match v.get("id") {
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
    }
    // Fallback heuristic for truly malformed JSON (e.g., truncated or syntax error)
    // Search for "id": followed by optional whitespace then a quoted string value
    if let Some(pos) = text.find(r#""id":"#) {
        let after = &text[pos + 5..];
        if let Some(quote_pos) = after.find('"') {
            let after_quote = &after[quote_pos + 1..];
            if let Some(end) = after_quote.find('"') {
                let extracted = &after_quote[..end];
                if !extracted.is_empty() {
                    return Some(extracted.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> CoreService {
        let (svc, _rx) = CoreService::new();
        svc
    }

    #[test]
    fn test_process_valid_ping() {
        let mut c = core();
        let resp = process_line(&mut c, r#"{"id":"1","method":"system.ping","params":{}}"#);
        assert_eq!(resp.id, Some("1".into()));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_process_unknown_method() {
        let mut c = core();
        let resp = process_line(&mut c, r#"{"id":"5","method":"foo.bar","params":{}}"#);
        assert_eq!(resp.id, Some("5".into()));
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(
            err.code,
            michelangelo_protocol::error::ErrorCode::MethodNotFound
        );
    }

    #[test]
    fn test_process_malformed_json() {
        let mut c = core();
        let resp = process_line(&mut c, r#"this is not json"#);
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_process_malformed_with_id_heuristic() {
        let mut c = core();
        let resp = process_line(&mut c, r#"{"id":"42","method":"system.ping"params:{}}"#);
        assert_eq!(resp.id, Some("42".into()));
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_project_create_smoke() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut c = core();
        let resp = process_line(
            &mut c,
            &format!(
                r#"{{"id":"c1","method":"project.create","params":{{"path":"{}","name":"smoke-test"}}}}"#,
                dir.path().display()
            ),
        );
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        assert!(resp.result.is_some());
        assert_eq!(resp.result.unwrap()["name"], "smoke-test");
    }

    #[test]
    fn test_project_open_smoke() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut c = core();

        // Create first
        let _ = process_line(
            &mut c,
            &format!(
                r#"{{"id":"c1","method":"project.create","params":{{"path":"{}","name":"open-test"}}}}"#,
                dir.path().display()
            ),
        );

        // Then open
        let resp = process_line(
            &mut c,
            &format!(
                r#"{{"id":"c2","method":"project.open","params":{{"path":"{}"}}}}"#,
                dir.path().display()
            ),
        );
        assert!(resp.error.is_none(), "open failed: {:?}", resp.error);
        let result = resp.result.unwrap();
        assert_eq!(result["name"], "open-test");
    }

    #[test]
    fn test_project_get_snapshot_smoke() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut c = core();

        // Create
        let _ = process_line(
            &mut c,
            &format!(
                r#"{{"id":"c1","method":"project.create","params":{{"path":"{}","name":"snap-test"}}}}"#,
                dir.path().display()
            ),
        );

        // Snapshot
        let resp = process_line(
            &mut c,
            r#"{"id":"c2","method":"project.get_snapshot","params":{}}"#,
        );
        assert!(resp.error.is_none(), "snapshot failed: {:?}", resp.error);
        let snap = resp.result.unwrap();
        assert_eq!(snap["workspace_status"], "active");
        assert_eq!(snap["project"]["name"], "snap-test");
    }

    #[test]
    fn test_project_get_snapshot_no_project_error() {
        let mut c = core();
        let resp = process_line(
            &mut c,
            r#"{"id":"e1","method":"project.get_snapshot","params":{}}"#,
        );
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(
            err.code,
            michelangelo_protocol::error::ErrorCode::WorkspaceNotOpen
        );
    }

    #[test]
    fn test_reopen_across_process_boundary() {
        // Simulate process boundary: create in one CoreService, open in another
        let dir = tempfile::TempDir::new().unwrap();

        // "Process 1": create project
        let mut c1 = core();
        let create_resp = process_line(
            &mut c1,
            &format!(
                r#"{{"id":"c1","method":"project.create","params":{{"path":"{}","name":"reopen-test"}}}}"#,
                dir.path().display()
            ),
        );
        assert!(create_resp.error.is_none());

        // "Process 2": open and get snapshot
        let mut c2 = core();
        let open_resp = process_line(
            &mut c2,
            &format!(
                r#"{{"id":"o1","method":"project.open","params":{{"path":"{}"}}}}"#,
                dir.path().display()
            ),
        );
        assert!(
            open_resp.error.is_none(),
            "reopen failed: {:?}",
            open_resp.error
        );
        assert_eq!(open_resp.result.unwrap()["name"], "reopen-test");

        let snap_resp = process_line(
            &mut c2,
            r#"{"id":"s1","method":"project.get_snapshot","params":{}}"#,
        );
        assert!(snap_resp.error.is_none());
        let snap = snap_resp.result.unwrap();
        assert_eq!(snap["project"]["name"], "reopen-test");
        assert_eq!(snap["workspace_status"], "active");
    }

    #[test]
    fn test_extract_id_whitespace_colon() {
        // Valid JSON with whitespace around colon — T-0009 acceptance
        let id = extract_id_from_partial(r#"{"id": "inv2", "params": {}}"#);
        assert_eq!(id, Some("inv2".into()));
    }

    #[test]
    fn test_extract_id_compact() {
        // Compact format — no whitespace
        let id = extract_id_from_partial(r#"{"id":"42","method":"foo"}"#);
        assert_eq!(id, Some("42".into()));
    }

    #[test]
    fn test_extract_id_numeric() {
        // Numeric id converted to string
        let id = extract_id_from_partial(r#"{"id":123,"method":"foo"}"#);
        assert_eq!(id, Some("123".into()));
    }

    #[test]
    fn test_extract_id_null() {
        // Explicit null id
        let id = extract_id_from_partial(r#"{"id":null,"method":"foo"}"#);
        assert_eq!(id, None);
    }

    #[test]
    fn test_extract_id_no_id() {
        // No id field at all
        let id = extract_id_from_partial(r#"{"method":"foo","params":{}}"#);
        assert_eq!(id, None);
    }

    #[test]
    fn test_extract_id_malformed_heuristic() {
        // Truly malformed JSON — falls back to string heuristic
        let id = extract_id_from_partial(r#"{"id":"42","method":"system.ping"params:{}}"#);
        assert_eq!(id, Some("42".into()));
    }

    #[test]
    fn test_process_invalid_request_preserves_id() {
        // Valid JSON, missing method — must preserve id with whitespace
        let mut c = core();
        let resp = process_line(&mut c, r#"{"id": "inv2", "params": {}}"#);
        assert_eq!(resp.id, Some("inv2".into()));
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn test_reopen_preserves_metadata() {
        // Verify metadata survives across CoreService instances
        let dir = tempfile::TempDir::new().unwrap();

        let mut c1 = core();
        let _ = process_line(
            &mut c1,
            &format!(
                r#"{{"id":"c1","method":"project.create","params":{{"path":"{}","name":"meta-survival"}}}}"#,
                dir.path().display()
            ),
        );

        // Second process — verify name/id match
        let mut c2 = core();
        let open_resp = process_line(
            &mut c2,
            &format!(
                r#"{{"id":"o1","method":"project.open","params":{{"path":"{}"}}}}"#,
                dir.path().display()
            ),
        );
        assert!(open_resp.error.is_none());
        let dto = open_resp.result.unwrap();
        assert_eq!(dto["name"], "meta-survival");
        assert_eq!(dto["id"], "meta-survival");
    }
}
