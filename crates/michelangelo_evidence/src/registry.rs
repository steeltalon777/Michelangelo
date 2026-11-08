//! The append-only registry of the evaluation reports (contract §2).
//!
//! The registry is the only place that holds the digest of an evaluation
//! report. The report itself never self-certifies: the accept gate
//! re-computes the digest of the file and compares it with the registry.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EvidenceError;

/// The name of the verification directory inside a run directory.
pub const VERIFICATION_DIR_NAME: &str = "verification";
/// The name of the evaluations directory inside the verification directory.
pub const EVALUATIONS_DIR_NAME: &str = "evaluations";
/// The name of the registry file inside the evaluations directory.
pub const REGISTRY_FILE_NAME: &str = "index.json";
/// The name of the report file inside an evaluation directory.
pub const REPORT_FILE_NAME: &str = "report.json";

/// The schema of the registry of the evaluations.
pub const EVALUATION_REGISTRY_SCHEMA: &str = "michelangelo.evaluation-registry/0";

/// One registered evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// The identifier of the evaluation.
    pub evaluation_id: String,
    /// The path of the report, relative to the run directory.
    pub report_path: String,
    /// The SHA-256 of the bytes of the report.
    pub sha256: String,
    /// The aggregate digest of the geometry the evaluation measured.
    pub target_digest: String,
    /// The creation time of the report (RFC 3339, UTC).
    pub created_at: String,
}

/// The append-only registry of the evaluations of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRegistry {
    /// The schema identifier.
    pub schema: String,
    /// The registered evaluations, ordered by the identifier.
    pub evaluations: Vec<RegistryEntry>,
}

impl Default for EvaluationRegistry {
    fn default() -> Self {
        EvaluationRegistry {
            schema: EVALUATION_REGISTRY_SCHEMA.to_string(),
            evaluations: Vec::new(),
        }
    }
}

impl EvaluationRegistry {
    /// An empty registry.
    pub fn empty() -> Self {
        EvaluationRegistry::default()
    }

    /// The entry of an evaluation (None when the evaluation is unknown).
    pub fn entry(&self, evaluation_id: &str) -> Option<&RegistryEntry> {
        self.evaluations
            .iter()
            .find(|entry| entry.evaluation_id == evaluation_id)
    }

    /// Register an evaluation.
    ///
    /// The registry is append-only: an entry is never overwritten. Registering
    /// the same evaluation twice with the same digest is a no-op, registering
    /// it with a different digest is an error.
    pub fn register(&mut self, entry: RegistryEntry) -> Result<(), EvidenceError> {
        if let Some(existing) = self.entry(&entry.evaluation_id) {
            if existing.sha256 == entry.sha256 && existing.report_path == entry.report_path {
                return Ok(());
            }
            return Err(EvidenceError::AlreadyRegistered {
                message: format!(
                    "the evaluation '{}' is already registered with the digest {}",
                    entry.evaluation_id, existing.sha256
                ),
            });
        }
        self.evaluations.push(entry);
        self.evaluations
            .sort_by(|left, right| left.evaluation_id.cmp(&right.evaluation_id));
        Ok(())
    }

    /// The next free identifier of an evaluation.
    pub fn next_evaluation_id(&self) -> String {
        let mut highest = 0_u32;
        for entry in &self.evaluations {
            if let Some(number) = entry
                .evaluation_id
                .strip_prefix("ev-")
                .and_then(|text| text.parse::<u32>().ok())
            {
                if number > highest {
                    highest = number;
                }
            }
        }
        format!("ev-{:04}", highest + 1)
    }
}

/// The path of the registry inside a run directory.
pub fn registry_path(run_dir: &Path) -> PathBuf {
    run_dir
        .join(VERIFICATION_DIR_NAME)
        .join(EVALUATIONS_DIR_NAME)
        .join(REGISTRY_FILE_NAME)
}

/// The path of the directory of an evaluation inside a run directory.
pub fn evaluation_dir(run_dir: &Path, evaluation_id: &str) -> PathBuf {
    run_dir
        .join(VERIFICATION_DIR_NAME)
        .join(EVALUATIONS_DIR_NAME)
        .join(evaluation_id)
}

/// The path of the report of an evaluation inside a run directory.
pub fn report_path(run_dir: &Path, evaluation_id: &str) -> PathBuf {
    evaluation_dir(run_dir, evaluation_id).join(REPORT_FILE_NAME)
}

/// The path of the report of an evaluation, relative to the run directory.
pub fn report_relative_path(evaluation_id: &str) -> String {
    format!("{VERIFICATION_DIR_NAME}/{EVALUATIONS_DIR_NAME}/{evaluation_id}/{REPORT_FILE_NAME}")
}

/// Read the registry of a run (an empty registry when it does not exist).
pub fn load_registry(run_dir: &Path) -> Result<EvaluationRegistry, EvidenceError> {
    let path = registry_path(run_dir);
    if !path.is_file() {
        return Ok(EvaluationRegistry::empty());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| EvidenceError::io(&path, format!("the registry cannot be read: {err}")))?;
    let registry: EvaluationRegistry =
        serde_json::from_str(&text).map_err(|err| EvidenceError::Invalid {
            message: format!("the registry '{}' is not usable: {err}", path.display()),
        })?;
    Ok(registry)
}

/// Write the registry of a run (write-to-temp + rename).
///
/// The entries are ordered by the identifier of the evaluation, so the file
/// is deterministic.
pub fn write_registry(run_dir: &Path, registry: &EvaluationRegistry) -> Result<(), EvidenceError> {
    let path = registry_path(run_dir);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|err| {
        EvidenceError::io(parent, format!("the directory cannot be created: {err}"))
    })?;
    let mut ordered = registry.clone();
    ordered
        .evaluations
        .sort_by(|left, right| left.evaluation_id.cmp(&right.evaluation_id));
    let text = serde_json::to_string_pretty(&ordered).map_err(|err| EvidenceError::Invalid {
        message: format!("the registry cannot be serialized: {err}"),
    })?;
    let temporary = parent.join(format!("{REGISTRY_FILE_NAME}.tmp"));
    std::fs::write(
        &temporary,
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
    .map_err(|err| EvidenceError::io(&temporary, format!("the file cannot be written: {err}")))?;
    std::fs::rename(&temporary, &path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        EvidenceError::io(&path, format!("the file cannot be published: {err}"))
    })?;
    Ok(())
}

/// The registered evaluations of a target (the identifier of a candidate or
/// of a scene revision).
///
/// The reports are immutable, so the lookup reads them from the disk. The
/// entries are returned in the order of the registry (the identifier of the
/// evaluation).
pub fn registered_evaluations_for_target(
    run_dir: &Path,
    registry: &EvaluationRegistry,
    target_id: &str,
) -> Vec<(RegistryEntry, crate::report::EvaluationReport)> {
    let mut found: Vec<(RegistryEntry, crate::report::EvaluationReport)> = Vec::new();
    for entry in &registry.evaluations {
        let path = run_dir.join(&entry.report_path);
        if let Ok(report) = crate::report::EvaluationReport::load(&path) {
            if report.target.id == target_id {
                found.push((entry.clone(), report));
            }
        }
    }
    found
}

/// Write the report of an evaluation and register it in the registry.
///
/// The report is written first and the digest of the written bytes is
/// registered: the registry never carries a digest of bytes that are not on
/// the disk.
pub fn write_and_register(
    run_dir: &Path,
    report: &crate::report::EvaluationReport,
) -> Result<RegistryEntry, EvidenceError> {
    let path = report_path(run_dir, &report.evaluation_id);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|err| {
        EvidenceError::io(parent, format!("the directory cannot be created: {err}"))
    })?;
    let text = report.to_json()?;
    let bytes = format!(
        "{text}
"
    )
    .into_bytes();
    let digest = crate::digest::sha256_hex(&bytes);

    // A published report is never overwritten. The registry entry and the
    // file are checked before anything is written: the registration of the
    // same evaluation with the same digest is a no-op, everything else is
    // refused.
    let mut registry = load_registry(run_dir)?;
    if let Some(existing) = registry.entry(&report.evaluation_id) {
        if existing.sha256 == digest
            && existing.report_path == report_relative_path(&report.evaluation_id)
        {
            return Ok(existing.clone());
        }
        return Err(EvidenceError::AlreadyRegistered {
            message: format!(
                "the evaluation '{}' is already registered with the digest {}",
                report.evaluation_id, existing.sha256
            ),
        });
    }
    if path.exists() {
        return Err(EvidenceError::AlreadyRegistered {
            message: format!(
                "the report of the evaluation '{}' already exists and is never overwritten: '{}'",
                report.evaluation_id,
                path.display()
            ),
        });
    }

    // The bytes are written into a temporary file and published with a link:
    // the link fails when the report exists, so a published report can never
    // be clobbered by a concurrent process.
    let temporary = parent.join(format!("{REPORT_FILE_NAME}.tmp"));
    std::fs::write(&temporary, &bytes).map_err(|err| {
        EvidenceError::io(&temporary, format!("the file cannot be written: {err}"))
    })?;
    match std::fs::hard_link(&temporary, &path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&temporary);
        }
        Err(_) => {
            let _ = std::fs::remove_file(&temporary);
            if path.exists() {
                return Err(EvidenceError::AlreadyRegistered {
                    message: format!(
                        "the report of the evaluation '{}' already exists and is never overwritten: '{}'",
                        report.evaluation_id,
                        path.display()
                    ),
                });
            }
            // A file system without hard links: the report is created
            // exclusively (it never replaces an existing one) and written.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|err| {
                    EvidenceError::io(&path, format!("the report cannot be created: {err}"))
                })?;
            file.write_all(&bytes).map_err(|err| {
                EvidenceError::io(&path, format!("the report cannot be written: {err}"))
            })?;
            file.sync_all().map_err(|err| {
                EvidenceError::io(&path, format!("the report cannot be flushed: {err}"))
            })?;
        }
    }

    let entry = RegistryEntry {
        evaluation_id: report.evaluation_id.clone(),
        report_path: report_relative_path(&report.evaluation_id),
        sha256: digest,
        target_digest: report.target.aggregate_sha256.clone(),
        created_at: report.created_at.clone(),
    };
    registry.register(entry.clone())?;
    write_registry(run_dir, &registry)?;
    Ok(entry)
}
