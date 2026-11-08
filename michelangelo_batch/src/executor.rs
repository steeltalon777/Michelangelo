//! Batch executor: reservation, admission, lifecycle, retry, resume, cancel.
//!
//! Design notes (match ADR 0005 §7.2 and the stream-C assignment):
//! - The durable state is the SQLite store; the in-memory loop is only a
//!   means of driving it. A crashed runner is reconciled from the store plus
//!   OS file locks, never from process memory.
//! - Attempt directories are reserved atomically and never overwritten.
//! - The heavy-worker slot (limit 1) is a cross-process OS file lock; a live
//!   worker in any process blocks a second worker everywhere.
//! - Every state change is a CAS transition, so cancel/resume/crashed runners
//!   arbitrate without double transitions and a late old attempt cannot
//!   publish (its CAS from `running` fails).
//! - Retry is bounded and only for transient failures. Invalid input and
//!   validation failures are permanent verdicts.
//! - Cancel kills the owned worker process and marks dependent stages
//!   cancelled/blocked; successful items are never rolled back.

use std::io::ErrorKind;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::locks::{attempt_lock_path, slot_lock_path, FileLock, LockError};
use crate::plan::{attempt_dir, BatchPlan, PlanError};
use crate::report::BatchReport;
use crate::store::{
    self, Store, StoreError, ATTEMPT_CANCELLED, ATTEMPT_INTERRUPTED, ATTEMPT_RESERVED,
    ATTEMPT_RUNNING, ATTEMPT_SUCCEEDED, BATCH_CANCELLED, BATCH_COMPLETED, BATCH_FAILED,
    BATCH_RUNNING, ITEM_AWAITING_REVIEW, ITEM_BUILT, ITEM_BLOCKED, ITEM_CANCELLED,
    ITEM_EXECUTION_FAILED, ITEM_PENDING, ITEM_RUNNING, ITEM_SUCCESS_STATES,
    ITEM_VALIDATION_FAILED,
};
use crate::worker::{worker_exit, WorkerInvocation, WorkerResult};
use crate::{exit_codes, timeutil};

/// How the executor selects claimable items.
#[derive(Debug, Clone)]
pub enum RunMode {
    /// Fresh run: only `pending` items are claimable; a cancelled batch is
    /// refused (use resume explicitly).
    FreshRun,
    /// Resume: re-verify inputs, reconcile dead owners, then claim pending,
    /// execution_failed, cancelled (explicit user intent) and dep-unblocked
    /// items. Successful items are never reset or re-run.
    Resume,
    /// Retry only the listed items that are in `execution_failed`, bounded by
    /// the plan's max_attempts. `validation_failed` is not retryable.
    Retry(Vec<String>),
}

impl RunMode {
    pub fn name(&self) -> &'static str {
        match self {
            RunMode::FreshRun => "run",
            RunMode::Resume => "resume",
            RunMode::Retry(_) => "retry",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecOptions {
    /// How long to wait for the heavy-worker slot before giving up. Giving up
    /// never starts a second worker.
    pub slot_wait_ms: u64,
    /// Test-only crash injection: exit abruptly right after the first attempt
    /// reservation (DB row + directory) but before spawning a worker.
    pub inject_crash_after_reserve: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        ExecOptions {
            slot_wait_ms: 10_000,
            inject_crash_after_reserve: false,
        }
    }
}

#[derive(Debug)]
pub enum ExecError {
    Plan(PlanError),
    Store(StoreError),
    Lock(LockError),
    Io(std::io::Error),
    SlotBusy { waited_ms: u64 },
    BatchCancelled,
    PlanNotFound { plan_id: String },
    Conflict(String),
    Json { context: String, source: serde_json::Error },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Plan(e) => write!(f, "plan error: {e}"),
            ExecError::Store(e) => write!(f, "store error: {e}"),
            ExecError::Lock(e) => write!(f, "lock error: {e}"),
            ExecError::Io(e) => write!(f, "io error: {e}"),
            ExecError::SlotBusy { waited_ms } => write!(
                f,
                "heavy worker slot is busy (waited {waited_ms} ms); no second worker was started"
            ),
            ExecError::BatchCancelled => {
                write!(f, "batch is cancelled; use resume to re-run explicitly")
            }
            ExecError::PlanNotFound { plan_id } => write!(f, "batch plan {plan_id:?} not found"),
            ExecError::Conflict(context) => write!(f, "conflict: {context}"),
            ExecError::Json { context, source } => write!(f, "json error ({context}): {source}"),
        }
    }
}

impl std::error::Error for ExecError {}

impl From<PlanError> for ExecError {
    fn from(e: PlanError) -> Self {
        ExecError::Plan(e)
    }
}
impl From<StoreError> for ExecError {
    fn from(e: StoreError) -> Self {
        ExecError::Store(e)
    }
}
impl From<LockError> for ExecError {
    fn from(e: LockError) -> Self {
        ExecError::Lock(e)
    }
}
impl From<std::io::Error> for ExecError {
    fn from(e: std::io::Error) -> Self {
        ExecError::Io(e)
    }
}

/// Load a plan file, validate it and register it immutably. Returns the
/// authoritative (pinned) plan actually used for execution plus its id and
/// provided-form fingerprint.
pub fn load_or_register(
    plan_file: &Path,
    store: &Store,
) -> Result<(BatchPlan, String, String), ExecError> {
    let provided = BatchPlan::load(plan_file)?;
    provided.validate()?;
    let fingerprint = provided.fingerprint()?;
    let plan_id = provided.resolved_plan_id()?;

    if let Ok((stored_json, stored_fp, _status)) = store.load_plan_json(&plan_id) {
        if stored_fp != fingerprint {
            return Err(ExecError::Store(StoreError::PlanImmutable {
                plan_id: plan_id.clone(),
            }));
        }
        let stored: BatchPlan = serde_json::from_str(&stored_json).map_err(|source| {
            ExecError::Json {
                context: "stored plan".into(),
                source,
            }
        })?;
        stored.validate()?;
        return Ok((stored, plan_id, stored_fp));
    }

    // New registration: pin actual input paths and fingerprints once.
    let mut pinned = provided;
    pinned.pin_inputs()?;
    let filled_json = pinned.to_canonical_json()?;
    let item_ids: Vec<String> = pinned.items.iter().map(|i| i.item_id.clone()).collect();
    store.register_plan(&plan_id, &fingerprint, &filled_json, &item_ids)?;
    Ok((pinned, plan_id, fingerprint))
}

/// Load a previously registered plan by id (resume/retry/status path).
pub fn load_registered(plan_id: &str, store: &Store) -> Result<(BatchPlan, String, String), ExecError> {
    let (plan_json, fingerprint, _status) = store.load_plan_json(plan_id)?;
    let plan: BatchPlan = serde_json::from_str(&plan_json).map_err(|source| ExecError::Json {
        context: "stored plan".into(),
        source,
    })?;
    plan.validate()?;
    Ok((plan, plan_id.to_string(), fingerprint))
}

/// Execute (run/resume/retry) and return the final report.
///
/// `db_path` is a dedicated disposable SQLite database, separate from any
/// project Workspace DB.
pub fn execute(
    plan: &BatchPlan,
    plan_id: &str,
    fingerprint: &str,
    db_path: &Path,
    runs_dir: &Path,
    mode: RunMode,
    opts: &ExecOptions,
) -> Result<BatchReport, ExecError> {
    let store = Store::open(db_path)?;
    execute_with_store(&store, plan, plan_id, fingerprint, runs_dir, mode, opts)
}

fn execute_with_store(
    store: &Store,
    plan: &BatchPlan,
    plan_id: &str,
    fingerprint: &str,
    runs_dir: &Path,
    mode: RunMode,
    opts: &ExecOptions,
) -> Result<BatchReport, ExecError> {
    std::fs::create_dir_all(runs_dir)?;
    let order = plan.topo_order()?;

    match &mode {
        RunMode::FreshRun => {
            if store.batch_status(plan_id)?.as_deref() == Some(BATCH_CANCELLED) {
                return Err(ExecError::BatchCancelled);
            }
        }
        _ => {}
    }
    store.set_batch_status(plan_id, BATCH_RUNNING, None)?;
    store.append_event(
        plan_id,
        None,
        None,
        "batch_run_started",
        Some(&serde_json::json!({ "mode": mode.name() }).to_string()),
    )?;

    let mut warnings: Vec<String> = Vec::new();
    let mut drifted_items: std::collections::BTreeSet<String> = Default::default();

    if !matches!(mode, RunMode::FreshRun) {
        let (w, drifted) = reconcile_before_resume(store, plan, plan_id)?;
        warnings.extend(w);
        drifted_items = drifted;
    }

    let mut stop_all: Option<String> = None;
    // Retry budget baseline per item, captured at the FIRST claim of this
    // execute() invocation (not per attempt, otherwise the budget never
    // exhausts within a cycle).
    let mut cycle_starts: std::collections::BTreeMap<String, u32> = Default::default();

    'items: for item_id in &order {
        if stop_all.is_some() {
            break 'items;
        }
        let mut attempt_no: u32;
        loop {
            let row = store.get_item(plan_id, item_id)?.ok_or_else(|| {
                ExecError::Conflict(format!("item {item_id:?} missing from store"))
            })?;

            // Successful items are never re-run; nothing resets them.
            if ITEM_SUCCESS_STATES.contains(&row.status.as_str()) {
                store.append_event(
                    plan_id,
                    Some(item_id),
                    None,
                    "item_skip_already_terminal",
                    Some(&serde_json::json!({ "status": row.status }).to_string()),
                )?;
                continue 'items;
            }
            // Fresh runs never touch non-pending items.
            if matches!(mode, RunMode::FreshRun) && row.status != ITEM_PENDING {
                continue 'items;
            }
            // Input-drifted items stay blocked while the drift is current.
            if row.status == ITEM_BLOCKED
                && row
                    .blocked_reason
                    .as_deref()
                    .map_or(false, |r| r.starts_with("input_drift"))
                && drifted_items.contains(item_id)
            {
                continue 'items;
            }

            // Dependency gating.
            match dep_gate(store, plan_id, plan, item_id)? {
                DepGate::Block(reason) => {
                    store.set_item_status(
                        plan_id,
                        item_id,
                        ITEM_BLOCKED,
                        None,
                        Some(&reason),
                        None,
                    )?;
                    store.append_event(
                        plan_id,
                        Some(item_id),
                        None,
                        "item_dep_blocked",
                        Some(&serde_json::json!({ "reason": reason }).to_string()),
                    )?;
                    continue 'items;
                }
                DepGate::Skip => {
                    // A dependency is pending/running elsewhere; do not block,
                    // do not double-run: another runner owns it.
                    continue 'items;
                }
                DepGate::Ok => {}
            }

            // Claim.
            let claim_allowed: &[&str] = match &mode {
                RunMode::FreshRun => &[ITEM_PENDING],
                RunMode::Resume => &[
                    ITEM_PENDING,
                    ITEM_EXECUTION_FAILED,
                    ITEM_CANCELLED,
                    ITEM_BLOCKED,
                ],
                RunMode::Retry(list) => {
                    if !list.iter().any(|s| s == item_id) {
                        continue 'items;
                    }
                    // pending: a previous retry cycle in this same invocation
                    // may have just handed the item back for its next attempt.
                    &[ITEM_EXECUTION_FAILED, ITEM_PENDING]
                }
            };
            if !store.set_item_status(
                plan_id,
                item_id,
                ITEM_RUNNING,
                Some(claim_allowed),
                None,
                None,
            )? {
                // Lost claim arbitration: someone else moved this item.
                continue 'items;
            }
            // Retry budget is per execution cycle: bounded backoff within
            // this run/resume/retry, while explicit user actions can grant a
            // fresh bounded cycle for the same immutable plan.
            cycle_starts
                .entry(item_id.clone())
                .or_insert(row.attempts_used);
            store.append_event(
                plan_id,
                Some(item_id),
                None,
                "item_claimed",
                Some(&serde_json::json!({ "from": row.status }).to_string()),
            )?;
            if matches!(mode, RunMode::Resume)
                && matches!(
                    row.status.as_str(),
                    ITEM_EXECUTION_FAILED | ITEM_CANCELLED | ITEM_BLOCKED
                )
            {
                store.append_event(
                    plan_id,
                    Some(item_id),
                    None,
                    "item_reopened",
                    Some(&serde_json::json!({ "from": row.status }).to_string()),
                )?;
            }

            // ---- attempt reservation -------------------------------------
            attempt_no = store.next_attempt_no(plan_id, item_id)?;
            let dir = attempt_dir(runs_dir, item_id, attempt_no);
            reserve_attempt_dir(&dir, store, plan_id, item_id, attempt_no)?;

            let invocation = WorkerInvocation {
                schema: crate::worker::WORKER_SPEC_SCHEMA.to_string(),
                plan_id: plan_id.to_string(),
                item_id: item_id.clone(),
                attempt_no,
                worker: plan
                    .items
                    .iter()
                    .find(|i| &i.item_id == item_id)
                    .map(|i| i.worker.clone())
                    .ok_or_else(|| {
                        ExecError::Conflict(format!("item {item_id:?} missing in plan"))
                    })?,
                inputs: plan
                    .items
                    .iter()
                    .find(|i| &i.item_id == item_id)
                    .map(|i| i.inputs.clone())
                    .unwrap_or_default(),
            };
            let spec_path = dir.join("worker_spec.json");
            std::fs::write(
                &spec_path,
                serde_json::to_string_pretty(&invocation).map_err(|source| ExecError::Json {
                    context: "worker spec".into(),
                    source,
                })?,
            )?;

            let owner_token = timeutil::new_owner_token();
            if !store.insert_attempt(plan_id, item_id, attempt_no, &dir, &owner_token)? {
                return Err(ExecError::Conflict(format!(
                    "attempt {item_id}#{attempt_no} already reserved by another process"
                )));
            }
            store.bump_attempts_used(plan_id, item_id, attempt_no)?;
            store.append_event(
                plan_id,
                Some(item_id),
                Some(attempt_no),
                "attempt_reserved",
                Some(&serde_json::json!({ "dir": dir.to_string_lossy() }).to_string()),
            )?;

            if opts.inject_crash_after_reserve {
                eprintln!(
                    "[inject] crash after reservation of attempt {item_id}#{attempt_no}"
                );
                std::process::exit(exit_codes::INJECTED_CRASH);
            }

            // ---- cross-process admission (heavy worker limit = 1) --------
            let slot_path = slot_lock_path(runs_dir);
            let _slot = match FileLock::acquire_with_wait(&slot_path, opts.slot_wait_ms) {
                Ok(lock) => lock,
                Err(LockError::Busy { .. }) => {
                    // Release our claim; we did NOT start a second worker.
                    store.set_item_status(
                        plan_id,
                        item_id,
                        ITEM_PENDING,
                        Some(&[ITEM_RUNNING]),
                        None,
                        None,
                    )?;
                    store.append_event(
                        plan_id,
                        Some(item_id),
                        Some(attempt_no),
                        "slot_busy_released",
                        None,
                    )?;
                    return Err(ExecError::SlotBusy {
                        waited_ms: opts.slot_wait_ms,
                    });
                }
                Err(e) => return Err(ExecError::Lock(e)),
            };

            // ---- spawn the worker process --------------------------------
            let exe = std::env::current_exe()?;
            let mut child = Command::new(&exe)
                .arg("worker")
                .arg("--spec")
                .arg(&spec_path)
                .arg("--attempt-dir")
                .arg(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|source| ExecError::Io(std::io::Error::new(
                    source.kind(),
                    format!("spawn worker failed: {source}"),
                )))?;
            let worker_pid = child.id();

            // reserved -> running; if we lose this CAS, cancel already
            // finalized the attempt: kill our own child and stop.
            if !store.attempt_transition(
                plan_id,
                item_id,
                attempt_no,
                &[ATTEMPT_RESERVED],
                ATTEMPT_RUNNING,
                Some(worker_pid as i64),
                None,
                None,
                None,
            )? {
                kill_tree(worker_pid);
                let _ = child.wait();
                store.append_event(
                    plan_id,
                    Some(item_id),
                    Some(attempt_no),
                    "attempt_arbitration_lost",
                    Some(&serde_json::json!({ "pid": worker_pid }).to_string()),
                )?;
                continue 'items;
            }
            store.append_event(
                plan_id,
                Some(item_id),
                Some(attempt_no),
                "attempt_started",
                Some(&serde_json::json!({ "pid": worker_pid }).to_string()),
            )?;

            // ---- wait (bounded), then finalize with CAS fencing ----------
            let status = wait_with_timeout(&mut child, plan.limits.attempt_timeout_ms)?;
            let code = status.code();
            let result = read_worker_result(&dir);
            let verdict = classify_exit(code, result.as_ref());

            let (attempt_state, failure_kind) = match &verdict {
                Verdict::Success { .. } => (ATTEMPT_SUCCEEDED, None),
                Verdict::ValidationFailed { .. } => (store::ATTEMPT_FAILED_PERMANENT, Some("validation")),
                Verdict::Permanent { kind, .. } => (store::ATTEMPT_FAILED_PERMANENT, Some(kind.as_str())),
                Verdict::Fenced => (store::ATTEMPT_FAILED_PERMANENT, Some("fenced")),
                Verdict::Transient { .. } | Verdict::Crash { .. } => {
                    (store::ATTEMPT_FAILED_TRANSIENT, Some("transient"))
                }
            };
            let result_json = result
                .as_ref()
                .map(|r| serde_json::to_string(r).unwrap_or_default());
            let reason_text = verdict.reason_text();

            // Fencing commit: only the process that CASes `running -> terminal`
            // owns the outcome. A cancelled or stale attempt loses here.
            let finalized = store.attempt_transition(
                plan_id,
                item_id,
                attempt_no,
                &[ATTEMPT_RUNNING],
                attempt_state,
                None,
                failure_kind,
                result_json.as_deref(),
                Some(&reason_text),
            )?;
            if !finalized {
                store.append_event(
                    plan_id,
                    Some(item_id),
                    Some(attempt_no),
                    "attempt_already_finalized_by_other",
                    Some(&serde_json::json!({ "pid": worker_pid }).to_string()),
                )?;
                continue 'items;
            }
            store.append_event(
                plan_id,
                Some(item_id),
                Some(attempt_no),
                "attempt_finished",
                Some(&serde_json::json!({
                    "state": attempt_state,
                    "exit_code": code,
                    "reason": reason_text,
                }).to_string()),
            )?;

            match verdict {
                Verdict::Success { outcome } => {
                    let item_status = if outcome == "awaiting_review" {
                        ITEM_AWAITING_REVIEW
                    } else {
                        ITEM_BUILT
                    };
                    store.set_item_status(
                        plan_id,
                        item_id,
                        item_status,
                        Some(&[ITEM_RUNNING]),
                        None,
                        None,
                    )?;
                    store.append_event(
                        plan_id,
                        Some(item_id),
                        Some(attempt_no),
                        "item_outcome",
                        Some(&serde_json::json!({ "status": item_status }).to_string()),
                    )?;
                    continue 'items;
                }
                Verdict::ValidationFailed { reason } => {
                    store.set_item_status(
                        plan_id,
                        item_id,
                        ITEM_VALIDATION_FAILED,
                        Some(&[ITEM_RUNNING]),
                        None,
                        Some(&reason),
                    )?;
                    store.append_event(
                        plan_id,
                        Some(item_id),
                        Some(attempt_no),
                        "item_outcome",
                        Some(&serde_json::json!({ "status": ITEM_VALIDATION_FAILED }).to_string()),
                    )?;
                    continue 'items;
                }
                Verdict::Permanent { kind, reason } => {
                    store.set_item_status(
                        plan_id,
                        item_id,
                        ITEM_EXECUTION_FAILED,
                        Some(&[ITEM_RUNNING]),
                        None,
                        Some(&reason),
                    )?;
                    store.append_event(
                        plan_id,
                        Some(item_id),
                        Some(attempt_no),
                        "item_outcome",
                        Some(&serde_json::json!({
                            "status": ITEM_EXECUTION_FAILED,
                            "kind": kind,
                        }).to_string()),
                    )?;
                    if kind == "io" {
                        // Disk/storage unreliable: stop launching new work,
                        // keep everything already committed intact.
                        stop_all = Some(reason.clone());
                    }
                    continue 'items;
                }
                Verdict::Fenced => {
                    store.set_item_status(
                        plan_id,
                        item_id,
                        ITEM_EXECUTION_FAILED,
                        Some(&[ITEM_RUNNING]),
                        None,
                        Some("fencing violation: attempt lock was already held"),
                    )?;
                    continue 'items;
                }
                Verdict::Transient { reason } | Verdict::Crash { reason } => {
                    let cycle_start = cycle_starts.get(item_id).copied().unwrap_or(0);
                    if attempt_no - cycle_start < plan.limits.max_attempts {
                        let backoff = backoff_ms(&plan.limits, attempt_no);
                        // Give the item back so the loop claims it again with
                        // a NEW attempt number and directory.
                        if !store.set_item_status(
                            plan_id,
                            item_id,
                            ITEM_PENDING,
                            Some(&[ITEM_RUNNING]),
                            None,
                            None,
                        )? {
                            // Cancelled meanwhile.
                            continue 'items;
                        }
                        store.append_event(
                            plan_id,
                            Some(item_id),
                            Some(attempt_no),
                            "retry_scheduled",
                            Some(&serde_json::json!({
                                "next_attempt": attempt_no + 1,
                                "backoff_ms": backoff,
                                "reason": reason,
                            }).to_string()),
                        )?;
                        std::thread::sleep(Duration::from_millis(backoff));
                        continue; // next attempt for the same item
                    }
                    store.set_item_status(
                        plan_id,
                        item_id,
                        ITEM_EXECUTION_FAILED,
                        Some(&[ITEM_RUNNING]),
                        None,
                        Some(&format!("retries exhausted after attempt {attempt_no}: {reason}")),
                    )?;
                    store.append_event(
                        plan_id,
                        Some(item_id),
                        Some(attempt_no),
                        "item_outcome",
                        Some(&serde_json::json!({
                            "status": ITEM_EXECUTION_FAILED,
                            "kind": "retries_exhausted",
                        }).to_string()),
                    )?;
                    continue 'items;
                }
            }
        }
    }

    let final_status = if stop_all.is_some() {
        if store.set_batch_status(plan_id, BATCH_FAILED, Some(&[BATCH_RUNNING]))? {
            store.append_event(
                plan_id,
                None,
                None,
                "batch_stopped_io",
                Some(&serde_json::json!({ "reason": stop_all }).to_string()),
            )?;
        }
        BATCH_FAILED.to_string()
    } else {
        // May lose to cancel's CAS; the cancelled status then stands.
        store.set_batch_status(plan_id, BATCH_COMPLETED, Some(&[BATCH_RUNNING]))?;
        store
            .batch_status(plan_id)?
            .unwrap_or_else(|| BATCH_COMPLETED.to_string())
    };
    store.append_event(
        plan_id,
        None,
        None,
        "batch_finished",
        Some(&serde_json::json!({ "status": final_status }).to_string()),
    )?;

    let mut report = BatchReport::build(store, plan, plan_id, fingerprint)?;
    report.warnings = warnings;
    if let Some(reason) = stop_all {
        report.warnings.push(format!(
            "further item launches stopped after IO/storage failure: {reason}"
        ));
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Resume reconciliation
// ---------------------------------------------------------------------------

/// Before resume/retry: verify input fingerprints, then reconcile attempts
/// whose owner died. Returns honest warnings and the set of items whose
/// inputs currently drift from the registered fingerprints.
fn reconcile_before_resume(
    store: &Store,
    plan: &BatchPlan,
    plan_id: &str,
) -> Result<(Vec<String>, std::collections::BTreeSet<String>), ExecError> {
    let mut warnings = Vec::new();

    let drifted: std::collections::BTreeSet<String> =
        plan.verify_input_fingerprints()?.into_iter().collect();
    for item_id in &drifted {
        let marked = store.set_item_status(
            plan_id,
            item_id,
            ITEM_BLOCKED,
            Some(&[ITEM_PENDING, ITEM_RUNNING, ITEM_EXECUTION_FAILED, ITEM_CANCELLED]),
            Some("input_drift: actual input files no longer match the registered fingerprints"),
            None,
        )?;
        if marked {
            warnings.push(format!("item {item_id} blocked: input files drifted"));
            store.append_event(
                plan_id,
                Some(item_id),
                None,
                "input_drift_blocked",
                None,
            )?;
        }
    }

    for attempt in store.list_live_attempts(plan_id)? {
        let lock_path = attempt_lock_path(Path::new(&attempt.dir));
        if FileLock::is_locked_elsewhere(&lock_path)? {
            let w = format!(
                "attempt {}#{} appears live (owner_pid={:?}); not starting a second worker for it",
                attempt.item_id, attempt.attempt_no, attempt.owner_pid
            );
            warnings.push(w.clone());
            store.append_event(
                plan_id,
                Some(&attempt.item_id),
                Some(attempt.attempt_no),
                "attempt_live_elsewhere",
                Some(&serde_json::json!({ "owner_pid": attempt.owner_pid }).to_string()),
            )?;
            continue;
        }
        // No OS lock owner => the recorded owner is dead: crash recovery.
        let interrupted = store.attempt_transition(
            plan_id,
            &attempt.item_id,
            attempt.attempt_no,
            &[ATTEMPT_RESERVED, ATTEMPT_RUNNING],
            ATTEMPT_INTERRUPTED,
            None,
            Some("crash"),
            None,
            Some("previous owner died (crash recovery)"),
        )?;
        if interrupted {
            store.set_item_status(
                plan_id,
                &attempt.item_id,
                ITEM_EXECUTION_FAILED,
                Some(&[ITEM_RUNNING]),
                None,
                Some(&format!(
                    "interrupted: attempt {} owner died; artifacts not promoted",
                    attempt.attempt_no
                )),
            )?;
            store.append_event(
                plan_id,
                Some(&attempt.item_id),
                Some(attempt.attempt_no),
                "attempt_interrupted",
                None,
            )?;
            warnings.push(format!(
                "attempt {}#{} marked interrupted (owner died)",
                attempt.item_id, attempt.attempt_no
            ));
        }
    }
    let _ = plan; // fingerprints already verified above
    Ok((warnings, drifted))
}

enum DepGate {
    Ok,
    Skip,
    Block(String),
}

fn dep_gate(
    store: &Store,
    plan_id: &str,
    plan: &BatchPlan,
    item_id: &str,
) -> Result<DepGate, ExecError> {
    let item = plan
        .items
        .iter()
        .find(|i| i.item_id == item_id)
        .ok_or_else(|| ExecError::Conflict(format!("item {item_id:?} missing in plan")))?;
    for dep in &item.depends_on {
        let row = store.get_item(plan_id, dep)?;
        let status = row.map(|r| r.status).unwrap_or_default();
        if ITEM_SUCCESS_STATES.contains(&status.as_str()) {
            continue;
        }
        match status.as_str() {
            ITEM_PENDING | ITEM_RUNNING => return Ok(DepGate::Skip),
            "" => {
                return Ok(DepGate::Block(format!(
                    "dependency {dep:?} missing in store"
                )))
            }
            other => {
                return Ok(DepGate::Block(format!(
                    "dependency {dep:?} ended as {other:?}; dependent stage not started"
                )))
            }
        }
    }
    Ok(DepGate::Ok)
}

// ---------------------------------------------------------------------------
// Attempt directory reservation (atomic, no overwrite)
// ---------------------------------------------------------------------------

fn reserve_attempt_dir(
    dir: &Path,
    store: &Store,
    plan_id: &str,
    item_id: &str,
    attempt_no: u32,
) -> Result<(), ExecError> {
    // Parents (runs root / per-item dir) are plain infrastructure; the leaf
    // attempt directory itself is the atomic reservation unit.
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            // A previous runner may have crashed between mkdir and the DB
            // insert. Never overwrite: preserve the old directory under an
            // orphan name, then reserve a fresh one.
            if store.get_attempt(plan_id, item_id, attempt_no)?.is_none() {
                let orphan = dir.with_file_name(format!(
                    "attempt_{:03}.orphan-{}",
                    attempt_no,
                    &timeutil::new_owner_token()[..12]
                ));
                std::fs::rename(dir, &orphan)?;
                store.append_event(
                    plan_id,
                    Some(item_id),
                    Some(attempt_no),
                    "orphan_attempt_dir_renamed",
                    Some(&serde_json::json!({
                        "from": dir.to_string_lossy(),
                        "to": orphan.to_string_lossy(),
                    }).to_string()),
                )?;
                std::fs::create_dir(dir).map_err(|e| {
                    ExecError::Conflict(format!(
                        "cannot reserve attempt dir {} after orphan rename: {e}",
                        dir.display()
                    ))
                })?;
                Ok(())
            } else {
                Err(ExecError::Conflict(format!(
                    "attempt dir {} already exists with a DB row; refusing to overwrite",
                    dir.display()
                )))
            }
        }
        Err(e) => Err(ExecError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// Worker wait / verdicts
// ---------------------------------------------------------------------------

fn wait_with_timeout(child: &mut Child, timeout_ms: u64) -> Result<std::process::ExitStatus, ExecError> {
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status),
            None => {}
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            kill_tree(child.id());
            return Ok(child.wait()?);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Kill the whole owned process tree. On Windows `taskkill /T /F` walks the
/// tree; the PoC worker never spawns grandchildren, but the tree-kill is used
/// anyway so the semantics hold for future real workers on this platform.
/// (A hardened implementation would use Windows Job Objects; see
/// INTEGRATION_NOTES.md.)
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[derive(Debug, Clone)]
enum Verdict {
    Success { outcome: String },
    ValidationFailed { reason: String },
    Permanent { kind: String, reason: String },
    Fenced,
    Transient { reason: String },
    Crash { reason: String },
}

impl Verdict {
    fn reason_text(&self) -> String {
        match self {
            Verdict::Success { outcome } => format!("worker success ({outcome})"),
            Verdict::ValidationFailed { reason } => format!("validation failed: {reason}"),
            Verdict::Permanent { kind, reason } => format!("permanent failure ({kind}): {reason}"),
            Verdict::Fenced => "fencing violation".to_string(),
            Verdict::Transient { reason } => format!("transient failure: {reason}"),
            Verdict::Crash { reason } => format!("worker crash: {reason}"),
        }
    }
}

fn read_worker_result(dir: &Path) -> Option<WorkerResult> {
    let raw = std::fs::read(dir.join("result.json")).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn classify_exit(code: Option<i32>, result: Option<&WorkerResult>) -> Verdict {
    let reason = |default: &str| -> String {
        result
            .and_then(|r| r.reason.clone())
            .unwrap_or_else(|| default.to_string())
    };
    match code {
        Some(worker_exit::SUCCESS) => match result {
            Some(r) if r.outcome == "built" || r.outcome == "awaiting_review" => {
                Verdict::Success {
                    outcome: r.outcome.clone(),
                }
            }
            Some(r) => Verdict::Crash {
                reason: format!("exit 0 with unexpected outcome {:?}", r.outcome),
            },
            None => Verdict::Crash {
                reason: "exit 0 without result.json".to_string(),
            },
        },
        Some(worker_exit::TRANSIENT) => Verdict::Transient {
            reason: reason("worker reported transient failure"),
        },
        Some(worker_exit::PERMANENT) => {
            let kind = result
                .and_then(|r| r.failure_kind.clone())
                .unwrap_or_else(|| "invalid_input".to_string());
            Verdict::Permanent {
                kind,
                reason: reason("worker reported permanent failure"),
            }
        }
        Some(worker_exit::VALIDATION_FAILED) => Verdict::ValidationFailed {
            reason: reason("worker reported validation failure"),
        },
        Some(worker_exit::FENCED) => Verdict::Fenced,
        _ => Verdict::Crash {
            reason: match code {
                Some(c) => format!("worker exited with code {c}"),
                None => "worker terminated by signal".to_string(),
            },
        },
    }
}

fn backoff_ms(limits: &crate::plan::PlanLimits, attempt_no: u32) -> u64 {
    let exp = attempt_no.saturating_sub(1).min(16);
    limits
        .backoff_base_ms
        .saturating_mul(1u64 << exp)
        .min(limits.backoff_cap_ms.max(limits.backoff_base_ms))
}

// ---------------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------------

/// Cancel a whole batch or selected items. Kills the owned worker process
/// tree of live attempts, marks dependent stages so they never start, and
/// never touches successful outcomes.
pub fn cancel_batch(
    plan_id: &str,
    db_path: &Path,
    item_filter: Option<&str>,
) -> Result<BatchReport, ExecError> {
    let store = Store::open(db_path)?;
    let (plan_json, fingerprint, _status) = store.load_plan_json(plan_id)?;
    let plan: BatchPlan = serde_json::from_str(&plan_json).map_err(|source| ExecError::Json {
        context: "stored plan".into(),
        source,
    })?;

    let targets: Vec<String> = match item_filter {
        Some(id) => vec![id.to_string()],
        None => plan.items.iter().map(|i| i.item_id.clone()).collect(),
    };
    store.append_event(
        plan_id,
        None,
        None,
        "batch_cancel_requested",
        Some(&serde_json::json!({ "items": targets }).to_string()),
    )?;

    for attempt in store.list_live_attempts(plan_id)? {
        if !targets.contains(&attempt.item_id) {
            continue;
        }
        let lock_path = attempt_lock_path(Path::new(&attempt.dir));
        // Only kill when the OS lock proves a live owner. A free lock means
        // the recorded pid is stale; killing it could hit an unrelated,
        // reused pid.
        if FileLock::is_locked_elsewhere(&lock_path)? {
            if let Some(pid) = attempt.owner_pid {
                store.append_event(
                    plan_id,
                    Some(&attempt.item_id),
                    Some(attempt.attempt_no),
                    "worker_kill_requested",
                    Some(&serde_json::json!({ "pid": pid }).to_string()),
                )?;
                kill_tree(pid as u32);
                wait_lock_free(&lock_path, 15_000)?;
            }
        }
        let cancelled = store.attempt_transition(
            plan_id,
            &attempt.item_id,
            attempt.attempt_no,
            &[ATTEMPT_RESERVED, ATTEMPT_RUNNING],
            ATTEMPT_CANCELLED,
            None,
            Some("cancel"),
            None,
            Some("cancelled by user"),
        )?;
        if cancelled {
            store.set_item_status(
                plan_id,
                &attempt.item_id,
                ITEM_CANCELLED,
                Some(&[ITEM_RUNNING]),
                None,
                Some("cancelled while running"),
            )?;
            store.append_event(
                plan_id,
                Some(&attempt.item_id),
                Some(attempt.attempt_no),
                "item_outcome",
                Some(&serde_json::json!({ "status": ITEM_CANCELLED }).to_string()),
            )?;
        }
    }

    // Pending/blocked items never start after cancel.
    for id in &targets {
        store.set_item_status(
            plan_id,
            id,
            ITEM_CANCELLED,
            Some(&[ITEM_PENDING, ITEM_BLOCKED]),
            None,
            Some("cancelled before start"),
        )?;
    }

    if item_filter.is_none() {
        store.set_batch_status(plan_id, BATCH_CANCELLED, None)?;
        store.append_event(
            plan_id,
            None,
            None,
            "batch_cancelled",
            None,
        )?;
    }

    BatchReport::build(&store, &plan, plan_id, &fingerprint).map_err(ExecError::from)
}

fn wait_lock_free(path: &Path, timeout_ms: u64) -> Result<(), ExecError> {
    let start = Instant::now();
    loop {
        if !FileLock::is_locked_elsewhere(path)? {
            return Ok(());
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Err(ExecError::Conflict(format!(
                "lock still held after kill: {}",
                path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

// ---------------------------------------------------------------------------
// Status (read-only)
// ---------------------------------------------------------------------------

pub fn read_status(plan_id: &str, db_path: &Path) -> Result<BatchReport, ExecError> {
    let store = Store::open(db_path)?;
    let (plan_json, fingerprint, _status) = store.load_plan_json(plan_id)?;
    let plan: BatchPlan = serde_json::from_str(&plan_json).map_err(|source| ExecError::Json {
        context: "stored plan".into(),
        source,
    })?;
    BatchReport::build(&store, &plan, plan_id, &fingerprint).map_err(ExecError::from)
}
