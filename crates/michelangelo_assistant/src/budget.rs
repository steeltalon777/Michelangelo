//! The per-run budget counters and the append-only audit log.
//!
//! The store of a run owns two files:
//!
//! ```text
//! <run_dir>/assistant/budget.json   the counters of the run
//! <run_dir>/assistant/audit.jsonl   one JSON object per assistant call
//! ```
//!
//! The counters are published with a write-to-temp + rename, so a reader
//! never observes a half-written file and the counters survive separate
//! invocations of the CLI. The audit log is append-only and never carries an
//! absolute path.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssistantError;
use crate::{Clock, ASSISTANT_DIR_NAME, AUDIT_FILE_NAME, BUDGET_FILE_NAME, BUDGET_SCHEMA};

/// The decision of a call in the audit log.
pub const DECISION_OK: &str = "ok";
/// The decision of a call that produced a proposed candidate.
pub const DECISION_PROPOSED: &str = "proposed";
/// The decision of a call whose candidate was refused.
pub const DECISION_REJECTED: &str = "rejected";
/// The decision of a call that failed.
pub const DECISION_ERROR: &str = "error";

/// The counters of the assistant calls of one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetCounters {
    /// The schema identifier.
    #[serde(default = "default_schema")]
    pub schema: String,
    /// The number of admitted assistant calls.
    #[serde(default)]
    pub calls_used: u64,
    /// The number of candidates that entered the store.
    #[serde(default)]
    pub candidates_submitted: u64,
    /// The number of bytes that entered the store.
    #[serde(default)]
    pub bytes_submitted: u64,
    /// The cost of the calls in US dollars (always 0 in this stage).
    #[serde(default)]
    pub cost_used_usd: f64,
    /// The time of the last call (RFC 3339, UTC).
    #[serde(default)]
    pub last_call_at: Option<String>,
    /// The time of the last update of the counters (RFC 3339, UTC).
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// The default schema of the counters.
fn default_schema() -> String {
    BUDGET_SCHEMA.to_string()
}

impl Default for BudgetCounters {
    fn default() -> Self {
        BudgetCounters {
            schema: default_schema(),
            calls_used: 0,
            candidates_submitted: 0,
            bytes_submitted: 0,
            cost_used_usd: 0.0,
            last_call_at: None,
            updated_at: None,
        }
    }
}

/// One line of the audit log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEntry {
    /// The time of the call (RFC 3339, UTC).
    pub ts: String,
    /// The tool of the call.
    pub tool: String,
    /// The decision of the call.
    pub decision: String,
    /// The identifier of the candidate (None when the call carried none).
    pub candidate_id: Option<String>,
    /// The identifier of the task (None when the call carried none).
    pub task_id: Option<String>,
    /// A human readable detail.
    pub detail: String,
}

impl AuditEntry {
    /// The JSON representation of the entry (one line, deterministic).
    pub fn to_line(&self) -> Result<String, AssistantError> {
        let mut text = serde_json::to_string(self).map_err(|error| AssistantError::Invalid {
            message: format!("the audit entry cannot be serialized: {error}"),
        })?;
        text = text.replace(['\r', '\n'], " ");
        Ok(text)
    }
}

/// One call that is written into the audit log.
#[derive(Debug, Clone, Copy)]
pub struct AuditRecord<'a> {
    /// The tool of the call.
    pub tool: &'a str,
    /// The decision of the call.
    pub decision: &'a str,
    /// The identifier of the candidate (None when the call carried none).
    pub candidate_id: Option<&'a str>,
    /// The identifier of the task (None when the call carried none).
    pub task_id: Option<&'a str>,
    /// A human readable detail.
    pub detail: &'a str,
}

/// The budget and audit store of a run.
#[derive(Debug, Clone)]
pub struct BudgetStore {
    /// The directory of the run.
    pub run_dir: PathBuf,
}

impl BudgetStore {
    /// The store of a run directory.
    pub fn new(run_dir: &Path) -> Self {
        BudgetStore {
            run_dir: run_dir.to_path_buf(),
        }
    }

    /// The assistant directory of the run.
    pub fn assistant_dir(&self) -> PathBuf {
        self.run_dir.join(ASSISTANT_DIR_NAME)
    }

    /// The path of the counters of the run.
    pub fn budget_path(&self) -> PathBuf {
        self.assistant_dir().join(BUDGET_FILE_NAME)
    }

    /// The path of the audit log of the run.
    pub fn audit_path(&self) -> PathBuf {
        self.assistant_dir().join(AUDIT_FILE_NAME)
    }

    /// Read the counters of the run (empty counters when the file is absent).
    pub fn load(&self) -> Result<BudgetCounters, AssistantError> {
        let path = self.budget_path();
        if !path.is_file() {
            return Ok(BudgetCounters::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|error| {
            AssistantError::io(
                &path,
                format!("the budget counters cannot be read: {error}"),
            )
        })?;
        let counters: BudgetCounters =
            serde_json::from_str(&text).map_err(|error| AssistantError::Invalid {
                message: format!("the budget counters are not usable: {error}"),
            })?;
        Ok(counters)
    }

    /// Publish the counters of the run (atomic).
    pub fn save(&self, counters: &BudgetCounters) -> Result<(), AssistantError> {
        let path = self.budget_path();
        let text =
            serde_json::to_string_pretty(counters).map_err(|error| AssistantError::Invalid {
                message: format!("the budget counters cannot be serialized: {error}"),
            })?;
        michelangelo_edit::scene::write_bytes_atomic(
            &path,
            format!(
                "{text}
"
            )
            .as_bytes(),
        )?;
        Ok(())
    }

    /// Check that another assistant call fits into the budget.
    pub fn check_call(
        &self,
        counters: &BudgetCounters,
        max_calls: u64,
    ) -> Result<(), AssistantError> {
        if counters.calls_used >= max_calls {
            return Err(AssistantError::BudgetExhausted {
                message: format!(
                    "the run used {} of {max_calls} assistant calls",
                    counters.calls_used
                ),
            });
        }
        Ok(())
    }

    /// Check that another candidate fits into the budget.
    pub fn check_bytes(
        &self,
        counters: &BudgetCounters,
        additional: u64,
        max_bytes: u64,
    ) -> Result<(), AssistantError> {
        let total = counters.bytes_submitted.saturating_add(additional);
        if total > max_bytes {
            return Err(AssistantError::BudgetExhausted {
                message: format!("the run submitted {total} bytes, the limit is {max_bytes} bytes"),
            });
        }
        Ok(())
    }

    /// Append one line to the audit log of the run.
    pub fn append_audit(&self, entry: &AuditEntry) -> Result<(), AssistantError> {
        let path = self.audit_path();
        let directory = self.assistant_dir();
        std::fs::create_dir_all(&directory).map_err(|error| {
            AssistantError::io(
                &directory,
                format!("the assistant directory cannot be created: {error}"),
            )
        })?;
        let line = entry.to_line()?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| {
                AssistantError::io(&path, format!("the audit log cannot be opened: {error}"))
            })?;
        writeln!(file, "{line}").map_err(|error| {
            AssistantError::io(&path, format!("the audit log cannot be written: {error}"))
        })?;
        file.flush().map_err(|error| {
            AssistantError::io(&path, format!("the audit log cannot be flushed: {error}"))
        })?;
        Ok(())
    }

    /// Append one line to the audit log and count the call.
    pub fn record_call(
        &self,
        counters: &mut BudgetCounters,
        record: AuditRecord<'_>,
        clock: &dyn Clock,
    ) -> Result<(), AssistantError> {
        let now = clock.now_rfc3339_utc();
        let entry = AuditEntry {
            ts: now.clone(),
            tool: record.tool.to_string(),
            decision: record.decision.to_string(),
            candidate_id: record.candidate_id.map(|value| value.to_string()),
            task_id: record.task_id.map(|value| value.to_string()),
            detail: record.detail.to_string(),
        };
        self.append_audit(&entry)?;
        counters.last_call_at = Some(now.clone());
        counters.updated_at = Some(now);
        Ok(())
    }

    /// The lines of the audit log of the run.
    pub fn read_audit(&self) -> Result<Vec<String>, AssistantError> {
        let path = self.audit_path();
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path).map_err(|error| {
            AssistantError::io(&path, format!("the audit log cannot be read: {error}"))
        })?;
        Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.to_string())
            .collect())
    }
}
