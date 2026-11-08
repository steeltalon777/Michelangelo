//! The tests of the MCP adapter.

#![cfg(test)]

use std::io::Cursor;

use serde_json::{json, Value};

use crate::budget::BudgetStore;
use crate::mcp::{
    McpServer, ERROR_BUDGET_EXHAUSTED, ERROR_INVALID_PARAMS, ERROR_INVALID_REQUEST,
    ERROR_METHOD_NOT_FOUND, ERROR_NOT_FOUND, ERROR_PARSE, ERROR_PATH_NOT_ALLOWED,
    ERROR_POLICY_DENIED, PROTOCOL_VERSION,
};
use crate::policy::AssistantPolicy;
use crate::testutil::Fixture;

/// A server of a fixture.
fn server(
    fixture: &Fixture,
    policy: AssistantPolicy,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    Ok(McpServer::new(
        fixture.workspace.clone(),
        policy,
        Box::new(fixture.clock()),
    )?)
}

/// A fixture with an initialised scene store.
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.init_scene(&["base"])?;
    Ok(fixture)
}

/// Send one request and return the response.
fn call(server: &mut McpServer, request: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let line = server.handle_line(&request.to_string());
    match line {
        Some(text) => Ok(serde_json::from_str(&text)?),
        None => Err("the request carries no response".into()),
    }
}

/// The error code of a response.
fn error_code(response: &Value) -> i64 {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_i64())
        .unwrap_or(0)
}

#[test]
fn the_initialize_method_returns_the_protocol_version() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
    )?;
    let result = response
        .get("result")
        .ok_or("the response carries a result")?;
    assert_eq!(
        result.get("protocolVersion"),
        Some(&Value::String(PROTOCOL_VERSION.to_string()))
    );
    assert_eq!(
        result
            .get("capabilities")
            .and_then(|value| value.get("tools")),
        Some(&json!({}))
    );
    let info = result
        .get("serverInfo")
        .ok_or("the response carries the server info")?;
    assert_eq!(
        info.get("name"),
        Some(&Value::String("michelangelo".to_string()))
    );
    assert!(info.get("version").is_some());
    Ok(())
}

#[test]
fn the_initialized_notification_is_accepted_silently() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let line = server
        .handle_line(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
    assert!(line.is_none());
    Ok(())
}

#[test]
fn the_ping_method_returns_an_empty_result() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}),
    )?;
    assert_eq!(response.get("result"), Some(&json!({})));
    Ok(())
}

#[test]
fn the_tools_list_returns_the_four_tools() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )?;
    let tools = response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(|tools| tools.as_array())
        .ok_or("the response carries the tools")?;
    assert_eq!(tools.len(), 4);
    let names: Vec<String> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|name| name.as_str()))
        .map(|name| name.to_string())
        .collect();
    assert!(names.contains(&"michelangelo_get_task".to_string()));
    assert!(names.contains(&"michelangelo_validate_package".to_string()));
    assert!(names.contains(&"michelangelo_submit_candidate".to_string()));
    assert!(names.contains(&"michelangelo_get_report".to_string()));
    Ok(())
}

#[test]
fn the_tools_list_is_filtered_by_the_policy() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let policy = AssistantPolicy {
        allowed_tools: vec!["get_task".to_string()],
        ..AssistantPolicy::default()
    };
    let mut server = server(&fixture, policy)?;
    let response = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )?;
    let tools = response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(|tools| tools.as_array())
        .ok_or("the response carries the tools")?;
    assert_eq!(tools.len(), 1);
    assert_eq!(
        tools[0].get("name"),
        Some(&Value::String("michelangelo_get_task".to_string()))
    );
    Ok(())
}

#[test]
fn an_unknown_method_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 3, "method": "fly"}),
    )?;
    assert_eq!(error_code(&response), ERROR_METHOD_NOT_FOUND);
    Ok(())
}

#[test]
fn a_malformed_line_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let line = server
        .handle_line("{ this is not json")
        .ok_or("the line has a response")?;
    let response: Value = serde_json::from_str(&line)?;
    assert_eq!(error_code(&response), ERROR_PARSE);
    Ok(())
}

#[test]
fn an_invalid_request_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(&mut server, json!({"id": 4, "method": "ping"}))?;
    assert_eq!(error_code(&response), ERROR_INVALID_REQUEST);
    Ok(())
}

#[test]
fn a_request_without_a_method_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(&mut server, json!({"jsonrpc": "2.0", "id": 4}))?;
    assert_eq!(error_code(&response), ERROR_INVALID_REQUEST);
    Ok(())
}

#[test]
fn a_tool_call_without_a_name_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {}}),
    )?;
    assert_eq!(error_code(&response), ERROR_INVALID_PARAMS);
    Ok(())
}

#[test]
fn an_unknown_tool_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "michelangelo_delete_everything", "arguments": {}}
        }),
    )?;
    assert_eq!(error_code(&response), ERROR_INVALID_PARAMS);
    Ok(())
}

#[test]
fn a_tool_call_without_its_arguments_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_task"}
        }),
    )?;
    assert_eq!(error_code(&response), ERROR_INVALID_PARAMS);
    Ok(())
}

#[test]
fn a_tool_that_the_policy_forbids_is_denied() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let policy = AssistantPolicy {
        allowed_tools: vec!["get_task".to_string()],
        ..AssistantPolicy::default()
    };
    let mut server = server(&fixture, policy)?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_report", "arguments": {"run_id": "run-0001"}}
        }),
    )?;
    assert_eq!(error_code(&response), ERROR_POLICY_DENIED);
    Ok(())
}

#[test]
fn a_missing_run_is_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_report", "arguments": {"run_id": "run-9999"}}
        }),
    )?;
    assert_eq!(error_code(&response), ERROR_NOT_FOUND);
    Ok(())
}

#[test]
fn a_path_outside_the_workspace_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_validate_package",
                "arguments": {"package": "../outside"}
            }
        }),
    )?;
    assert_eq!(error_code(&response), ERROR_PATH_NOT_ALLOWED);
    Ok(())
}

#[test]
fn an_absolute_path_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_validate_package",
                "arguments": {"package": "/etc"}
            }
        }),
    )?;
    assert_eq!(error_code(&response), ERROR_PATH_NOT_ALLOWED);
    Ok(())
}

#[test]
fn the_get_task_tool_returns_the_bounded_task() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_task", "arguments": {"run_id": "run-0001"}}
        }),
    )?;
    let content = response
        .get("result")
        .and_then(|result| result.get("structuredContent"))
        .ok_or("the response carries the content")?;
    let task = content.get("task").ok_or("the content carries the task")?;
    assert_eq!(
        task.get("schema"),
        Some(&Value::String(crate::TASK_SCHEMA.to_string()))
    );
    assert_eq!(
        content.get("path"),
        Some(&Value::String("assistant/tasks/asst-0001.json".to_string()))
    );
    Ok(())
}

#[test]
fn the_get_task_tool_returns_the_task_of_an_unknown() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    // The run carries a binding of one unknown.
    let directory = fixture.run_dir.join("verification/bindings");
    std::fs::create_dir_all(&directory)?;
    let document = json!({
        "schema": "michelangelo.unknown-bindings/0",
        "version": "b0001",
        "created_at": "2026-01-02T03:04:05Z",
        "bindings": [{
            "unknown_id": "u_lid_inside",
            "components": ["lid"],
            "region": {"min": [-0.05, 0.04, -0.05], "max": [0.05, 0.10, 0.05]},
            "by": "alice",
            "claim": "the inside of the lid is a plain box"
        }]
    });
    std::fs::write(directory.join("b0001.json"), document.to_string())?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_get_task",
                "arguments": {"run_id": "run-0001", "unknown": "u_lid_inside"}
            }
        }),
    )?;
    let content = response
        .get("result")
        .and_then(|result| result.get("structuredContent"))
        .ok_or("the response carries the content")?;
    let task = content.get("task").ok_or("the content carries the task")?;
    assert_eq!(
        task.get("unknown_id"),
        Some(&Value::String("u_lid_inside".to_string()))
    );
    assert_eq!(
        task.get("allowed_components"),
        Some(&json!(["lid"])),
        "the task is narrowed to the binding of the unknown"
    );
    let criteria = task
        .get("unknown_criteria")
        .ok_or("the task carries the criteria of the unknown")?;
    assert_eq!(
        criteria.get("unknown_id"),
        Some(&Value::String("u_lid_inside".to_string()))
    );
    Ok(())
}

#[test]
fn the_responses_carry_no_absolute_path() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let line = server
        .handle_line(
            &json!({
                "jsonrpc": "2.0",
                "id": 8,
                "method": "tools/call",
                "params": {"name": "michelangelo_get_task", "arguments": {"run_id": "run-0001"}}
            })
            .to_string(),
        )
        .ok_or("the request has a response")?;
    let rendered = fixture.workspace.display().to_string();
    assert!(!line.contains(&rendered));
    Ok(())
}

#[test]
fn a_valid_candidate_is_proposed_over_the_transport() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_task", "arguments": {"run_id": "run-0001"}}
        }),
    )?;
    fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_submit_candidate",
                "arguments": {
                    "run_id": "run-0001",
                    "task_id": "asst-0001",
                    "candidate_dir": "candidate-valid"
                }
            }
        }),
    )?;
    let content = response
        .get("result")
        .and_then(|result| result.get("structuredContent"))
        .ok_or("the response carries the content")?;
    assert_eq!(
        content.get("decision"),
        Some(&Value::String("proposed".to_string()))
    );
    assert_eq!(content.get("resulting_scene_revision"), Some(&Value::Null));
    Ok(())
}

#[test]
fn an_io_error_over_the_transport_does_not_leak_an_absolute_path(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_task", "arguments": {"run_id": "run-0001"}}
        }),
    )?;
    let candidate = fixture.write_candidate("candidate-broken", "cand-0002", "asst-0001")?;
    std::fs::remove_file(candidate.join("geometry").join("lid.obj"))?;
    let response = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_submit_candidate",
                "arguments": {
                    "run_id": "run-0001",
                    "task_id": "asst-0001",
                    "candidate_dir": "candidate-broken"
                }
            }
        }),
    )?;
    assert_eq!(error_code(&response), crate::mcp::ERROR_INTERNAL);
    let message = response
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
        .ok_or("the error carries a message")?;
    let workspace = fixture.workspace.display().to_string();
    assert!(
        !message.contains(&workspace),
        "message leaks the workspace: {message}"
    );
    assert!(
        !message.contains(":\\"),
        "message leaks a drive path: {message}"
    );
    assert!(
        !message.contains(":/"),
        "message leaks a drive path: {message}"
    );
    Ok(())
}

#[test]
fn the_budget_of_the_run_is_enforced_over_the_transport() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = fixture()?;
    let policy = AssistantPolicy {
        max_calls_per_run: 1,
        ..AssistantPolicy::default()
    };
    let mut server = server(&fixture, policy)?;
    call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_task", "arguments": {"run_id": "run-0001"}}
        }),
    )?;
    fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let first = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_submit_candidate",
                "arguments": {
                    "run_id": "run-0001",
                    "task_id": "asst-0001",
                    "candidate_dir": "candidate-valid"
                }
            }
        }),
    )?;
    assert_eq!(
        first
            .get("result")
            .and_then(|result| result.get("structuredContent"))
            .and_then(|content| content.get("decision")),
        Some(&Value::String("proposed".to_string()))
    );
    let second = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_submit_candidate",
                "arguments": {
                    "run_id": "run-0001",
                    "task_id": "asst-0001",
                    "candidate_dir": "candidate-valid"
                }
            }
        }),
    )?;
    assert_eq!(error_code(&second), ERROR_BUDGET_EXHAUSTED);
    Ok(())
}

#[test]
fn the_audit_log_has_one_line_per_call() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_report", "arguments": {"run_id": "run-0001"}}
        }),
    )?;
    let missing = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "tools/call",
            "params": {"name": "michelangelo_get_report", "arguments": {"run_id": "run-9999"}}
        }),
    )?;
    assert_eq!(error_code(&missing), ERROR_NOT_FOUND);
    // A failed call of a known run is audited as an error.
    let failed = call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "tools/call",
            "params": {
                "name": "michelangelo_validate_package",
                "arguments": {
                    "run_id": "run-0001",
                    "package": "../outside"
                }
            }
        }),
    )?;
    assert_eq!(error_code(&failed), ERROR_PATH_NOT_ALLOWED);
    let store = BudgetStore::new(&fixture.run_dir);
    let lines = store.read_audit()?;
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("get_report"));
    assert!(lines[1].contains("error"));
    Ok(())
}

#[test]
fn the_server_survives_a_malformed_line() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let malformed = server
        .handle_line("{ broken")
        .ok_or("the line has a response")?;
    let response: Value = serde_json::from_str(&malformed)?;
    assert_eq!(error_code(&response), ERROR_PARSE);
    let alive = call(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 14, "method": "ping"}),
    )?;
    assert_eq!(alive.get("result"), Some(&json!({})));
    Ok(())
}

#[test]
fn the_end_of_the_input_terminates_the_server() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let mut input = Cursor::new(b"".to_vec());
    let mut output: Vec<u8> = Vec::new();
    server.serve(&mut input, &mut output)?;
    assert!(output.is_empty());
    Ok(())
}

#[test]
fn the_server_answers_the_lines_of_the_input() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let mut server = server(&fixture, AssistantPolicy::default())?;
    let mut input = Cursor::new(
        br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}
{"jsonrpc":"2.0","id":2,"method":"ping"}
"#
        .to_vec(),
    );
    let mut output: Vec<u8> = Vec::new();
    server.serve(&mut input, &mut output)?;
    let text = String::from_utf8(output)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(PROTOCOL_VERSION));
    assert!(lines[1].contains(r#""result":{}"#));
    Ok(())
}
