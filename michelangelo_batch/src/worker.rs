//! Fake worker process implementation.
//!
//! Runs as a separate OS process (`mbatch worker ...`) so that ownership,
//! fencing and crash semantics are real cross-process properties, not
//! in-process simulations. The worker never touches the database and never
//! writes outside its private attempt directory (staging only). A future real
//! (Blender) worker replaces this binary invocation behind the same spawn
//! contract: spec file in, artifacts + result.json out, exit code protocol.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::locks::{attempt_lock_path, FileLock};
use crate::plan::{fake_artifact_bytes, sha256_hex, FailureMode, PlanInput, WorkerSpec};
use crate::timeutil;

/// Provisional schema marker for the worker invocation file.
pub const WORKER_SPEC_SCHEMA: &str = "michelangelo.batch-worker-spec/provisional-0";

/// What the executor passes to the worker process (private DTO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInvocation {
    pub schema: String,
    pub plan_id: String,
    pub item_id: String,
    pub attempt_no: u32,
    pub worker: WorkerSpec,
    #[serde(default)]
    pub inputs: Vec<PlanInput>,
}

/// What the worker writes back on non-crash terminations (private DTO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResult {
    /// "built" | "awaiting_review" | "validation_failed" | "execution_failed"
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<WorkerArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerArtifact {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Worker exit-code protocol (provisional PoC contract).
pub mod worker_exit {
    pub const SUCCESS: i32 = 0;
    pub const TRANSIENT: i32 = 3;
    pub const PERMANENT: i32 = 4;
    pub const VALIDATION_FAILED: i32 = 5;
    /// Attempt lock already held at start: a duplicate/stale worker for this
    /// attempt tried to run; it must not proceed (fencing).
    pub const FENCED: i32 = 21;
    pub const CRASH: i32 = 70;
}

pub fn read_invocation(spec_path: &Path) -> Result<WorkerInvocation, String> {
    let raw = std::fs::read(spec_path)
        .map_err(|e| format!("cannot read worker spec {}: {e}", spec_path.display()))?;
    let spec: WorkerInvocation =
        serde_json::from_slice(&raw).map_err(|e| format!("worker spec json invalid: {e}"))?;
    if spec.schema != WORKER_SPEC_SCHEMA {
        return Err(format!(
            "worker spec schema mismatch: expected {WORKER_SPEC_SCHEMA:?}, got {:?}",
            spec.schema
        ));
    }
    Ok(spec)
}

/// Entry point of the fake worker process. Returns the process exit code.
pub fn run_worker(spec_path: &Path, attempt_dir: &Path) -> i32 {
    let log_path = attempt_dir.join("worker.log");
    let mut log = std::fs::File::create(&log_path).ok();

    macro_rules! logln {
        ($($arg:tt)*) => {
            if let Some(l) = log.as_mut() {
                let _ = writeln!(l, $($arg)*);
                let _ = l.flush();
            }
        }
    }

    let invocation = match read_invocation(spec_path) {
        Ok(inv) => inv,
        Err(e) => {
            eprintln!("worker: {e}");
            return worker_exit::CRASH;
        }
    };

    logln!(
        "fake worker start item={} attempt={} pid={}",
        invocation.item_id,
        invocation.attempt_no,
        std::process::id()
    );

    // Fencing: hold this attempt's lock for our whole lifetime. If it is
    // already held, another (stale/duplicate) worker owns this attempt and we
    // must not proceed.
    let _fence = match FileLock::acquire(&attempt_lock_path(attempt_dir)) {
        Ok(lock) => lock,
        Err(e) => {
            logln!("FENCED: attempt lock held elsewhere: {e}");
            eprintln!("worker fenced: attempt lock already held: {e}");
            return worker_exit::FENCED;
        }
    };

    // Record our identity inside the attempt dir (audit aid; DB owner_pid is
    // written by the executor, which spawned us).
    let meta = serde_json::json!({
        "item_id": invocation.item_id,
        "attempt_no": invocation.attempt_no,
        "owner_pid": std::process::id(),
        "owner_token": crate::timeutil::new_owner_token(),
        "started_at": timeutil::now_iso8601(),
    });
    if let Err(e) = std::fs::write(attempt_dir.join("attempt_meta.json"), meta.to_string()) {
        logln!("io error writing attempt_meta.json: {e}");
    }

    if invocation.worker.delay_ms > 0 {
        logln!("simulating work for {} ms", invocation.worker.delay_ms);
        std::thread::sleep(Duration::from_millis(invocation.worker.delay_ms));
    }

    let write_result = |result: &WorkerResult| -> Result<(), String> {
        let path = attempt_dir.join("result.json");
        let json = serde_json::to_string_pretty(result)
            .map_err(|e| format!("result serialization: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("cannot write result.json: {e}"))
    };

    match invocation.worker.failure_mode {
        FailureMode::Hang => {
            logln!("hang mode: sleeping until killed by attempt timeout");
            loop {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        FailureMode::FailIo => {
            logln!("simulated disk/io failure while writing artifact");
            let result = WorkerResult {
                outcome: "execution_failed".into(),
                failure_kind: Some("io".into()),
                reason: Some("simulated disk/io failure".into()),
                artifact: None,
            };
            if let Err(e) = write_result(&result) {
                logln!("{e}");
            }
            worker_exit::PERMANENT
        }
        FailureMode::Permanent => {
            logln!("permanent failure (invalid input): no retry expected");
            let result = WorkerResult {
                outcome: "execution_failed".into(),
                failure_kind: Some("invalid_input".into()),
                reason: Some(if invocation.worker.reason.is_empty() {
                    "invalid input".to_string()
                } else {
                    invocation.worker.reason.clone()
                }),
                artifact: None,
            };
            if let Err(e) = write_result(&result) {
                logln!("{e}");
            }
            worker_exit::PERMANENT
        }
        FailureMode::ValidationFail => {
            logln!("fake validation failed: quality verdict, no retry");
            let result = WorkerResult {
                outcome: "validation_failed".into(),
                failure_kind: Some("validation".into()),
                reason: Some(if invocation.worker.reason.is_empty() {
                    "fake validation failed".to_string()
                } else {
                    invocation.worker.reason.clone()
                }),
                artifact: None,
            };
            if let Err(e) = write_result(&result) {
                logln!("{e}");
            }
            worker_exit::VALIDATION_FAILED
        }
        FailureMode::CrashBeforeFinalize => {
            // Crash injection around finalize: artifact exists on disk, but no
            // result and no DB commit happen. The executor must treat this as
            // a crash (transient), and the artifact must never be promoted.
            let bytes = fake_artifact_bytes(
                &invocation.item_id,
                invocation.attempt_no,
                invocation.worker.artifact_bytes,
            );
            let artifact_path = attempt_dir.join(&invocation.worker.artifact_name);
            let _ = std::fs::write(&artifact_path, &bytes);
            logln!("crash injection: exiting without result.json");
            eprintln!("worker: crash before finalize (injected)");
            worker_exit::CRASH
        }
        FailureMode::TransientUntilAttempt
            if invocation.attempt_no < invocation.worker.transient_until_attempt =>
        {
            let until = invocation.worker.transient_until_attempt;
            logln!(
                "transient failure on attempt {} (fails until attempt {until})",
                invocation.attempt_no
            );
            let result = WorkerResult {
                outcome: "execution_failed".into(),
                failure_kind: Some("transient".into()),
                reason: Some(format!(
                    "transient failure on attempt {} (flaky infra)",
                    invocation.attempt_no
                )),
                artifact: None,
            };
            if let Err(e) = write_result(&result) {
                logln!("{e}");
            }
            worker_exit::TRANSIENT
        }
        FailureMode::None | FailureMode::TransientUntilAttempt => {
            let bytes = fake_artifact_bytes(
                &invocation.item_id,
                invocation.attempt_no,
                invocation.worker.artifact_bytes,
            );
            let artifact_path = attempt_dir.join(&invocation.worker.artifact_name);
            if let Err(e) = std::fs::write(&artifact_path, &bytes) {
                logln!("io error writing artifact: {e}");
                let result = WorkerResult {
                    outcome: "execution_failed".into(),
                    failure_kind: Some("io".into()),
                    reason: Some(format!("artifact write failed: {e}")),
                    artifact: None,
                };
                let _ = write_result(&result);
                return worker_exit::PERMANENT;
            }
            let outcome = if invocation.worker.simulate_validation {
                "awaiting_review"
            } else {
                "built"
            };
            logln!("built artifact {} ({outcome})", artifact_path.display());
            let result = WorkerResult {
                outcome: outcome.into(),
                failure_kind: None,
                reason: None,
                artifact: Some(WorkerArtifact {
                    name: invocation.worker.artifact_name.clone(),
                    sha256: sha256_hex(&bytes),
                    bytes: bytes.len() as u64,
                }),
            };
            if let Err(e) = write_result(&result) {
                logln!("{e}");
                return worker_exit::CRASH;
            }
            worker_exit::SUCCESS
        }
    }
}
