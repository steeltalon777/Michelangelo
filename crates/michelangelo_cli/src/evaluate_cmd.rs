//! The `michelangelo evaluate` subcommand: the evaluation of a candidate or
//! of a scene revision (IT-8B).
//!
//! The command renders the actual geometry of the target and of the base
//! revision, compares both with the pinned masks of the run and writes an
//! immutable evaluation report into the store. The Core registers the digest
//! of the report; the report is registered even when the verdict fails (the
//! evidence of a refusal).
//!
//! Exit codes:
//!
//! * `0` — the evaluation passes,
//! * `1` — the evaluation fails or the evidence is insufficient,
//! * `2` — usage error, IO error or a failed render or comparison.

use std::path::Path;
use std::process::ExitCode;

use michelangelo_eval::{evaluate_candidate, evaluate_revision, EvalError, EvaluationOptions};
use michelangelo_evidence::report::{EvaluationReport, TriState};

/// Exit code: the evaluation passed.
pub const EXIT_OK: u8 = 0;
/// Exit code: the evaluation failed or the evidence is insufficient.
pub const EXIT_FAILED: u8 = 1;
/// Exit code: usage or IO error.
pub const EXIT_USAGE: u8 = 2;

/// Execute the evaluation of a candidate or of a revision.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    run: &Path,
    candidate: Option<&str>,
    revision: Option<&str>,
    evaluation: Option<&Path>,
    blender: Option<&Path>,
    json: bool,
) -> ExitCode {
    match (candidate, revision) {
        (Some(_), Some(_)) => {
            eprintln!(
                "[michelangelo] the evaluation needs either --candidate or --revision, not both"
            );
            return ExitCode::from(EXIT_USAGE);
        }
        (None, None) => {
            eprintln!("[michelangelo] the evaluation needs --candidate or --revision");
            return ExitCode::from(EXIT_USAGE);
        }
        _ => {}
    }
    if !run.is_dir() {
        eprintln!(
            "[michelangelo] the run directory '{}' does not exist",
            run.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }

    let options = EvaluationOptions {
        evaluation_package: evaluation.map(|path| path.to_path_buf()),
        blender: blender.map(|path| path.to_path_buf()),
        ..EvaluationOptions::default()
    };

    let report = match (candidate, revision) {
        (Some(candidate), _) => evaluate_candidate(run, candidate, &options),
        (_, Some(revision)) => evaluate_revision(run, revision, &options),
        _ => unreachable!(),
    };
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[michelangelo] {error}");
            return ExitCode::from(exit_code_of(&error));
        }
    };

    if json {
        match report.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the report cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        print_summary(&report);
    }
    match report.verdict() {
        TriState::Pass => ExitCode::from(EXIT_OK),
        _ => ExitCode::from(EXIT_FAILED),
    }
}

/// Print the human readable summary of a report.
fn print_summary(report: &EvaluationReport) {
    println!("evaluation: {}", report.evaluation_id);
    println!(
        "target: {} {}",
        report.target.kind.as_str(),
        report.target.id
    );
    println!("mean_iou: {:.4}", report.visible.mean_iou);
    println!("min_iou: {:.4}", report.visible.min_iou);
    println!(
        "projection_conformance: {}",
        report.results.projection_conformance.as_str()
    );
    println!(
        "author_geometric_conditions: {}",
        report.results.author_geometric_conditions.as_str()
    );
    println!(
        "independent_ground_truth: {}",
        report.results.independent_ground_truth.as_str()
    );
    println!(
        "author_hidden_decision: {}",
        report.results.author_hidden_decision.as_str()
    );
    println!("verdict: {}", report.verdict().as_str());
}

/// The exit code of a failed evaluation.
fn exit_code_of(error: &EvalError) -> u8 {
    match error {
        EvalError::Missing { .. } | EvalError::Invalid { .. } => EXIT_FAILED,
        _ => EXIT_USAGE,
    }
}
