//! JSONL stdin/stdout transport loop for the Michelangelo Core Protocol.
//!
//! Reads one JSON message per line from stdin, dispatches it to the
//! core service, and writes one JSON response per line to stdout.
//! All human-readable diagnostics go to stderr.

use std::io::{self, BufRead, Write};

use michelangelo_core::CoreService;
use michelangelo_protocol::response::ResponseEnvelope;

/// Run the JSONL stdin/stdout loop until EOF on stdin.
///
/// Returns `Ok(())` after a clean EOF, or `Err` on an unrecoverable I/O error.
pub fn run_loop() -> Result<(), io::Error> {
    let mut core = CoreService::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;

        if line.trim().is_empty() {
            continue;
        }

        let response = process_line(&mut core, &line);
        let json = serde_json::to_string(&response).unwrap_or_else(|e| {
            eprintln!("[michelangelo] fatal serialization error: {e}");
            r#"{"id":null,"error":{"code":"InternalError","message":"internal serialization error"}}"#.to_string()
        });

        writeln!(stdout_lock, "{json}")?;
        stdout_lock.flush()?;
    }

    Ok(())
}

/// Parse a single input line and dispatch it through the core service.
fn process_line(core: &mut CoreService, line: &str) -> ResponseEnvelope {
    let cmd = match serde_json::from_str(line) {
        Ok(cmd) => cmd,
        Err(e) => {
            let id = extract_id_from_partial(line);
            return ResponseEnvelope::error(
                id,
                michelangelo_protocol::error::ProtocolError::new(
                    michelangelo_protocol::error::ErrorCode::ParseError,
                    format!("invalid JSON: {e}"),
                ),
            );
        }
    };

    core.handle_command(&cmd)
}

/// Attempt to extract an `id` field from a string that may not be valid JSON.
fn extract_id_from_partial(text: &str) -> Option<String> {
    if let Some(start) = text.find(r#""id":""#) {
        let value_start = start + 6;
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

    fn core() -> CoreService {
        CoreService::new()
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
}
