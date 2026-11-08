//! The no-clobber registry of validation reports plus verification
//! (`validation.verify` port of the contract gate §7.5).
//!
//! Guarantees (IT-8D D1.3 analog for the new bundle path):
//! - the report id and its work directory are reserved BEFORE the heavy
//!   evaluation; the reservation is an atomic create-new file;
//! - a registered report is never overwritten: same id + same digest is an
//!   idempotent no-op, same id + different bytes is `NO_CLOBBER_VIOLATION`;
//! - the registry index is append-only and carries the digest of the bytes
//!   actually written;
//! - `verify` recomputes the report digest, re-reads the inputs (F2), and
//!   compares the pinned profile basis (F19).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::canonical::{digest_document, sha256_hex, to_pretty_json};
use crate::error::ValidationError;
use crate::ids::{validate_id, validate_rel_path, resolve_under};
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;

/// The draft registry schema tag (module-local, provisional).
pub const REGISTRY_SCHEMA: &str = "michelangelo.validation-report-registry/draft-0.1";

/// Directory names inside the validation workspace.
pub const VALIDATION_DIR: &str = "validation";
/// The reservations directory.
pub const RESERVATIONS_DIR: &str = "reservations";
/// The reports directory.
pub const REPORTS_DIR: &str = "reports";
/// The acceptance-record replay index directory (test seam device).
pub const ACCEPTANCE_DIR: &str = "acceptance-records";

/// One registered report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub report_id: String,
    /// The report path relative to the validation workspace root.
    pub report_path: String,
    /// The SHA-256 of the report bytes actually written.
    pub sha256: String,
    /// The digest of the measured bundle manifest bytes.
    pub bundle_sha256: String,
    /// The pinned profile digest of the evaluation.
    pub profile_sha256: String,
    /// The aggregate verdict at registration time.
    pub verdict: crate::check::Verdict,
    pub created_at: String,
}

/// The append-only registry index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRegistry {
    pub schema: String,
    pub entries: Vec<RegistryEntry>,
}

impl Default for ReportRegistry {
    fn default() -> Self {
        ReportRegistry {
            schema: REGISTRY_SCHEMA.to_string(),
            entries: Vec::new(),
        }
    }
}

/// The path of the registry index.
pub fn registry_path(root: &std::path::Path) -> PathBuf {
    root.join(VALIDATION_DIR).join(REPORTS_DIR).join("index.json")
}

/// The path of a report file.
pub fn report_file_path(root: &std::path::Path, report_id: &str) -> PathBuf {
    root.join(VALIDATION_DIR)
        .join(REPORTS_DIR)
        .join(report_id)
        .join("report.json")
}

/// The path of a reservation file.
pub fn reservation_path(root: &std::path::Path, report_id: &str) -> PathBuf {
    root.join(VALIDATION_DIR)
        .join(RESERVATIONS_DIR)
        .join(format!("{report_id}.reservation.json"))
}

/// Load the registry (empty when the index does not exist yet).
pub fn load_registry(root: &std::path::Path) -> Result<ReportRegistry, ValidationError> {
    let path = registry_path(root);
    if !path.is_file() {
        return Ok(ReportRegistry::default());
    }
    let bytes = std::fs::read(&path)
        .map_err(|err| ValidationError::io(&path, format!("the registry cannot be read: {err}")))?;
    let registry: ReportRegistry = serde_json::from_slice(&bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the registry index is not usable: {err}"),
        )
    })?;
    Ok(registry)
}

/// Write the registry index (write-to-temp + rename, entries sorted by id).
pub fn write_registry(root: &std::path::Path, registry: &ReportRegistry) -> Result<(), ValidationError> {
    let path = registry_path(root);
    let parent = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    std::fs::create_dir_all(&parent)
        .map_err(|err| ValidationError::io(&parent, format!("the directory cannot be created: {err}")))?;
    let mut ordered = registry.clone();
    ordered.entries.sort_by(|a, b| a.report_id.cmp(&b.report_id));
    let bytes = to_pretty_json(&ordered)?;
    let temporary = parent.join("index.json.tmp");
    std::fs::write(&temporary, &bytes)
        .map_err(|err| ValidationError::io(&temporary, format!("the index cannot be written: {err}")))?;
    std::fs::rename(&temporary, &path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        ValidationError::io(&path, format!("the index cannot be published: {err}"))
    })?;
    Ok(())
}

/// The reservation of a report id before the heavy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub report_id: String,
    /// The digest of the bundle manifest that will be measured.
    pub bundle_sha256: String,
    pub reserved_at: String,
}

impl Reservation {
    /// Reserve a report id atomically. A second reservation of the same id
    /// fails with `RESERVATION_CONFLICT` (the first reservation holder wins;
    /// an exhausted/aborted reservation is released explicitly or ignored).
    pub fn reserve(
        root: &std::path::Path,
        report_id: &str,
        bundle_sha256: &str,
        reserved_at: &str,
    ) -> Result<Reservation, ValidationError> {
        validate_id(report_id, "report_id")?;
        validate_rel_path(&format!("{VALIDATION_DIR}/{report_id}"))?;
        let path = reservation_path(root, report_id);
        let parent = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        std::fs::create_dir_all(&parent)
            .map_err(|err| ValidationError::io(&parent, format!("the directory cannot be created: {err}")))?;
        let reservation = Reservation {
            report_id: report_id.to_string(),
            bundle_sha256: bundle_sha256.to_string(),
            reserved_at: reserved_at.to_string(),
        };
        let bytes = to_pretty_json(&reservation)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    ValidationError::rule(
                        crate::error::RESERVATION_CONFLICT,
                        format!(
                            "the report id '{}' is already reserved; a registered report is \
                             never overwritten",
                            report_id
                        ),
                    )
                } else {
                    ValidationError::io(&path, format!("the reservation cannot be written: {err}"))
                }
            })?;
        std::io::Write::write_all(&mut file, &bytes)
            .map_err(|err| ValidationError::io(&path, format!("the reservation cannot be written: {err}")))?;
        Ok(reservation)
    }

    /// Release a reservation (idempotent).
    pub fn release(&self, root: &std::path::Path) -> Result<(), ValidationError> {
        let path = reservation_path(root, &self.report_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(ValidationError::io(
                &path,
                format!("the reservation cannot be released: {err}"),
            )),
        }
    }

    /// Load the reservation of a report id (None when absent).
    pub fn load(root: &std::path::Path, report_id: &str) -> Result<Option<Reservation>, ValidationError> {
        let path = reservation_path(root, report_id);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .map_err(|err| ValidationError::io(&path, format!("the reservation cannot be read: {err}")))?;
        let reservation: Reservation = serde_json::from_slice(&bytes).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the reservation is not usable: {err}"),
            )
        })?;
        Ok(Some(reservation))
    }
}

/// Write the report and register its digest (no-clobber).
///
/// Rules: registration requires a live reservation of the same id (D1.3);
/// the report file is written exclusively (temp + hard-link / create-new)
/// so a concurrent process can never clobber it; the same id + same digest
/// is an idempotent no-op; the same id + different digest refuses with
/// `NO_CLOBBER_VIOLATION`. The reservation is consumed on success.
pub fn write_and_register(
    root: &std::path::Path,
    reservation: &Reservation,
    report: &ValidationReport,
) -> Result<RegistryEntry, ValidationError> {
    if reservation.report_id != report.report_id {
        return Err(ValidationError::rule(
            crate::error::RESERVATION_REQUIRED,
            format!(
                "the reservation '{}' does not match the report '{}'",
                reservation.report_id, report.report_id
            ),
        ));
    }
    let bytes = report.to_json_bytes()?;
    let digest = sha256_hex(&bytes);
    let bundle_sha256 = report.bundle_ref.sha256.clone();
    let profile_sha256 = report.profile_ref.profile_sha256.clone();

    let existing = load_registry(root)?;
    if let Some(entry) = existing
        .entries
        .iter()
        .find(|entry| entry.report_id == report.report_id)
    {
        if entry.sha256 == digest && entry.bundle_sha256 == bundle_sha256 {
            // Idempotent re-registration of the same registered report.
            reservation.release(root)?;
            return Ok(entry.clone());
        }
        return Err(ValidationError::rule(
            crate::error::NO_CLOBBER_VIOLATION,
            format!(
                "the report '{}' is already registered with the digest {}; a repeated report \
                 is never overwritten",
                report.report_id, entry.sha256
            ),
        ));
    }

    let path = report_file_path(root, &report.report_id);
    let parent = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    std::fs::create_dir_all(&parent)
        .map_err(|err| ValidationError::io(&parent, format!("the directory cannot be created: {err}")))?;
    if path.exists() {
        return Err(ValidationError::rule(
            crate::error::NO_CLOBBER_VIOLATION,
            format!(
                "the report file '{}' already exists and is never overwritten",
                path.display()
            ),
        ));
    }
    let temporary = parent.join("report.json.tmp");
    std::fs::write(&temporary, &bytes)
        .map_err(|err| ValidationError::io(&temporary, format!("the report cannot be written: {err}")))?;
    match std::fs::hard_link(&temporary, &path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&temporary);
        }
        Err(_) => {
            let _ = std::fs::remove_file(&temporary);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|err| {
                    if err.kind() == std::io::ErrorKind::AlreadyExists {
                        ValidationError::rule(
                            crate::error::NO_CLOBBER_VIOLATION,
                            format!(
                                "the report file '{}' already exists and is never overwritten",
                                path.display()
                            ),
                        )
                    } else {
                        ValidationError::io(&path, format!("the report cannot be created: {err}"))
                    }
                })?;
            std::io::Write::write_all(&mut file, &bytes)
                .map_err(|err| ValidationError::io(&path, format!("the report cannot be written: {err}")))?;
        }
    }

    let entry = RegistryEntry {
        report_id: report.report_id.clone(),
        report_path: format!(
            "{VALIDATION_DIR}/{REPORTS_DIR}/{}/report.json",
            report.report_id
        ),
        sha256: digest,
        bundle_sha256,
        profile_sha256,
        verdict: report.verdict,
        created_at: report.created_at.clone(),
    };
    let mut registry = load_registry(root)?;
    registry.entries.push(entry.clone());
    write_registry(root, &registry)?;
    reservation.release(root)?;
    Ok(entry)
}

/// The outcome of a successful verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    /// The recomputed digest of the registered report bytes.
    pub report_sha256: String,
    /// The verdict recorded in the report.
    pub verdict: crate::check::Verdict,
    /// The bundle digest the report is bound to.
    pub bundle_sha256: String,
    /// The profile digest the report was evaluated against.
    pub profile_sha256: String,
}

/// Verify a registered report (the `validation.verify` port).
///
/// Checks: the report is registered with a matching bytes digest; the stored
/// verdict matches the fold; every `not_required` carries an applicability
/// reference (F16); the inputs can be re-read and their digests still match
/// (F2 / F3); with a current profile: the pinned profile digest matches
/// (F19) and the required checks are covered (F8).
pub fn verify_registered_report(
    root: &std::path::Path,
    report_id: &str,
    current_profile: Option<&ValidationProfile>,
) -> Result<VerifyOutcome, ValidationError> {
    let registry = load_registry(root)?;
    let entry = registry
        .entries
        .iter()
        .find(|entry| entry.report_id == report_id)
        .ok_or_else(|| {
            ValidationError::rule(
                crate::error::VALIDATION_REPORT_NOT_REGISTERED,
                format!("the report '{}' is not registered", report_id),
            )
        })?;

    let report_path = resolve_under(root, &entry.report_path)?;
    let bytes = std::fs::read(&report_path).map_err(|err| {
        ValidationError::rule(
            crate::error::VALIDATION_INPUT_MISSING,
            format!(
                "the registered report '{}' cannot be re-read: {}",
                report_id, err
            ),
        )
    })?;
    let actual_digest = sha256_hex(&bytes);
    if actual_digest != entry.sha256 {
        return Err(ValidationError::rule(
            crate::error::NO_CLOBBER_VIOLATION,
            format!(
                "the registered report '{}' bytes changed: registered {}, actual {}",
                report_id, entry.sha256, actual_digest
            ),
        ));
    }
    let report = ValidationReport::from_json_bytes(&bytes)?;

    // The registered binding fields must match the stored report.
    if entry.bundle_sha256 != report.bundle_ref.sha256
        || entry.profile_sha256 != report.profile_ref.profile_sha256
        || entry.verdict != report.verdict
    {
        return Err(ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!(
                "the registry entry of '{}' does not match the stored report",
                report_id
            ),
        ));
    }

    // Re-read every recorded input and compare digests (F2/F3).
    for (path, digest) in &report.input_digests {
        let joined = resolve_under(root, path)?;
        let bytes = std::fs::read(&joined).map_err(|_| {
            ValidationError::rule(
                crate::error::VALIDATION_INPUT_MISSING,
                format!("the input '{}' of the registered report is missing", path),
            )
        })?;
        let actual = sha256_hex(&bytes);
        if &actual != digest {
            return Err(ValidationError::rule(
                crate::error::VALIDATION_INPUT_CHANGED,
                format!(
                    "the input '{}' changed after the evaluation: recorded {}, actual {}",
                    path, digest, actual
                ),
            ));
        }
    }

    if let Some(current) = current_profile {
        current.validate()?;
        let current_digest = digest_document(current)?;
        if current_digest != report.profile_ref.profile_sha256 {
            return Err(ValidationError::rule(
                crate::error::VALIDATION_PROFILE_CHANGED,
                format!(
                    "the profile basis changed: the report pins {} but the current profile \
                     digest is {} (stale approval basis)",
                    report.profile_ref.profile_sha256, current_digest
                ),
            ));
        }
        // Required-check coverage (F8 behavioral analog).
        let missing: Vec<&str> = current
            .required_checks
            .iter()
            .filter(|required| !report.checks.iter().any(|record| &record.check_id == *required))
            .map(|check| check.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(ValidationError::rule(
                crate::error::MISSING_REQUIRED_CHECK,
                format!("the required checks {:?} have no results", missing),
            ));
        }
    }

    Ok(VerifyOutcome {
        report_sha256: entry.sha256.clone(),
        verdict: report.verdict,
        bundle_sha256: report.bundle_ref.sha256.clone(),
        profile_sha256: report.profile_ref.profile_sha256.clone(),
    })
}
