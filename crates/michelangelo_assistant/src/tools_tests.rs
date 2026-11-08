//! The tests of the four tools of the assistant surface.

#![cfg(test)]

use crate::budget::BudgetStore;
use crate::policy::AssistantPolicy;
use crate::task::{generate_task, TaskRequest};
use crate::testutil::{candidate_document, Fixture};
use crate::tools::{self, ToolContext, ToolOutcome};
use crate::AssistantError;

/// A fixture with an initialised scene store and a policy.
fn fixture() -> Result<(Fixture, AssistantPolicy), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.init_scene(&["base"])?;
    let policy = AssistantPolicy::default();
    Ok((fixture, policy))
}

/// The context of a tool call.
fn context<'a>(fixture: &'a Fixture, policy: &'a AssistantPolicy) -> ToolContext<'a> {
    ToolContext::new(fixture.workspace.clone(), policy, &fixture.clock)
}

/// The task of the fixture.
fn task_of(
    fixture: &Fixture,
    policy: &AssistantPolicy,
) -> Result<crate::AssistantTask, AssistantError> {
    generate_task(
        &fixture.run_dir,
        "asst-0001",
        &TaskRequest::new(),
        policy,
        &fixture.clock,
    )
}

#[test]
fn the_get_task_tool_writes_the_task_under_the_run() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let outcome = tools::get_task(&ctx, &fixture.run_dir, None)?;
    match outcome {
        ToolOutcome::Task { task, path } => {
            assert_eq!(task.task_id, "asst-0001");
            assert_eq!(path, "assistant/tasks/asst-0001.json");
            assert!(fixture
                .run_dir
                .join("assistant")
                .join("tasks")
                .join("asst-0001.json")
                .is_file());
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    Ok(())
}

#[test]
fn the_get_task_tool_refuses_a_run_without_a_scene() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let policy = AssistantPolicy::default();
    let ctx = context(&fixture, &policy);
    let error =
        tools::get_task(&ctx, &fixture.run_dir, None).expect_err("the run has no scene store");
    assert!(matches!(error, AssistantError::NotFound { .. }));
    Ok(())
}

#[test]
fn the_validate_package_tool_returns_a_summary() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let outcome = tools::validate_package(&ctx, &fixture.run_dir.join("package"))?;
    match outcome {
        ToolOutcome::Package(summary) => {
            assert_eq!(summary.schema, "michelangelo.input-package-report/0");
            assert_eq!(summary.package_id, "demo-lantern");
            assert_eq!(summary.revision, 1);
            assert!(summary.resources >= 1);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    Ok(())
}

#[test]
fn the_validate_package_tool_refuses_a_missing_package() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let error = tools::validate_package(&ctx, &fixture.workspace.join("ghost"))
        .expect_err("the package is missing");
    assert!(matches!(error, AssistantError::NotFound { .. }));
    Ok(())
}

#[test]
fn the_get_report_tool_returns_a_summary() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let outcome = tools::get_report(&ctx, &fixture.run_dir)?;
    match outcome {
        ToolOutcome::Report(summary) => {
            assert_eq!(summary.schema, "michelangelo.reconstruction-report/0");
            assert_eq!(summary.package_id, "demo-lantern");
            assert_eq!(summary.status, "pass");
            assert_eq!(summary.thresholds.source_min_iou, 0.85);
            assert_eq!(summary.unresolved.len(), 1);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    Ok(())
}

#[test]
fn a_valid_candidate_is_proposed() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let outcome = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)?;
    match outcome {
        ToolOutcome::Decision(report) => {
            assert_eq!(
                report.decision,
                michelangelo_edit::validate::Decision::Proposed
            );
            assert_eq!(report.candidate_id, "cand-0001");
            assert_eq!(report.resulting_scene_revision, None);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    Ok(())
}

#[test]
fn the_stored_candidate_carries_the_origin_ai_author() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)?;
    let stored = fixture
        .run_dir
        .join("candidates")
        .join("cand-0001")
        .join("candidate.json");
    let text = std::fs::read_to_string(stored)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(
        value
            .get("provenance")
            .and_then(|value| value.get("origin")),
        Some(&serde_json::Value::String("ai_author".to_string()))
    );
    Ok(())
}

#[test]
fn the_submission_consumes_the_budget() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)?;
    let store = BudgetStore::new(&fixture.run_dir);
    let counters = store.load()?;
    assert_eq!(counters.calls_used, 1);
    assert_eq!(counters.candidates_submitted, 1);
    assert!(counters.bytes_submitted > 0);
    Ok(())
}

#[test]
fn the_second_submission_is_refused_by_the_budget() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, base_policy) = fixture()?;
    let policy = AssistantPolicy {
        max_calls_per_run: 1,
        ..base_policy
    };
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let first = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    tools::submit_candidate(&ctx, &fixture.run_dir, &task, &first)?;
    let second = fixture.write_candidate("candidate-valid-2", "cand-0002", "asst-0001")?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &second)
        .expect_err("the budget is exhausted");
    assert!(matches!(error, AssistantError::BudgetExhausted { .. }));
    Ok(())
}

#[test]
fn an_oversized_candidate_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, base_policy) = fixture()?;
    let policy = AssistantPolicy {
        max_candidate_bytes: 1,
        ..base_policy
    };
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the candidate is too large");
    assert!(matches!(error, AssistantError::BudgetExhausted { .. }));
    Ok(())
}

#[test]
fn a_candidate_of_another_run_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let mut document = candidate_document("cand-0001", "asst-0001");
    document
        .get_mut("base")
        .and_then(|base| base.as_object_mut())
        .ok_or("the base is an object")?
        .insert(
            "run_id".to_string(),
            serde_json::Value::String("other".to_string()),
        );
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    std::fs::write(
        candidate.join("candidate.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the candidate belongs to another run");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_candidate_that_changes_a_protected_component_is_refused(
) -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let mut document = candidate_document("cand-0001", "asst-0001");
    document
        .get_mut("changes")
        .and_then(|changes| changes.as_array_mut())
        .ok_or("the changes are an array")?[0]
        .as_object_mut()
        .ok_or("the change is an object")?
        .insert(
            "component".to_string(),
            serde_json::Value::String("base".to_string()),
        );
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    std::fs::write(
        candidate.join("candidate.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the protected component is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_candidate_with_a_wider_region_is_clamped_to_the_task() -> Result<(), Box<dyn std::error::Error>>
{
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let mut document = candidate_document("cand-0001", "asst-0001");
    document
        .get_mut("task")
        .and_then(|value| value.as_object_mut())
        .ok_or("the task is an object")?
        .insert(
            "allowed_region".to_string(),
            serde_json::json!({
                "min": [-1.0, -1.0, -1.0],
                "max": [1.0, 1.0, 1.0]
            }),
        );
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    std::fs::write(
        candidate.join("candidate.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let outcome = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)?;
    match outcome {
        ToolOutcome::Decision(report) => {
            assert_eq!(
                report.decision,
                michelangelo_edit::validate::Decision::Proposed
            );
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    // The staged candidate carries the region of the task.
    let stored = fixture
        .run_dir
        .join("candidates")
        .join("cand-0001")
        .join("candidate.json");
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(stored)?)?;
    let region = value
        .get("task")
        .and_then(|task| task.get("allowed_region"))
        .ok_or("the candidate carries the region")?;
    assert_eq!(
        region.get("min"),
        Some(&serde_json::json!([-0.044, 0.048, -0.044]))
    );
    Ok(())
}

#[test]
fn a_candidate_whose_region_misses_the_task_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let mut document = candidate_document("cand-0001", "asst-0001");
    document
        .get_mut("task")
        .and_then(|value| value.as_object_mut())
        .ok_or("the task is an object")?
        .insert(
            "allowed_region".to_string(),
            serde_json::json!({
                "min": [5.0, 5.0, 5.0],
                "max": [6.0, 6.0, 6.0]
            }),
        );
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    std::fs::write(
        candidate.join("candidate.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the region is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_candidate_that_does_not_protect_the_task_is_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let mut document = candidate_document("cand-0001", "asst-0001");
    document
        .get_mut("task")
        .and_then(|value| value.as_object_mut())
        .ok_or("the task is an object")?
        .insert(
            "protected".to_string(),
            serde_json::Value::Array(Vec::new()),
        );
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    std::fs::write(
        candidate.join("candidate.json"),
        serde_json::to_string_pretty(&document)?,
    )?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the protection is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_broken_candidate_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, policy) = fixture()?;
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    std::fs::write(candidate.join("candidate.json"), "{ not a candidate")?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the candidate is broken");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_policy_that_forbids_the_submission_denies_it() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, base_policy) = fixture()?;
    let policy = AssistantPolicy {
        allow_submit: false,
        ..base_policy
    };
    let ctx = context(&fixture, &policy);
    let task = task_of(&fixture, &policy)?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let error = tools::submit_candidate(&ctx, &fixture.run_dir, &task, &candidate)
        .expect_err("the policy denies the submission");
    assert!(matches!(error, AssistantError::PolicyDenied { .. }));
    Ok(())
}

#[test]
fn the_size_of_a_candidate_directory_is_counted() -> Result<(), Box<dyn std::error::Error>> {
    let (fixture, _policy) = fixture()?;
    let candidate = fixture.write_candidate("candidate-valid", "cand-0001", "asst-0001")?;
    let size = tools::directory_size(&candidate)?;
    assert!(size > 0);
    Ok(())
}
