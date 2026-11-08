//! The acceptance seam: the boundary between measurements and decisions.
//!
//! `ValidationReport` never creates author approval (ADR 0005 §5.2/§8). The
//! `AcceptanceRecord` is a SEPARATE decision document binding exactly one
//! candidate bundle digest to one registered report digest. This module
//! provides:
//! - the private provisional `AcceptanceRecord` DTO (mirrors the GLM-1
//!   `acceptance_record.schema.json` draft; there is no `approved: true`
//!   input field anywhere on the wire);
//! - the `CanonicalAcceptSeam` trait — the ONLY extension point where the
//!   future canonical accept Core plugs in. This module never publishes:
//!   the default seam returns `ACCEPT_SEAM_NOT_AVAILABLE`;
//! - a registry-backed prerequisite check (F14/F5/F19) that the future Core
//!   can reuse as the pre-publication gate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::canonical::{digest_document, sha256_hex, to_pretty_json};
use crate::error::ValidationError;
use crate::ids::{validate_id, validate_rfc3339, validate_sha256};
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;
use crate::registry::{load_registry, registry_path, verify_registered_report};

/// The draft acceptance-record schema tag (provisional mirror).
pub const ACCEPTANCE_SCHEMA: &str = "michelangelo.acceptance-record/draft-0.1";

/// The actor of a decision (audit attribution only, not authentication).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    /// `owner` or `delegate`.
    pub role: String,
}

/// The target of a decision: exactly one candidate bundle digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceTarget {
    /// Constant `candidate_asset_bundle` in v0.1.
    pub kind: String,
    pub candidate_id: String,
    pub bundle_sha256: String,
    pub asset_id: String,
    /// null = create with no-clobber; a value = compare-and-swap against the
    /// current revision under lock (the future canonical Core enforces it).
    pub expected_current_revision: Option<u32>,
}

/// The basis of a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceBasis {
    pub validation_report_sha256: String,
    /// Must be true on the wire; verified against the registry anyway.
    pub report_registered: bool,
    pub profile_sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_refs: Vec<crate::check::DocRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The decision document. Strict deserialization: unknown fields (e.g. a
/// smuggled `approved` flag) are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceRecord {
    pub schema: String,
    pub record_id: String,
    pub created_at: String,
    /// `accepted` or `rejected`.
    pub decision: String,
    pub target: AcceptanceTarget,
    pub basis: AcceptanceBasis,
    pub actor: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_note: Option<String>,
}

impl AcceptanceRecord {
    /// Parse and structurally validate a record from bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<AcceptanceRecord, ValidationError> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the acceptance record is not usable: {err}"),
            )
        })?;
        let record: AcceptanceRecord = serde_json::from_value(value).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the acceptance record is not usable: {err}"),
            )
        })?;
        record.validate()?;
        Ok(record)
    }

    /// Structural validation of the record.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema != ACCEPTANCE_SCHEMA {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!(
                    "the record schema '{}' is not '{}'",
                    self.schema, ACCEPTANCE_SCHEMA
                ),
            ));
        }
        validate_id(&self.record_id, "record_id")?;
        validate_rfc3339(&self.created_at)?;
        if self.decision != "accepted" && self.decision != "rejected" {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the decision '{}' is not accepted|rejected", self.decision),
            ));
        }
        if self.target.kind != "candidate_asset_bundle" {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the target kind '{}' is not candidate_asset_bundle", self.target.kind),
            ));
        }
        validate_id(&self.target.candidate_id, "candidate_id")?;
        validate_id(&self.target.asset_id, "asset_id")?;
        validate_sha256(&self.target.bundle_sha256, "bundle_sha256")?;
        validate_sha256(&self.basis.validation_report_sha256, "validation_report_sha256")?;
        validate_sha256(&self.basis.profile_sha256, "profile_sha256")?;
        if !self.basis.report_registered {
            return Err(ValidationError::rule(
                crate::error::VALIDATION_REPORT_NOT_REGISTERED,
                "the record declares report_registered=false; an acceptance basis must be a \
                 registered report"
                    .to_string(),
            ));
        }
        validate_id(&self.actor.name, "actor.name")?;
        if self.actor.role != "owner" && self.actor.role != "delegate" {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the actor role '{}' is not owner|delegate", self.actor.role),
            ));
        }
        for reference in &self.basis.policy_refs {
            crate::ids::validate_rel_path(&reference.path)?;
            validate_sha256(&reference.sha256, "policy ref digest")?;
        }
        Ok(())
    }

    /// The canonical digest of the record.
    pub fn digest(&self) -> Result<String, ValidationError> {
        digest_document(self)
    }
}

/// The facts the future canonical accept Core needs before publishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrerequisiteReport {
    /// The cited report digest is registered and its bytes match.
    pub report_registered: bool,
    /// The registered report's bundle digest equals the record target digest.
    pub bundle_binding_matches: bool,
    /// The registered report's profile digest equals the record basis digest.
    pub profile_binding_matches: bool,
    /// For an `accepted` decision: the report verdict is pass.
    pub verdict_sufficient: bool,
    /// The digest of the report bytes (recomputed during the check).
    pub report_sha256: String,
}

/// The adapter seam to the future canonical accept Core. This module is NOT
/// a publisher: the only default implementation refuses publication.
pub trait CanonicalAcceptSeam {
    /// Pre-publication prerequisite checks (F14/F5/F19).
    fn check_prerequisites(
        &self,
        record: &AcceptanceRecord,
    ) -> Result<PrerequisiteReport, ValidationError>;

    /// Publication. The canonical accept Core (a future Core-side component)
    /// implements this; nothing else may. The provided default refuses.
    fn submit_for_publication(
        &self,
        _record: &AcceptanceRecord,
    ) -> Result<String, ValidationError> {
        Err(ValidationError::rule(
            crate::error::ACCEPT_SEAM_NOT_AVAILABLE,
            "the canonical accept Core is the only publisher; this module never publishes \
             and no second publisher exists (ADR 0005 §5.1/§5.3)"
                .to_string(),
        ))
    }
}

/// The registry-backed seam: prerequisite checks against the local
/// no-clobber report registry. It performs NO publication.
pub struct RegistryBackedSeam<'a> {
    /// The validation workspace root that holds the report registry.
    pub root: &'a std::path::Path,
}

impl CanonicalAcceptSeam for RegistryBackedSeam<'_> {
    fn check_prerequisites(
        &self,
        record: &AcceptanceRecord,
    ) -> Result<PrerequisiteReport, ValidationError> {
        record.validate()?;
        // F14: the cited report must be registered; verify recomputes the
        // bytes digest and re-reads the inputs (F2 inside the seam).
        let outcome = verify_registered_report(self.root, &locate_report(self.root, &record.basis.validation_report_sha256)?, None)?;
        if outcome.report_sha256 != record.basis.validation_report_sha256 {
            return Err(ValidationError::rule(
                crate::error::VALIDATION_REPORT_NOT_REGISTERED,
                format!(
                    "the cited report digest {} is not registered in this workspace",
                    record.basis.validation_report_sha256
                ),
            ));
        }
        // F5 (replay binding): the report must bind exactly the target bundle.
        let bundle_binding_matches = outcome.bundle_sha256 == record.target.bundle_sha256;
        // F19 (stale basis): the profile digest of the report must equal the
        // record basis; a changed profile/threshold invalidates the basis.
        let profile_binding_matches = outcome.profile_sha256 == record.basis.profile_sha256;
        // An accepted decision requires a pass verdict (unavailable is not a
        // pass and never converts into one).
        let verdict_sufficient = record.decision == "rejected" || outcome.verdict == crate::check::Verdict::Pass;
        Ok(PrerequisiteReport {
            report_registered: true,
            bundle_binding_matches,
            profile_binding_matches,
            verdict_sufficient,
            report_sha256: outcome.report_sha256,
        })
    }
}

/// Locate the report id whose registered digest matches, refusing when the
/// digest is unknown (F14).
fn locate_report(root: &std::path::Path, report_sha256: &str) -> Result<String, ValidationError> {
    let registry = load_registry(root)?;
    registry
        .entries
        .iter()
        .find(|entry| entry.sha256 == report_sha256)
        .map(|entry| entry.report_id.clone())
        .ok_or_else(|| {
            ValidationError::rule(
                crate::error::VALIDATION_REPORT_NOT_REGISTERED,
                format!(
                    "no registered report carries the digest {}; acceptance without a \
                     registered validation report is refused",
                    report_sha256
                ),
            )
        })
}

/// A test-level append-only index of acceptance records used to detect
/// replay (`ACCEPTANCE_REPLAY`, F5): the same record id bound to a different
/// bundle digest. This device is NOT a publisher and NOT a pointer store;
/// the canonical Core owns the real decision persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LocalReplayIndex {
    pub schema: String,
    /// record_id -> bundle_sha256.
    pub bindings: BTreeMap<String, String>,
}

impl LocalReplayIndex {
    /// Load the replay index (empty when absent).
    pub fn load(root: &std::path::Path) -> Result<LocalReplayIndex, ValidationError> {
        let path = registry_path(root)
            .parent()
            .map(|parent| parent.join("acceptance-index.json"))
            .unwrap_or_else(|| root.join("acceptance-index.json"));
        if !path.is_file() {
            return Ok(LocalReplayIndex {
                schema: "michelangelo.acceptance-replay-index/draft-0.1".to_string(),
                bindings: BTreeMap::new(),
            });
        }
        let bytes = std::fs::read(&path)
            .map_err(|err| ValidationError::io(&path, format!("the replay index cannot be read: {err}")))?;
        let index: LocalReplayIndex = serde_json::from_slice(&bytes).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the replay index is not usable: {err}"),
            )
        })?;
        Ok(index)
    }

    /// Record a decision binding; a replay of the same record id for a
    /// different bundle digest is refused (F5).
    pub fn record(
        &mut self,
        root: &std::path::Path,
        record: &AcceptanceRecord,
    ) -> Result<(), ValidationError> {
        if let Some(existing) = self.bindings.get(&record.record_id) {
            if existing != &record.target.bundle_sha256 {
                return Err(ValidationError::rule(
                    crate::error::ACCEPTANCE_REPLAY,
                    format!(
                        "the acceptance record '{}' is already bound to the bundle {}; it is \
                         replayed for {}",
                        record.record_id, existing, record.target.bundle_sha256
                    ),
                ));
            }
            return Ok(());
        }
        self.bindings
            .insert(record.record_id.clone(), record.target.bundle_sha256.clone());
        let path = registry_path(root)
            .parent()
            .map(|parent| parent.join("acceptance-index.json"))
            .unwrap_or_else(|| root.join("acceptance-index.json"));
        let parent = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        std::fs::create_dir_all(&parent)
            .map_err(|err| ValidationError::io(&parent, format!("the directory cannot be created: {err}")))?;
        let bytes = to_pretty_json(self)?;
        let temporary = parent.join("acceptance-index.json.tmp");
        std::fs::write(&temporary, &bytes)
            .map_err(|err| ValidationError::io(&temporary, format!("the index cannot be written: {err}")))?;
        std::fs::rename(&temporary, &path).map_err(|err| {
            let _ = std::fs::remove_file(&temporary);
            ValidationError::io(&path, format!("the index cannot be published: {err}"))
        })?;
        Ok(())
    }
}

/// Convenience: the bytes digest of a stored acceptance record file (used by
/// tests to prove the record itself is immutable evidence).
pub fn record_bytes_digest(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

/// Compile-time assertion helper used in tests: the wire DTO carries no
/// approval flag. Any unknown field is rejected by deny_unknown_fields.
#[doc(hidden)]
pub fn assert_no_approval_field() {
    let json = r#"{
        "schema": "michelangelo.acceptance-record/draft-0.1",
        "record_id": "rec-1",
        "created_at": "2026-11-08T12:00:00Z",
        "decision": "accepted",
        "target": {"kind": "candidate_asset_bundle", "candidate_id": "c", "bundle_sha256": "0000000000000000000000000000000000000000000000000000000000000000", "asset_id": "a", "expected_current_revision": null},
        "basis": {"validation_report_sha256": "0000000000000000000000000000000000000000000000000000000000000000", "report_registered": true, "profile_sha256": "0000000000000000000000000000000000000000000000000000000000000000"},
        "actor": {"name": "owner", "role": "owner"},
        "approved": true
    }"#;
    let result = AcceptanceRecord::from_json_bytes(json.as_bytes());
    debug_assert!(result.is_err(), "an 'approved' field must never parse");
}

/// Validation profile reference for seam tests.
pub type SeamProfile = ValidationProfile;

/// Re-export of the report type for the seam consumers.
pub type SeamReport = ValidationReport;
