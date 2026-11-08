//! Crash injection and orphan recovery tests: crash around reservation,
//! crash around finalize, runner killed while worker lives, immutability.

mod common;

use std::time::Duration;

use common::*;

const PLAN_SCHEMA: &str = "michelangelo.batch-plan/provisional-0";

fn plan_with(
    plan_id: &str,
    item_id: &str,
    failure_mode: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut worker = serde_json::json!({
        "failure_mode": failure_mode,
        "delay_ms": 50,
        "artifact_name": "artifact.bin",
        "artifact_bytes": 64,
        "simulate_validation": true
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            worker[k] = v.clone();
        }
    }
    serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": plan_id,
        "items": [{"item_id": item_id, "inputs": [], "depends_on": [], "worker": worker}],
        "limits": {
            "max_attempts": 3,
            "backoff_base_ms": 20,
            "backoff_cap_ms": 60,
            "attempt_timeout_ms": 15000,
            "heavy_workers": 1
        }
    })
}

/// Runner dies right after the attempt reservation (DB row + directory) but
/// before spawning a worker. Recovery in a new process: attempt is marked
/// interrupted, a NEW attempt number and directory are used, the old
/// directory is never overwritten, no duplicate records appear.
#[test]
fn runner_crash_after_reserve_recovers_with_new_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan_with("bp-crash-reserve", "solo", "none", serde_json::json!({}));
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    // Runner exits abruptly via injected crash after reservation.
    let out = run_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s,
                        "--inject-crash-after-reserve"]);
    assert_eq!(out.status.code(), Some(70), "injected crash exit code expected");

    // State right after the crash: item claimed (running), attempt reserved.
    let store = michelangelo_batch::Store::open(&db).unwrap();
    let items = store.list_items("bp-crash-reserve").unwrap();
    assert_eq!(items[0].status, "running");
    let attempts = store.list_attempts("bp-crash-reserve").unwrap();
    assert_eq!(attempts.len(), 1, "exactly one reserved attempt");
    assert_eq!(attempts[0].state, "reserved");
    drop(store);
    assert_eq!(count_attempt_dirs(&runs, "solo"), 1);

    // Recovery run in a new process.
    let (stdout, stderr, code) =
        run_cli_ok(&["resume", "--plan-id", "bp-crash-reserve", "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 0, "resume failed: {stderr}\n{stdout}");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "solo"), "awaiting_review");
    assert_eq!(item_attempts(&report, "solo"), 2);

    // Old attempt dir preserved untouched; second attempt produced artifact.
    assert_eq!(count_attempt_dirs(&runs, "solo"), 2);
    let attempt1 = runs.join("solo").join("attempt_001");
    let attempt2 = runs.join("solo").join("attempt_002");
    assert!(
        !attempt1.join("artifact.bin").exists(),
        "interrupted attempt must not contain a promoted artifact"
    );
    assert!(attempt2.join("artifact.bin").exists());
    assert!(attempt2.join("result.json").exists());

    // No duplicate DB rows: attempt 1 interrupted, attempt 2 succeeded.
    let store = michelangelo_batch::Store::open(&db).unwrap();
    let attempts = store.list_attempts("bp-crash-reserve").unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].state, "interrupted");
    assert_eq!(attempts[1].state, "succeeded");
}

/// Worker crashes after writing the artifact but before finalizing: the
/// executor retries (crash is transient, bounded); the crashed attempt's
/// artifact is never promoted as success; the final success comes from the
/// new attempt.
#[test]
fn worker_crash_before_finalize_retries_and_never_promotes_old_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan_with("bp-crash-finalize", "solo", "crash_before_finalize", serde_json::json!({}));
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, stderr, code) =
        run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 2, "crash-loop must end with partial failure: {stderr}\n{stdout}");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "solo"), "execution_failed");
    assert_eq!(
        item_attempts(&report, "solo"), 3,
        "bounded attempts (max_attempts=3), not infinite retry"
    );

    // Each crashed attempt has an artifact but no result: file presence is
    // NOT success.
    for no in 1..=3u32 {
        let dir = runs.join("solo").join(format!("attempt_{no:03}"));
        assert!(dir.join("artifact.bin").exists(), "attempt {no} artifact");
        assert!(
            !dir.join("result.json").exists(),
            "attempt {no} must have no result"
        );
    }

    let store = michelangelo_batch::Store::open(&db).unwrap();
    let attempts = store.list_attempts("bp-crash-finalize").unwrap();
    assert_eq!(attempts.len(), 3);
    for a in &attempts {
        assert_eq!(a.state, "failed_transient");
        assert!(a.worker_result_json.is_none());
    }
}

/// The runner process dies while its worker lives on: the orphaned worker
/// keeps the OS locks, so a new runner must NOT start a second worker for the
/// same item; cancel kills the orphan; resume then completes cleanly.
#[test]
fn orphan_worker_blocks_second_worker_until_cancel() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan_with("bp-orphan", "solo", "none", serde_json::json!({"delay_ms": 9000}));
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    // Runner process (parent of the worker).
    let mut runner = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);

    let mid = poll_status(&db, "bp-orphan", Duration::from_secs(20), |v| {
        item_status(v, "solo") == "running"
            && v["items"][0]["live_attempt"]["owner_pid"].as_i64().unwrap_or(0) > 0
    });
    let worker_pid = mid["items"][0]["live_attempt"]["owner_pid"]
        .as_i64()
        .unwrap() as u32;
    assert!(crate_pid_alive(worker_pid));

    // Kill ONLY the runner; the worker is orphaned but alive and holds locks.
    runner.kill().unwrap();
    let _ = runner.wait();
    std::thread::sleep(Duration::from_millis(300));
    assert!(crate_pid_alive(worker_pid), "orphan worker must still be alive");

    // A restart (resume) in a new process must refuse to start a second
    // worker while the orphan holds the attempt lock.
    let (so, _se, code) =
        run_cli_ok(&["resume", "--plan-id", "bp-orphan", "--db", &db_s, "--runs", &runs_s]);
    assert_ne!(code, 0, "resume must not claim success while orphan lives");
    let resume_report: serde_json::Value = serde_json::from_str(so.trim())
        .unwrap_or_else(|e| panic!("resume output not json: {e}; stdout={so}"));
    let warnings: Vec<String> = resume_report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        warnings.iter().any(|w| w.contains("appears live")),
        "expected a live-attempt warning, got {warnings:?}"
    );
    assert_eq!(resume_report["items"][0]["status"], "running");
    assert_eq!(count_attempt_dirs(&runs, "solo"), 1, "no second attempt dir");

    let store = michelangelo_batch::Store::open(&db).unwrap();
    assert_eq!(store.list_attempts("bp-orphan").unwrap().len(), 1);
    drop(store);

    // Cancel: verifies liveness via the OS lock, then kills the orphan.
    let (_so, se, code) = run_cli_ok(&["cancel", "--plan-id", "bp-orphan", "--db", &db_s]);
    assert_eq!(code, 0, "cancel failed: {se}");
    let mut orphan_gone = false;
    for _ in 0..300 {
        if !crate_pid_alive(worker_pid) {
            orphan_gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(orphan_gone, "orphan worker survived cancel");

    // After the orphan is gone, resume creates a fresh attempt and succeeds.
    let (stdout, se, code) =
        run_cli_ok(&["resume", "--plan-id", "bp-orphan", "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 0, "resume after cancel failed: {se}\n{stdout}");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "solo"), "awaiting_review");
    assert_eq!(item_attempts(&report, "solo"), 2);
    assert_eq!(count_attempt_dirs(&runs, "solo"), 2);
}

/// Plan identity is immutable: the same plan id with a different payload is
/// rejected, idempotent registration with the same payload is accepted.
#[test]
fn plan_identity_is_immutable_and_registration_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = plan_with("bp-immutable", "solo", "none", serde_json::json!({}));
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (_so, _se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 0);

    // Same payload again: idempotent, no new attempts.
    let (stdout, _se, code) =
        run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 0, "idempotent re-run must succeed");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_attempts(&report, "solo"), 1, "no new attempts on re-run");
    assert_eq!(count_attempt_dirs(&runs, "solo"), 1);

    // Mutated payload under the same plan id: rejected.
    let mut mutated = plan.clone();
    mutated["items"][0]["worker"]["delay_ms"] = serde_json::json!(999);
    std::fs::write(&plan_file, mutated.to_string()).unwrap();
    let (_so, se, code) = run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 4, "mutated plan must be rejected");
    assert_contains!(se, "immutable");
}

fn crate_pid_alive(pid: u32) -> bool {
    // tasklist can be flaky under parallel load: probe twice.
    for attempt in 0..2 {
        if probe_pid_once(pid) {
            return true;
        }
        if attempt == 0 {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    }
    false
}

fn probe_pid_once(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout).to_string();
                s.contains(&format!("\"{pid}\""))
            }
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}
