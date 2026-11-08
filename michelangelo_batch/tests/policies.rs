//! Failure-policy tests: distinct outcomes, bounded transient retry,
//! independent retry/resume, partial failure isolation, input drift.

mod common;

use std::time::Duration;

use common::*;

const PLAN_SCHEMA: &str = "michelangelo.batch-plan/provisional-0";

fn item(id: &str, failure_mode: &str, deps: &[&str], extra: serde_json::Value) -> serde_json::Value {
    let mut worker = serde_json::json!({
        "failure_mode": failure_mode,
        "delay_ms": 30,
        "artifact_name": format!("{id}.out"),
        "artifact_bytes": 32,
        "simulate_validation": true
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            worker[k] = v.clone();
        }
    }
    serde_json::json!({
        "item_id": id,
        "inputs": [],
        "depends_on": deps,
        "worker": worker
    })
}

fn plan(plan_id: &str, items: serde_json::Value, max_attempts: u32) -> serde_json::Value {
    serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": plan_id,
        "items": items,
        "limits": {
            "max_attempts": max_attempts,
            "backoff_base_ms": 15,
            "backoff_cap_ms": 40,
            "attempt_timeout_ms": 10000,
            "heavy_workers": 1
        }
    })
}

/// Permanent (invalid input) failure: exactly one attempt, no retry; the
/// dependent item is blocked; the independent item's success is untouched.
#[test]
fn permanent_failure_no_retry_dependent_blocked_independent_success_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan(
        "bp-partial",
        serde_json::json!([
            item("ok-alone", "none", &[], serde_json::json!({})),
            item("bad", "permanent", &[], serde_json::json!({"reason": "unsupported input format"})),
            item("child-of-bad", "none", &["bad"], serde_json::json!({}))
        ]),
        3,
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 2, "partial failure exit code expected");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert_eq!(item_status(&report, "bad"), "execution_failed");
    assert_eq!(item_attempts(&report, "bad"), 1, "permanent: no retry");
    assert_eq!(item_status(&report, "child-of-bad"), "blocked");
    assert_eq!(item_attempts(&report, "child-of-bad"), 0);
    assert_eq!(item_status(&report, "ok-alone"), "awaiting_review");
    assert_eq!(item_attempts(&report, "ok-alone"), 1);

    // Success artifact of the independent item still on disk with its result.
    let artifact = runs.join("ok-alone").join("attempt_001").join("ok-alone.out");
    assert!(artifact.exists());
    assert!(runs.join("ok-alone").join("attempt_001").join("result.json").exists());
}

/// Transient failure: bounded automatic retry with new attempt per retry;
/// when the flakiness ends before the budget, the item succeeds.
#[test]
fn transient_retry_bounded_and_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan(
        "bp-transient",
        serde_json::json!([
            item("flaky", "transient_until_attempt", &[], serde_json::json!({"transient_until_attempt": 3}))
        ]),
        4,
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 0, "flaky item must recover within budget");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "flaky"), "awaiting_review");
    assert_eq!(item_attempts(&report, "flaky"), 3, "two transient failures then success");
    assert_eq!(count_attempt_dirs(&runs, "flaky"), 3);

    let store = michelangelo_batch::Store::open(&db).unwrap();
    let attempts = store.list_attempts("bp-transient").unwrap();
    assert_eq!(attempts[0].state, "failed_transient");
    assert_eq!(attempts[1].state, "failed_transient");
    assert_eq!(attempts[2].state, "succeeded");
}

/// Transient budget exhausted: honest execution_failed after exactly
/// max_attempts; explicit retry creates NEW attempts (never overwrites).
#[test]
fn transient_budget_exhausted_then_explicit_retry_gets_new_attempts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan(
        "bp-exhaust",
        serde_json::json!([
            item("flaky", "transient_until_attempt", &[], serde_json::json!({"transient_until_attempt": 9}))
        ]),
        3,
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 2);
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "flaky"), "execution_failed");
    assert_eq!(item_attempts(&report, "flaky"), 3);

    // Explicit retry of the selected failed item: bounded again (3 more),
    // still failing while the fake infra is still flaky.
    let (stdout, _se, code) = run_cli_ok(&[
        "retry", "--plan-id", "bp-exhaust", "--items", "flaky", "--db", &db_s, "--runs", &runs_s,
    ]);
    assert_eq!(code, 2);
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "flaky"), "execution_failed");
    assert_eq!(item_attempts(&report, "flaky"), 6, "explicit retry appends new attempts");
    assert_eq!(count_attempt_dirs(&runs, "flaky"), 6, "no directory reuse");
}

/// Validation failure is a distinct outcome and is never auto-retried;
/// `retry` refuses it (needs a new validated plan, not a retry).
#[test]
fn validation_failed_is_distinct_and_not_retryable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan(
        "bp-validation",
        serde_json::json!([
            item("bad-quality", "validation_fail", &[], serde_json::json!({"reason": "silhouette drifted"}))
        ]),
        3,
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 2);
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "bad-quality"), "validation_failed");
    assert_eq!(item_attempts(&report, "bad-quality"), 1, "quality verdict: no retry");

    let (_so, se, _code) = run_cli_ok(&[
        "retry", "--plan-id", "bp-validation", "--items", "bad-quality", "--db", &db_s, "--runs", &runs_s,
    ]);
    let report = poll_status(&db, "bp-validation", Duration::from_secs(5), |_| true);
    assert_eq!(item_status(&report, "bad-quality"), "validation_failed");
    assert_eq!(item_attempts(&report, "bad-quality"), 1, "retry must not touch validation verdicts");
    assert!(se.is_empty() || !se.contains("panicked"));
}

/// Simulated IO/storage failure stops further launches; already committed
/// successes remain; the not-yet-started item stays pending (no fake pass).
#[test]
fn io_failure_stops_launches_and_keeps_successes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan(
        "bp-io",
        serde_json::json!([
            item("a-ok", "none", &[], serde_json::json!({})),
            item("b-io", "fail_io", &[], serde_json::json!({})),
            item("c-never", "none", &[], serde_json::json!({}))
        ]),
        3,
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 2);
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(report["batch_status"], "failed");
    assert_eq!(item_status(&report, "a-ok"), "awaiting_review");
    assert_eq!(item_status(&report, "b-io"), "execution_failed");
    assert_eq!(item_status(&report, "c-never"), "pending", "launches stopped after IO failure");
    assert_eq!(item_attempts(&report, "c-never"), 0);

    // Honest warning recorded in the report.
    let warnings = report["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("IO")),
        "expected an IO stop warning: {warnings:?}"
    );
}

/// Resume re-verifies real input fingerprints: changed input blocks the
/// affected item (no blind re-run), successful items are never reset.
#[test]
fn resume_verifies_input_fingerprints_and_success_is_stable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");
    let inputs = root.join("inputs");
    std::fs::create_dir_all(&inputs).unwrap();

    let good_input = write_input(&inputs, "good.txt", "stable content");
    let drift_input = write_input(&inputs, "drift.txt", "original bytes");

    let mk = |drift_path: &std::path::Path| {
        serde_json::json!({
            "schema": PLAN_SCHEMA,
            "plan_id": "bp-drift",
            "items": [
                {"item_id": "stable", "inputs": [{"path": good_input.to_string_lossy()}], "depends_on": [],
                 "worker": {"failure_mode": "none", "delay_ms": 20, "simulate_validation": true}},
                {"item_id": "drifty", "inputs": [{"path": drift_path.to_string_lossy()}], "depends_on": [],
                 "worker": {"failure_mode": "permanent", "reason": "boom", "delay_ms": 20, "simulate_validation": false}}
            ],
            "limits": {"max_attempts": 2, "backoff_base_ms": 15, "backoff_cap_ms": 30,
                        "attempt_timeout_ms": 10000, "heavy_workers": 1}
        })
    };

    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, mk(&drift_input).to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (_so, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 2, "drifty is permanent-failed");

    // Change the actual input bytes of `drifty` after registration.
    std::fs::write(&drift_input, "TAMPERED bytes").unwrap();

    // Resume: drifty must be blocked as input_drift, NOT re-executed;
    // stable must stay awaiting_review (never reset, never re-run).
    let (stdout, _se, _code) =
        run_cli_ok(&["resume", "--plan-id", "bp-drift", "--db", &db_s, "--runs", &runs_s]);
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "stable"), "awaiting_review");
    assert_eq!(item_attempts(&report, "stable"), 1);
    assert_eq!(item_status(&report, "drifty"), "blocked");
    assert_eq!(item_attempts(&report, "drifty"), 1, "no attempt after input drift");
    let reason = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["item_id"] == "drifty")
        .unwrap()["blocked_reason"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(reason.starts_with("input_drift"), "reason: {reason}");

    // Restore the input; resume again: drifty may run again (still fails:
    // permanent mode), the reason updates but stable remains untouched.
    std::fs::write(&drift_input, "original bytes").unwrap();
    let (stdout, _se, _code) =
        run_cli_ok(&["resume", "--plan-id", "bp-drift", "--db", &db_s, "--runs", &runs_s]);
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "stable"), "awaiting_review");
    assert_eq!(item_attempts(&report, "stable"), 1);
    assert_eq!(item_status(&report, "drifty"), "execution_failed");
    assert_eq!(item_attempts(&report, "drifty"), 2);
}

/// Structured event log records the lifecycle and keeps outcomes distinct.
#[test]
fn event_log_records_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan(
        "bp-events",
        serde_json::json!([item("solo", "none", &[], serde_json::json!({}))]),
        2,
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);

    let store = michelangelo_batch::Store::open(&db).unwrap();
    let events = store.events_tail("bp-events", 100).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.3.as_str()).collect();
    for expected in [
        "batch_run_started",
        "item_claimed",
        "attempt_reserved",
        "attempt_started",
        "attempt_finished",
        "item_outcome",
        "batch_finished",
    ] {
        assert!(
            kinds.contains(&expected),
            "event log missing {expected}; got {kinds:?}"
        );
    }
}
