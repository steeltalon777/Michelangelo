//! The tests of the workspace policy.

#![cfg(test)]

use std::path::PathBuf;

use crate::policy::{
    find_policy_file, load_policy, policy_path, AssistantPolicy, ALL_TOOLS,
    DEFAULT_MAX_CALLS_PER_RUN, DEFAULT_MAX_CANDIDATE_BYTES,
};
use crate::testutil::Fixture;
use crate::AssistantError;

#[test]
fn the_default_policy_denies_the_application_and_allows_the_submission() {
    let policy = AssistantPolicy::default();
    assert!(!policy.allow_apply);
    assert!(policy.allow_submit);
    assert!(!policy.allow_external_service);
}

#[test]
fn the_default_policy_allows_the_four_tools() {
    let policy = AssistantPolicy::default();
    assert_eq!(policy.allowed_tools.len(), ALL_TOOLS.len());
    for tool in ALL_TOOLS {
        assert!(policy.allows_tool(tool));
    }
    assert_eq!(policy.max_calls_per_run, DEFAULT_MAX_CALLS_PER_RUN);
    assert_eq!(policy.max_candidate_bytes, DEFAULT_MAX_CANDIDATE_BYTES);
}

#[test]
fn an_unknown_field_is_rejected() {
    let text = r#"{"allow_apply": false, "unknown": 1}"#;
    let error = AssistantPolicy::parse(text).expect_err("the unknown field is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn an_unknown_tool_is_rejected() {
    let text = r#"{"allowed_tools": ["get_task", "delete_everything"]}"#;
    let error = AssistantPolicy::parse(text).expect_err("the unknown tool is rejected");
    match error {
        AssistantError::Invalid { message } => assert!(message.contains("delete_everything")),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn a_duplicated_tool_is_rejected() {
    let text = r#"{"allowed_tools": ["get_task", "get_task"]}"#;
    let error = AssistantPolicy::parse(text).expect_err("the duplicated tool is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn an_empty_tool_set_is_rejected() {
    let text = r#"{"allowed_tools": []}"#;
    let error = AssistantPolicy::parse(text).expect_err("the empty tool set is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn a_budget_without_a_call_is_rejected() {
    let text = r#"{"max_calls_per_run": 0}"#;
    let error = AssistantPolicy::parse(text).expect_err("the budget is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn a_candidate_below_one_kibibyte_is_rejected() {
    let text = r#"{"max_candidate_bytes": 1023}"#;
    let error = AssistantPolicy::parse(text).expect_err("the size limit is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn a_candidate_above_one_gibibyte_is_rejected() {
    let text = r#"{"max_candidate_bytes": 1073741825}"#;
    let error = AssistantPolicy::parse(text).expect_err("the size limit is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn a_policy_that_allows_an_application_is_rejected() {
    let text = r#"{"allow_apply": true}"#;
    let error = AssistantPolicy::parse(text).expect_err("the application is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn a_policy_with_another_schema_is_rejected() {
    let text = r#"{"schema": "other/0"}"#;
    let error = AssistantPolicy::parse(text).expect_err("the schema is rejected");
    assert!(matches!(error, AssistantError::Invalid { .. }));
}

#[test]
fn a_valid_policy_is_parsed() {
    let text = r#"{
        "allow_apply": false,
        "allow_submit": true,
        "allowed_tools": ["get_task", "get_report"],
        "max_calls_per_run": 2,
        "max_candidate_bytes": 2048,
        "allow_external_service": false
    }"#;
    let policy = AssistantPolicy::parse(text).expect("the policy is usable");
    assert_eq!(policy.allowed_tools.len(), 2);
    assert_eq!(policy.max_calls_per_run, 2);
    assert_eq!(policy.max_candidate_bytes, 2048);
    assert!(policy.allows_tool("get_task"));
    assert!(!policy.allows_tool("submit_candidate"));
}

#[test]
fn an_absent_policy_file_yields_the_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let policy = load_policy(&fixture.workspace)?;
    assert_eq!(policy, AssistantPolicy::default());
    Ok(())
}

#[test]
fn the_policy_file_is_found_by_walking_up() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let directory = fixture.workspace.join(".michelangelo");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        policy_path(&fixture.workspace),
        r#"{"max_calls_per_run": 1}"#,
    )?;
    let found = find_policy_file(&fixture.run_dir);
    assert!(found.is_some());
    let policy = load_policy(&fixture.run_dir)?;
    assert_eq!(policy.max_calls_per_run, 1);
    Ok(())
}

#[test]
fn a_broken_policy_file_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let directory = fixture.workspace.join(".michelangelo");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(policy_path(&fixture.workspace), "{ this is not json")?;
    let error = load_policy(&fixture.run_dir).expect_err("the policy is broken");
    assert!(matches!(error, AssistantError::Invalid { .. }));
    Ok(())
}

#[test]
fn the_stricter_of_the_policy_and_the_task_budget_wins() {
    let policy = AssistantPolicy {
        max_calls_per_run: 2,
        max_candidate_bytes: 4096,
        ..AssistantPolicy::default()
    };
    assert_eq!(policy.effective_max_calls(5), 2);
    assert_eq!(policy.effective_max_calls(1), 1);
    assert_eq!(policy.effective_max_candidate_bytes(8192), 4096);
    assert_eq!(policy.effective_max_candidate_bytes(1024), 1024);
}

#[test]
fn the_policy_path_is_inside_the_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let path: PathBuf = policy_path(&fixture.workspace);
    let rendered = path.to_string_lossy().replace('\\', "/");
    assert!(rendered.ends_with(".michelangelo/assistant_policy.json"));
    Ok(())
}
