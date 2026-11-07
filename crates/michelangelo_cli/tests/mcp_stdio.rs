//! The integration test of the MCP stdio transport.
//!
//! The test spawns the real binary, writes the requests of a client into its
//! stdin and reads the JSON-RPC responses from its stdout.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use common::Fixture;

/// The requests of the client.
fn requests() -> Vec<String> {
    vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.to_string(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"michelangelo_get_task","arguments":{"run_id":"run-0001"}}}"#
            .to_string(),
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"michelangelo_submit_candidate","arguments":{"run_id":"run-0001","task_id":"asst-0001","candidate_dir":"candidate-valid"}}}"#
            .to_string(),
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"michelangelo_submit_candidate","arguments":{"run_id":"run-0001","task_id":"asst-0001","candidate_dir":"candidate-valid"}}}"#
            .to_string(),
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"michelangelo_submit_candidate","arguments":{"run_id":"run-0001","task_id":"asst-0001","candidate_dir":"../outside"}}}"#
            .to_string(),
        "{ this is not json".to_string(),
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"michelangelo_get_report","arguments":{"run_id":"run-0001"}}}"#
            .to_string(),
        r#"{"jsonrpc":"2.0","id":8,"method":"unknown/method"}"#.to_string(),
    ]
}

/// The error code of a response.
fn error_code(response: &serde_json::Value) -> i64 {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_i64())
        .unwrap_or(0)
}

/// The result of a response.
fn result(response: &serde_json::Value) -> &serde_json::Value {
    response.get("result").unwrap_or(response)
}

/// Run the server over the stdio transport and return the responses.
fn exchange(fixture: &Fixture) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let mut child = Command::new(common::binary())
        .arg("mcp")
        .arg("serve")
        .arg("--workspace")
        .arg(&fixture.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let mut stdin = child.stdin.take().ok_or("the child has no stdin")?;
        for request in requests() {
            writeln!(stdin, "{request}")?;
        }
        // The end of the input terminates the server.
        drop(stdin);
    }
    let stdout = child.stdout.take().ok_or("the child has no stdout")?;
    let reader = BufReader::new(stdout);
    let mut responses: Vec<serde_json::Value> = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        responses.push(serde_json::from_str(&line)?);
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("the server exited with {status}").into());
    }
    Ok(responses)
}

/// Build the fixture of the transport.
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.write_policy(r#"{"max_calls_per_run": 1}"#)?;
    let output = Command::new(common::binary())
        .arg("edit")
        .arg("init")
        .arg("--run")
        .arg(&fixture.run_dir)
        .arg("--protect")
        .arg("base")
        .output()?;
    if !output.status.success() {
        return Err("the scene store is not initialised".into());
    }
    fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    Ok(fixture)
}

#[test]
fn the_transport_answers_the_handshake_the_tools_and_the_calls(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let responses = exchange(&fixture)?;
    // The notification carries no response.
    assert_eq!(responses.len(), 9);

    // 1. initialize.
    let initialize = &responses[0];
    assert_eq!(
        result(initialize)
            .get("protocolVersion")
            .and_then(|value| value.as_str()),
        Some("2025-06-18")
    );
    assert_eq!(
        result(initialize)
            .get("serverInfo")
            .and_then(|info| info.get("name"))
            .and_then(|value| value.as_str()),
        Some("michelangelo")
    );

    // 2. tools/list.
    let tools = result(&responses[1])
        .get("tools")
        .and_then(|tools| tools.as_array())
        .ok_or("the response carries the tools")?;
    assert_eq!(tools.len(), 4);

    // 3. get_task.
    let task = result(&responses[2])
        .get("structuredContent")
        .and_then(|content| content.get("task"))
        .ok_or("the response carries the task")?;
    assert_eq!(
        task.get("schema").and_then(|value| value.as_str()),
        Some("michelangelo.assistant-task/0")
    );

    // 4. submit_candidate: the decision is proposed.
    let decision = result(&responses[3])
        .get("structuredContent")
        .and_then(|content| content.get("decision"))
        .and_then(|value| value.as_str());
    assert_eq!(decision, Some("proposed"));

    // 5. the second submission is refused by the budget.
    assert_eq!(error_code(&responses[4]), -32003);

    // 6. a path outside the workspace is refused.
    assert_eq!(error_code(&responses[5]), -32002);

    // 7. a malformed line does not kill the server.
    assert_eq!(error_code(&responses[6]), -32700);

    // 8. the server is still alive.
    let report = result(&responses[7])
        .get("structuredContent")
        .ok_or("the response carries the summary")?;
    assert_eq!(
        report.get("package_id").and_then(|value| value.as_str()),
        Some("demo-lantern")
    );

    // 9. an unknown method.
    assert_eq!(error_code(&responses[8]), -32601);

    Ok(())
}

#[test]
fn the_responses_carry_no_absolute_path() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let responses = exchange(&fixture)?;
    let rendered = fixture.workspace.display().to_string();
    for response in &responses {
        assert!(!response.to_string().contains(&rendered));
    }
    Ok(())
}

#[test]
fn the_audit_log_carries_one_line_per_call() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let _ = exchange(&fixture)?;
    let audit = std::fs::read_to_string(fixture.run_dir.join("assistant").join("audit.jsonl"))?;
    let lines: Vec<&str> = audit
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert!(lines.len() >= 4);
    for line in &lines {
        let value: serde_json::Value = serde_json::from_str(line)?;
        assert!(value.get("ts").is_some());
        assert!(value.get("tool").is_some());
        assert!(value.get("decision").is_some());
    }
    Ok(())
}

#[test]
fn the_server_exits_with_zero_at_the_end_of_the_input() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut child = Command::new(common::binary())
        .arg("mcp")
        .arg("serve")
        .arg("--workspace")
        .arg(&fixture.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    drop(child.stdin.take());
    let status = child.wait()?;
    assert_eq!(status.code(), Some(0));
    Ok(())
}
