//! `michelangelo assistant` subcommands: the bounded assistant surface (Stage D2).
//!
//! The commands generate a bounded task from a reconstruction run and submit
//! a candidate against it. The human readable summaries go to stdout, the
//! machine readable documents are printed with `--json` and the errors go to
//! stderr.
//!
//! Exit codes:
//!
//! * `0` — the task was written or the candidate was proposed,
//! * `1` — the candidate was rejected, the policy denied the operation or
//!   the budget of the run is exhausted,
//! * `2` — usage error, IO error or a document that cannot be interpreted.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use michelangelo_assistant::budget::{AuditEntry, BudgetStore};
use michelangelo_assistant::policy::load_policy;
use michelangelo_assistant::task::{generate_task, load_task, next_task_id, TaskRequest};
use michelangelo_assistant::tools::{self, ToolContext, ToolOutcome};
use michelangelo_assistant::{AssistantError, AssistantTask, Clock, SystemClock};
use michelangelo_edit::candidate::Region;

/// Exit code: the operation succeeded.
pub const EXIT_OK: u8 = 0;
/// Exit code: the candidate was rejected or the operation was refused.
pub const EXIT_REFUSED: u8 = 1;
/// Exit code: usage or IO error.
pub const EXIT_USAGE: u8 = 2;

/// Subcommands of `michelangelo assistant`.
#[derive(Debug, Subcommand)]
pub enum AssistantCommand {
    /// Generate the bounded assistant task of a run.
    Task {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The components the task may change (comma separated).
        #[arg(long, value_name = "NAME", value_delimiter = ',', required = true)]
        allowed: Vec<String>,
        /// The region the task may touch (`min=max`, three comma separated
        /// coordinates on each side).
        #[arg(long, value_name = "MIN=MAX")]
        region: Option<String>,
        /// Write the task into this file instead of the run directory.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Print the task JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Submit a candidate against a bounded task.
    Submit {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The bounded task of the run (the file with the task JSON).
        #[arg(long, value_name = "FILE")]
        task: PathBuf,
        /// The candidate directory (the directory with candidate.json).
        #[arg(long, value_name = "DIR")]
        candidate: PathBuf,
        /// The author that submits the candidate.
        #[arg(long, value_name = "NAME")]
        by: Option<String>,
        /// Print the decision report instead of a human summary.
        #[arg(long)]
        json: bool,
    },
}

/// Execute an `assistant` subcommand.
pub fn execute(command: &AssistantCommand) -> ExitCode {
    match command {
        AssistantCommand::Task {
            run,
            allowed,
            region,
            out,
            json,
        } => task(run, allowed, region.as_deref(), out.as_deref(), *json),
        AssistantCommand::Submit {
            run,
            task,
            candidate,
            by,
            json,
        } => submit(run, task, candidate, by.as_deref(), *json),
    }
}

/// Generate the bounded task of a run.
fn task(
    run: &Path,
    allowed: &[String],
    region: Option<&str>,
    out: Option<&Path>,
    json: bool,
) -> ExitCode {
    if allowed.is_empty() {
        eprintln!("[michelangelo] the task needs at least one allowed component");
        return ExitCode::from(EXIT_USAGE);
    }
    let policy = match load_policy(run) {
        Ok(policy) => policy,
        Err(error) => return failure(&error),
    };
    let region = match region.map(parse_region) {
        Some(Err(message)) => {
            eprintln!("[michelangelo] the region is not usable: {message}");
            return ExitCode::from(EXIT_USAGE);
        }
        Some(Ok(region)) => Some(region),
        None => None,
    };
    let request = TaskRequest {
        allowed: Some(allowed.to_vec()),
        region,
    };
    let task_id = match next_task_id(run) {
        Ok(task_id) => task_id,
        Err(error) => return failure(&error),
    };
    let task = match generate_task(run, &task_id, &request, &policy, &SystemClock) {
        Ok(task) => task,
        Err(error) => return failure(&error),
    };
    let path = match write_task(run, out, &task) {
        Ok(path) => path,
        Err(error) => return failure(&error),
    };
    if json {
        match task.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => return failure(&error),
        }
    } else {
        println!("task_id: {}", task.task_id);
        println!("schema: {}", task.schema);
        println!("run_id: {}", task.base.run_id);
        println!("scene_revision: {}", task.base.scene_revision);
        println!(
            "package: {} (revision {})",
            task.base.package_id, task.base.package_revision
        );
        println!("allowed_components: {}", task.allowed_components.join(", "));
        println!("protected: {}", task.protected.join(", "));
        println!("path: {}", path.display());
    }
    ExitCode::from(EXIT_OK)
}

/// Write the task into the run directory or into the requested file.
fn write_task(
    run: &Path,
    out: Option<&Path>,
    task: &AssistantTask,
) -> Result<PathBuf, AssistantError> {
    match out {
        Some(path) => {
            let text = task.to_json()?;
            michelangelo_edit::scene::write_bytes_atomic(
                path,
                format!(
                    "{text}
"
                )
                .as_bytes(),
            )?;
            Ok(path.to_path_buf())
        }
        None => michelangelo_assistant::task::write_task(run, task),
    }
}

/// Submit a candidate against a bounded task.
fn submit(
    run: &Path,
    task_path: &Path,
    candidate: &Path,
    by: Option<&str>,
    json: bool,
) -> ExitCode {
    let policy = match load_policy(run) {
        Ok(policy) => policy,
        Err(error) => return failure(&error),
    };
    let task = match load_task(task_path) {
        Ok(task) => task,
        Err(error) => return failure(&error),
    };
    let run_name = run
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !run_name.is_empty() && task.base.run_id != run_name {
        eprintln!(
            "[michelangelo] the task belongs to the run '{}' and not to '{run_name}'",
            task.base.run_id
        );
        return ExitCode::from(EXIT_USAGE);
    }

    let context = ToolContext::new(run.to_path_buf(), &policy, &SystemClock);
    let outcome = tools::submit_candidate(&context, run, &task, candidate);
    match outcome {
        Ok(ToolOutcome::Decision(report)) => {
            let decision = report.decision.as_str().to_string();
            let detail = format!("the candidate '{}' is '{}'", report.candidate_id, decision);
            audit(
                run,
                &decision,
                Some(&report.candidate_id),
                &task.task_id,
                &detail,
                by,
            );
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        eprintln!("[michelangelo] the decision cannot be serialized: {error}");
                        return ExitCode::from(EXIT_USAGE);
                    }
                }
            } else {
                println!("decision: {decision}");
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
                for check in &report.checks {
                    println!(
                        "  {:<7} {:<16} {}",
                        check.status.as_str(),
                        check.check,
                        check.detail
                    );
                }
            }
            match report.decision {
                michelangelo_edit::validate::Decision::Proposed => ExitCode::from(EXIT_OK),
                michelangelo_edit::validate::Decision::Rejected => ExitCode::from(EXIT_REFUSED),
                michelangelo_edit::validate::Decision::Accepted => ExitCode::from(EXIT_REFUSED),
                michelangelo_edit::validate::Decision::Error => ExitCode::from(EXIT_USAGE),
            }
        }
        Ok(other) => {
            eprintln!("[michelangelo] the submission returned an unexpected result: {other:?}");
            ExitCode::from(EXIT_USAGE)
        }
        Err(error) => {
            audit(run, "error", None, &task.task_id, &error.to_string(), by);
            failure(&error)
        }
    }
}

/// Append one line to the audit log of the run.
fn audit(
    run: &Path,
    decision: &str,
    candidate_id: Option<&str>,
    task_id: &str,
    detail: &str,
    by: Option<&str>,
) {
    let store = BudgetStore::new(run);
    let detail = match by {
        Some(by) => format!("{detail} (by '{by}')"),
        None => detail.to_string(),
    };
    let entry = AuditEntry {
        ts: SystemClock.now_rfc3339_utc(),
        tool: "submit_candidate".to_string(),
        decision: decision.to_string(),
        candidate_id: candidate_id.map(|value| value.to_string()),
        task_id: Some(task_id.to_string()),
        detail,
    };
    if let Err(error) = store.append_audit(&entry) {
        eprintln!("[michelangelo] the audit line cannot be written: {error}");
    }
}

/// Parse a region of the form `min=max`.
fn parse_region(text: &str) -> Result<Region, String> {
    let (min, max) = text
        .split_once('=')
        .ok_or("the region must have the form 'min=max'")?;
    let min = parse_corner(min, "min")?;
    let max = parse_corner(max, "max")?;
    let region = Region { min, max };
    if !region.is_valid() {
        return Err("the minimum must be below the maximum on every axis".to_string());
    }
    Ok(region)
}

/// Parse one corner of a region.
fn parse_corner(text: &str, name: &str) -> Result<[f64; 3], String> {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 3 {
        return Err(format!(
            "the corner '{name}' must carry three comma separated coordinates"
        ));
    }
    let mut corner = [0.0_f64; 3];
    for (index, part) in parts.iter().enumerate() {
        let value: f64 = part
            .trim()
            .parse()
            .map_err(|_| format!("the coordinate '{part}' of '{name}' is not a number"))?;
        if !value.is_finite() {
            return Err(format!("the coordinate '{part}' of '{name}' is not finite"));
        }
        corner[index] = value;
    }
    Ok(corner)
}

/// Print an error and map it to the exit code.
fn failure(error: &AssistantError) -> ExitCode {
    eprintln!("[michelangelo] {error}");
    ExitCode::from(error.exit_code())
}
