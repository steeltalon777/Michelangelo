//! `michelangelo reconstruct` subcommands: run, score and show.
//!
//! The commands drive the Stage B reconstruction pipeline
//! (`michelangelo_recon`). Human readable summaries go to stdout, the
//! machine readable report JSON is printed with `--json`, and errors go to
//! stderr.
//!
//! Exit codes:
//!
//! * `0` — the run passed every quality threshold (or `show` succeeded),
//! * `1` — the run completed but a quality threshold was not met,
//! * `2` — invalid input, a broken setup or an I/O error.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use michelangelo_recon::report::ReconstructionReport;
use michelangelo_recon::{ReconOptions, ScoreOptions};

/// The name of the report file inside a run directory.
pub const REPORT_FILE_NAME: &str = "report.json";

/// Subcommands of `michelangelo reconstruct`.
#[derive(Debug, Subcommand)]
pub enum ReconstructCommand {
    /// Run the full reconstruction pipeline for an input package.
    Run {
        /// The input package directory (the directory with package.json).
        #[arg(long, value_name = "DIR")]
        package: PathBuf,
        /// The workspace that owns the run store.
        #[arg(long, value_name = "DIR")]
        workspace: PathBuf,
        /// The voxel resolution of the solver.
        #[arg(long, value_name = "N")]
        resolution: Option<u32>,
        /// The solver script (default: the solver of this repository).
        #[arg(long, value_name = "PATH")]
        solver: Option<PathBuf>,
        /// The Python interpreter (default: python).
        #[arg(long, value_name = "PATH")]
        python: Option<PathBuf>,
        /// The Blender executable (default: BLENDER_BIN or blender).
        #[arg(long, value_name = "PATH")]
        blender: Option<PathBuf>,
        /// The timeout of the Blender job in milliseconds.
        #[arg(long, value_name = "MS")]
        timeout_ms: Option<u64>,
        /// Print the report JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Score a finished run against a held-out evaluation package.
    Score {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The evaluation package directory.
        #[arg(long, value_name = "DIR")]
        evaluation: PathBuf,
        /// The Blender executable (default: BLENDER_BIN or blender).
        #[arg(long, value_name = "PATH")]
        blender: Option<PathBuf>,
        /// Print the report JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Print the stored report of a run.
    Show {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// Print the report JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
}

/// Execute a `reconstruct` subcommand.
pub fn execute(command: &ReconstructCommand) -> ExitCode {
    match command {
        ReconstructCommand::Run {
            package,
            workspace,
            resolution,
            solver,
            python,
            blender,
            timeout_ms,
            json,
        } => {
            let options = ReconOptions {
                resolution: *resolution,
                solver: solver.clone(),
                python: python.clone(),
                blender: blender.clone(),
                timeout_ms: *timeout_ms,
                solver_timeout_ms: None,
                fake_blender: None,
            };
            match michelangelo_recon::run(package, workspace, &options) {
                Ok(report) => print_report(&report, *json),
                Err(error) => failure(&error),
            }
        }
        ReconstructCommand::Score {
            run,
            evaluation,
            blender,
            json,
        } => {
            let options = ScoreOptions {
                blender: blender.clone(),
                timeout_ms: None,
                solver_timeout_ms: None,
                fake_blender: None,
            };
            match michelangelo_recon::score(run, evaluation, &options) {
                Ok(report) => print_report(&report, *json),
                Err(error) => failure(&error),
            }
        }
        ReconstructCommand::Show { run, json } => show(run, *json),
    }
}

/// Print the stored report of a run.
///
/// The command succeeds when the report can be read; the stored status is
/// part of the printed output, not of the exit code.
fn show(run_dir: &Path, json: bool) -> ExitCode {
    let path = run_dir.join(REPORT_FILE_NAME);
    let report = match ReconstructionReport::load(&path) {
        Ok(report) => report,
        Err(error) => return failure(&error),
    };
    if json {
        match report.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => return failure(&error),
        }
    } else {
        print_summary(&report);
    }
    ExitCode::from(0)
}

/// Print the report (JSON or summary) and map the status to the exit code.
fn print_report(report: &ReconstructionReport, json: bool) -> ExitCode {
    if json {
        match report.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => return failure(&error),
        }
    } else {
        print_summary(report);
    }
    ExitCode::from(report.status.exit_code())
}

/// Print a human readable summary of a report.
fn print_summary(report: &ReconstructionReport) {
    println!("run: {}", report.run.run_id);
    println!("run_dir: {}", report.run.run_dir);
    println!(
        "package: {} (revision {})",
        report.package.package_id, report.package.revision
    );
    println!("validation: {}", report.package.validation_status);
    println!(
        "solver: {} v{} (resolution {}, grid {}x{}x{})",
        report.solver.name,
        report.solver.version,
        report.solver.resolution,
        report.solver.grid[0],
        report.solver.grid[1],
        report.solver.grid[2]
    );
    println!(
        "build: {} components, {} supplements, {} unknowns",
        report.build.components.len(),
        report.build.supplements.len(),
        report.build.unknowns.len()
    );
    println!("worker: {}", report.worker.status);
    println!(
        "quality: source mean IoU {:.4}, min IoU {:.4} ({}), status {}",
        report.quality.source.mean_iou,
        report.quality.source.min_iou,
        report.quality.status,
        report.quality.status
    );
    if let Some(evaluation) = &report.quality.evaluation {
        println!(
            "evaluation: mean IoU {:.4}, min IoU {:.4}",
            evaluation.mean_iou, evaluation.min_iou
        );
    }
    println!("status: {}", report.status.as_str());
    for item in &report.unresolved {
        println!("unresolved: {} ({})", item.id, item.reason);
    }
}

/// Print an error and map it to the exit code.
fn failure(error: &michelangelo_recon::ReconError) -> ExitCode {
    eprintln!("[michelangelo] {error}");
    ExitCode::from(error.exit_code())
}
