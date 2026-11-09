//! Durable state for the batch PoC in a dedicated disposable SQLite database.
//!
//! Deliberately separate from the project Workspace storage: no shared tables,
//! no migration of the existing schema. All status changes are guarded by
//! compare-and-set (CAS) updates so concurrent runner/cancel/status processes
//! arbitrate without double transitions. Multi-process access uses WAL plus a
//! busy timeout.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use crate::timeutil;

// ---------------------------------------------------------------------------
// Status vocabularies (execution axis vs domain outcome axis)
// ---------------------------------------------------------------------------

pub const ITEM_PENDING: &str = "pending";
pub const ITEM_RUNNING: &str = "running";
pub const ITEM_BUILT: &str = "built";
pub const ITEM_AWAITING_REVIEW: &str = "awaiting_review";
pub const ITEM_VALIDATION_FAILED: &str = "validation_failed";
pub const ITEM_EXECUTION_FAILED: &str = "execution_failed";
pub const ITEM_BLOCKED: &str = "blocked";
pub const ITEM_CANCELLED: &str = "cancelled";

/// Domain outcomes that count as "successful execution" for dependency gating.
pub const ITEM_SUCCESS_STATES: &[&str] = &[ITEM_BUILT, ITEM_AWAITING_REVIEW];
/// Terminal item states (no further transitions except explicit resume).
pub const ITEM_TERMINAL_STATES: &[&str] = &[
    ITEM_BUILT,
    ITEM_AWAITING_REVIEW,
    ITEM_VALIDATION_FAILED,
    ITEM_EXECUTION_FAILED,
    ITEM_BLOCKED,
    ITEM_CANCELLED,
];

pub const ATTEMPT_RESERVED: &str = "reserved";
pub const ATTEMPT_RUNNING: &str = "running";
pub const ATTEMPT_SUCCEEDED: &str = "succeeded";
pub const ATTEMPT_FAILED_TRANSIENT: &str = "failed_transient";
pub const ATTEMPT_FAILED_PERMANENT: &str = "failed_permanent";
pub const ATTEMPT_CANCELLED: &str = "cancelled";
pub const ATTEMPT_INTERRUPTED: &str = "interrupted";

pub const BATCH_RUNNING: &str = "running";
pub const BATCH_COMPLETED: &str = "completed";
pub const BATCH_CANCELLED: &str = "cancelled";
pub const BATCH_FAILED: &str = "failed";

/// Honest reason recorded for an item that was left `running` by a runner
/// that died before its attempt row was written.
pub const RUNNING_WITHOUT_ATTEMPT_REASON: &str =
    "orphaned running state: no live attempt (runner died before the attempt was recorded)";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum StoreError {
    Sqlite {
        context: String,
        source: rusqlite::Error,
    },
    Serde {
        context: String,
        source: serde_json::Error,
    },
    Io(std::io::Error),
    PlanNotFound {
        plan_id: String,
    },
    PlanImmutable {
        plan_id: String,
    },
    Inconsistent {
        context: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite { context, source } => {
                write!(f, "sqlite error ({context}): {source}")
            }
            StoreError::Serde { context, source } => {
                write!(f, "serde error ({context}): {source}")
            }
            StoreError::Io(e) => write!(f, "store io error: {e}"),
            StoreError::PlanNotFound { plan_id } => write!(f, "batch plan {plan_id:?} not found"),
            StoreError::PlanImmutable { plan_id } => write!(
                f,
                "plan id {plan_id:?} is already registered with a different payload (immutable)"
            ),
            StoreError::Inconsistent { context } => write!(f, "inconsistent state: {context}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

fn sql_err(context: impl Into<String>) -> impl FnOnce(rusqlite::Error) -> StoreError {
    let context = context.into();
    move |source| StoreError::Sqlite { context, source }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ItemRow {
    pub plan_id: String,
    pub item_id: String,
    pub status: String,
    pub blocked_reason: Option<String>,
    pub attempts_used: u32,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct AttemptRow {
    pub plan_id: String,
    pub item_id: String,
    pub attempt_no: u32,
    pub dir: String,
    pub state: String,
    pub failure_kind: Option<String>,
    pub owner_pid: Option<i64>,
    pub owner_token: Option<String>,
    pub worker_result_json: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

/// One row of the append-only event log: (ts, item_id, attempt_no, kind).
pub type EventRow = (String, String, Option<u32>, String);

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub struct Store {
    conn: Connection,
}

/// Result of registering a plan payload under a plan id.
#[derive(Debug, PartialEq, Eq)]
pub enum RegisterOutcome {
    Created,
    AlreadyPresent,
}

impl Store {
    pub fn open(db_path: &Path) -> Result<Store, StoreError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path).map_err(sql_err("open"))?;
        conn.busy_timeout(Duration::from_millis(5000))
            .map_err(sql_err("busy_timeout"))?;
        // WAL: concurrent readers (status) during writer (runner/cancel).
        let _mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(sql_err("journal_mode"))?;
        let store = Store { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), StoreError> {
        // Only take the write lock when the schema is actually missing:
        // read-only users (status from another process) must never contend
        // with writers just to re-assert CREATE TABLE IF NOT EXISTS.
        let have_tables: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('mb_batches','mb_items','mb_attempts','mb_events')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_err("probe schema"))?;
        if have_tables == 4 {
            return Ok(());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(sql_err("begin schema tx"))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS mb_batches (
                plan_id          TEXT PRIMARY KEY,
                plan_fingerprint TEXT NOT NULL,
                plan_json        TEXT NOT NULL,
                status           TEXT NOT NULL,
                created_at       TEXT NOT NULL,
                updated_at       TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS mb_items (
                plan_id        TEXT NOT NULL,
                item_id        TEXT NOT NULL,
                status         TEXT NOT NULL,
                blocked_reason TEXT,
                attempts_used  INTEGER NOT NULL DEFAULT 0,
                last_error     TEXT,
                updated_at     TEXT NOT NULL,
                PRIMARY KEY (plan_id, item_id)
            );
            CREATE TABLE IF NOT EXISTS mb_attempts (
                plan_id            TEXT NOT NULL,
                item_id            TEXT NOT NULL,
                attempt_no         INTEGER NOT NULL,
                dir                TEXT NOT NULL,
                state              TEXT NOT NULL,
                failure_kind       TEXT,
                owner_pid          INTEGER,
                owner_token        TEXT,
                worker_result_json TEXT,
                error              TEXT,
                created_at         TEXT NOT NULL,
                finished_at        TEXT,
                PRIMARY KEY (plan_id, item_id, attempt_no)
            );
            CREATE TABLE IF NOT EXISTS mb_events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                ts           TEXT NOT NULL,
                plan_id      TEXT NOT NULL,
                item_id      TEXT,
                attempt_no   INTEGER,
                kind         TEXT NOT NULL,
                payload_json TEXT
            );",
        )
        .map_err(sql_err("create schema"))?;
        tx.commit().map_err(sql_err("commit schema"))?;
        Ok(())
    }

    fn tx(&self) -> Result<rusqlite::Transaction<'_>, StoreError> {
        self.conn
            .unchecked_transaction()
            .map_err(sql_err("begin tx"))
    }

    // -- batch ---------------------------------------------------------------

    /// Register an immutable plan. If the plan id already exists, the payload
    /// fingerprint must match exactly; otherwise this is an immutability
    /// violation and is rejected.
    pub fn register_plan(
        &self,
        plan_id: &str,
        fingerprint: &str,
        plan_json: &str,
        item_ids: &[String],
    ) -> Result<RegisterOutcome, StoreError> {
        let tx = self.tx()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT plan_fingerprint FROM mb_batches WHERE plan_id = ?1",
                params![plan_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_err("select batch"))?;
        if let Some(existing_fp) = existing {
            if existing_fp != fingerprint {
                return Err(StoreError::PlanImmutable {
                    plan_id: plan_id.to_string(),
                });
            }
            return Ok(RegisterOutcome::AlreadyPresent);
        }
        let now = timeutil::now_iso8601();
        tx.execute(
            "INSERT INTO mb_batches (plan_id, plan_fingerprint, plan_json, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![plan_id, fingerprint, plan_json, BATCH_RUNNING, now],
        )
        .map_err(sql_err("insert batch"))?;
        for item_id in item_ids {
            tx.execute(
                "INSERT INTO mb_items (plan_id, item_id, status, attempts_used, updated_at)
                 VALUES (?1, ?2, ?3, 0, ?4)",
                params![plan_id, item_id, ITEM_PENDING, now],
            )
            .map_err(sql_err("insert item"))?;
        }
        tx.commit().map_err(sql_err("commit register"))?;
        Ok(RegisterOutcome::Created)
    }

    pub fn load_plan_json(&self, plan_id: &str) -> Result<(String, String, String), StoreError> {
        self.conn
            .query_row(
                "SELECT plan_json, plan_fingerprint, status FROM mb_batches WHERE plan_id = ?1",
                params![plan_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql_err("load plan"))?
            .ok_or_else(|| StoreError::PlanNotFound {
                plan_id: plan_id.to_string(),
            })
    }

    pub fn batch_status(&self, plan_id: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row(
                "SELECT status FROM mb_batches WHERE plan_id = ?1",
                params![plan_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_err("batch status"))
    }

    /// CAS the batch status. `expected` None = from any state.
    pub fn set_batch_status(
        &self,
        plan_id: &str,
        status: &str,
        expected: Option<&[&str]>,
    ) -> Result<bool, StoreError> {
        let now = timeutil::now_iso8601();
        let plan_id_s = plan_id.to_string();
        let status_s = status.to_string();
        let cond: Vec<String> = expected
            .unwrap_or(&[])
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sql = match expected {
            None => {
                "UPDATE mb_batches SET status = ?2, updated_at = ?3 WHERE plan_id = ?1".to_string()
            }
            Some(list) => {
                let placeholders = (0..list.len())
                    .map(|i| format!("?{}", i + 4))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "UPDATE mb_batches SET status = ?2, updated_at = ?3
                     WHERE plan_id = ?1 AND status IN ({placeholders})"
                )
            }
        };
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(sql_err("prepare batch cas"))?;
        let mut p: Vec<&dyn rusqlite::ToSql> = vec![&plan_id_s, &status_s, &now];
        for c in &cond {
            p.push(c);
        }
        let n = stmt.execute(p.as_slice()).map_err(sql_err("batch cas"))?;
        Ok(n > 0)
    }

    // -- items ---------------------------------------------------------------

    pub fn list_items(&self, plan_id: &str) -> Result<Vec<ItemRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_id, status, blocked_reason, attempts_used, last_error, updated_at
                 FROM mb_items WHERE plan_id = ?1 ORDER BY item_id",
            )
            .map_err(sql_err("prepare list items"))?;
        let rows = stmt
            .query_map(params![plan_id], |row| {
                Ok(ItemRow {
                    plan_id: plan_id.to_string(),
                    item_id: row.get(0)?,
                    status: row.get(1)?,
                    blocked_reason: row.get(2)?,
                    attempts_used: row.get::<_, i64>(3)? as u32,
                    last_error: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(sql_err("query items"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(sql_err("collect items"))
    }

    pub fn get_item(&self, plan_id: &str, item_id: &str) -> Result<Option<ItemRow>, StoreError> {
        self.conn
            .query_row(
                "SELECT item_id, status, blocked_reason, attempts_used, last_error, updated_at
                 FROM mb_items WHERE plan_id = ?1 AND item_id = ?2",
                params![plan_id, item_id],
                |row| {
                    Ok(ItemRow {
                        plan_id: plan_id.to_string(),
                        item_id: row.get(0)?,
                        status: row.get(1)?,
                        blocked_reason: row.get(2)?,
                        attempts_used: row.get::<_, i64>(3)? as u32,
                        last_error: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(sql_err("get item"))
    }

    /// CAS item status into `to`. `expected` None = from any state.
    pub fn set_item_status(
        &self,
        plan_id: &str,
        item_id: &str,
        to: &str,
        expected: Option<&[&str]>,
        blocked_reason: Option<&str>,
        last_error: Option<&str>,
    ) -> Result<bool, StoreError> {
        let now = timeutil::now_iso8601();
        let plan_id_s = plan_id.to_string();
        let item_id_s = item_id.to_string();
        let to_s = to.to_string();
        let blocked_s = blocked_reason.map(|s| s.to_string());
        let error_s = last_error.map(|s| s.to_string());
        let cond: Vec<String> = expected
            .unwrap_or(&[])
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut sql = String::from(
            "UPDATE mb_items SET status = ?3, blocked_reason = COALESCE(?4, blocked_reason), \
             last_error = COALESCE(?5, last_error), updated_at = ?6 WHERE plan_id = ?1 AND item_id = ?2",
        );
        if expected.is_some() {
            let placeholders = (0..cond.len())
                .map(|i| format!("?{}", i + 7))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" AND status IN ({placeholders})"));
        }
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(sql_err("prepare item cas"))?;
        let mut p: Vec<&dyn rusqlite::ToSql> =
            vec![&plan_id_s, &item_id_s, &to_s, &blocked_s, &error_s, &now];
        for c in &cond {
            p.push(c);
        }
        let n = stmt.execute(p.as_slice()).map_err(sql_err("item cas"))?;
        Ok(n > 0)
    }

    // -- attempts ------------------------------------------------------------

    pub fn next_attempt_no(&self, plan_id: &str, item_id: &str) -> Result<u32, StoreError> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(attempt_no), 0) + 1 FROM mb_attempts
                 WHERE plan_id = ?1 AND item_id = ?2",
                params![plan_id, item_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v as u32)
            .map_err(sql_err("next attempt no"))
    }

    /// Reserve an attempt row.
    ///
    /// The (plan, item, attempt_no) triple is the primary key, so a double
    /// reservation across processes is rejected by the database itself: the
    /// call fails with [`StoreError::Sqlite`] ("attempt already reserved")
    /// instead of silently overwriting the row of the first owner.
    pub fn insert_attempt(
        &self,
        plan_id: &str,
        item_id: &str,
        attempt_no: u32,
        dir: &Path,
        owner_token: &str,
    ) -> Result<(), StoreError> {
        self.insert_attempt_row(plan_id, item_id, attempt_no, dir, owner_token)
    }

    fn insert_attempt_row(
        &self,
        plan_id: &str,
        item_id: &str,
        attempt_no: u32,
        dir: &Path,
        owner_token: &str,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO mb_attempts (plan_id, item_id, attempt_no, dir, state, owner_token, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    plan_id,
                    item_id,
                    attempt_no as i64,
                    dir.to_string_lossy(),
                    ATTEMPT_RESERVED,
                    owner_token,
                    timeutil::now_iso8601()
                ],
            )
            .map_err(|e| {
                if matches!(
                    e.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::ConstraintViolation)
                ) {
                    // Unique PK conflict = someone reserved this attempt first.
                    StoreError::Sqlite {
                        context: "attempt already reserved".into(),
                        source: e,
                    }
                } else {
                    StoreError::Sqlite {
                        context: "insert attempt".into(),
                        source: e,
                    }
                }
            })?;
        Ok(())
    }

    /// Claim an item AND register its attempt row in ONE transaction.
    ///
    /// The item claim (`running`) and the attempt row are written atomically,
    /// so the limbo state "item running without an attempt row" is not
    /// representable: a runner that dies in this window rolls the claim back
    /// and leaves the item `pending`, instead of stranding it in `running`
    /// where no other runner could ever claim it again.
    ///
    /// `between_claim_and_insert` runs inside the transaction, after the
    /// claim and before the attempt row is inserted: it reserves the attempt
    /// directory and writes the worker spec (and hosts the crash injection).
    /// Database side effects are rolled back with the transaction; filesystem
    /// side effects are not, so a leftover attempt directory is preserved as
    /// an orphan by the next run instead of being overwritten.
    ///
    /// Returns false when the claim was lost: another owner moved the item.
    #[allow(clippy::too_many_arguments)]
    pub fn claim_item_with_attempt<F>(
        &self,
        plan_id: &str,
        item_id: &str,
        attempt_no: u32,
        dir: &Path,
        owner_token: &str,
        claim_allowed: &[&str],
        between_claim_and_insert: &mut F,
    ) -> Result<bool, StoreError>
    where
        F: FnMut() -> Result<(), StoreError>,
    {
        let tx = self.tx()?;
        let now = timeutil::now_iso8601();
        let plan_id_s = plan_id.to_string();
        let item_id_s = item_id.to_string();
        let running_s = ITEM_RUNNING.to_string();
        let cond: Vec<String> = claim_allowed.iter().map(|s| s.to_string()).collect();
        let placeholders = (0..cond.len())
            .map(|i| format!("?{}", i + 5))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE mb_items SET status = ?3, updated_at = ?4
              WHERE plan_id = ?1 AND item_id = ?2 AND status IN ({placeholders})"
        );
        {
            let mut stmt = self
                .conn
                .prepare(&sql)
                .map_err(sql_err("prepare claim cas"))?;
            let mut p: Vec<&dyn rusqlite::ToSql> = vec![&plan_id_s, &item_id_s, &running_s, &now];
            for c in &cond {
                p.push(c);
            }
            let n = stmt.execute(p.as_slice()).map_err(sql_err("claim cas"))?;
            if n == 0 {
                // Lost arbitration: roll the transaction back so nothing is
                // written for an item we do not own.
                let _ = tx.rollback();
                return Ok(false);
            }
        }
        // Attempt reservation (filesystem) inside the transaction.
        between_claim_and_insert()?;
        // The attempt row: same transaction, so the item can never be
        // `running` without an attempt.
        self.insert_attempt_row(plan_id, item_id, attempt_no, dir, owner_token)?;
        tx.commit().map_err(sql_err("commit claim"))?;
        Ok(true)
    }

    pub fn bump_attempts_used(
        &self,
        plan_id: &str,
        item_id: &str,
        attempt_no: u32,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE mb_items SET attempts_used = ?3, updated_at = ?4
                 WHERE plan_id = ?1 AND item_id = ?2",
                params![plan_id, item_id, attempt_no as i64, timeutil::now_iso8601()],
            )
            .map_err(sql_err("bump attempts"))?;
        Ok(())
    }

    /// CAS an attempt between states; optionally records pid/result/kind.
    #[allow(clippy::too_many_arguments)]
    pub fn attempt_transition(
        &self,
        plan_id: &str,
        item_id: &str,
        attempt_no: u32,
        from: &[&str],
        to: &str,
        owner_pid: Option<i64>,
        failure_kind: Option<&str>,
        worker_result_json: Option<&str>,
        error: Option<&str>,
    ) -> Result<bool, StoreError> {
        let now = timeutil::now_iso8601();
        let plan_id_s = plan_id.to_string();
        let item_id_s = item_id.to_string();
        let attempt_no_s = attempt_no.to_string();
        let to_s = to.to_string();
        let kind_s = failure_kind.map(|s| s.to_string());
        let result_s = worker_result_json.map(|s| s.to_string());
        let error_s = error.map(|s| s.to_string());
        let cond: Vec<String> = from.iter().map(|s| s.to_string()).collect();
        let now_idx = 9 + cond.len();
        let placeholders = (0..cond.len())
            .map(|i| format!("?{}", i + 9))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE mb_attempts SET state = ?4, owner_pid = COALESCE(?5, owner_pid), \
             failure_kind = COALESCE(?6, failure_kind), \
             worker_result_json = COALESCE(?7, worker_result_json), \
             error = COALESCE(?8, error), \
             finished_at = CASE WHEN ?4 IN ('succeeded','failed_transient','failed_permanent','cancelled','interrupted') THEN ?{now_idx} ELSE finished_at END \
             WHERE plan_id = ?1 AND item_id = ?2 AND attempt_no = ?3 AND state IN ({placeholders})"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(sql_err("prepare attempt cas"))?;
        let mut p: Vec<&dyn rusqlite::ToSql> = vec![
            &plan_id_s,
            &item_id_s,
            &attempt_no_s,
            &to_s,
            &owner_pid,
            &kind_s,
            &result_s,
            &error_s,
        ];
        for c in &cond {
            p.push(c);
        }
        p.push(&now);
        let n = stmt.execute(p.as_slice()).map_err(sql_err("attempt cas"))?;
        Ok(n > 0)
    }

    pub fn get_attempt(
        &self,
        plan_id: &str,
        item_id: &str,
        attempt_no: u32,
    ) -> Result<Option<AttemptRow>, StoreError> {
        self.conn
            .query_row(
                "SELECT item_id, attempt_no, dir, state, failure_kind, owner_pid, owner_token, \
                        worker_result_json, error, created_at, finished_at
                 FROM mb_attempts WHERE plan_id = ?1 AND item_id = ?2 AND attempt_no = ?3",
                params![plan_id, item_id, attempt_no as i64],
                |row| Ok(attempt_from_row(plan_id, row)),
            )
            .optional()
            .map_err(sql_err("get attempt"))
    }

    /// The attempt row (of ANY item and ANY plan) that points at the given
    /// directory, if there is one.
    ///
    /// The comparison uses the Windows-normalized form of the path: on NTFS
    /// "runs/Build" and "runs/build" are one and the same directory, so a
    /// plain string comparison would miss the attempt of a DIFFERENT item
    /// that physically owns the directory. The lookup is deliberately not
    /// restricted to one plan: an attempt directory is a shared filesystem
    /// resource.
    pub fn find_attempt_by_dir(&self, dir: &Path) -> Result<Option<AttemptRow>, StoreError> {
        let key = crate::plan::dir_key(dir);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_id, attempt_no, dir, state, failure_kind, owner_pid, owner_token, \
                        worker_result_json, error, created_at, finished_at, plan_id
                 FROM mb_attempts ORDER BY plan_id, item_id, attempt_no",
            )
            .map_err(sql_err("prepare attempts by dir"))?;
        let rows = stmt
            .query_map([], |row| {
                let plan_id: String = row.get(11)?;
                Ok(attempt_from_row(&plan_id, row))
            })
            .map_err(sql_err("query attempts by dir"))?;
        for row in rows {
            let row = row.map_err(sql_err("collect attempts by dir"))?;
            if crate::plan::dir_key(Path::new(&row.dir)) == key {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    /// All attempts that are not in a terminal state (reserved or running).
    pub fn list_live_attempts(&self, plan_id: &str) -> Result<Vec<AttemptRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_id, attempt_no, dir, state, failure_kind, owner_pid, owner_token, \
                        worker_result_json, error, created_at, finished_at
                 FROM mb_attempts
                 WHERE plan_id = ?1 AND state IN ('reserved', 'running')
                 ORDER BY item_id, attempt_no",
            )
            .map_err(sql_err("prepare live attempts"))?;
        let rows = stmt
            .query_map(params![plan_id], |row| Ok(attempt_from_row(plan_id, row)))
            .map_err(sql_err("query live attempts"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(sql_err("collect live attempts"))
    }

    pub fn list_attempts(&self, plan_id: &str) -> Result<Vec<AttemptRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_id, attempt_no, dir, state, failure_kind, owner_pid, owner_token, \
                        worker_result_json, error, created_at, finished_at
                 FROM mb_attempts WHERE plan_id = ?1 ORDER BY item_id, attempt_no",
            )
            .map_err(sql_err("prepare attempts"))?;
        let rows = stmt
            .query_map(params![plan_id], |row| Ok(attempt_from_row(plan_id, row)))
            .map_err(sql_err("query attempts"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(sql_err("collect attempts"))
    }

    /// Items currently in the `running` state. A crashed runner can leave an
    /// item there without any attempt (see [`Store::claim_item_with_attempt`]
    /// and the executor reconciliation).
    pub fn list_running_items(&self, plan_id: &str) -> Result<Vec<ItemRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_id, status, blocked_reason, attempts_used, last_error, updated_at
                 FROM mb_items WHERE plan_id = ?1 AND status = 'running' ORDER BY item_id",
            )
            .map_err(sql_err("prepare running items"))?;
        let rows = stmt
            .query_map(params![plan_id], |row| {
                Ok(ItemRow {
                    plan_id: plan_id.to_string(),
                    item_id: row.get(0)?,
                    status: row.get(1)?,
                    blocked_reason: row.get(2)?,
                    attempts_used: row.get::<_, i64>(3)? as u32,
                    last_error: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(sql_err("query running items"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(sql_err("collect running items"))
    }

    /// The attempt with the highest number for an item (if any).
    pub fn latest_attempt(
        &self,
        plan_id: &str,
        item_id: &str,
    ) -> Result<Option<AttemptRow>, StoreError> {
        self.conn
            .query_row(
                "SELECT item_id, attempt_no, dir, state, failure_kind, owner_pid, owner_token, \
                        worker_result_json, error, created_at, finished_at
                 FROM mb_attempts WHERE plan_id = ?1 AND item_id = ?2
                 ORDER BY attempt_no DESC LIMIT 1",
                params![plan_id, item_id],
                |row| Ok(attempt_from_row(plan_id, row)),
            )
            .optional()
            .map_err(sql_err("latest attempt"))
    }

    // -- events ---------------------------------------------------------------

    pub fn append_event(
        &self,
        plan_id: &str,
        item_id: Option<&str>,
        attempt_no: Option<u32>,
        kind: &str,
        payload_json: Option<&str>,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO mb_events (ts, plan_id, item_id, attempt_no, kind, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    timeutil::now_iso8601(),
                    plan_id,
                    item_id,
                    attempt_no.map(|n| n as i64),
                    kind,
                    payload_json
                ],
            )
            .map_err(sql_err("append event"))?;
        Ok(())
    }

    pub fn events_tail(&self, plan_id: &str, limit: usize) -> Result<Vec<EventRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ts, item_id, attempt_no, kind FROM mb_events
                 WHERE plan_id = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(sql_err("prepare events"))?;
        let rows = stmt
            .query_map(params![plan_id, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    row.get::<_, Option<i64>>(2)?.map(|v| v as u32),
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(sql_err("query events"))?;
        let mut out: Vec<EventRow> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err("collect events"))?;
        out.reverse();
        Ok(out)
    }
}

fn attempt_from_row(plan_id: &str, row: &rusqlite::Row<'_>) -> AttemptRow {
    AttemptRow {
        plan_id: plan_id.to_string(),
        item_id: row.get(0).unwrap_or_default(),
        attempt_no: row.get::<_, i64>(1).unwrap_or(0) as u32,
        dir: row.get(2).unwrap_or_default(),
        state: row.get(3).unwrap_or_default(),
        failure_kind: row.get(4).unwrap_or(None),
        owner_pid: row.get(5).unwrap_or(None),
        owner_token: row.get(6).unwrap_or(None),
        worker_result_json: row.get(7).unwrap_or(None),
        error: row.get(8).unwrap_or(None),
        created_at: row.get(9).unwrap_or_default(),
        finished_at: row.get(10).unwrap_or(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("state.db")).unwrap();
        (tmp, store)
    }

    #[test]
    fn register_is_idempotent_for_same_fingerprint_and_rejects_mutation() {
        let (_tmp, store) = open();
        let items = vec!["a".to_string()];
        assert_eq!(
            store
                .register_plan("bp-1", "fp-1", "{\"v\":1}", &items)
                .unwrap(),
            RegisterOutcome::Created
        );
        assert_eq!(
            store
                .register_plan("bp-1", "fp-1", "{\"v\":1}", &items)
                .unwrap(),
            RegisterOutcome::AlreadyPresent
        );
        let err = store
            .register_plan("bp-1", "fp-2", "{\"v\":2}", &items)
            .unwrap_err();
        assert!(matches!(err, StoreError::PlanImmutable { .. }));
    }

    #[test]
    fn item_cas_respects_expected_states() {
        let (_tmp, store) = open();
        store
            .register_plan("bp-1", "fp", "{}", &["a".to_string()])
            .unwrap();
        // pending -> running
        assert!(store
            .set_item_status("bp-1", "a", ITEM_RUNNING, Some(&[ITEM_PENDING]), None, None)
            .unwrap());
        // second claim from pending must fail
        assert!(!store
            .set_item_status("bp-1", "a", ITEM_RUNNING, Some(&[ITEM_PENDING]), None, None)
            .unwrap());
        assert!(store
            .set_item_status(
                "bp-1",
                "a",
                ITEM_AWAITING_REVIEW,
                Some(&[ITEM_RUNNING]),
                None,
                None
            )
            .unwrap());
    }

    #[test]
    fn attempt_reservation_and_transitions() {
        let (_tmp, store) = open();
        store
            .register_plan("bp-1", "fp", "{}", &["a".to_string()])
            .unwrap();
        assert_eq!(store.next_attempt_no("bp-1", "a").unwrap(), 1);
        store
            .insert_attempt("bp-1", "a", 1, std::path::Path::new("/tmp/d1"), "tok")
            .unwrap();
        // Double reservation is rejected.
        assert!(store
            .insert_attempt("bp-1", "a", 1, std::path::Path::new("/tmp/d1"), "tok")
            .unwrap_err()
            .to_string()
            .contains("already reserved"));
        assert_eq!(store.next_attempt_no("bp-1", "a").unwrap(), 2);

        assert!(store
            .attempt_transition(
                "bp-1",
                "a",
                1,
                &[ATTEMPT_RESERVED],
                ATTEMPT_RUNNING,
                Some(4242),
                None,
                None,
                None
            )
            .unwrap());
        assert!(store
            .attempt_transition(
                "bp-1",
                "a",
                1,
                &[ATTEMPT_RUNNING],
                ATTEMPT_SUCCEEDED,
                None,
                None,
                None,
                None
            )
            .unwrap());
        // Double finalize must fail (already terminal).
        assert!(!store
            .attempt_transition(
                "bp-1",
                "a",
                1,
                &[ATTEMPT_RUNNING],
                ATTEMPT_SUCCEEDED,
                None,
                None,
                None,
                None
            )
            .unwrap());
        let row = store.get_attempt("bp-1", "a", 1).unwrap().unwrap();
        assert_eq!(row.state, ATTEMPT_SUCCEEDED);
        assert_eq!(row.owner_pid, Some(4242));
    }

    #[test]
    fn insert_attempt_rejects_a_double_reservation() {
        let (_tmp, store) = open();
        store
            .register_plan("bp-1", "fp", "{}", &["a".to_string()])
            .unwrap();
        let dir = std::path::Path::new("/tmp/double");
        store.insert_attempt("bp-1", "a", 1, dir, "tok-1").unwrap();

        // The (plan, item, attempt_no) triple is the primary key: a second
        // insertion must fail instead of reporting success.
        let err = store
            .insert_attempt("bp-1", "a", 1, dir, "tok-2")
            .unwrap_err();
        assert!(
            err.to_string().contains("already reserved"),
            "expected an honest double-reservation error, got: {err}"
        );
        // The first reservation is untouched.
        let row = store.get_attempt("bp-1", "a", 1).unwrap().unwrap();
        assert_eq!(row.owner_token.as_deref(), Some("tok-1"));
        assert_eq!(store.list_attempts("bp-1").unwrap().len(), 1);
    }

    #[test]
    fn find_attempt_by_dir_finds_the_owning_attempt_of_another_item() {
        let (_tmp, store) = open();
        store
            .register_plan("bp-1", "fp", "{}", &["a".to_string(), "b".to_string()])
            .unwrap();
        let dir = std::path::Path::new("/tmp/runs/a/attempt_001");
        store.insert_attempt("bp-1", "a", 1, dir, "tok").unwrap();

        let found = store.find_attempt_by_dir(dir).unwrap().unwrap();
        assert_eq!(found.item_id, "a");
        assert_eq!(found.attempt_no, 1);

        // A directory nobody owns is not found.
        assert!(store
            .find_attempt_by_dir(std::path::Path::new("/tmp/runs/a/attempt_002"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn events_are_append_only_and_ordered() {
        let (_tmp, store) = open();
        store
            .register_plan("bp-1", "fp", "{}", &["a".to_string()])
            .unwrap();
        for kind in ["e1", "e2", "e3"] {
            store
                .append_event("bp-1", Some("a"), None, kind, None)
                .unwrap();
        }
        let tail = store.events_tail("bp-1", 10).unwrap();
        let kinds: Vec<&str> = tail.iter().map(|e| e.3.as_str()).collect();
        assert_eq!(kinds, vec!["e1", "e2", "e3"]);
    }
}
