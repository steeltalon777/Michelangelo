//! `michelangelo unknown` subcommands: the state machine of the unknowns
//! (IT-8C).
//!
//! The commands drive the immutable state documents of the verification store
//! of a reconstruction run (`michelangelo_edit::unknowns`). The human
//! readable summaries go to stdout, the machine readable documents are
//! printed with `--json`, and the errors go to stderr.
//!
//! Exit codes:
//!
//! * `0` — the operation succeeded,
//! * `1` — the operation was refused (a typed code is printed),
//! * `2` — usage error, IO error or a busy store (`STORE_BUSY`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use michelangelo_edit::apply::accept_candidate_with_options;
use michelangelo_edit::candidate::Region;
use michelangelo_edit::gate::{resolve_evaluation_by_id, AcceptOptions};
use michelangelo_edit::scene::{load_current, SceneStore};
use michelangelo_edit::unknowns;
use michelangelo_edit::{EditError, SystemClock};

/// Exit code: the operation succeeded.
pub const EXIT_OK: u8 = 0;
/// Exit code: the operation was refused.
pub const EXIT_REFUSED: u8 = 1;
/// Exit code: usage or IO error.
pub const EXIT_USAGE: u8 = 2;

/// Subcommands of `michelangelo unknown`.
#[derive(Debug, Subcommand)]
pub enum UnknownCommand {
    /// Re-import the unknowns of the immutable report of the run.
    Sync {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// Print the report as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// List the unknowns of a run with their state.
    List {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The revision to inspect (the current one by default).
        #[arg(long, value_name = "REVISION")]
        revision: Option<String>,
        /// Print the list as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Show the state, the bindings, the confirmations and the evidence of
    /// the unknowns of a run.
    Show {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The revision to inspect (the current one by default).
        #[arg(long, value_name = "REVISION")]
        revision: Option<String>,
        /// Print the view as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Bind an unknown to a set of components and to a region.
    Bind {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The identifier of the unknown.
        #[arg(long, value_name = "ID")]
        unknown: String,
        /// The components the binding covers (comma separated).
        #[arg(long, value_name = "NAME", value_delimiter = ',')]
        components: Vec<String>,
        /// The region the binding covers (`min=max`, three comma separated
        /// coordinates on each side).
        #[arg(long, value_name = "MIN=MAX")]
        region: String,
        /// The author that creates the binding.
        #[arg(long, value_name = "NAME")]
        by: String,
        /// The claim of the author about the unknown.
        #[arg(long, value_name = "TEXT")]
        claim: Option<String>,
        /// Print the binding document as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Confirm an unknown on behalf of the author.
    Confirm {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The identifier of the unknown.
        #[arg(long, value_name = "ID")]
        unknown: String,
        /// The author that confirms (the owner or an authorized operator).
        #[arg(long, value_name = "NAME")]
        by: String,
        /// The identifier of the evaluation the confirmation is bound to.
        #[arg(long, value_name = "EVAL-ID")]
        evaluation: String,
        /// The evidence file of the author.
        #[arg(long, value_name = "FILE")]
        evidence: Option<PathBuf>,
        /// The claim the author accepts.
        #[arg(long, value_name = "TEXT")]
        claim: Option<String>,
        /// Print the confirmation as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Propose the resolution of an unknown (the scene is not modified).
    Propose {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The identifier of the unknown.
        #[arg(long, value_name = "ID")]
        unknown: String,
        /// The identifier of the candidate that resolves the unknown.
        #[arg(long, value_name = "ID")]
        candidate: String,
        /// The identifier of the evaluation of the candidate.
        #[arg(long, value_name = "EVAL-ID")]
        evaluation: String,
        /// The author that decides the resolution.
        #[arg(long, value_name = "NAME")]
        by: String,
        /// Print the resolution as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Resolve an unknown: the candidate is accepted and the revision
    /// references the resolution.
    Resolve {
        /// The run directory (the directory with report.json).
        #[arg(long, value_name = "DIR")]
        run: PathBuf,
        /// The identifier of the unknown.
        #[arg(long, value_name = "ID")]
        unknown: String,
        /// The identifier of the candidate that resolves the unknown.
        #[arg(long, value_name = "ID")]
        candidate: Option<String>,
        /// The identifier of the evaluation of the candidate.
        #[arg(long, value_name = "EVAL-ID")]
        evaluation: Option<String>,
        /// The author that decides the resolution.
        #[arg(long, value_name = "NAME")]
        by: String,
        /// Print the decision report as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
}

/// Execute an `unknown` subcommand.
pub fn execute(command: &UnknownCommand) -> ExitCode {
    match command {
        UnknownCommand::Sync { run, json } => sync(run, *json),
        UnknownCommand::List {
            run,
            revision,
            json,
        } => list(run, revision.as_deref(), *json),
        UnknownCommand::Show {
            run,
            revision,
            json,
        } => show(run, revision.as_deref(), *json),
        UnknownCommand::Bind {
            run,
            unknown,
            components,
            region,
            by,
            claim,
            json,
        } => bind(
            run,
            unknown,
            components,
            region,
            by,
            claim.as_deref(),
            *json,
        ),
        UnknownCommand::Confirm {
            run,
            unknown,
            by,
            evaluation,
            evidence,
            claim,
            json,
        } => confirm(
            run,
            unknown,
            by,
            evaluation,
            evidence.as_deref(),
            claim.as_deref(),
            *json,
        ),
        UnknownCommand::Propose {
            run,
            unknown,
            candidate,
            evaluation,
            by,
            json,
        } => propose(run, unknown, candidate, evaluation, by, *json),
        UnknownCommand::Resolve {
            run,
            unknown,
            candidate,
            evaluation,
            by,
            json,
        } => resolve(
            run,
            unknown,
            candidate.as_deref(),
            evaluation.as_deref(),
            by,
            *json,
        ),
    }
}

/// Re-import the unknowns of the immutable report of the run.
fn sync(run: &Path, json: bool) -> ExitCode {
    let store = SceneStore::new(run);
    let report = match unknowns::sync_unknowns(&store, &SystemClock) {
        Ok(report) => report,
        Err(error) => return failure(&error),
    };
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the report cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("scene_revision: {}", report.scene_revision);
        println!("imported: {}", report.imported);
        println!("state_existed: {}", report.state_existed);
        if report.missing.is_empty() {
            println!("missing: -");
        } else {
            println!("missing: {}", report.missing.join(", "));
        }
    }
    ExitCode::from(EXIT_OK)
}

/// List the unknowns of a run.
fn list(run: &Path, revision: Option<&str>, json: bool) -> ExitCode {
    let store = SceneStore::new(run);
    let entries = match unknowns::list_unknowns(&store, revision) {
        Ok(entries) => entries,
        Err(error) => return failure(&error),
    };
    if json {
        let value = serde_json::json!({
            "schema": "michelangelo.unknown-list/0",
            "unknowns": entries,
        });
        match serde_json::to_string_pretty(&value) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the list cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        for entry in &entries {
            println!(
                "{}: {} ({})",
                entry.unknown_id,
                entry.status,
                entry.reason.as_deref().unwrap_or("-")
            );
        }
    }
    ExitCode::from(EXIT_OK)
}

/// Show the state of the unknowns of a run.
fn show(run: &Path, revision: Option<&str>, json: bool) -> ExitCode {
    let store = SceneStore::new(run);
    let view = match unknowns::show_unknowns(&store, revision) {
        Ok(view) => view,
        Err(error) => return failure(&error),
    };
    if json {
        match serde_json::to_string_pretty(&view) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the view cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("scene_revision: {}", view.scene_revision);
        println!(
            "binding_version: {}",
            view.binding_version.as_deref().unwrap_or("-")
        );
        println!("unresolved: {}", view.unresolved.join(", "));
        for entry in &view.unknowns {
            println!(
                "  {}: {}{}",
                entry.unknown_id,
                entry.status,
                match &entry.reason {
                    Some(reason) => format!(" ({reason})"),
                    None => String::new(),
                }
            );
            if let Some(binding) = &entry.binding {
                println!(
                    "    binding: {} [{}]",
                    binding.by,
                    binding.components.join(", ")
                );
            }
            if let Some(resolution) = &entry.resolution {
                println!("    resolution: {}", resolution.resolution_id);
            }
            for confirmation in &entry.confirmations {
                println!(
                    "    confirmation: {} by {}",
                    confirmation.confirmation_id, confirmation.by
                );
            }
        }
    }
    ExitCode::from(EXIT_OK)
}

/// Bind an unknown to a set of components and to a region.
fn bind(
    run: &Path,
    unknown: &str,
    components: &[String],
    region: &str,
    by: &str,
    claim: Option<&str>,
    json: bool,
) -> ExitCode {
    let region = match parse_region(region) {
        Ok(region) => region,
        Err(message) => {
            eprintln!("[michelangelo] the region is not usable: {message}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    if components.is_empty() {
        eprintln!("[michelangelo] a binding needs at least one component");
        return ExitCode::from(EXIT_USAGE);
    }
    let store = SceneStore::new(run);
    let document = match unknowns::bind_unknown(
        &store,
        unknown,
        components,
        region,
        by,
        claim,
        &SystemClock,
    ) {
        Ok(document) => document,
        Err(error) => return failure(&error),
    };
    if json {
        match serde_json::to_string_pretty(&document) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the binding cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("version: {}", document.version);
        println!("bindings: {}", document.bindings.len());
        for binding in &document.bindings {
            println!(
                "  {}: {} by {}",
                binding.unknown_id,
                binding.components.join(", "),
                binding.by
            );
        }
    }
    ExitCode::from(EXIT_OK)
}

/// Confirm an unknown on behalf of the author.
fn confirm(
    run: &Path,
    unknown: &str,
    by: &str,
    evaluation: &str,
    evidence: Option<&Path>,
    claim: Option<&str>,
    json: bool,
) -> ExitCode {
    let store = SceneStore::new(run);
    let binding = match resolve_evaluation_by_id(&store, evaluation) {
        Ok(binding) => binding,
        Err(error) => return failure(&error),
    };
    let confirmation = match unknowns::confirm_unknown(
        &store,
        unknown,
        by,
        &binding,
        evidence,
        claim,
        &SystemClock,
    ) {
        Ok(confirmation) => confirmation,
        Err(error) => return failure(&error),
    };
    if json {
        match serde_json::to_string_pretty(&confirmation) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the confirmation cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("confirmation_id: {}", confirmation.confirmation_id);
        println!("unknown_id: {}", confirmation.unknown_id);
        println!("evaluation_id: {}", confirmation.evaluation_id);
        println!("by: {}", confirmation.by);
        println!("claim_digest: {}", confirmation.claim_digest);
        println!(
            "target_geometry_digest: {}",
            confirmation.target_geometry_digest
        );
    }
    ExitCode::from(EXIT_OK)
}

/// Propose the resolution of an unknown.
fn propose(
    run: &Path,
    unknown: &str,
    candidate: &str,
    evaluation: &str,
    by: &str,
    json: bool,
) -> ExitCode {
    let store = SceneStore::new(run);
    let binding = match resolve_evaluation_by_id(&store, evaluation) {
        Ok(binding) => binding,
        Err(error) => return failure(&error),
    };
    let manifest = match michelangelo_edit::verification::load_snapshot(&store, candidate) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            eprintln!(
                "[michelangelo] the candidate '{candidate}' carries no snapshot: it cannot resolve an unknown"
            );
            return ExitCode::from(EXIT_REFUSED);
        }
        Err(error) => return failure(&error),
    };
    let document = match load_candidate(&store, candidate) {
        Ok(document) => document,
        Err(error) => return failure(&error),
    };
    let resolution = match unknowns::propose_resolution(
        &store,
        unknown,
        &document,
        &manifest,
        &binding,
        by,
        &SystemClock,
    ) {
        Ok(resolution) => resolution,
        Err(error) => return failure(&error),
    };
    if json {
        match serde_json::to_string_pretty(&resolution) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the resolution cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("resolution_id: {}", resolution.resolution_id);
        println!("unknown_id: {}", resolution.unknown_id);
        println!("binding_version: {}", resolution.binding_version);
        println!("candidate_id: {}", resolution.candidate_id);
        println!("evaluation_id: {}", resolution.evaluation_id);
        println!("confirmation_id: {}", resolution.confirmation_id);
        println!("by: {}", resolution.by);
    }
    ExitCode::from(EXIT_OK)
}

/// Resolve an unknown: the candidate is accepted and the revision references
/// the resolution.
fn resolve(
    run: &Path,
    unknown: &str,
    candidate: Option<&str>,
    evaluation: Option<&str>,
    by: &str,
    json: bool,
) -> ExitCode {
    let store = SceneStore::new(run);
    let candidate = match candidate {
        Some(candidate) => candidate.to_string(),
        None => match unknowns::latest_resolution(&store, unknown) {
            Ok(Some(resolution)) => resolution.candidate_id,
            Ok(None) => {
                eprintln!(
                    "[michelangelo] the unknown '{unknown}' carries no proposal: run 'michelangelo unknown propose' first or pass --candidate"
                );
                return ExitCode::from(EXIT_USAGE);
            }
            Err(error) => return failure(&error),
        },
    };
    let evaluation = match evaluation {
        Some(evaluation) => evaluation.to_string(),
        None => {
            let pointer = match load_current(&store) {
                Ok(Some(pointer)) => pointer,
                Ok(None) => {
                    eprintln!("[michelangelo] the scene store is not initialized");
                    return ExitCode::from(EXIT_USAGE);
                }
                Err(error) => return failure(&error),
            };
            let _ = pointer;
            match unknowns::latest_resolution(&store, unknown) {
                Ok(Some(resolution)) => resolution.evaluation_id,
                Ok(None) => {
                    eprintln!(
                        "[michelangelo] the unknown '{unknown}' carries no proposal: pass --evaluation"
                    );
                    return ExitCode::from(EXIT_USAGE);
                }
                Err(error) => return failure(&error),
            }
        }
    };
    let path = store
        .run_dir
        .join(michelangelo_evidence::registry::report_relative_path(
            &evaluation,
        ));
    let options = AcceptOptions {
        evaluation: Some(path),
        resolve: Some(unknown.to_string()),
    };
    let report = match accept_candidate_with_options(run, &candidate, by, &SystemClock, options) {
        Ok(report) => report,
        Err(error) => return failure(&error),
    };
    if json {
        match report.to_json() {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("[michelangelo] the decision cannot be serialized: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        println!("decision: {}", report.decision.as_str());
        println!("candidate: {}", report.candidate_id);
        println!(
            "resulting_scene_revision: {}",
            report.resulting_scene_revision.as_deref().unwrap_or("-")
        );
        println!(
            "decided_by: {}",
            report.decided_by.as_deref().unwrap_or("-")
        );
    }
    ExitCode::from(EXIT_OK)
}

/// Read the candidate document of the store.
fn load_candidate(
    store: &SceneStore,
    candidate_id: &str,
) -> Result<michelangelo_edit::EditCandidate, EditError> {
    let path = store.candidate_path(candidate_id);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        michelangelo_edit::io_error(&path, format!("the candidate cannot be read: {error}"))
    })?;
    michelangelo_edit::candidate::parse_candidate(&text).map_err(|error| EditError::Schema {
        message: format!(
            "the candidate '{candidate_id}' is not usable: {}",
            error.message
        ),
    })
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
fn failure(error: &EditError) -> ExitCode {
    eprintln!("[michelangelo] {error}");
    ExitCode::from(error.exit_code())
}
