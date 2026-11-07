//! The integration tests of the `michelangelo assistant` subcommands.

mod common;

use std::path::Path;
use std::process::Command;

use common::Fixture;

/// Run the CLI and return the exit code and the output.
fn run(arguments: &[&str]) -> (i32, String, String) {
    let output = Command::new(common::binary())
        .args(arguments)
        .output()
        .expect("the CLI runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Initialise the scene store of the fixture.
fn init(fixture: &Fixture) {
    let (code, _, stderr) = run(&[
        "edit",
        "init",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--protect",
        "base",
    ]);
    assert_eq!(code, 0, "the scene store is initialised: {stderr}");
}

#[test]
fn the_task_command_writes_a_bounded_task() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    let (code, stdout, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
        "--json",
    ]);
    assert_eq!(code, 0, "the task is generated: {stderr}");
    let task: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(
        task.get("schema"),
        Some(&serde_json::Value::String(
            "michelangelo.assistant-task/0".to_string()
        ))
    );
    assert_eq!(
        task.get("allowed_components"),
        Some(&serde_json::json!(["lid"]))
    );
    assert_eq!(task.get("protected"), Some(&serde_json::json!(["base"])));
    assert!(!stdout.contains(&fixture.workspace.display().to_string()));
    let written = fixture
        .run_dir
        .join("assistant")
        .join("tasks")
        .join("asst-0001.json");
    assert!(written.is_file());
    Ok(())
}

#[test]
fn the_task_command_refuses_an_unknown_component() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "ghost",
    ]);
    assert_eq!(code, 2, "the component is refused: {stderr}");
    Ok(())
}

#[test]
fn the_task_command_writes_the_task_into_the_requested_file(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    let out = fixture.workspace.join("task.json");
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
        "--out",
        &out.display().to_string(),
    ]);
    assert_eq!(code, 0, "the task is written: {stderr}");
    assert!(out.is_file());
    let text = std::fs::read_to_string(&out)?;
    assert!(text.contains("michelangelo.assistant-task/0"));
    Ok(())
}

#[test]
fn the_submit_command_proposes_a_valid_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 0, "the task is generated: {stderr}");
    let task = fixture
        .run_dir
        .join("assistant")
        .join("tasks")
        .join("asst-0001.json");
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let (code, stdout, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &task.display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
        "--json",
    ]);
    assert_eq!(code, 0, "the candidate is proposed: {stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(
        report.get("decision"),
        Some(&serde_json::Value::String("proposed".to_string()))
    );
    // The scene is never modified by the assistant.
    let pointer = std::fs::read_to_string(fixture.run_dir.join("current.json"))?;
    assert!(pointer.contains("s0001"));
    // The audit log carries the call.
    let audit = std::fs::read_to_string(fixture.run_dir.join("assistant").join("audit.jsonl"))?;
    assert!(audit.contains("submit_candidate"));
    assert!(audit.contains("proposed"));
    assert!(!audit.contains(&fixture.workspace.display().to_string()));
    Ok(())
}

#[test]
fn an_accepted_candidate_publishes_a_new_scene_revision() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 0, "the task is generated: {stderr}");
    let task = fixture
        .run_dir
        .join("assistant")
        .join("tasks")
        .join("asst-0001.json");
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let (code, _, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &task.display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code, 0, "the candidate is proposed: {stderr}");
    // The scene is still the first revision: the assistant never applies.
    let pointer = std::fs::read_to_string(fixture.run_dir.join("current.json"))?;
    assert!(pointer.contains("s0001"));
    // The explicit acceptance of the author publishes the new revision.
    let (code, _, stderr) = run(&[
        "edit",
        "accept",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
    ]);
    assert_eq!(code, 0, "the candidate is accepted: {stderr}");
    let pointer = std::fs::read_to_string(fixture.run_dir.join("current.json"))?;
    assert!(pointer.contains("s0002"));
    assert!(fixture
        .run_dir
        .join("scene")
        .join("s0002")
        .join("scene.json")
        .is_file());
    Ok(())
}

#[test]
fn the_submit_command_refuses_a_candidate_beyond_the_budget(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.write_policy(r#"{"max_calls_per_run": 1}"#)?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 0, "the task is generated: {stderr}");
    let task = fixture
        .run_dir
        .join("assistant")
        .join("tasks")
        .join("asst-0001.json");
    let first = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let (code, _, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &task.display().to_string(),
        "--candidate",
        &first.display().to_string(),
    ]);
    assert_eq!(code, 0, "the first candidate is proposed: {stderr}");
    let second = fixture.write_candidate("candidate-valid-2", "cand-0002", "asst-0001")?;
    let (code, _, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &task.display().to_string(),
        "--candidate",
        &second.display().to_string(),
    ]);
    assert_eq!(code, 1, "the budget is exhausted: {stderr}");
    Ok(())
}

#[test]
fn the_submit_command_refuses_a_candidate_of_another_run() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 0, "the task is generated: {stderr}");
    let task = fixture
        .run_dir
        .join("assistant")
        .join("tasks")
        .join("asst-0001.json");
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let mut document = common::candidate_document("cand-0001", "asst-0001");
    document
        .get_mut("base")
        .and_then(|base| base.as_object_mut())
        .ok_or("the base is an object")?
        .insert(
            "run_id".to_string(),
            serde_json::Value::String("other".to_string()),
        );
    std::fs::write(
        candidate.join("candidate.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let (code, _, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &task.display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code, 2, "the candidate is refused: {stderr}");
    Ok(())
}

#[test]
fn the_submit_command_refuses_a_task_of_another_run() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 0, "the task is generated: {stderr}");
    let task = fixture
        .run_dir
        .join("assistant")
        .join("tasks")
        .join("asst-0001.json");
    let other = fixture.workspace.join("other-task.json");
    let mut document: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&task)?)?;
    document
        .get_mut("base")
        .and_then(|base| base.as_object_mut())
        .ok_or("the base is an object")?
        .insert(
            "run_id".to_string(),
            serde_json::Value::String("other".to_string()),
        );
    std::fs::write(&other, serde_json::to_string_pretty(&document)?)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let (code, _, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &other.display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code, 2, "the task is refused: {stderr}");
    Ok(())
}

#[test]
fn the_submit_command_refuses_a_missing_task() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let (code, _, stderr) = run(&[
        "assistant",
        "submit",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--task",
        &fixture.workspace.join("ghost.json").display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code, 2, "the task is missing: {stderr}");
    Ok(())
}

#[test]
fn a_broken_policy_is_a_startup_error() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.write_policy(r#"{"unknown_field": 1}"#)?;
    init(&fixture);
    let (code, _, stderr) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.run_dir.display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 2, "the policy is broken: {stderr}");
    Ok(())
}

#[test]
fn the_help_of_the_assistant_command_is_available() {
    let (code, stdout, _) = run(&["assistant", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("task"));
    assert!(stdout.contains("submit"));
}

#[test]
fn the_exit_codes_are_stable() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    init(&fixture);
    // A usage error: the run does not exist.
    let (code, _, _) = run(&[
        "assistant",
        "task",
        "--run",
        &fixture.workspace.join("ghost").display().to_string(),
        "--allowed",
        "lid",
    ]);
    assert_eq!(code, 2);
    Ok(())
}

/// The path of the CLI, used by the tests of the transport.
pub fn cli_path() -> &'static Path {
    Path::new(common::binary())
}
