//! The no-clobber registry of validation reports plus verification
//! (`validation.verify` port of the contract gate §7.5).
//!
//! Guarantees (IT-8D D1.3 analog for the new bundle path):
//! - the report id and its work directory are reserved BEFORE the heavy
//!   evaluation; the reservation is an atomic create-new file and
//!   registration re-reads it from disk (an in-memory value alone is not
//!   evidence, M1);
//! - the pinned profile of a registered report is stored digest-named next to
//!   the registry so verification can re-apply the required-check guard;
//!   required checks must be `pass|fail|unavailable`, never `not_required`
//!   (H2);
//! - a registered report is never overwritten: same id + same digest is an
//!   idempotent no-op, same id + different bytes is `NO_CLOBBER_VIOLATION`;
//! - the registry index is append-only and carries the digest of the bytes
//!   actually written;
//! - `verify` recomputes the report digest, re-reads the inputs (F2), and
//!   compares the pinned profile basis (F19).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical::{digest_document, sha256_hex, to_pretty_json};
use crate::check::CheckStatus;
use crate::error::ValidationError;
use crate::ids::{resolve_under, validate_id, validate_rel_path, validate_sha256};
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
/// The pinned-profile copies directory (the basis a report was registered with).
pub const PROFILES_DIR: &str = "profiles";
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
    root.join(VALIDATION_DIR)
        .join(REPORTS_DIR)
        .join("index.json")
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

/// The path of the stored copy of the pinned profile (digest-named).
pub fn pinned_profile_path(root: &Path, profile_sha256: &str) -> PathBuf {
    root.join(VALIDATION_DIR)
        .join(PROFILES_DIR)
        .join(format!("{profile_sha256}.json"))
}

/// Store the validated profile that is the pinned basis of a registered
/// report. The copy is digest-named and no-clobber: an existing copy must
/// carry exactly the same bytes.
pub fn store_pinned_profile(
    root: &Path,
    profile: &ValidationProfile,
) -> Result<(), ValidationError> {
    profile.validate()?;
    let digest = digest_document(profile)?;
    let path = pinned_profile_path(root, &digest);
    let bytes = to_pretty_json(profile)?;
    if path.is_file() {
        let existing = std::fs::read(&path).map_err(|err| {
            ValidationError::io(&path, format!("the pinned profile cannot be read: {err}"))
        })?;
        if existing == bytes {
            return Ok(());
        }
        return Err(ValidationError::rule(
            crate::error::NO_CLOBBER_VIOLATION,
            format!(
                "the pinned profile copy '{}' already exists with different bytes",
                path.display()
            ),
        ));
    }
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    std::fs::create_dir_all(&parent).map_err(|err| {
        ValidationError::io(&parent, format!("the directory cannot be created: {err}"))
    })?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                ValidationError::rule(
                    crate::error::NO_CLOBBER_VIOLATION,
                    format!(
                        "the pinned profile copy '{}' was created concurrently and is never \
                         overwritten",
                        path.display()
                    ),
                )
            } else {
                ValidationError::io(
                    &path,
                    format!("the pinned profile cannot be written: {err}"),
                )
            }
        })?;
    std::io::Write::write_all(&mut file, &bytes).map_err(|err| {
        ValidationError::io(
            &path,
            format!("the pinned profile cannot be written: {err}"),
        )
    })?;
    Ok(())
}

/// Load the stored copy of a pinned profile by its digest (None when absent).
pub fn load_pinned_profile(
    root: &Path,
    profile_sha256: &str,
) -> Result<Option<ValidationProfile>, ValidationError> {
    validate_sha256(profile_sha256, "profile_sha256")?;
    let path = pinned_profile_path(root, profile_sha256);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|err| {
        ValidationError::io(&path, format!("the pinned profile cannot be read: {err}"))
    })?;
    let profile: ValidationProfile = serde_json::from_slice(&bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the pinned profile is not usable: {err}"),
        )
    })?;
    profile.validate()?;
    if digest_document(&profile)? != profile_sha256 {
        return Err(ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!(
                "the pinned profile copy '{}' does not match its digest",
                path.display()
            ),
        ));
    }
    Ok(Some(profile))
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
pub fn write_registry(
    root: &std::path::Path,
    registry: &ReportRegistry,
) -> Result<(), ValidationError> {
    let path = registry_path(root);
    let parent = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    std::fs::create_dir_all(&parent).map_err(|err| {
        ValidationError::io(&parent, format!("the directory cannot be created: {err}"))
    })?;
    let mut ordered = registry.clone();
    ordered
        .entries
        .sort_by(|a, b| a.report_id.cmp(&b.report_id));
    let bytes = to_pretty_json(&ordered)?;
    let temporary = parent.join("index.json.tmp");
    std::fs::write(&temporary, &bytes).map_err(|err| {
        ValidationError::io(&temporary, format!("the index cannot be written: {err}"))
    })?;
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
        std::fs::create_dir_all(&parent).map_err(|err| {
            ValidationError::io(&parent, format!("the directory cannot be created: {err}"))
        })?;
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
        std::io::Write::write_all(&mut file, &bytes).map_err(|err| {
            ValidationError::io(&path, format!("the reservation cannot be written: {err}"))
        })?;
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
    pub fn load(
        root: &std::path::Path,
        report_id: &str,
    ) -> Result<Option<Reservation>, ValidationError> {
        validate_id(report_id, "report_id")?;
        validate_rel_path(&format!("{VALIDATION_DIR}/{report_id}"))?;
        let path = reservation_path(root, report_id);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(|err| {
            ValidationError::io(&path, format!("the reservation cannot be read: {err}"))
        })?;
        let reservation: Reservation = serde_json::from_slice(&bytes).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the reservation is not usable: {err}"),
            )
        })?;
        Ok(Some(reservation))
    }
}

/// Enforce the required-check rules of the pinned profile against a set of
/// records: every required check has a result and that result is `pass`,
/// `fail` or `unavailable` — never `not_required` (H2). This keeps
/// `not_required` reserved for checks the profile leaves optional; otherwise a
/// hand-assembled report could mask a required measurement.
fn guard_required_results(
    profile: &ValidationProfile,
    checks: &[crate::check::CheckRecord],
) -> Result<(), ValidationError> {
    for required in &profile.required_checks {
        match checks.iter().find(|record| &record.check_id == required) {
            None => {
                return Err(ValidationError::rule(
                    crate::error::MISSING_REQUIRED_CHECK,
                    format!(
                        "the required check '{}' has no result in the report",
                        required.as_str()
                    ),
                ));
            }
            Some(record) if record.result == CheckStatus::NotRequired => {
                return Err(ValidationError::rule(
                    crate::error::INVALID_NOT_REQUIRED,
                    format!(
                        "the required check '{}' is recorded as not_required; a required check \
                         must be pass, fail or unavailable",
                        required.as_str()
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Write the report and register its digest (no-clobber).
///
/// Rules: registration requires a LIVE on-disk reservation of the same id and
/// bundle (M1: an in-memory `Reservation` value alone is not evidence); the
/// passed profile must be the pinned basis of the report and every required
/// check must be `pass|fail|unavailable` (H2); a check id may appear at most
/// once (a duplicate [pass, not_required] pair must not slip past the
/// first-match guard); the report file is written exclusively (temp +
/// hard-link / create-new) so a concurrent process can never clobber it; the
/// same id + same digest is an idempotent no-op; the same id + different
/// digest refuses with `NO_CLOBBER_VIOLATION`. The pinned profile is stored
/// next to the registry (digest-named, no-clobber) so that verification can
/// re-apply the required-check guard. The reservation is consumed on success.
pub fn write_and_register(
    root: &Path,
    reservation: &Reservation,
    report: &ValidationReport,
    profile: &ValidationProfile,
) -> Result<RegistryEntry, ValidationError> {
    // M1: the reservation must exist on disk and match the passed value.
    validate_id(&report.report_id, "report_id")?;
    validate_rel_path(&format!("{VALIDATION_DIR}/{}", report.report_id))?;
    let live = Reservation::load(root, &report.report_id)?.ok_or_else(|| {
        ValidationError::rule(
            crate::error::RESERVATION_REQUIRED,
            format!(
                "the report id '{}' has no live reservation on disk; registration requires a \
                 reservation taken before the heavy evaluation",
                report.report_id
            ),
        )
    })?;
    if &live != reservation {
        return Err(ValidationError::rule(
            crate::error::RESERVATION_REQUIRED,
            format!(
                "the in-memory reservation of '{}' does not match the live reservation file; a \
                 fabricated reservation never authorizes registration",
                report.report_id
            ),
        ));
    }
    if reservation.bundle_sha256 != report.bundle_ref.sha256 {
        return Err(ValidationError::rule(
            crate::error::RESERVATION_REQUIRED,
            format!(
                "the reservation of '{}' was taken for the bundle {} but the report measures {}",
                report.report_id, reservation.bundle_sha256, report.bundle_ref.sha256
            ),
        ));
    }

    // H2: the profile handed to registration must be the pinned basis and
    // must not leave a required check as not_required.
    profile.validate()?;
    if digest_document(profile)? != report.profile_ref.profile_sha256 {
        return Err(ValidationError::rule(
            crate::error::VALIDATION_PROFILE_CHANGED,
            format!(
                "the profile passed to registration does not match the report basis {}",
                report.profile_ref.profile_sha256
            ),
        ));
    }
    // Registration-side structural guard (P3): at most one record per check
    // id. `guard_required_results` inspects the first match, so a duplicate
    // pair [pass, not_required] would otherwise pass registration while
    // verification (re-assembly from the stored bytes) refuses the document.
    // The registry must never accept a report it can never verify.
    let mut seen_checks = std::collections::BTreeSet::new();
    for record in &report.checks {
        if !seen_checks.insert(record.check_id) {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!(
                    "the check '{}' has more than one result; registration never accepts a \
                     report that verification would refuse",
                    record.check_id.as_str()
                ),
            ));
        }
    }
    guard_required_results(profile, &report.checks)?;

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
            store_pinned_profile(root, profile)?;
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

    // The digest-named copy is the profile basis verification later re-applies.
    store_pinned_profile(root, profile)?;

    let path = report_file_path(root, &report.report_id);
    let parent = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    std::fs::create_dir_all(&parent).map_err(|err| {
        ValidationError::io(&parent, format!("the directory cannot be created: {err}"))
    })?;
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
    std::fs::write(&temporary, &bytes).map_err(|err| {
        ValidationError::io(&temporary, format!("the report cannot be written: {err}"))
    })?;
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
            std::io::Write::write_all(&mut file, &bytes).map_err(|err| {
                ValidationError::io(&path, format!("the report cannot be written: {err}"))
            })?;
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
/// (F19); with the pinned basis profile (the supplied current profile or the
/// digest-named copy stored at registration): every required check has a
/// `pass|fail|unavailable` result — a required check recorded as
/// `not_required` is refused (H2). A report with no retrievable basis profile
/// is refused as unprovable: the required-check guard cannot be re-applied to
/// ANY report, so `basis=None` without a pinned copy never verifies.
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

    // The basis profile: the caller's current profile when supplied, else the
    // digest-named copy stored at registration.
    let basis: Option<ValidationProfile> = match current_profile {
        Some(current) => {
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
            Some(current.clone())
        }
        None => load_pinned_profile(root, &report.profile_ref.profile_sha256)?,
    };
    // The basis profile is mandatory for verification: without it neither the
    // required-check coverage nor the optionality of a `not_required` record
    // can be proven. Registration always stores the digest-named copy, so a
    // registered report whose basis cannot be loaded is forged or damaged;
    // refusing is the only honest outcome for ANY report, not only for
    // reports that happen to carry `not_required` records.
    let Some(basis) = basis else {
        return Err(ValidationError::rule(
            crate::error::VALIDATION_PROFILE_BASIS_MISSING,
            format!(
                "the report '{}' pins the profile basis {} but no pinned profile copy is \
                 available and no current profile was supplied; the required-check guard \
                 cannot be re-applied",
                report_id, report.profile_ref.profile_sha256
            ),
        ));
    };
    // Required-check coverage plus the H2 result guard.
    guard_required_results(&basis, &report.checks)?;

    Ok(VerifyOutcome {
        report_sha256: entry.sha256.clone(),
        verdict: report.verdict,
        bundle_sha256: report.bundle_ref.sha256.clone(),
        profile_sha256: report.profile_ref.profile_sha256.clone(),
    })
}
