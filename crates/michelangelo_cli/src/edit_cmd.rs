//! `michelangelo edit` subcommands: the local edit boundary (Stage C).
//!
//! The commands drive the scene revision store of a reconstruction run
//! (`michelangelo_edit`). Human readable summaries go to stdout, the machine
//! readable decision reports are printed with `--json`, and the errors go to
//! stderr.
//!
//! Exit codes:
//!
//! * `0` — the operation succeeded (the candidate was accepted, proposed or
//!   rolled back),
//! * `1` — the candidate was rejected by the validator,
//! * `2` — usage error, IO error or a candidate that cannot be interpreted.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use michelangelo_edit::apply::{
    accept_candidate, reject_candidate, rollback_scene, show_scene, submit_candidate,
};
use michelangelo_edit::scene::init_scene_with_protection;
use michelangelo_edit::validate::{CheckStatus, Decision, DecisionReport, Severity};
use michelangelo_edit::SystemClock;

/// Exit code: the operation succeeded.
pub const EXIT_OK: u8 = 0;
/// Exit code: the candidate was rejected.
pub const EXIT_REJECTED: u8 = 1;
/// Exit code: usage or IO error.
pub const EXIT_USAGE: u8 = 2;

/// Subcommands of `michelangelo edit`.
#[derive(Debug, Subcommand)]
pub enum EditCommand {
    /// Create the first scene revision of a reconstruction run.
    Init {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// Components that the author protects against modification (comma separated, repeatable).
        #[arg(long = "protect", value_name = "NAME", value_delimiter = ',')]
        protect: Vec<String>,
        /// Print the scene document instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Submit an edit candidate into the store of a run.
    Submit {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The candidate directory (the directory with candidate.json).
        #[arg(long, value_name = "DIR")]
        candidate: PathBuf,
        /// Print the decision report instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Accept a candidate and publish a new scene revision.
    Accept {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The identifier of the candidate.
        #[arg(long, value_name = "ID")]
        candidate: String,
        /// The author that accepts the candidate.
        #[arg(long, value_name = "NAME")]
        by: String,
        /// Print the decision report instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Reject a candidate.
    Reject {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The identifier of the candidate.
        #[arg(long, value_name = "ID")]
        candidate: String,
        /// The author that rejects the candidate.
        #[arg(long, value_name = "NAME")]
        by: String,
        /// The reason of the rejection.
        #[arg(long, value_name = "TEXT")]
        reason: Option<String>,
        /// Print the decision report instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Point the store back to a previous scene revision.
    Rollback {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The revision the store is rolled back to.
        #[arg(long, value_name = "REVISION")]
        to: String,
        /// The author that performs the rollback.
        #[arg(long, value_name = "NAME")]
        by: String,
        /// Print the rollback report instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Show the state of the scene store of a run.
    Show {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// Show one candidate instead of the whole store.
        #[arg(long, value_name = "ID")]
        candidate: Option<String>,
        /// Print the view as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
}

/// Execute an `edit` subcommand.
pub fn execute(command: &EditCommand) -> ExitCode {
    let clock = SystemClock;
    match command {
        EditCommand::Init { run, protect, json } => init(run, protect, *json),
        EditCommand::Submit {
            run,
            candidate,
            json,
        } => {
            let report = match submit_candidate(run, candidate, &clock) {
                Ok(report) => report,
                Err(error) => return failure(&error),
            };
            print_decision(&report, *json)
        }
        EditCommand::Accept {
            run,
            candidate,
            by,
            json,
        } => {
            let report = match accept_candidate(run, candidate, by, &clock) {
                Ok(report) => report,
                Err(error) => return failure(&error),
            };
            print_decision(&report, *json)
        }
        EditCommand::Reject {
            run,
            candidate,
            by,
            reason,
            json,
        } => {
            let report = match reject_candidate(run, candidate, by, reason.as_deref(), &clock) {
                Ok(report) => report,
                Err(error) => return failure(&error),
            };
            print_decision(&report, *json)
        }
        EditCommand::Rollback { run, to, by, json } => {
            let report = match rollback_scene(run, to, by, &clock) {
                Ok(report) => report,
                Err(error) => return failure(&error),
            };
            if *json {
                match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        eprintln!("[michelangelo] the report cannot be serialized: {error}");
                        return ExitCode::from(EXIT_USAGE);
                    }
                }
            } else {
                println!(
                    "rollback: {} -> {}",
                    report.from_scene_revision.as_deref().unwrap_or("-"),
                    report.to_scene_revision
                );
                println!("by: {}", report.by);
                println!("at: {}", report.rolled_back_at);
            }
            ExitCode::from(EXIT_OK)
        }
        EditCommand::Show {
            run,
            candidate,
            json,
        } => show(run, candidate.as_deref(), *json),
    }
}

/// Create the first scene revision of a run.
fn init(run: &std::path::Path, protect: &[String], json: bool) -> ExitCode {
    let scene = match init_scene_with_protection(run, protect, &SystemClock) {
        Ok(scene) => scene,
        Err(error) => return failure(&error),
    };
    if json {
        match serde_json::to_string_pretty(&scene) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the scene cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("scene_revision: {}", scene.scene_revision);
        println!("run_id: {}", scene.run_id);
        println!(
            "package: {} (revision {})",
            scene.source.package_id, scene.source.package_revision
        );
        println!("components: {}", scene.components.len());
        for component in &scene.components {
            println!("  {}: sha256 {}", component.name, component.sha256);
        }
    }
    ExitCode::from(EXIT_OK)
}

/// Show the state of the scene store.
fn show(run: &std::path::Path, candidate: Option<&str>, json: bool) -> ExitCode {
    let view = match show_scene(run, candidate) {
        Ok(view) => view,
        Err(error) => return failure(&error),
    };
    if json {
        match view.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => return failure(&error),
        }
    } else {
        println!("run_id: {}", view.run_id);
        match &view.current {
            Some(pointer) => println!(
                "current: {} (candidate {})",
                pointer.scene_revision,
                pointer.candidate_id.as_deref().unwrap_or("-")
            ),
            None => println!("current: -"),
        }
        println!("revisions: {}", view.revisions.len());
        for revision in &view.revisions {
            println!(
                "  {}: {} components, created {}",
                revision.scene_revision,
                revision.components.len(),
                revision.created_at
            );
        }
        println!("candidates: {}", view.candidates.len());
        for entry in &view.candidates {
            println!(
                "  {}: {} ({})",
                entry.candidate_id,
                entry.status,
                entry.scene_revision.as_deref().unwrap_or("-")
            );
        }
        if !view.rollbacks.is_empty() {
            println!("rollbacks: {}", view.rollbacks.len());
            for rollback in &view.rollbacks {
                println!(
                    "  {} -> {} by {}",
                    rollback.from_scene_revision.as_deref().unwrap_or("-"),
                    rollback.to_scene_revision,
                    rollback.by
                );
            }
        }
        if let Some(shown) = &view.candidate {
            println!("candidate: {}", shown.candidate_id);
            println!("status: {}", shown.status);
            println!("task: {}", shown.candidate.task.task_id);
            println!(
                "allowed_components: {}",
                shown.candidate.task.allowed_components.join(", ")
            );
            println!("protected: {}", shown.candidate.task.protected.join(", "));
            if let Some(decision) = &shown.decision {
                println!("decision: {}", decision.decision.as_str());
            }
        }
    }
    ExitCode::from(EXIT_OK)
}

/// Print a decision report and map the decision to the exit code.
fn print_decision(report: &DecisionReport, json: bool) -> ExitCode {
    if json {
        match report.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => return failure(&error),
        }
    } else {
        println!("decision: {}", report.decision.as_str());
        println!("candidate: {}", report.candidate_id);
        println!(
            "base_scene_revision: {}",
            report.base_scene_revision.as_deref().unwrap_or("-")
        );
        println!(
            "resulting_scene_revision: {}",
            report.resulting_scene_revision.as_deref().unwrap_or("-")
        );
        println!("decided_at: {}", report.decided_at);
        println!(
            "decided_by: {}",
            report.decided_by.as_deref().unwrap_or("-")
        );
        println!("checks:");
        for check in &report.checks {
            println!(
                "  {:<7} {:<16} {}",
                check.status.as_str(),
                check.check,
                check.detail
            );
        }
        if !report.diagnostics.is_empty() {
            println!("diagnostics:");
            for diagnostic in &report.diagnostics {
                println!(
                    "  {:<7} {:<20} {}",
                    diagnostic.severity.as_str(),
                    diagnostic.code,
                    diagnostic.message
                );
            }
        }
    }
    match report.decision {
        Decision::Accepted | Decision::Proposed => ExitCode::from(EXIT_OK),
        Decision::Rejected => ExitCode::from(EXIT_REJECTED),
        Decision::Error => ExitCode::from(EXIT_USAGE),
    }
}

/// The status text of a check (kept for the human summary).
#[allow(dead_code)]
fn status_text(status: CheckStatus) -> &'static str {
    status.as_str()
}

/// The severity text of a diagnostic (kept for the human summary).
#[allow(dead_code)]
fn severity_text(severity: Severity) -> &'static str {
    severity.as_str()
}

/// Print an error and map it to the exit code.
fn failure(error: &michelangelo_edit::EditError) -> ExitCode {
    eprintln!("[michelangelo] {error}");
    ExitCode::from(error.exit_code())
}
