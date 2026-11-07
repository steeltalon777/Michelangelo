//! Michelangelo assistant surface (Stage D2).
//!
//! This crate implements the normative contract of
//! `docs/tz/TZ_ASSISTANT_STAGE_D.md`:
//!
//! * `task.rs` — the bounded assistant task (`michelangelo.assistant-task/0`),
//!   generated deterministically from a reconstruction run,
//! * `policy.rs` — the optional workspace policy
//!   (`<workspace>/.michelangelo/assistant_policy.json`),
//! * `budget.rs` — the per-run budget counters and the append-only audit log,
//! * `tools.rs` — the four allow-listed tools of the assistant surface,
//! * `mcp.rs` — the newline-delimited JSON-RPC 2.0 stdio adapter.
//!
//! The surface is **local and bounded**: it embeds no model, opens no network
//! connection and never applies a candidate. An external agent supplies the
//! candidate; the assistant validates it against the task and the budget and
//! routes it through the local edit boundary, where it always waits for an
//! explicit author acceptance.

#![forbid(unsafe_code)]

pub mod budget;
pub mod error;
pub mod mcp;
pub mod policy;
pub mod task;
pub mod tools;

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod task_tests;
#[cfg(test)]
mod testutil;
#[cfg(test)]
mod tools_tests;

use std::path::Path;

pub use budget::{AuditEntry, BudgetCounters, BudgetStore};
pub use error::AssistantError;
pub use mcp::{McpServer, RpcError};
pub use policy::{AssistantPolicy, ALL_TOOLS};
pub use task::{
    AcceptanceCriteria, AssistantTask, DataPolicy, TaskBase, TaskBudget, TaskProvenance,
    TaskReferences, TaskRequest, TaskSupplement, TaskUnknown,
};
pub use tools::{PackageSummary, ReportSummary, ToolOutcome};

// The clock of the store is the clock of the edit boundary: one trait, one
// fixed implementation for the tests.
pub use michelangelo_edit::{Clock, FixedClock, SystemClock};

/// The schema of the bounded assistant task.
pub const TASK_SCHEMA: &str = "michelangelo.assistant-task/0";
/// The schema of the assistant policy file.
pub const POLICY_SCHEMA: &str = "michelangelo.assistant-policy/0";
/// The schema of the budget counters of a run.
pub const BUDGET_SCHEMA: &str = "michelangelo.assistant-budget/0";
/// The schema of the audit log of a run.
pub const AUDIT_SCHEMA: &str = "michelangelo.assistant-audit/0";

/// The name of the assistant directory inside a run directory.
pub const ASSISTANT_DIR_NAME: &str = "assistant";
/// The name of the tasks directory inside the assistant directory.
pub const TASKS_DIR_NAME: &str = "tasks";
/// The name of the temporary directory of the assistant directory.
pub const TMP_DIR_NAME: &str = "tmp";
/// The name of the budget file inside the assistant directory.
pub const BUDGET_FILE_NAME: &str = "budget.json";
/// The name of the audit file inside the assistant directory.
pub const AUDIT_FILE_NAME: &str = "audit.jsonl";

/// Replace the absolute roots of a text with placeholders.
///
/// The task, the audit log and the MCP responses must never leak the absolute
/// location of the workspace, so every message that is written out is
/// sanitized first.
pub fn sanitize_paths(text: &str, roots: &[&Path]) -> String {
    let mut sanitized = text.to_string();
    for root in roots {
        let rendered = root.display().to_string();
        if rendered.is_empty() {
            continue;
        }
        sanitized = sanitized.replace(&rendered, "<path>");
        sanitized = sanitized.replace(&rendered.replace('\\', "/"), "<path>");
    }
    sanitized
}

/// True when a relative path stays inside its directory.
///
/// The check rejects absolute paths, the drive prefixes and the `..`
/// traversal; it is the first gate of every reference of a task.
pub fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    if path.contains(':') {
        return false;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            return false;
        }
        parts.push(part);
    }
    !parts.is_empty()
}

/// True when a path is absolute or escapes its directory.
///
/// The check is lexical, so it also refuses the POSIX and the Windows drive
/// prefixes on every platform.
pub fn is_escaping_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return true;
    }
    path.split(['/', '\\'])
        .any(|part| part == ".." || part == "...")
}

/// True when a relative path names a forbidden directory.
///
/// The evaluation data and the ground truth of a package never enter a task.
pub fn is_forbidden_reference(path: &str) -> bool {
    path.split(['/', '\\']).any(|part| {
        let part = part.to_ascii_lowercase();
        part == "evaluation"
            || part == "evaluation_renders"
            || part == "ground_truth"
            || part == "ground-truth"
    })
}
