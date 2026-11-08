//! Structured batch report with the distinct domain outcome taxonomy.
//!
//! "built", "awaiting_review", "validation_failed", "execution_failed",
//! "blocked", "cancelled" are separate results. The presence of an artifact
//! file is never treated as acceptance; canonical accept is out of PoC scope.

use std::fmt;

use serde::Serialize;

use crate::plan::BatchPlan;
use crate::store::{ItemRow, Store, StoreError};

pub const REPORT_SCHEMA: &str = "michelangelo.batch-report/provisional-0";

#[derive(Debug, Clone, Serialize)]
pub struct BatchReport {
    pub schema: String,
    pub plan_id: String,
    pub plan_fingerprint: String,
    pub batch_status: String,
    pub generated_at: String,
    pub counts: ReportCounts,
    pub items: Vec<ItemReport>,
    /// Non-fatal but honest notes (e.g. live attempt owned by another process).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ReportCounts {
    pub items_total: u32,
    pub pending: u32,
    pub running: u32,
    pub built: u32,
    pub awaiting_review: u32,
    pub validation_failed: u32,
    pub execution_failed: u32,
    pub blocked: u32,
    pub cancelled: u32,
    pub attempts_total: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemReport {
    pub item_id: String,
    pub status: String,
    pub attempts_used: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_attempt: Option<LiveAttemptInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveAttemptInfo {
    pub attempt_no: u32,
    pub state: String,
    pub owner_pid: Option<i64>,
}

impl BatchReport {
    pub fn build(store: &Store, plan: &BatchPlan, plan_id: &str, fingerprint: &str) -> Result<BatchReport, StoreError> {
        let items = store.list_items(plan_id)?;
        let attempts = store.list_attempts(plan_id)?;
        let batch_status = store
            .batch_status(plan_id)?
            .unwrap_or_else(|| "unknown".to_string());

        let mut counts = ReportCounts {
            items_total: items.len() as u32,
            ..Default::default()
        };
        let mut reports = Vec::with_capacity(items.len());
        for row in &items {
            match row.status.as_str() {
                crate::store::ITEM_PENDING => counts.pending += 1,
                crate::store::ITEM_RUNNING => counts.running += 1,
                crate::store::ITEM_BUILT => counts.built += 1,
                crate::store::ITEM_AWAITING_REVIEW => counts.awaiting_review += 1,
                crate::store::ITEM_VALIDATION_FAILED => counts.validation_failed += 1,
                crate::store::ITEM_EXECUTION_FAILED => counts.execution_failed += 1,
                crate::store::ITEM_BLOCKED => counts.blocked += 1,
                crate::store::ITEM_CANCELLED => counts.cancelled += 1,
                _ => {}
            }
            let live = attempts
                .iter()
                .find(|a| a.item_id == row.item_id && (a.state == "reserved" || a.state == "running"))
                .map(|a| LiveAttemptInfo {
                    attempt_no: a.attempt_no,
                    state: a.state.clone(),
                    owner_pid: a.owner_pid,
                });
            counts.attempts_total = counts.attempts_total.max(row.attempts_used);
            reports.push(ItemReport {
                item_id: row.item_id.clone(),
                status: row.status.clone(),
                attempts_used: row.attempts_used,
                blocked_reason: row.blocked_reason.clone(),
                last_error: row.last_error.clone(),
                live_attempt: live,
            });
        }
        let _ = plan;
        Ok(BatchReport {
            schema: REPORT_SCHEMA.to_string(),
            plan_id: plan_id.to_string(),
            plan_fingerprint: fingerprint.to_string(),
            batch_status,
            generated_at: crate::timeutil::now_iso8601(),
            counts,
            items: reports,
            warnings: Vec::new(),
        })
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// True when every item ended in a successful domain outcome.
    pub fn all_success(&self) -> bool {
        self.counts
            .pending
            + self.counts.running
            + self.counts.validation_failed
            + self.counts.execution_failed
            + self.counts.blocked
            + self.counts.cancelled
            == 0
            && self.counts.items_total > 0
            && self.counts.built + self.counts.awaiting_review == self.counts.items_total
    }
}

impl fmt::Display for BatchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "batch {} [{}] built={} awaiting_review={} validation_failed={} execution_failed={} blocked={} cancelled={} pending={} attempts={}",
            self.plan_id,
            self.batch_status,
            self.counts.built,
            self.counts.awaiting_review,
            self.counts.validation_failed,
            self.counts.execution_failed,
            self.counts.blocked,
            self.counts.cancelled,
            self.counts.pending + self.counts.running,
            self.counts.attempts_total,
        )
    }
}

/// Helper used by the executor to mark item outcomes consistently.
pub fn item_report_row(row: &ItemRow) -> ItemReport {
    ItemReport {
        item_id: row.item_id.clone(),
        status: row.status.clone(),
        attempts_used: row.attempts_used,
        blocked_reason: row.blocked_reason.clone(),
        last_error: row.last_error.clone(),
        live_attempt: None,
    }
}
