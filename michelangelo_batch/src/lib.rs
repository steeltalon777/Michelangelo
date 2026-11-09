//! michelangelo_batch — ADR 0005 stream C PoC: durable batch executor, fake worker first.
//!
//! PROVISIONAL PoC. This crate is a standalone workspace (not a member of the
//! repository root Cargo workspace). It does NOT modify the existing jobs
//! scheduler, Workspace migrations, Canonical Publisher, IT-8D accept or CLI
//! routing. Nothing here is production-ready, UE-ready or a GPU-safe sandbox.
//!
//! Scope implemented (ADR 0005 §§5, 7, 9, 11.4, fake worker only):
//! - Immutable resolved plan identity: plan fingerprint pinned at registration;
//!   a different payload under the same plan id is rejected.
//! - Durable batch/item/attempt transitions in a dedicated disposable SQLite
//!   database (separate from the project Workspace DB).
//! - Atomic reservation of unique per-attempt directories, never overwritten;
//!   every retry gets a fresh attempt number and directory.
//! - Cross-process admission for the heavy worker slot (limit 1) using an OS
//!   file lock, which the kernel releases when the owner process dies. A single
//!   in-memory mutex is explicitly NOT sufficient and is not used for this.
//! - Fencing: each attempt directory carries its own lock file; a finalize is
//!   committed by a compare-and-set transition, so a late/orphaned old attempt
//!   cannot publish over newer state.
//! - Bounded retry with backoff for transient failures only; invalid input and
//!   validation/quality failures are never auto-retried.
//! - Resume re-verifies actual input file fingerprints; already-successful
//!   items are never reset back to pending/running.
//! - Distinct domain outcomes: built, awaiting_review, validation_failed,
//!   execution_failed, blocked, cancelled (plus pending/running as execution
//!   axis). "Artifact file exists" is never treated as accepted.
//!
//! `wait=false` semantics of the existing jobs scheduler are NOT a batch
//! runner; this crate is an explicit, separate execution path.

pub mod executor;
pub mod locks;
pub mod plan;
pub mod report;
pub mod store;
pub mod timeutil;
pub mod worker;

pub use executor::{cancel_batch, execute, RunMode};
pub use plan::{BatchPlan, FailureMode, PlanError, PlanItem, PlanLimits, WorkerSpec};
pub use report::{BatchReport, ItemReport, REPORT_SCHEMA};
pub use store::{
    ItemRow, Store, StoreError, ATTEMPT_CANCELLED, ATTEMPT_FAILED_PERMANENT,
    ATTEMPT_FAILED_TRANSIENT, ATTEMPT_INTERRUPTED, ATTEMPT_RESERVED, ATTEMPT_RUNNING,
    ATTEMPT_SUCCEEDED, ITEM_AWAITING_REVIEW, ITEM_BLOCKED, ITEM_BUILT, ITEM_CANCELLED,
    ITEM_EXECUTION_FAILED, ITEM_PENDING, ITEM_RUNNING, ITEM_VALIDATION_FAILED,
};

/// Exit codes used by the `mbatch` CLI (provisional PoC contract).
pub mod exit_codes {
    /// All targeted items reached a successful domain outcome.
    pub const OK: i32 = 0;
    /// Unexpected internal error.
    pub const INTERNAL: i32 = 1;
    /// Run finished but at least one item ended in a non-success outcome.
    pub const PARTIAL_FAILURE: i32 = 2;
    /// Heavy worker slot busy (no second worker was started).
    pub const SLOT_BUSY: i32 = 3;
    /// Invalid plan, invalid arguments or immutable-plan violation.
    pub const USAGE: i32 = 4;
    /// Injected crash (test-only).
    pub const INJECTED_CRASH: i32 = 70;
}
