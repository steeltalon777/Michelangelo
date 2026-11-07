//! `michelangelo input` subcommands: package validation and description.
//!
//! The commands work on an input package revision directory (the directory that
//! contains `package.json`) and never write into it. Human readable
//! diagnostics go to stderr; stdout carries the machine readable report JSON
//! of `input validate` and the summary text of `input describe`.
//!
//! Exit codes (shared with the rest of the CLI):
//!
//! * `0` — the package is valid (complete or partial),
//! * `1` — the package is invalid, or incomplete while `--require-complete`
//!   is set,
//! * `2` — usage or IO error (missing package directory, unreadable manifest,
//!   report path inside the package directory, report file cannot be written).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use michelangelo_input::manifest::ViewKind;
use michelangelo_input::{
    load_manifest, validate_package, CompletenessStatus, InputPackageReport, ReportStatus,
    ValidateOptions,
};

/// Exit code: the package is valid.
pub const EXIT_OK: u8 = 0;
/// Exit code: the package is invalid (or incomplete with `--require-complete`).
pub const EXIT_INVALID: u8 = 1;
/// Exit code: usage or IO error.
pub const EXIT_USAGE: u8 = 2;

/// Subcommands of `michelangelo input`.
#[derive(Debug, Subcommand)]
pub enum InputCommand {
    /// Validate an input package and print the validation report as JSON.
    Validate {
        /// Package revision directory (the directory that contains package.json).
        #[arg(long, value_name = "DIR")]
        package: PathBuf,
        /// Fail when the package is not complete for metric reconstruction.
        #[arg(long)]
        require_complete: bool,
        /// Do not verify resource hashes; a mismatch becomes a warning.
        #[arg(long)]
        no_hash_verify: bool,
        /// Also write the report JSON to this file (must be outside the package).
        #[arg(long, value_name = "FILE")]
        report: Option<PathBuf>,
        /// Print only a one-line summary on stderr; stdout stays empty.
        #[arg(long)]
        quiet: bool,
    },
    /// Print a short summary of an input package.
    Describe {
        /// Package revision directory (the directory that contains package.json).
        #[arg(long, value_name = "DIR")]
        package: PathBuf,
    },
}

/// Run an `input` subcommand and return the process exit code.
pub fn run(command: &InputCommand) -> ExitCode {
    match command {
        InputCommand::Validate {
            package,
            require_complete,
            no_hash_verify,
            report,
            quiet,
        } => validate(
            package,
            *require_complete,
            *no_hash_verify,
            report.as_deref(),
            *quiet,
        ),
        InputCommand::Describe { package } => describe(package),
    }
}

fn validate(
    package: &Path,
    require_complete: bool,
    no_hash_verify: bool,
    report_path: Option<&Path>,
    quiet: bool,
) -> ExitCode {
    let directory = match resolve_package_dir(package) {
        Ok(directory) => directory,
        Err(message) => return usage_error(&message),
    };

    if let Some(path) = report_path {
        if is_inside_package(&directory, path) {
            return usage_error(&format!(
                "report path '{}' is inside the package directory '{}'; the CLI never writes into the package",
                path.display(),
                directory.display()
            ));
        }
    }

    let options = ValidateOptions {
        require_complete,
        verify_hashes: !no_hash_verify,
    };
    let report = validate_package(&directory, &options);

    let json = match serde_json::to_string_pretty(&report) {
        Ok(json) => json,
        Err(err) => return usage_error(&format!("the report cannot be serialized: {err}")),
    };

    if let Some(path) = report_path {
        if let Err(err) = fs::write(path, format!("{json}\n")) {
            return usage_error(&format!(
                "the report cannot be written to '{}': {err}",
                path.display()
            ));
        }
    }

    if !quiet {
        println!("{json}");
    }
    eprintln!("{}", summary_line(&report));

    exit_code(&report, require_complete)
}

fn describe(package: &Path) -> ExitCode {
    let directory = match resolve_package_dir(package) {
        Ok(directory) => directory,
        Err(message) => return usage_error(&message),
    };

    let report = validate_package(&directory, &ValidateOptions::default());
    if report.levels.format_safety.errors > 0 {
        for diagnostic in report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.level == 1)
        {
            eprintln!(
                "[michelangelo] {} {:?} at {}: {}",
                diagnostic.code, diagnostic.severity, diagnostic.address, diagnostic.message
            );
        }
        eprintln!("{}", summary_line(&report));
        return ExitCode::from(EXIT_INVALID);
    }

    let manifest = match load_manifest(&directory) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("[michelangelo] the manifest cannot be read: {err}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    println!("package_id: {}", manifest.package_id);
    println!("revision: {}", manifest.revision);
    println!("status: {}", status_text(&report));
    println!(
        "completeness: {}",
        completeness_text(report.completeness.status)
    );
    println!("calibrated_views: {}", report.completeness.calibrated_views);
    println!(
        "distinct_directions: {}",
        report.completeness.distinct_directions
    );
    println!(
        "axes_covered: {}",
        report.completeness.axes_covered.join(", ")
    );
    println!("views:");
    for view in &manifest.views {
        println!("  {}: {}", view.view_id, view_kind_text(view.kind));
    }
    println!("parts: {}", manifest.parts.len());
    println!("supplements: {}", manifest.supplements.len());
    println!("unknowns: {}", manifest.unknowns.len());
    println!("measurements: {}", manifest.measurements.len());

    ExitCode::from(EXIT_OK)
}

fn resolve_package_dir(package: &Path) -> Result<PathBuf, String> {
    if !package.exists() {
        return Err(format!(
            "package directory '{}' does not exist",
            package.display()
        ));
    }
    if !package.is_dir() {
        return Err(format!(
            "package path '{}' is not a directory",
            package.display()
        ));
    }
    Ok(package.to_path_buf())
}

fn is_inside_package(package: &Path, candidate: &Path) -> bool {
    let package_root = match fs::canonicalize(package) {
        Ok(root) => root,
        Err(_) => return false,
    };
    let candidate = match canonicalize_target(candidate) {
        Ok(path) => path,
        Err(_) => return false,
    };
    candidate.starts_with(&package_root)
}

/// Canonicalize a path that may not exist yet (its parent must exist).
fn canonicalize_target(path: &Path) -> Result<PathBuf, String> {
    if let Ok(canonical) = fs::canonicalize(path) {
        return Ok(canonical);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("'{}' is not a usable file path", path.display()))?;
    match parent {
        Some(parent) => {
            let canonical_parent = fs::canonicalize(parent).map_err(|err| {
                format!(
                    "the directory of '{}' cannot be resolved: {err}",
                    path.display()
                )
            })?;
            Ok(canonical_parent.join(file_name))
        }
        None => Err(format!("'{}' is not a usable file path", path.display())),
    }
}

fn status_text(report: &InputPackageReport) -> &'static str {
    match report.status {
        ReportStatus::ValidComplete => "valid_complete",
        ReportStatus::ValidPartial => "valid_partial",
        ReportStatus::Invalid => "invalid",
    }
}

fn completeness_text(status: CompletenessStatus) -> &'static str {
    match status {
        CompletenessStatus::Complete => "complete",
        CompletenessStatus::Partial => "partial",
    }
}

fn view_kind_text(kind: ViewKind) -> &'static str {
    match kind {
        ViewKind::Calibrated => "calibrated",
        ViewKind::Reference => "reference",
    }
}

fn summary_line(report: &InputPackageReport) -> String {
    format!(
        "[michelangelo] package {} (revision {}): {}, errors {}, warnings {}",
        report.package_id,
        report.revision,
        status_text(report),
        report.levels.format_safety.errors + report.levels.sufficiency.errors,
        report.levels.format_safety.warnings + report.levels.sufficiency.warnings,
    )
}

fn exit_code(report: &InputPackageReport, require_complete: bool) -> ExitCode {
    if report.has_errors() {
        return ExitCode::from(EXIT_INVALID);
    }
    if require_complete && report.status != ReportStatus::ValidComplete {
        return ExitCode::from(EXIT_INVALID);
    }
    ExitCode::from(EXIT_OK)
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("[michelangelo] {message}");
    ExitCode::from(EXIT_USAGE)
}
