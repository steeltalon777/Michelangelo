//! The tests of the budget counters and of the audit log.

#![cfg(test)]

use crate::budget::{AuditEntry, AuditRecord, BudgetStore, DECISION_ERROR, DECISION_OK};
use crate::testutil::Fixture;
use crate::AssistantError;

#[test]
fn the_counters_are_empty_at_the_start() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let counters = store.load()?;
    assert_eq!(counters.calls_used, 0);
    assert_eq!(counters.candidates_submitted, 0);
    assert_eq!(counters.bytes_submitted, 0);
    assert_eq!(counters.cost_used_usd, 0.0);
    assert_eq!(counters.last_call_at, None);
    Ok(())
}

#[test]
fn the_counters_are_written_and_read_back() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    counters.calls_used = 2;
    counters.bytes_submitted = 128;
    counters.last_call_at = Some(crate::testutil::CLOCK_TIMESTAMP.to_string());
    counters.updated_at = Some(crate::testutil::CLOCK_TIMESTAMP.to_string());
    store.save(&counters)?;
    assert!(store.budget_path().is_file());
    let read = store.load()?;
    assert_eq!(read.calls_used, 2);
    assert_eq!(read.bytes_submitted, 128);
    Ok(())
}

#[test]
fn the_counters_survive_a_new_store_instance() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    counters.calls_used = 1;
    store.save(&counters)?;
    // A new instance reads the same run directory: the counter survives.
    let restarted = BudgetStore::new(&fixture.run_dir);
    assert_eq!(restarted.load()?.calls_used, 1);
    Ok(())
}

#[test]
fn the_call_budget_is_enforced() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    store.check_call(&counters, 1)?;
    counters.calls_used = 1;
    let error = store
        .check_call(&counters, 1)
        .expect_err("the budget is exhausted");
    assert!(matches!(error, AssistantError::BudgetExhausted { .. }));
    Ok(())
}

#[test]
fn the_byte_budget_is_enforced() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    store.check_bytes(&counters, 1024, 2048)?;
    counters.bytes_submitted = 1024;
    let error = store
        .check_bytes(&counters, 1025, 2048)
        .expect_err("the budget is exhausted");
    assert!(matches!(error, AssistantError::BudgetExhausted { .. }));
    Ok(())
}

#[test]
fn the_audit_log_has_one_line_per_call() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    store.record_call(
        &mut counters,
        AuditRecord {
            tool: "get_task",
            decision: DECISION_OK,
            candidate_id: None,
            task_id: Some("asst-0001"),
            detail: "the task was written",
        },
        &fixture.clock,
    )?;
    store.record_call(
        &mut counters,
        AuditRecord {
            tool: "submit_candidate",
            decision: DECISION_ERROR,
            candidate_id: Some("cand-0001"),
            task_id: Some("asst-0001"),
            detail: "the budget is exhausted",
        },
        &fixture.clock,
    )?;
    let lines = store.read_audit()?;
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let value: serde_json::Value = serde_json::from_str(line)?;
        assert!(value.get("tool").is_some());
        assert!(value.get("ts").is_some());
        assert!(value.get("decision").is_some());
    }
    Ok(())
}

#[test]
fn the_audit_log_is_append_only() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    store.record_call(
        &mut counters,
        AuditRecord {
            tool: "get_task",
            decision: DECISION_OK,
            candidate_id: None,
            task_id: None,
            detail: "first",
        },
        &fixture.clock,
    )?;
    store.record_call(
        &mut counters,
        AuditRecord {
            tool: "get_task",
            decision: DECISION_OK,
            candidate_id: None,
            task_id: None,
            detail: "second",
        },
        &fixture.clock,
    )?;
    let lines = store.read_audit()?;
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("first"));
    assert!(lines[1].contains("second"));
    Ok(())
}

#[test]
fn the_audit_log_carries_no_absolute_path() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let store = BudgetStore::new(&fixture.run_dir);
    let mut counters = store.load()?;
    store.record_call(
        &mut counters,
        AuditRecord {
            tool: "submit_candidate",
            decision: DECISION_ERROR,
            candidate_id: None,
            task_id: None,
            detail: &fixture.run_dir.display().to_string(),
        },
        &fixture.clock,
    )?;
    let lines = store.read_audit()?;
    let rendered = fixture.workspace.display().to_string();
    assert!(!lines[0].contains(&rendered));
    Ok(())
}

#[test]
fn the_audit_entry_is_one_line() -> Result<(), Box<dyn std::error::Error>> {
    let entry = AuditEntry {
        ts: crate::testutil::CLOCK_TIMESTAMP.to_string(),
        tool: "get_task".to_string(),
        decision: DECISION_OK.to_string(),
        candidate_id: None,
        task_id: Some("asst-0001".to_string()),
        detail: "a line\nwith a break".to_string(),
    };
    let line = entry.to_line()?;
    assert_eq!(line.lines().count(), 1);
    assert!(line.contains("asst-0001"));
    Ok(())
}
