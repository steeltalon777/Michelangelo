//! The accept gate of the local edit boundary (IT-8B).
//!
//! The gate is the bridge between the evidence of the run and the applier:
//! it resolves the evaluation report of an acceptance, verifies it against
//! the pins of the store and refuses the acceptance with a typed error when
//! the evidence does not hold.

use std::path::PathBuf;

use michelangelo_evidence::gate::{verify_report, GateContext};
use michelangelo_evidence::registry::{
    load_registry, registered_evaluations_for_target, report_relative_path,
};
use michelangelo_evidence::report::{
    package_mask_paths, package_measurement_ids, package_views, EvaluationReport, TargetKind,
    ViewPin,
};
use serde_json::Value;

use crate::candidate::EditCandidate;
use crate::error::EditError;
use crate::scene::SceneStore;
use crate::verification::{file_sha256, package_digest, SnapshotManifest};

/// The options of an acceptance.
#[derive(Debug, Clone, Default)]
pub struct AcceptOptions {
    /// The path of the evaluation report the acceptance is based on.
    ///
    /// When the option is absent, the gate uses the last evaluation that the
    /// Core registered for the candidate. A candidate without a registered
    /// evaluation can never be accepted.
    pub evaluation: Option<PathBuf>,
    /// The unknown the acceptance resolves.
    ///
    /// The option is optional: an acceptance without it accepts the geometry
    /// and leaves every unknown open (partial acceptance, contract §7).
    pub resolve: Option<String>,
}

/// The evaluation an acceptance is based on.
#[derive(Debug, Clone)]
pub struct EvaluationBinding {
    /// The identifier of the evaluation.
    pub evaluation_id: String,
    /// The path of the report, relative to the run directory.
    pub report_path: String,
    /// The digest of the bytes of the report.
    pub sha256: String,
    /// The report itself.
    pub report: EvaluationReport,
}

/// Resolve the evaluation of an acceptance.
///
/// The report must be registered in the registry of the Core: an
/// unregistered report is never trusted, and the digest of the file is
/// re-computed instead of being taken from the report.
pub fn resolve_evaluation(
    store: &SceneStore,
    candidate_id: &str,
    options: &AcceptOptions,
) -> Result<EvaluationBinding, EditError> {
    let registry = load_registry(&store.run_dir).map_err(EditError::from)?;
    match &options.evaluation {
        Some(path) => {
            if !path.is_file() {
                return Err(EditError::EvaluationMissing {
                    message: format!(
                        "the evaluation report '{}' does not exist: the candidate '{candidate_id}' cannot be accepted without an evaluation",
                        path.display()
                    ),
                });
            }
            let bytes = std::fs::read(path).map_err(|err| {
                crate::io_error(path, format!("the evaluation report cannot be read: {err}"))
            })?;
            let sha256 = michelangelo_evidence::digest::sha256_hex(&bytes);
            let report: EvaluationReport =
                serde_json::from_slice(&bytes).map_err(|err| EditError::EvaluationMissing {
                    message: format!(
                        "the evaluation report '{}' is not usable: {err}",
                        path.display()
                    ),
                })?;
            let entry = match registry.entry(&report.evaluation_id) {
                Some(entry) => entry,
                None => {
                    return Err(EditError::EvaluationUnregistered {
                        message: format!(
                            "the evaluation '{}' is not registered in the registry of the run",
                            report.evaluation_id
                        ),
                    })
                }
            };
            if entry.sha256 != sha256 {
                return Err(EditError::EvaluationStale {
                    message: format!(
                        "the evaluation report '{}' carries the digest {sha256} but the registry pins {}",
                        path.display(),
                        entry.sha256
                    ),
                });
            }
            // The registered path must carry the registered bytes: a valid
            // copy of a corrupted report is never trusted.
            let registered = store.run_dir.join(&entry.report_path);
            let registered_sha256 = file_sha256(&registered)?;
            if registered_sha256 != entry.sha256 {
                return Err(EditError::EvaluationStale {
                    message: format!(
                        "the registered evaluation report '{}' carries the digest {registered_sha256} but the registry pins {}",
                        registered.display(),
                        entry.sha256
                    ),
                });
            }
            Ok(EvaluationBinding {
                evaluation_id: report.evaluation_id.clone(),
                report_path: entry.report_path.clone(),
                sha256,
                report,
            })
        }
        None => {
            let registered =
                registered_evaluations_for_target(&store.run_dir, &registry, candidate_id);
            let (entry, report) = match registered.last() {
                Some(found) => found.clone(),
                None => {
                    return Err(EditError::EvaluationMissing {
                        message: format!(
                            "the candidate '{candidate_id}' carries no registered evaluation: run 'michelangelo evaluate' before the acceptance"
                        ),
                    })
                }
            };
            let path = store.run_dir.join(&entry.report_path);
            let sha256 = file_sha256(&path)?;
            if sha256 != entry.sha256 {
                return Err(EditError::EvaluationStale {
                    message: format!(
                        "the evaluation report '{}' carries the digest {sha256} but the registry pins {}",
                        path.display(),
                        entry.sha256
                    ),
                });
            }
            Ok(EvaluationBinding {
                evaluation_id: entry.evaluation_id.clone(),
                report_path: entry.report_path.clone(),
                sha256,
                report,
            })
        }
    }
}

/// Resolve a registered evaluation by its identifier.
///
/// The report must be registered in the registry of the Core and the digest
/// of the file is re-computed: an unregistered or a tampered report is never
/// trusted.
pub fn resolve_evaluation_by_id(
    store: &SceneStore,
    evaluation_id: &str,
) -> Result<EvaluationBinding, EditError> {
    let registry = load_registry(&store.run_dir).map_err(EditError::from)?;
    let entry = registry
        .entry(evaluation_id)
        .ok_or_else(|| EditError::EvaluationUnregistered {
            message: format!(
                "the evaluation '{evaluation_id}' is not registered in the registry of the run"
            ),
        })?;
    let path = store.run_dir.join(&entry.report_path);
    if !path.is_file() {
        return Err(EditError::EvaluationMissing {
            message: format!(
                "the report of the evaluation '{evaluation_id}' does not exist: {}",
                path.display()
            ),
        });
    }
    let sha256 = file_sha256(&path)?;
    if sha256 != entry.sha256 {
        return Err(EditError::EvaluationStale {
            message: format!(
                "the evaluation report of '{evaluation_id}' carries the digest {sha256} but the registry pins {}",
                entry.sha256
            ),
        });
    }
    let report = michelangelo_evidence::report::EvaluationReport::load(&path).map_err(|error| {
        EditError::EvaluationStale {
            message: format!(
                "the report of the evaluation '{evaluation_id}' is not usable: {}",
                error.message()
            ),
        }
    })?;
    Ok(EvaluationBinding {
        evaluation_id: report.evaluation_id.clone(),
        report_path: entry.report_path.clone(),
        sha256,
        report,
    })
}

/// Verify the evaluation of an acceptance against the store.
///
/// The checks are the ones of the contract §7: the registered digest, the
/// target aggregate, the base revision, the pinned inputs, the projection
/// conformance, the required author geometric conditions and the per-view
/// regression.
pub fn verify_evaluation(
    store: &SceneStore,
    candidate: &EditCandidate,
    manifest: &SnapshotManifest,
    binding: &EvaluationBinding,
) -> Result<(), EditError> {
    let registry = load_registry(&store.run_dir).map_err(EditError::from)?;
    let package = package_json(store);
    let mut current_views: Vec<ViewPin> = match &package {
        Some(value) => package_views(value)
            .iter()
            .map(|view| view.pin(false))
            .collect(),
        None => Vec::new(),
    };
    // The pins of the run are the digests of the bytes the run really
    // carries: the mask file of every view is read again and hashed.
    let mask_digests = match &package {
        Some(value) => current_mask_digests(store, value)?,
        None => Vec::new(),
    };
    for view in current_views.iter_mut() {
        if let Some((_, digest)) = mask_digests
            .iter()
            .find(|(view_id, _)| *view_id == view.view_id)
        {
            view.mask_sha256 = digest.clone();
        }
    }
    let required_conditions: Vec<String> = match &package {
        Some(value) => package_measurement_ids(value),
        None => Vec::new(),
    };
    let decision_digest =
        latest_decision(store, &candidate.candidate_id).and_then(|report| report.evaluation_sha256);

    let package_sha256 = package_digest(store);
    let run_id = crate::scene::run_id_of(&store.run_dir);
    let context = GateContext {
        run_id: &run_id,
        target_kind: TargetKind::Candidate,
        target_id: &candidate.candidate_id,
        target_aggregate_sha256: &manifest.aggregate_sha256,
        base_scene_revision: Some(candidate.base.scene_revision.as_str()),
        current_package_sha256: package_sha256.as_deref(),
        current_source_views: &current_views,
        required_condition_ids: &required_conditions,
        decision_evaluation_sha256: decision_digest.as_deref(),
    };
    verify_report(&registry, &binding.report, &binding.sha256, &context).map_err(EditError::from)
}

/// The reference of an evaluation inside the bundle of a revision.
pub fn evaluation_ref(binding: &EvaluationBinding) -> crate::scene::EvaluationRef {
    crate::scene::EvaluationRef {
        evaluation_id: binding.evaluation_id.clone(),
        path: binding.report_path.clone(),
        sha256: binding.sha256.clone(),
    }
}

/// The relative path of the report of an evaluation.
pub fn relative_report_path(evaluation_id: &str) -> String {
    report_relative_path(evaluation_id)
}

/// The digests of the mask files the run really carries.
///
/// The pairs are the identifier of the view and the SHA-256 of the bytes of
/// the mask file. A mask file that is missing is an input that changed: the
/// acceptance is refused.
fn current_mask_digests(
    store: &SceneStore,
    package: &Value,
) -> Result<Vec<(String, String)>, EditError> {
    let package_dir = match store.package_copy_path().parent() {
        Some(parent) => parent.to_path_buf(),
        None => return Ok(Vec::new()),
    };
    let separator = std::path::MAIN_SEPARATOR.to_string();
    let mut digests: Vec<(String, String)> = Vec::new();
    for (view_id, relative) in package_mask_paths(package) {
        let path = package_dir.join(relative.replace('/', &separator));
        if !path.is_file() {
            return Err(EditError::InputChanged {
                message: format!(
                    "the mask file of the view '{view_id}' is missing: '{}'",
                    path.display()
                ),
            });
        }
        digests.push((view_id, file_sha256(&path)?));
    }
    Ok(digests)
}

/// The copy of the input package of the run (None when the run carries none).
fn package_json(store: &SceneStore) -> Option<Value> {
    let path = store.package_copy_path();
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The latest decision of a candidate (None when it has none).
fn latest_decision(
    store: &SceneStore,
    candidate_id: &str,
) -> Option<crate::validate::DecisionReport> {
    let directory = store.decisions_dir(candidate_id);
    if !directory.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(&directory).ok()?;
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name == "decision.json" {
            continue;
        }
        let sequence = match name
            .split('-')
            .next()
            .and_then(|text| text.parse::<u32>().ok())
        {
            Some(sequence) => sequence,
            None => continue,
        };
        let replace = match &best {
            Some((current, _)) => sequence > *current,
            None => true,
        };
        if replace {
            best = Some((sequence, entry.path()));
        }
    }
    let path = best?.1;
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}
