//! Shared helpers for cross-process integration tests.

#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::Value;

pub fn mbatch_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mbatch"))
}

/// Run the CLI and capture output.
pub fn run_cli(args: &[&str]) -> Output {
    let mut cmd = Command::new(mbatch_exe());
    cmd.args(args);
    let out = cmd
        .output()
        .expect("failed to spawn mbatch; tests must run via cargo test");
    out
}

/// Run the CLI with a time limit by polling (the child outlives the call only
/// for background runs; this helper waits for exit).
pub fn run_cli_ok(args: &[&str]) -> (String, String, i32) {
    let out = run_cli(args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Spawn the CLI in the background (true second process).
pub fn spawn_cli(args: &[&str]) -> std::process::Child {
    let mut cmd = Command::new(mbatch_exe());
    cmd.args(args);
    cmd.spawn()
        .expect("failed to spawn mbatch in background")
}

/// Poll `mbatch status` from this (second) process until `pred` holds.
/// Transient status failures (DB contention under parallel load) are retried
/// within the timeout; only a timeout is fatal.
pub fn poll_status<F>(db: &Path, plan_id: &str, timeout: Duration, mut pred: F) -> Value
where
    F: FnMut(&Value) -> bool,
{
    let db_str = db.to_string_lossy().to_string();
    let start = Instant::now();
    let mut last_valid: Option<Value> = None;
    loop {
        let (stdout, stderr, code) = run_cli_ok(&["status", "--plan-id", plan_id, "--db", &db_str]);
        match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(v) => {
                if pred(&v) {
                    return v;
                }
                last_valid = Some(v);
            }
            Err(_) => {
                // Keep waiting: transient contention or batch not yet
                // registered (PlanNotFound). stderr carries the honest reason.
                let _ = stderr;
                let _ = code;
            }
        }
        if start.elapsed() > timeout {
            panic!(
                "timeout waiting for status predicate; last status: {}",
                last_valid
                    .map(|v| serde_json::to_string_pretty(&v).unwrap())
                    .unwrap_or_else(|| "<none>".to_string())
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn item_status(report: &Value, item_id: &str) -> String {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["item_id"] == item_id)
        .unwrap_or_else(|| panic!("item {item_id} missing in report"))
        ["status"]
        .as_str()
        .unwrap()
        .to_string()
}

pub fn item_attempts(report: &Value, item_id: &str) -> u64 {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["item_id"] == item_id)
        .unwrap_or_else(|| panic!("item {item_id} missing in report"))
        ["attempts_used"]
        .as_u64()
        .unwrap()
}

pub fn count_attempt_dirs(runs: &Path, item_id: &str) -> usize {
    let item_dir = runs.join(item_id);
    if !item_dir.exists() {
        return 0;
    }
    std::fs::read_dir(&item_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("attempt_")
        })
        .count()
}

pub fn read_file(path: &Path) -> String {
    let mut s = String::new();
    std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
        .read_to_string(&mut s)
        .unwrap();
    s
}

/// Write a small input file and return its path.
pub fn write_input(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

#[macro_export]
macro_rules! assert_contains {
    ($hay:expr, $needle:expr) => {
        assert!(
            $hay.contains($needle),
            "expected to find {:?} in:\n{}",
            $needle,
            $hay
        );
    };
}
