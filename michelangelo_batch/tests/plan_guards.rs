//! Guards on the plan contract: declared input hashes, item-id grammar and
//! input-drift verification in every run mode.

mod common;

use std::time::Duration;

use common::*;

const PLAN_SCHEMA: &str = "michelangelo.batch-plan/provisional-0";

fn limits(max_attempts: u32) -> serde_json::Value {
    serde_json::json!({
        "max_attempts": max_attempts,
        "backoff_base_ms": 15,
        "backoff_cap_ms": 40,
        "attempt_timeout_ms": 15000,
        "heavy_workers": 1
    })
}

fn item_with_input(id: &str, input: &std::path::Path, sha: Option<&str>) -> serde_json::Value {
    let mut input_obj = serde_json::json!({ "path": input.to_string_lossy() });
    if let Some(sha) = sha {
        input_obj["sha256"] = serde_json::json!(sha);
    }
    serde_json::json!({
        "item_id": id,
        "inputs": [input_obj],
        "depends_on": [],
        "worker": {
            "failure_mode": "none",
            "delay_ms": 20,
            "artifact_name": format!("{id}.out"),
            "artifact_bytes": 32,
            "simulate_validation": true
        }
    })
}

/// A plan that DECLARES the sha256 of its input must be rejected when the
/// actual file bytes hash to something else. Registration must never
/// silently overwrite the declared hash with the observed one.
#[test]
fn declared_input_hash_mismatch_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");
    let inputs = root.join("inputs");
    std::fs::create_dir_all(&inputs).unwrap();
    let input = write_input(&inputs, "a.txt", "the real bytes");

    let wrong = michelangelo_batch::plan::sha256_hex(b"completely different bytes");
    let plan = serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": "bp-bad-hash",
        "items": [item_with_input("solo", &input, Some(&wrong))],
        "limits": limits(3)
    });
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let out = run_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "a declared hash mismatch must be a usage error, stdout={:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("hash mismatch"),
        "expected an honest hash mismatch error, got: {stderr}"
    );

    // Nothing was registered and nothing was executed.
    let (_so, _se, code) = run_cli_ok(&["status", "--plan-id", "bp-bad-hash", "--db", &db_s]);
    assert_ne!(code, 0, "the rejected plan must not be registered");
    assert!(
        !runs.join("solo").exists(),
        "no attempt may be started for a rejected plan"
    );
}

/// The positive counterpart: a correctly declared hash is accepted and the
/// pinned fingerprint of the registered plan matches the actual bytes.
#[test]
fn correct_declared_input_hash_is_accepted_and_pinned() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");
    let inputs = root.join("inputs");
    std::fs::create_dir_all(&inputs).unwrap();
    let input = write_input(&inputs, "a.txt", "the real bytes");
    let correct = michelangelo_batch::plan::sha256_hex(b"the real bytes");

    let plan = serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": "bp-good-hash",
        "items": [item_with_input("solo", &input, Some(&correct))],
        "limits": limits(3)
    });
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let (stdout, stderr, code) =
        run_cli_ok(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(code, 0, "run failed: {stderr}\n{stdout}");
    let report: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(item_status(&report, "solo"), "awaiting_review");

    // The registered plan carries the fingerprint of the ACTUAL bytes.
    let store = michelangelo_batch::Store::open(&db).unwrap();
    let (plan_json, _fp, _status) = store.load_plan_json("bp-good-hash").unwrap();
    let registered: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    assert_eq!(
        registered["items"][0]["inputs"][0]["sha256"].as_str(),
        Some(correct.as_str()),
        "the registered plan must pin the actual bytes"
    );
    assert!(
        registered["items"][0]["inputs"][0]["path"]
            .as_str()
            .unwrap()
            .starts_with('/')
            || registered["items"][0]["inputs"][0]["path"]
                .as_str()
                .unwrap()
                .len()
                > 1,
        "the input path must be canonicalized"
    );
}

/// An item id that is a path traversal token must be rejected before
/// anything is written: `..` would place the attempt directory outside the
/// runs root.
#[test]
fn item_id_dot_dot_is_rejected_before_anything_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": "bp-traversal",
        "items": [{
            "item_id": "..",
            "inputs": [],
            "depends_on": [],
            "worker": {
                "failure_mode": "none",
                "delay_ms": 20,
                "artifact_name": "x.out",
                "artifact_bytes": 32,
                "simulate_validation": true
            }
        }],
        "limits": limits(3)
    });
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let out = run_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "an invalid item id must be a usage error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("invalid"),
        "expected an honest invalid-item-id error, got: {stderr}"
    );

    // Nothing was written outside the runs root.
    assert!(
        !root.join("attempt_001").exists(),
        "the attempt directory must not escape the runs root"
    );
    assert!(
        !runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none(),
        "the runs root must stay empty"
    );

    // The plan was not registered at all.
    let (_so, _se, code) = run_cli_ok(&["status", "--plan-id", "bp-traversal", "--db", &db_s]);
    assert_ne!(code, 0, "the rejected plan must not be registered");
}

/// Input drift is verified in EVERY mode, including a fresh run, and right
/// before the claim: an item whose input bytes changed after the batch
/// started must never be executed.
#[test]
fn fresh_run_verifies_input_drift_before_claiming_an_item() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");
    let inputs = root.join("inputs");
    std::fs::create_dir_all(&inputs).unwrap();
    let drift_input = write_input(&inputs, "b.txt", "original bytes");

    // "a-slow" runs first and long enough to change the input of "b-drifty"
    // while the batch is already running.
    let plan = serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": "bp-fresh-drift",
        "items": [
            {
                "item_id": "a-slow",
                "inputs": [],
                "depends_on": [],
                "worker": {
                    "failure_mode": "none",
                    "delay_ms": 4000,
                    "artifact_name": "a.out",
                    "artifact_bytes": 32,
                    "simulate_validation": true
                }
            },
            item_with_input("b-drifty", &drift_input, None)
        ],
        "limits": limits(2)
    });
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let mut runner = spawn_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);

    // Wait until the first item is really running, then tamper with the
    // input of the item that has not been claimed yet.
    poll_status(&db, "bp-fresh-drift", Duration::from_secs(20), |v| {
        item_status(v, "a-slow") == "running"
    });
    std::fs::write(&drift_input, "TAMPERED bytes").unwrap();

    let out = runner.wait().expect("runner wait failed");
    let code = out.code().unwrap_or(-1);
    assert_eq!(code, 2, "the drifted item must not be a success");

    let report = poll_status(&db, "bp-fresh-drift", Duration::from_secs(10), |v| {
        v["batch_status"] == "completed"
    });
    assert_eq!(item_status(&report, "a-slow"), "awaiting_review");
    assert_eq!(
        item_status(&report, "b-drifty"),
        "blocked",
        "a drifted item must not be executed"
    );
    assert_eq!(
        item_attempts(&report, "b-drifty"),
        0,
        "a drifted item must not get an attempt"
    );
    assert!(
        item_blocked_reason(&report, "b-drifty").starts_with("input_drift"),
        "honest reason missing: {:?}",
        item_blocked_reason(&report, "b-drifty")
    );
}

/// A Windows device name must never become a directory name: `CON` is the
/// console device on Windows, not a folder. The rejection happens on every
/// platform, so a plan accepted on Linux cannot break on Windows.
#[test]
fn windows_device_name_item_id_is_rejected_before_anything_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": "bp-device-name",
        "items": [{
            "item_id": "CON",
            "inputs": [],
            "depends_on": [],
            "worker": {
                "failure_mode": "none",
                "delay_ms": 20,
                "artifact_name": "x.out",
                "artifact_bytes": 32,
                "simulate_validation": true
            }
        }],
        "limits": limits(3)
    });
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let out = run_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "a reserved device name must be a usage error, stdout={:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("invalid"),
        "expected an honest invalid-item-id error, got: {stderr}"
    );
    assert!(
        !runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none(),
        "the runs root must stay empty"
    );
    let (_so, _se, code) = run_cli_ok(&["status", "--plan-id", "bp-device-name", "--db", &db_s]);
    assert_ne!(code, 0, "the rejected plan must not be registered");
}

/// On Windows the filesystem folds case: "Build" and "build" are one and the
/// same directory, so two items with such ids must be rejected before
/// anything is written (they would fight over one attempt directory).
#[cfg(windows)]
#[test]
fn case_colliding_item_ids_are_rejected_on_windows() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let db = root.join("state.db");
    let runs = root.join("runs");

    let plan = serde_json::json!({
        "schema": PLAN_SCHEMA,
        "plan_id": "bp-case-collision",
        "items": [
            {
                "item_id": "Build",
                "inputs": [],
                "depends_on": [],
                "worker": {
                    "failure_mode": "none",
                    "delay_ms": 20,
                    "artifact_name": "b.out",
                    "artifact_bytes": 32,
                    "simulate_validation": true
                }
            },
            {
                "item_id": "build",
                "inputs": [],
                "depends_on": [],
                "worker": {
                    "failure_mode": "none",
                    "delay_ms": 20,
                    "artifact_name": "b2.out",
                    "artifact_bytes": 32,
                    "simulate_validation": true
                }
            }
        ],
        "limits": limits(3)
    });
    let plan_file = root.join("plan.json");
    std::fs::write(&plan_file, plan.to_string()).unwrap();
    let db_s = db.to_string_lossy().to_string();
    let runs_s = runs.to_string_lossy().to_string();
    let plan_s = plan_file.to_string_lossy().to_string();

    let out = run_cli(&["run", "--plan", &plan_s, "--db", &db_s, "--runs", &runs_s]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "case-colliding item ids must be a usage error, stdout={:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("duplicate"),
        "expected an honest duplicate-item-id error, got: {stderr}"
    );
    assert!(
        !runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none(),
        "the runs root must stay empty"
    );
    let (_so, _se, code) = run_cli_ok(&["status", "--plan-id", "bp-case-collision", "--db", &db_s]);
    assert_ne!(code, 0, "the rejected plan must not be registered");
}
