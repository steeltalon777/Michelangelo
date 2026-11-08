//! Cross-process integration tests: delayed finish with status/cancel from a
//! second process, concurrent runners, admission serialization.

mod common;

use std::time::Duration;

use common::*;

const PLAN_SCHEMA: &str = "michelangelo.batch-plan/provisional-0";

fn simple_plan(plan_id: &str, items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": plan_id,
        "items": items,
        "limits": {
            "max_attempts": 3,
            "backoff_base_ms": 20,
            "backoff_cap_ms": 60,
            "attempt_timeout_ms": 15000,
            "heavy_workers": 1
        }
    })
}

fn build_item(id: &str, delay_ms: u64, deps: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "inputs": [],
        "depends_on": deps,
        "worker": {
            "failure_mode": "none",
            "delay_ms": delay_ms,
            "artifact_name": format!("{id}.out"),
            "artifact_bytes": 48,
            "simulate_validation": true
        }
    })
}

/// Status must be observable from a second process while a worker is running
/// (control path available during execution).
#[test]
fn delayed_finish_status_from_second_process() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = simple_plan("bp-delayed", serde_json::json!([build_item("solo", 4000, &[])]));
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    // First process: the runner.
    let mut runner = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);

    // Second process (this test): observe a live running attempt mid-flight.
    let mid = poll_status(&db, "bp-delayed", Duration::from_secs(20), |v| {
        item_status(v, "solo") == "running"
            && v["items"][0]["live_attempt"]["owner_pid"].as_i64().unwrap_or(0) > 0
    });
    let live = mid["items"][0]["live_attempt"].clone();
    assert!(
        live["owner_pid"].as_i64().unwrap_or(0) > 0,
        "expected live attempt with owner pid mid-run: {mid}"
    );
    assert_eq!(live["state"], "running");

    // Worker process must be alive right now (cross-process ownership).
    let pid = live["owner_pid"].as_i64().unwrap() as u32;
    assert!(pid_alive(pid), "worker pid {pid} should be alive mid-run");

    // The runner finishes on its own.
    let out = runner.wait().expect("runner wait failed");
    let code = out.code().unwrap_or(-1);
    let final_report = poll_status(&db, "bp-delayed", Duration::from_secs(5), |v| {
        v["batch_status"] == "completed"
    });
    assert_eq!(item_status(&final_report, "solo"), "awaiting_review");
    assert_eq!(code, 0, "runner exit code must be OK for all-success");
}

/// Cancel from a second process: kills the owned worker, prevents dependent
/// stages, never rolls back successful items. Resume may re-run explicitly.
#[test]
fn cancel_from_second_process_stops_worker_and_dependents() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = simple_plan(
        "bp-cancel",
        serde_json::json!([
            build_item("first", 8000, &[]),
            build_item("second", 50, &["first"])
        ]),
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let mut runner = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);

    // Wait until the worker of `first` is live (pid recorded).
    let mid = poll_status(&db, "bp-cancel", Duration::from_secs(20), |v| {
        item_status(v, "first") == "running"
            && v["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["item_id"] == "first")
                .map(|i| i["live_attempt"]["owner_pid"].as_i64().unwrap_or(0) > 0)
                .unwrap_or(false)
    });
    let worker_pid = mid["items"][0]["live_attempt"]["owner_pid"]
        .as_i64()
        .unwrap() as u32;
    assert!(pid_alive(worker_pid));

    // Cancel from the second process.
    let (_stdout, stderr, code) = run_cli_ok(&["cancel", "--plan-id", "bp-cancel", "--db", &db_s]);
    assert_eq!(code, 0, "cancel failed: {stderr}");

    // Worker process tree must die.
    let mut worker_gone = false;
    for _ in 0..300 {
        if !pid_alive(worker_pid) {
            worker_gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(worker_gone, "worker pid {worker_pid} survived cancel");

    // Dependent stage never ran; running item is cancelled.
    let report = poll_status(&db, "bp-cancel", Duration::from_secs(10), |v| {
        item_status(v, "first") == "cancelled" && item_status(v, "second") == "cancelled"
    });
    assert_eq!(report["batch_status"], "cancelled");
    assert_eq!(item_attempts(&report, "second"), 0);

    // The runner process observes the arbitration loss and exits non-zero
    // with a distinct report (no false success).
    let _ = runner.wait();
    let after = poll_status(&db, "bp-cancel", Duration::from_secs(10), |v| {
        v["batch_status"] == "cancelled"
    });
    assert_eq!(item_status(&after, "first"), "cancelled");
    assert_eq!(item_status(&after, "second"), "cancelled");

    // Explicit resume re-runs cancelled items and completes.
    let (stdout, stderr, code) = run_cli_ok(&["resume", "--plan-id", "bp-cancel", "--db", &db_s, "--runs", &runs_s]);
    assert!(code == 0, "resume failed: {stderr}\n{stdout}");
    let resumed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&resumed, "first"), "awaiting_review");
    assert_eq!(item_status(&resumed, "second"), "awaiting_review");
    // Fresh attempt directories, old ones never overwritten.
    assert_eq!(count_attempt_dirs(&runs, "first"), 2);
    assert_eq!(count_attempt_dirs(&runs, "second"), 1);
}

/// Two concurrent runner processes on the same plan: each item is executed
/// exactly once (claim CAS + admission slot); no duplicate attempts/dirs.
#[test]
fn two_concurrent_runners_execute_each_item_exactly_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = simple_plan(
        "bp-concurrent",
        serde_json::json!([
            build_item("c1", 120, &[]),
            build_item("c2", 120, &[]),
            build_item("c3", 120, &[]),
            build_item("c4", 120, &[])
        ]),
    );
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let mut p1 = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    let mut p2 = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    let r1 = p1.wait().unwrap();
    let r2 = p2.wait().unwrap();
    // At least one runner must fully succeed; the other may legitimately
    // report partial/busy, but neither may double-execute items.
    let codes = [r1.code().unwrap_or(-1), r2.code().unwrap_or(-1)];
    assert!(
        codes.contains(&0),
        "at least one runner must succeed; got {codes:?}"
    );

    let report = poll_status(&db, "bp-concurrent", Duration::from_secs(10), |v| {
        v["batch_status"] == "completed"
    });
    for id in ["c1", "c2", "c3", "c4"] {
        assert_eq!(item_status(&report, id), "awaiting_review", "item {id}");
        assert_eq!(item_attempts(&report, id), 1, "item {id} attempt count");
        assert_eq!(count_attempt_dirs(&runs, id), 1, "item {id} dir count");
    }
}

/// Admission: while one runner works, a second process cannot acquire the
/// heavy-worker slot; the wait is bounded and never starts a second worker.
#[test]
fn heavy_worker_slot_is_exclusive_across_processes() {
    use michelangelo_batch::locks::{slot_lock_path, FileLock};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = simple_plan("bp-slot", serde_json::json!([build_item("long", 9000, &[])]));
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let mut runner = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);

    poll_status(&db, "bp-slot", Duration::from_secs(20), |v| {
        item_status(v, "long") == "running"
            && v["items"][0]["live_attempt"]["owner_pid"].as_i64().unwrap_or(0) > 0
    });

    // From this second process: the slot must be busy while the worker runs.
    let slot = slot_lock_path(&runs);
    let started = std::time::Instant::now();
    let err = FileLock::acquire_with_wait(&slot, 300).unwrap_err();
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "lock acquisition returned too early"
    );
    assert!(
        matches!(err, michelangelo_batch::locks::LockError::Busy { .. }),
        "slot must be busy during execution"
    );

    // A second runner with a tiny wait must fail honestly without starting
    // any worker for the running item.
    let (_so, se, code) = run_cli_ok(&[
        "run",
        "--plan",
        &plan_s,
        "--db",
        &db_s,
        "--runs",
        &runs_s,
        "--slot-wait-ms",
        "150",
    ]);
    assert_ne!(code, 0, "second runner must not silently succeed");
    // Either it skipped the non-pending item (partial) or reported busy.
    assert!(
        se.contains("busy") || code == 2,
        "expected honest busy/partial report, stderr: {se}"
    );

    let _ = runner.wait();
    let report = poll_status(&db, "bp-slot", Duration::from_secs(10), |v| {
        v["batch_status"] == "completed"
    });
    assert_eq!(item_status(&report, "long"), "awaiting_review");
    assert_eq!(item_attempts(&report, "long"), 1);
}

/// PID liveness probe without extra dependencies. tasklist can be flaky
/// under heavy parallel load, so probe twice before reporting "dead".
pub fn pid_alive(pid: u32) -> bool {
    for attempt in 0..2 {
        if probe_pid_once(pid) {
            return true;
        }
        if attempt == 0 {
            std::thread::sleep(Duration::from_millis(120));
        }
    }
    false
}

fn probe_pid_once(pid: u32) -> bool {
    #[cfg(windows)]
    {
        // `tasklist /FI "PID eq N"` prints the image name when found.
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
