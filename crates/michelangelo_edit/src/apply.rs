//! The applier of the edit boundary.
//!
//! The applier implements the operations of the boundary:
//!
//! * `submit` — a candidate enters the store, is validated and is
//!   registered together with an immutable snapshot of the **full**
//!   post-change result,
//! * `accept` — an author accepts a candidate and a new scene revision is
//!   published **from the snapshot**, never from the candidate directory,
//! * `reject` — an author refuses a candidate,
//! * `rollback` — the store points to a previous revision again.
//!
//! Every mutation of the store runs under the exclusive OS lock of the run
//! (`verification/run.lock`): a busy lock is refused (`STORE_BUSY`), the
//! boundary never waits. The decision history of a candidate is append-only
//! (`candidates/<id>/decisions/NNNN-<kind>.json`); `decision.json` is the
//! mirror of the latest decision.
//!
//! An accepted candidate is never applied twice: a repeated acceptance
//! returns the historical result, appends an `accept-idempotent` record and
//! does not switch the scene back (also after a rollback).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate::{is_valid_component_name, parse_candidate, CandidateAction, EditCandidate};
use crate::error::{io_error, EditError};
use crate::gate::{self, AcceptOptions};
use crate::policy::load_policy;
use crate::scene::{
    component_bytes, list_revisions, load_current, load_index, load_scene, next_revision,
    publish_revision, write_bytes_atomic, write_current, write_index, BindingRef,
    CandidateIndexEntry, ConfirmationRef, CurrentPointer, EvaluationRef, ResolutionRef,
    RollbackRecord, Scene, SceneComponent, SceneStore, SceneVerification, UnknownStateRef,
    VerificationPolicies, COMPONENTS_DIR_NAME,
};
use crate::unknowns::{self, ResolutionMaterial};
use crate::validate::{
    validate, Check, CheckStatus, Decision, DecisionReport, Diagnostic, Severity,
    ValidationContext, ValidationMode,
};
use crate::verification::{
    aggregate_digest, canonical_json, file_sha256, load_snapshot, package_digest,
    package_policy_digest, snapshot_file_path, snapshot_identity, tool_versions,
    unknown_state_relative_path, verify_revision_bundle, verify_snapshot, verify_snapshot_diff,
    verify_snapshot_identity, workspace_policy_digest, write_bytes_new, write_unknown_state,
    SnapshotFile, SnapshotIdentity, SnapshotManifest, SnapshotSource, StoreLock, UnknownState,
};
use crate::{Clock, INDEX_SCHEMA};

/// The schema of the rollback report.
pub const ROLLBACK_SCHEMA: &str = "michelangelo.edit-rollback/0";
/// The schema of the scene view.
pub const VIEW_SCHEMA: &str = "michelangelo.edit-scene-view/0";

/// The kind of the decision record of a submission.
pub const DECISION_SUBMIT: &str = "submit";
/// The kind of the decision record of an acceptance.
pub const DECISION_ACCEPT: &str = "accept";
/// The kind of the decision record of a repeated acceptance.
pub const DECISION_ACCEPT_IDEMPOTENT: &str = "accept-idempotent";
/// The kind of the decision record of a rejection.
pub const DECISION_REJECT: &str = "reject";
/// The kind of the decision record of a repeated rejection.
pub const DECISION_REJECT_IDEMPOTENT: &str = "reject-idempotent";
/// The kind of the decision record of a refused rejection.
pub const DECISION_REJECT_REFUSED: &str = "reject-refused";
pub const DECISION_ACCEPT_REFUSED: &str = "accept-refused";

/// Record a refused acceptance in the append-only history.
///
/// The latest-decision mirror of the candidate is left untouched: the refusal
/// is an attempt, not a new decision about the candidate. The history keeps
/// the diagnostic code so a tampered/legacy candidate stays auditable.
fn refuse_accept(
    store: &SceneStore,
    candidate_id: &str,
    by: &str,
    clock: &dyn Clock,
    code: &str,
    message: String,
) -> Result<DecisionReport, EditError> {
    let decided_at = clock.now_rfc3339_utc();
    let mut report =
        DecisionReport::empty(candidate_id.to_string(), Decision::Rejected, &decided_at);
    report.decided_by = Some(by.to_string());
    report.checks.push(Check {
        check: "evidence".to_string(),
        status: CheckStatus::Fail,
        detail: message.clone(),
    });
    report.diagnostics.push(Diagnostic {
        code: code.to_string(),
        severity: Severity::Error,
        message: message.clone(),
        address: None,
    });
    write_history(store, &report, DECISION_ACCEPT_REFUSED)?;
    Err(coded_edit_error(code, message))
}

/// Rebuild the typed error of a refusal code.
fn coded_edit_error(code: &str, message: String) -> EditError {
    match code {
        crate::error::SNAPSHOT_MISSING => EditError::SnapshotMissing { message },
        crate::error::SNAPSHOT_MISMATCH => EditError::SnapshotMismatch { message },
        crate::error::EVIDENCE_MISMATCH => EditError::EvidenceMismatch { message },
        crate::error::STALE_BASE => EditError::StaleBase { message },
        crate::error::EVALUATION_MISSING => EditError::EvaluationMissing { message },
        crate::error::EVALUATION_STALE => EditError::EvaluationStale { message },
        crate::error::EVALUATION_UNREGISTERED => EditError::EvaluationUnregistered { message },
        crate::error::INPUT_CHANGED => EditError::InputChanged { message },
        crate::error::VISIBLE_FAILED => EditError::VisibleFailed { message },
        crate::error::AUTHOR_CONDITION_FAILED => EditError::AuthorConditionFailed { message },
        crate::error::REGRESSION => EditError::Regression { message },
        crate::error::NO_GROUND_TRUTH => EditError::NoGroundTruth { message },
        _ => EditError::Internal { message },
    }
}

/// The report of a rollback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollbackReport {
    /// The schema identifier.
    pub schema: String,
    /// The revision that was current before the rollback.
    pub from_scene_revision: Option<String>,
    /// The revision the store points to now.
    pub to_scene_revision: String,
    /// The author that performed the rollback.
    pub by: String,
    /// The time of the rollback (RFC 3339, UTC).
    pub rolled_back_at: String,
}

/// The view of a scene revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionView {
    /// The identifier of the revision.
    pub scene_revision: String,
    /// The creation time of the revision.
    pub created_at: String,
    /// The revision the revision was derived from.
    pub base_scene_revision: Option<String>,
    /// The candidate that produced the revision.
    pub parent_candidate_id: Option<String>,
    /// The components of the revision.
    pub components: Vec<SceneComponent>,
}

/// The view of a candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateView {
    /// The identifier of the candidate.
    pub candidate_id: String,
    /// The status of the candidate.
    pub status: String,
    /// The candidate document.
    pub candidate: EditCandidate,
    /// The decision of the candidate.
    pub decision: Option<DecisionReport>,
}

/// The view of a scene store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowReport {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the run.
    pub run_id: String,
    /// The authoritative pointer of the store.
    pub current: Option<CurrentPointer>,
    /// The revisions of the store, ordered by name.
    pub revisions: Vec<RevisionView>,
    /// The candidates of the store, ordered by identifier.
    pub candidates: Vec<CandidateIndexEntry>,
    /// The rollbacks of the store.
    pub rollbacks: Vec<RollbackRecord>,
    /// The requested candidate.
    pub candidate: Option<CandidateView>,
}

impl ShowReport {
    /// The JSON representation of the view (pretty, deterministic).
    pub fn to_json(&self) -> Result<String, EditError> {
        serde_json::to_string_pretty(self).map_err(|err| EditError::Internal {
            message: format!("the view cannot be serialized: {err}"),
        })
    }
}

/// The hooks of an acceptance.
///
/// The hooks exist for the failure tests of the contract: they inject a
/// change of the store between two steps of the acceptance. They are never
/// used by the production code.
#[doc(hidden)]
#[derive(Default)]
pub struct AcceptHooks<'a> {
    /// Called after the validation and before the publication.
    pub after_validation: Option<&'a mut dyn FnMut()>,
    /// Called after the revision directory is published and before the
    /// pointer is switched (a crash here leaves the old bundle
    /// authoritative).
    pub before_publish: Option<&'a mut dyn FnMut() -> Result<(), EditError>>,
    /// Called after the pointer is switched and before the history and the
    /// index are written (a crash here leaves the pointer on the new
    /// revision: the next attempt completes the same operation).
    pub after_pointer: Option<&'a mut dyn FnMut() -> Result<(), EditError>>,
}

/// Submit a candidate into the store of a run.
///
/// The candidate is copied into the store, validated against the current
/// scene revision and registered together with an immutable snapshot of the
/// full post-change result.
pub fn submit_candidate(
    run_dir: &Path,
    candidate_dir: &Path,
    clock: &dyn Clock,
) -> Result<DecisionReport, EditError> {
    let store = SceneStore::new(run_dir);
    let candidate_file = candidate_dir.join(crate::scene::CANDIDATE_FILE_NAME);
    let text = std::fs::read_to_string(&candidate_file).map_err(|err| {
        io_error(
            &candidate_file,
            format!("the candidate document cannot be read: {err}"),
        )
    })?;

    let parsed = parse_candidate(&text);
    let candidate_id = candidate_id_of(candidate_dir, &text, &parsed)?;

    // The whole submission is one critical section: the candidate enters the
    // store and its snapshot is registered atomically.
    let _lock = StoreLock::acquire(&store, "submit")?;

    let target = store.candidate_dir(&candidate_id);
    if target.exists() {
        return Err(EditError::Schema {
            message: format!("the candidate '{candidate_id}' is already in the store"),
        });
    }

    let temporary = store.candidate_dir(&format!("{candidate_id}.tmp"));
    if temporary.exists() {
        return Err(io_error(
            &temporary,
            "a temporary candidate directory already exists",
        ));
    }
    if let Err(error) = copy_tree(candidate_dir, &temporary) {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, &target) {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(io_error(
            &target,
            format!("the candidate cannot be stored: {error}"),
        ));
    }

    let decided_at = clock.now_rfc3339_utc();
    let mut report = DecisionReport::empty(candidate_id.clone(), Decision::Error, &decided_at);
    report.base_scene_revision = base_scene_revision_hint(&text);

    let candidate = match parsed {
        Ok(candidate) => candidate,
        Err(error) => {
            report.checks.push(Check {
                check: "candidate_schema".to_string(),
                status: CheckStatus::Fail,
                detail: error.message.clone(),
            });
            report.diagnostics.push(Diagnostic {
                code: error.code.to_string(),
                severity: Severity::Error,
                message: error.message.clone(),
                address: None,
            });
            report.decision = if error.code == "BUDGET_INVALID" {
                Decision::Rejected
            } else {
                Decision::Error
            };
            write_decision(&store, &report, DECISION_SUBMIT)?;
            record_index(&store, &report, None)?;
            return Ok(report);
        }
    };

    let pointer = load_current(&store)?.ok_or_else(|| EditError::Scene {
        message: "the scene store is not initialized: run 'michelangelo edit init' first"
            .to_string(),
    })?;
    let base_scene = load_scene(&store, &pointer.scene_revision)?;
    let policy = load_policy(run_dir)?;
    let store_candidate_dir = store.candidate_dir(&candidate_id);

    let package_policy = unknowns::package_policy(&store);
    let context = ValidationContext {
        candidate: &candidate,
        candidate_dir: &store_candidate_dir,
        base_scene: &base_scene,
        current_scene_revision: &pointer.scene_revision,
        mode: ValidationMode::Submit,
        decided_by: None,
        package_policy: Some(&package_policy),
    };
    let outcome = validate(&context, policy.as_ref());

    report.base_scene_revision = Some(pointer.scene_revision.clone());
    report.checks = outcome.checks.clone();
    report.diagnostics = outcome.diagnostics.clone();

    if !outcome.valid {
        report.decision = Decision::Rejected;
        write_decision(&store, &report, DECISION_SUBMIT)?;
        record_index(&store, &report, None)?;
        return Ok(report);
    }

    // The immutable snapshot of the full post-change result: the unchanged
    // components are copied byte-identically from the base revision.
    let manifest = create_snapshot(&store, &candidate, &base_scene, &outcome, clock)?;
    report.snapshot_id = Some(manifest.snapshot_id.clone());
    report.aggregate_sha256 = Some(manifest.aggregate_sha256.clone());

    // The decision pins the identity of the snapshot: the digest of the
    // manifest, of the candidate document, of the task, of the package and of
    // the base revision. The acceptance verifies them before it publishes
    // anything.
    let identity = snapshot_identity(&store, &manifest)?;
    report.snapshot_manifest_sha256 = Some(identity.manifest_sha256.clone());
    report.candidate_sha256 = identity.candidate_sha256.clone();
    report.task_sha256 = identity.task_sha256.clone();
    report.package_sha256 = identity.package_sha256.clone();
    report.base_aggregate_sha256 = identity.base_aggregate_sha256.clone();

    if outcome.auto_accept {
        // The automatic acceptance of the workspace policy cannot publish a
        // revision: an acceptance always needs a registered evaluation
        // (contract §7). The candidate stays proposed until the evaluation
        // exists and an author accepts it.
        report.decision = Decision::Proposed;
        report.checks.push(Check {
            check: "evaluation".to_string(),
            status: CheckStatus::Skipped,
            detail: String::from(
                "the acceptance needs a registered evaluation: the candidate stays proposed",
            ),
        });
        report.diagnostics.push(Diagnostic {
            code: String::from("EVALUATION_REQUIRED"),
            severity: Severity::Info,
            message: String::from(
                "the automatic acceptance of the workspace policy is deferred: an acceptance needs a registered evaluation (run 'michelangelo evaluate')",
            ),
            address: None,
        });
        write_decision(&store, &report, DECISION_SUBMIT)?;
        record_index(&store, &report, None)?;
        return Ok(report);
    }

    report.decision = Decision::Proposed;
    write_decision(&store, &report, DECISION_SUBMIT)?;
    record_index(&store, &report, None)?;
    Ok(report)
}

/// Accept a candidate of the store and publish a new scene revision.
///
/// The acceptance is strict: it needs an evaluation report that the Core
/// registered for the candidate. Without a registered evaluation the
/// acceptance is refused (`EVALUATION_MISSING`) and no revision is
/// published.
pub fn accept_candidate(
    run_dir: &Path,
    candidate_id: &str,
    by: &str,
    clock: &dyn Clock,
) -> Result<DecisionReport, EditError> {
    accept_candidate_with_options(run_dir, candidate_id, by, clock, AcceptOptions::default())
}

/// Accept a candidate with the options of the acceptance.
///
/// The option `evaluation` names the report of the evaluation the
/// acceptance is based on. When it is absent the gate uses the last
/// evaluation that the Core registered for the candidate.
pub fn accept_candidate_with_options(
    run_dir: &Path,
    candidate_id: &str,
    by: &str,
    clock: &dyn Clock,
    options: AcceptOptions,
) -> Result<DecisionReport, EditError> {
    accept_candidate_with_hooks_and_options(
        run_dir,
        candidate_id,
        by,
        clock,
        AcceptHooks::default(),
        options,
    )
}

/// Accept a candidate with the test hooks of the acceptance.
#[doc(hidden)]
pub fn accept_candidate_with_hooks(
    run_dir: &Path,
    candidate_id: &str,
    by: &str,
    clock: &dyn Clock,
    hooks: AcceptHooks<'_>,
) -> Result<DecisionReport, EditError> {
    accept_candidate_with_hooks_and_options(
        run_dir,
        candidate_id,
        by,
        clock,
        hooks,
        AcceptOptions::default(),
    )
}

/// Accept a candidate with the test hooks and the options of the acceptance.
#[doc(hidden)]
pub fn accept_candidate_with_hooks_and_options(
    run_dir: &Path,
    candidate_id: &str,
    by: &str,
    clock: &dyn Clock,
    mut hooks: AcceptHooks<'_>,
    options: AcceptOptions,
) -> Result<DecisionReport, EditError> {
    let store = SceneStore::new(run_dir);
    let candidate = load_stored_candidate(&store, candidate_id)?;

    // The lock is taken before any state of the store is read: the transition
    // checks, the verification of the evidence and the writes of the decision
    // all happen under the same lock. A concurrent rejection can never read a
    // stale status and overwrite an acceptance.
    let _lock = StoreLock::acquire(&store, "accept")?;

    // An accepted candidate is never applied twice: the historical result is
    // returned, the scene is not switched back (also after a rollback).
    if let Some(historical) = historical_acceptance(&store, candidate_id)? {
        let mut report = historical;
        report.decided_at = clock.now_rfc3339_utc();
        write_decision(&store, &report, DECISION_ACCEPT_IDEMPOTENT)?;
        return Ok(report);
    }

    // A crash between the switch of the pointer and the write of the decision
    // leaves the pointer on the revision of the candidate: the bookkeeping is
    // completed here.
    if let Some(pointer) = load_current(&store)? {
        if pointer.candidate_id.as_deref() == Some(candidate_id) {
            return complete_acceptance(&store, candidate_id, by, clock, &options, &pointer);
        }
    }

    // The evidence of the candidate: without a snapshot the candidate cannot
    // be accepted (a legacy candidate stays readable, it is never accepted).
    // Every refusal is appended to the decision history of the candidate.
    let manifest = match load_snapshot(&store, candidate_id)? {
        Some(manifest) => manifest,
        None => {
            return refuse_accept(
                &store,
                candidate_id,
                by,
                clock,
                crate::error::SNAPSHOT_MISSING,
                format!(
                    "the candidate '{candidate_id}' carries no snapshot: it cannot be accepted"
                ),
            );
        }
    };
    if let Err(error) = verify_snapshot(&store, &manifest) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }
    // The identity of the snapshot must be the one the submit decision
    // pinned: a manifest that was rewritten after the submission (even
    // together with its geometry) is refused.
    let identity = match identity_of_decision(&store, candidate_id) {
        Ok(identity) => identity,
        Err(error) => {
            return refuse_accept(
                &store,
                candidate_id,
                by,
                clock,
                error.code(),
                error.message().to_string(),
            );
        }
    };
    if let Err(error) = verify_snapshot_identity(&store, &manifest, &identity) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }
    // The full difference of the snapshot: the file set against the base
    // revision and the declared changes, the unchanged components against the
    // bytes of the base revision and the changed components against the bytes
    // the candidate declares.
    let base_of_snapshot = load_scene(&store, &manifest.base.scene_revision)?;
    if let Err(error) = verify_snapshot_diff(&store, &manifest, &base_of_snapshot, &candidate) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }
    if let Err(error) = verify_candidate_geometry(&store, &candidate, &manifest) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }

    let pointer = load_current(&store)?.ok_or_else(|| EditError::Scene {
        message: "the scene store is not initialized: run 'michelangelo edit init' first"
            .to_string(),
    })?;
    let base_scene = load_scene(&store, &pointer.scene_revision)?;
    let policy = load_policy(run_dir)?;

    let candidate_dir = store.candidate_dir(candidate_id);
    let package_policy = unknowns::package_policy(&store);
    let context = ValidationContext {
        candidate: &candidate,
        candidate_dir: &candidate_dir,
        base_scene: &base_scene,
        current_scene_revision: &pointer.scene_revision,
        mode: ValidationMode::Accept,
        decided_by: Some(by),
        package_policy: Some(&package_policy),
    };
    let outcome = validate(&context, policy.as_ref());
    let decided_at = clock.now_rfc3339_utc();

    let mut report =
        DecisionReport::empty(candidate_id.to_string(), Decision::Rejected, &decided_at);
    report.base_scene_revision = Some(pointer.scene_revision.clone());
    report.checks = outcome.checks.clone();
    report.diagnostics = outcome.diagnostics.clone();
    report.decided_by = Some(by.to_string());

    if !outcome.valid {
        write_decision(&store, &report, DECISION_REJECT)?;
        record_index(&store, &report, None)?;
        return Ok(report);
    }

    // The hook runs between the validation and the publication.
    if let Some(hook) = hooks.after_validation.as_mut() {
        hook();
    }

    // The pointer is re-read under the lock: the candidate may only be
    // applied to the revision it was built on.
    let current = load_current(&store)?.ok_or_else(|| EditError::Scene {
        message: "the scene store is not initialized: run 'michelangelo edit init' first"
            .to_string(),
    })?;
    if current.scene_revision != pointer.scene_revision
        || candidate.base.scene_revision != current.scene_revision
    {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            crate::error::STALE_BASE,
            format!(
                "the candidate was built on the revision '{}' but the store points to the revision '{}'",
                candidate.base.scene_revision, current.scene_revision
            ),
        );
    }

    // The evidence is verified again under the lock.
    if let Err(error) = verify_snapshot(&store, &manifest) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }
    if let Err(error) = verify_candidate_geometry(&store, &candidate, &manifest) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }
    if let Err(error) = verify_snapshot_identity(&store, &manifest, &identity) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }
    if let Err(error) = verify_snapshot_diff(&store, &manifest, &base_of_snapshot, &candidate) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }

    // The accept gate: the acceptance needs a registered evaluation whose
    // digest, target aggregate, base revision and pinned inputs match the
    // state of the store, whose projection conformance passes, whose required
    // author geometric conditions pass and whose source views do not
    // regress against the base revision.
    let binding = match gate::resolve_evaluation(&store, candidate_id, &options) {
        Ok(binding) => binding,
        Err(error) => {
            return refuse_accept(
                &store,
                candidate_id,
                by,
                clock,
                error.code(),
                error.message().to_string(),
            );
        }
    };
    if let Err(error) = gate::verify_evaluation(&store, &candidate, &manifest, &binding) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }

    // The resolution of an unknown (optional): the unknown is closed only
    // in the revision that references the resolution.
    let material = match options.resolve.as_deref() {
        Some(unknown_id) => {
            let material = unknowns::materialize_resolution_locked(
                &store, unknown_id, &candidate, &manifest, &binding, by, clock,
            )?;
            Some(material)
        }
        None => None,
    };

    let revision = publish_snapshot(
        &store,
        &manifest,
        &candidate,
        &base_scene,
        clock,
        Some(&gate::evaluation_ref(&binding)),
        material.as_ref(),
    )?;

    // The hook runs between the publication of the revision and the switch of
    // the pointer: a crash here leaves the old bundle authoritative and the
    // revision is reused by the next attempt.
    if let Some(hook) = hooks.before_publish.as_mut() {
        hook()?;
    }

    report.decision = Decision::Accepted;
    report.resulting_scene_revision = Some(revision.clone());
    report.snapshot_id = Some(manifest.snapshot_id.clone());
    report.aggregate_sha256 = Some(manifest.aggregate_sha256.clone());
    report.snapshot_manifest_sha256 = Some(identity.manifest_sha256.clone());
    report.candidate_sha256 = identity.candidate_sha256.clone();
    report.task_sha256 = identity.task_sha256.clone();
    report.package_sha256 = identity.package_sha256.clone();
    report.base_aggregate_sha256 = identity.base_aggregate_sha256.clone();
    report.evaluation_id = Some(binding.evaluation_id.clone());
    report.evaluation_sha256 = Some(binding.sha256.clone());

    // Every document the new bundle refers to is loaded by its path and its
    // digest is re-computed before the pointer is switched: a bundle that does
    // not hold never becomes authoritative.
    if let Err(error) = verify_revision_bundle(&store, &revision) {
        return refuse_accept(
            &store,
            candidate_id,
            by,
            clock,
            error.code(),
            error.message().to_string(),
        );
    }

    set_current(&store, &revision, Some(candidate_id.to_string()), clock)?;

    // The hook runs after the switch of the pointer and before the history
    // and the index are written: a crash here leaves the pointer on the new
    // revision and the next attempt completes the same operation.
    if let Some(hook) = hooks.after_pointer.as_mut() {
        hook()?;
    }

    write_decision(&store, &report, DECISION_ACCEPT)?;
    record_index(&store, &report, Some(revision.clone()))?;
    Ok(report)
}

/// Reject a candidate of the store.
pub fn reject_candidate(
    run_dir: &Path,
    candidate_id: &str,
    by: &str,
    reason: Option<&str>,
    clock: &dyn Clock,
) -> Result<DecisionReport, EditError> {
    let store = SceneStore::new(run_dir);
    let candidate = load_stored_candidate(&store, candidate_id)?;

    // The lock is taken before the status of the candidate is read: the
    // transition is checked and written under the same lock. A rejection can
    // never overwrite an acceptance that won the race.
    let _lock = StoreLock::acquire(&store, "reject")?;

    let decided_at = clock.now_rfc3339_utc();
    let index = load_index(&store)?;
    let status = index
        .entry(candidate_id)
        .map(|entry| entry.status.clone())
        .unwrap_or_default();

    if status == "accepted" {
        // The acceptance of a candidate is never rewritten: the rejection is
        // refused and only the refusal is recorded.
        let mut report =
            DecisionReport::empty(candidate_id.to_string(), Decision::Rejected, &decided_at);
        report.base_scene_revision = Some(candidate.base.scene_revision.clone());
        report.decided_by = Some(by.to_string());
        report.checks.push(Check {
            check: "author_decision".to_string(),
            status: CheckStatus::Fail,
            detail: format!(
                "the candidate '{candidate_id}' was already accepted and cannot be rejected"
            ),
        });
        report.diagnostics.push(Diagnostic {
            code: crate::error::CANDIDATE_ALREADY_ACCEPTED.to_string(),
            severity: Severity::Error,
            message: format!(
                "the candidate '{candidate_id}' was already accepted: its history is not rewritten"
            ),
            address: None,
        });
        write_history(&store, &report, DECISION_REJECT_REFUSED)?;
        return Ok(report);
    }

    if status == "rejected" {
        // The rejection is idempotent: the same rejected result is returned.
        let mut report = match last_history(&store, candidate_id, DECISION_REJECT)? {
            Some(report) => report,
            None => rejection_report(&candidate, candidate_id, by, reason, &decided_at),
        };
        report.decided_at = decided_at;
        write_decision(&store, &report, DECISION_REJECT_IDEMPOTENT)?;
        return Ok(report);
    }

    let report = rejection_report(&candidate, candidate_id, by, reason, &decided_at);
    write_decision(&store, &report, DECISION_REJECT)?;
    record_index(&store, &report, None)?;
    Ok(report)
}

/// Build the report of a rejection.
fn rejection_report(
    candidate: &EditCandidate,
    candidate_id: &str,
    by: &str,
    reason: Option<&str>,
    decided_at: &str,
) -> DecisionReport {
    let mut report =
        DecisionReport::empty(candidate_id.to_string(), Decision::Rejected, decided_at);
    report.base_scene_revision = Some(candidate.base.scene_revision.clone());
    report.decided_by = Some(by.to_string());
    let detail = match reason {
        Some(reason) if !reason.trim().is_empty() => {
            format!("the candidate was rejected by '{by}': {reason}")
        }
        _ => format!("the candidate was rejected by '{by}'"),
    };
    report.checks.push(Check {
        check: "author_decision".to_string(),
        status: CheckStatus::Fail,
        detail,
    });
    report
}

/// Roll the store back to a previous scene revision.
///
/// The target revision keeps its own bundle: its geometry, its unknown state
/// and its references are the ones that become authoritative again.
pub fn rollback_scene(
    run_dir: &Path,
    to_scene_revision: &str,
    by: &str,
    clock: &dyn Clock,
) -> Result<RollbackReport, EditError> {
    let store = SceneStore::new(run_dir);
    if !store.revision_dir(to_scene_revision).is_dir() {
        return Err(EditError::Scene {
            message: format!("the scene revision '{to_scene_revision}' does not exist"),
        });
    }
    let _lock = StoreLock::acquire(&store, "rollback")?;
    let from = load_current(&store)?.map(|pointer| pointer.scene_revision);
    let at = clock.now_rfc3339_utc();
    let pointer = CurrentPointer {
        scene_revision: to_scene_revision.to_string(),
        candidate_id: None,
        accepted_at: at.clone(),
    };
    write_current(&store, &pointer)?;

    let mut index = load_index(&store)?;
    index.rollbacks.push(RollbackRecord {
        from_scene_revision: from.clone(),
        to_scene_revision: to_scene_revision.to_string(),
        by: by.to_string(),
        at: at.clone(),
    });
    write_index(&store, &index)?;

    crate::verification::append_event(
        &store,
        &serde_json::json!({
            "event": "rollback",
            "at": at,
            "by": by,
            "from_scene_revision": from,
            "to_scene_revision": to_scene_revision,
        }),
    )?;

    Ok(RollbackReport {
        schema: ROLLBACK_SCHEMA.to_string(),
        from_scene_revision: from,
        to_scene_revision: to_scene_revision.to_string(),
        by: by.to_string(),
        rolled_back_at: at,
    })
}

/// Show the state of a scene store.
pub fn show_scene(run_dir: &Path, candidate_id: Option<&str>) -> Result<ShowReport, EditError> {
    let store = SceneStore::new(run_dir);
    let current = load_current(&store)?;
    let mut revisions = Vec::new();
    for revision in list_revisions(&store)? {
        let scene = load_scene(&store, &revision)?;
        revisions.push(RevisionView {
            scene_revision: scene.scene_revision,
            created_at: scene.created_at,
            base_scene_revision: scene.base_scene_revision,
            parent_candidate_id: scene.parent_candidate_id,
            components: scene.components,
        });
    }
    let index = load_index(&store)?;
    let candidate = match candidate_id {
        Some(candidate_id) => {
            let candidate = load_stored_candidate(&store, candidate_id)?;
            let status = index
                .entry(candidate_id)
                .map(|entry| entry.status.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let decision_path = store.decision_path(candidate_id);
            let decision = if decision_path.is_file() {
                Some(DecisionReport::load(&decision_path)?)
            } else {
                None
            };
            Some(CandidateView {
                candidate_id: candidate_id.to_string(),
                status,
                candidate,
                decision,
            })
        }
        None => None,
    };
    Ok(ShowReport {
        schema: VIEW_SCHEMA.to_string(),
        run_id: crate::scene::run_id_of(run_dir),
        current,
        revisions,
        candidates: index.candidates.clone(),
        rollbacks: index.rollbacks.clone(),
        candidate,
    })
}

/// Create the immutable snapshot of the full post-change result of a
/// candidate.
///
/// The snapshot materialises every component of the result: the changed and
/// the added components come from the candidate, the unchanged ones are
/// copied byte-identically from the base revision.
fn create_snapshot(
    store: &SceneStore,
    candidate: &EditCandidate,
    base_scene: &Scene,
    outcome: &crate::validate::ValidationOutcome,
    clock: &dyn Clock,
) -> Result<SnapshotManifest, EditError> {
    let mut entries: Vec<(String, SnapshotSource, Vec<u8>)> = Vec::new();
    for component in &base_scene.components {
        match outcome
            .prepared
            .iter()
            .find(|change| change.component == component.name)
        {
            Some(change) => {
                let bytes = std::fs::read(&change.source).map_err(|err| {
                    io_error(
                        &change.source,
                        format!("the geometry cannot be read: {err}"),
                    )
                })?;
                entries.push((component.name.clone(), SnapshotSource::Candidate, bytes));
            }
            None => {
                // The component is untouched: its bytes are copied verbatim.
                let bytes = component_bytes(store, &base_scene.scene_revision, &component.name)?;
                entries.push((component.name.clone(), SnapshotSource::Base, bytes));
            }
        }
    }
    for change in &outcome.prepared {
        if change.action != CandidateAction::Add {
            continue;
        }
        if entries.iter().any(|(name, _, _)| name == &change.component) {
            continue;
        }
        let bytes = std::fs::read(&change.source).map_err(|err| {
            io_error(
                &change.source,
                format!("the geometry cannot be read: {err}"),
            )
        })?;
        entries.push((change.component.clone(), SnapshotSource::Candidate, bytes));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let files: Vec<SnapshotFile> = entries
        .iter()
        .map(|(name, source, bytes)| SnapshotFile {
            component: name.clone(),
            path: format!("{}/{name}.obj", crate::verification::GEOMETRY_DIR_NAME),
            sha256: michelangelo_input::sha256::sha256_hex(bytes),
            source: *source,
        })
        .collect();
    let components: Vec<(String, String)> = files
        .iter()
        .map(|file| (file.component.clone(), file.sha256.clone()))
        .collect();
    let aggregate_sha256 = aggregate_digest(&components);
    let base_components: Vec<(String, String)> = base_scene
        .components
        .iter()
        .map(|component| (component.name.clone(), component.sha256.clone()))
        .collect();

    let task = serde_json::to_value(&candidate.task).map_err(|err| EditError::Internal {
        message: format!("the task of the candidate cannot be serialized: {err}"),
    })?;
    let manifest = SnapshotManifest {
        schema: crate::verification::SNAPSHOT_SCHEMA.to_string(),
        snapshot_id: candidate.candidate_id.clone(),
        created_at: clock.now_rfc3339_utc(),
        base: crate::verification::SnapshotBase {
            package_id: base_scene.source.package_id.clone(),
            package_revision: base_scene.source.package_revision,
            run_id: base_scene.run_id.clone(),
            scene_revision: base_scene.scene_revision.clone(),
            geometry_digest: aggregate_digest(&base_components),
        },
        candidate_sha256: file_sha256(&store.candidate_path(&candidate.candidate_id)).ok(),
        task_sha256: Some(michelangelo_input::sha256::sha256_hex(
            canonical_json(&task).as_bytes(),
        )),
        package_sha256: package_digest(store),
        bindings_sha256: None,
        package_policy_sha256: package_policy_digest(store),
        files,
        aggregate_sha256,
        tool_versions: tool_versions(store),
    };

    // The snapshot is written into a temporary directory and renamed into
    // place: it is immutable afterwards.
    let target = store.snapshot_dir(&candidate.candidate_id);
    if target.exists() {
        return Err(io_error(
            &target,
            "the snapshot already exists and is never overwritten",
        ));
    }
    let temporary = store.snapshot_dir(&format!("{}.tmp", candidate.candidate_id));
    if temporary.exists() {
        return Err(io_error(
            &temporary,
            "a temporary snapshot directory already exists",
        ));
    }
    let geometry_dir = temporary.join(crate::verification::GEOMETRY_DIR_NAME);
    std::fs::create_dir_all(&geometry_dir).map_err(|err| {
        io_error(
            &geometry_dir,
            format!("the snapshot geometry cannot be created: {err}"),
        )
    })?;
    for (name, _, bytes) in &entries {
        let path = geometry_dir.join(format!("{name}.obj"));
        std::fs::write(&path, bytes).map_err(|err| {
            io_error(
                &path,
                format!("the snapshot geometry cannot be written: {err}"),
            )
        })?;
    }
    let text = serde_json::to_string_pretty(&manifest).map_err(|err| EditError::Internal {
        message: format!("the snapshot manifest cannot be serialized: {err}"),
    })?;
    write_bytes_atomic(
        &temporary.join(crate::verification::MANIFEST_FILE_NAME),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )?;
    std::fs::rename(&temporary, &target).map_err(|err| {
        let _ = std::fs::remove_dir_all(&temporary);
        io_error(&target, format!("the snapshot cannot be published: {err}"))
    })?;
    Ok(manifest)
}

/// Verify that the geometry of the candidate still matches the snapshot.
///
/// The published geometry comes from the snapshot; a candidate that was
/// modified after the submission is refused (`EVIDENCE_MISMATCH`).
fn verify_candidate_geometry(
    store: &SceneStore,
    candidate: &EditCandidate,
    manifest: &SnapshotManifest,
) -> Result<(), EditError> {
    for change in &candidate.changes {
        let file = match manifest.file(&change.component) {
            Some(file) => file,
            None => continue,
        };
        if file.source != SnapshotSource::Candidate {
            continue;
        }
        let path = store
            .candidate_dir(&candidate.candidate_id)
            .join(&change.obj);
        let digest = file_sha256(&path)?;
        if digest != file.sha256 {
            return Err(EditError::EvidenceMismatch {
                message: format!(
                    "the geometry of the component '{}' of the candidate '{}' carries the digest {digest} but the snapshot pins {}",
                    change.component, candidate.candidate_id, file.sha256
                ),
            });
        }
    }
    Ok(())
}

/// The documents a new revision refers to.
///
/// The bundle of a revision is the set of the immutable documents it is
/// based on: the evaluation, the bindings, the confirmations and the
/// resolutions.
#[derive(Debug, Clone, Default)]
struct BundleRefs {
    evaluations: Vec<EvaluationRef>,
    bindings: Vec<BindingRef>,
    confirmations: Vec<ConfirmationRef>,
    resolutions: Vec<ResolutionRef>,
}

/// Build the references of the bundle of a new revision.
///
/// The bundle references the evaluation of the acceptance, the binding
/// documents, the author confirmations and the resolutions. Every resolved
/// unknown of the recomputed state contributes its resolution, its
/// confirmation and the version of the bindings it used: a resolution that
/// survives a later revision keeps its evidence in the new bundle. Every
/// document is verified: it must exist and it is hashed.
fn bundle_refs(
    store: &SceneStore,
    state: &UnknownState,
    evaluation: Option<&EvaluationRef>,
    material: Option<&ResolutionMaterial>,
) -> Result<BundleRefs, EditError> {
    let evaluations = match evaluation {
        Some(reference) => vec![reference.clone()],
        None => Vec::new(),
    };

    // The resolutions of the resolved unknowns of the state: the new one and
    // the ones that survive the revision.
    let mut documents = unknowns::resolved_resolutions(store, state)?;
    if let Some(material) = material {
        if !documents
            .iter()
            .any(|document| document.resolution_id == material.resolution.resolution_id)
        {
            documents.push(material.resolution.clone());
            documents.sort_by(|left, right| left.resolution_id.cmp(&right.resolution_id));
        }
    }

    // The binding document of the run is referenced as well: it is the
    // binding the revision is based on.
    let mut versions: Vec<String> = match unknowns::latest_binding_document(store)? {
        Some(document) => vec![document.version.clone()],
        None => Vec::new(),
    };
    let mut confirmations: BTreeMap<String, ConfirmationRef> = BTreeMap::new();
    let mut resolutions: Vec<ResolutionRef> = Vec::new();
    for document in &documents {
        versions.push(document.binding_version.clone());
        let path = store.resolution_path(&document.resolution_id);
        let sha256 = hash_document(&path, &document.resolution_id, "resolution")?;
        resolutions.push(ResolutionRef {
            resolution_id: document.resolution_id.clone(),
            path: unknowns::resolution_relative_path(&document.resolution_id),
            sha256,
        });
        let path = store.confirmation_path(&document.confirmation_id);
        let sha256 = hash_document(&path, &document.confirmation_id, "confirmation")?;
        confirmations.insert(
            document.confirmation_id.clone(),
            ConfirmationRef {
                confirmation_id: document.confirmation_id.clone(),
                path: unknowns::confirmation_relative_path(&document.confirmation_id),
                sha256,
            },
        );
    }

    versions.sort();
    versions.dedup();
    let mut bindings: Vec<BindingRef> = Vec::new();
    for version in &versions {
        let path = store.binding_path(version);
        let sha256 = hash_document(&path, version, "binding")?;
        bindings.push(BindingRef {
            version: version.clone(),
            path: unknowns::binding_relative_path(version),
            sha256,
        });
    }

    Ok(BundleRefs {
        evaluations,
        bindings,
        confirmations: confirmations.into_values().collect(),
        resolutions,
    })
}

/// The digest of a document the bundle refers to.
///
/// The document must exist: a bundle never references a document that is
/// missing.
fn hash_document(path: &Path, id: &str, kind: &str) -> Result<String, EditError> {
    if !path.is_file() {
        return Err(EditError::EvidenceMismatch {
            message: format!("the {kind} '{id}' is missing: '{}'", path.display()),
        });
    }
    file_sha256(path)
}

/// Publish a new scene revision from the snapshot of a candidate.
///
/// The components of the revision are copied from the snapshot, never from
/// the candidate directory. A revision of the same candidate that already
/// exists (a crash between the publication and the switch of the pointer) is
/// reused, never duplicated.
fn publish_snapshot(
    store: &SceneStore,
    manifest: &SnapshotManifest,
    candidate: &EditCandidate,
    base_scene: &Scene,
    clock: &dyn Clock,
    evaluation: Option<&EvaluationRef>,
    resolution: Option<&ResolutionMaterial>,
) -> Result<String, EditError> {
    let mut components: Vec<SceneComponent> = Vec::new();
    let mut payloads: Vec<(String, Vec<u8>)> = Vec::new();
    for file in &manifest.files {
        let path = snapshot_file_path(store, &manifest.snapshot_id, &file.path);
        let bytes = std::fs::read(&path).map_err(|err| {
            io_error(
                &path,
                format!("the snapshot geometry cannot be read: {err}"),
            )
        })?;
        let parsed = crate::obj::parse_obj_file(&path).map_err(|err: crate::obj::ObjError| {
            EditError::Scene {
                message: format!(
                    "the component '{}' of the snapshot is not usable: {err}",
                    file.component
                ),
            }
        })?;
        let base_component = base_scene.component(&file.component);
        components.push(SceneComponent {
            name: file.component.clone(),
            part_id: base_component.and_then(|component| component.part_id.clone()),
            obj: format!("{}/{}.obj", COMPONENTS_DIR_NAME, file.component),
            sha256: parsed.sha256.clone(),
            bbox_min: parsed.mesh.bbox.min,
            bbox_max: parsed.mesh.bbox.max,
            protected: base_component
                .map(|component| component.protected)
                .unwrap_or(false),
        });
        payloads.push((file.component.clone(), bytes));
    }
    components.sort_by(|left, right| left.name.cmp(&right.name));
    payloads.sort_by(|left, right| left.0.cmp(&right.0));

    // Every revision carries its own immutable state of the unknowns. The
    // state is recomputed: a resolution whose dependencies changed is not
    // carried forward (the unknown becomes 'open' with the reason
    // 'evidence_stale').
    let revision = next_revision(store)?;
    let created_at = clock.now_rfc3339_utc();
    let digests: Vec<(String, String)> = components
        .iter()
        .map(|component| (component.name.clone(), component.sha256.clone()))
        .collect();
    let state = unknowns::recompute_state(
        store,
        &base_scene.scene_revision,
        &revision,
        base_scene,
        &digests,
        resolution.map(|material| &material.resolution),
        &created_at,
    )?;

    // The bundle of the revision references the documents of the evidence it
    // is based on: the evaluation, the bindings, the confirmations and the
    // resolutions. Every resolved unknown of the recomputed state keeps its
    // references: a resolution that survives a later revision carries its
    // confirmation and its binding into the new bundle.
    let refs = bundle_refs(store, &state, evaluation, resolution)?;

    // A revision that a previous attempt published is reused only when the
    // whole intended result matches: the candidate, the base revision, the
    // geometry, the evaluation and the resolution. A retry with another
    // evaluation or another resolution never reuses the old bundle.
    if let Some(revision) = find_revision_of_candidate(
        store,
        &candidate.candidate_id,
        &base_scene.scene_revision,
        &manifest.aggregate_sha256,
        &refs,
    )? {
        return Ok(revision);
    }

    write_unknown_state(store, &state)?;
    let unknown_sha256 = file_sha256(&store.unknown_state_path(&revision))?;

    let scene = Scene {
        schema: crate::SCENE_SCHEMA.to_string(),
        run_id: base_scene.run_id.clone(),
        scene_revision: revision.clone(),
        base_scene_revision: Some(base_scene.scene_revision.clone()),
        parent_candidate_id: Some(candidate.candidate_id.clone()),
        created_at: created_at.clone(),
        components,
        source: base_scene.source.clone(),
        notes: base_scene.notes.clone(),
        verification: Some(SceneVerification {
            geometry_digest: manifest.aggregate_sha256.clone(),
            bindings: refs.bindings.clone(),
            unknowns: Some(UnknownStateRef {
                revision: revision.clone(),
                path: unknown_state_relative_path(&revision),
                sha256: unknown_sha256,
            }),
            evaluations: refs.evaluations.clone(),
            confirmations: refs.confirmations.clone(),
            resolutions: refs.resolutions.clone(),
            policies: VerificationPolicies {
                package_sha256: manifest.package_sha256.clone(),
                workspace_policy_sha256: workspace_policy_digest(store),
            },
            tool_versions: manifest.tool_versions.clone(),
            created_at: created_at.clone(),
        }),
    };
    publish_revision(store, &scene, &payloads)?;
    Ok(revision)
}

/// Find the revision that a candidate already produced.
///
/// A crash between the publication of a revision and the switch of the
/// pointer leaves the revision behind: the next attempt reuses it instead of
/// publishing a second one.
fn find_revision_of_candidate(
    store: &SceneStore,
    candidate_id: &str,
    base_revision: &str,
    aggregate_sha256: &str,
    intended: &BundleRefs,
) -> Result<Option<String>, EditError> {
    for revision in list_revisions(store)? {
        let scene = match load_scene(store, &revision) {
            Ok(scene) => scene,
            Err(_) => continue,
        };
        if scene.parent_candidate_id.as_deref() != Some(candidate_id) {
            continue;
        }
        if scene.base_scene_revision.as_deref() != Some(base_revision) {
            continue;
        }
        let components: Vec<(String, String)> = scene
            .components
            .iter()
            .map(|component| (component.name.clone(), component.sha256.clone()))
            .collect();
        if aggregate_digest(&components) != aggregate_sha256 {
            continue;
        }
        // The whole intended result must match: the evaluation, the
        // bindings, the confirmations and the resolutions. A revision of
        // another evaluation is never reused.
        let bundle = match &scene.verification {
            Some(bundle) => bundle,
            None => continue,
        };
        if bundle.evaluations != intended.evaluations {
            continue;
        }
        if bundle.bindings != intended.bindings {
            continue;
        }
        if bundle.confirmations != intended.confirmations {
            continue;
        }
        if bundle.resolutions != intended.resolutions {
            continue;
        }
        return Ok(Some(revision));
    }
    Ok(None)
}

/// The identity of the snapshot, as the submit decision pinned it.
///
/// A decision that does not pin the identity of its snapshot cannot be
/// accepted: the acceptance would trust a snapshot that was never registered.
fn identity_of_decision(
    store: &SceneStore,
    candidate_id: &str,
) -> Result<SnapshotIdentity, EditError> {
    let report = last_history(store, candidate_id, DECISION_SUBMIT)?.ok_or_else(|| {
        EditError::SnapshotMismatch {
            message: format!(
                "the candidate '{candidate_id}' carries no submission decision: the identity of its snapshot is not pinned"
            ),
        }
    })?;
    let snapshot_id = report
        .snapshot_id
        .clone()
        .ok_or_else(|| EditError::SnapshotMismatch {
            message: format!(
                "the submission decision of the candidate '{candidate_id}' pins no snapshot"
            ),
        })?;
    let aggregate_sha256 =
        report
            .aggregate_sha256
            .clone()
            .ok_or_else(|| EditError::SnapshotMismatch {
                message: format!(
                "the submission decision of the candidate '{candidate_id}' pins no aggregate digest"
            ),
            })?;
    let manifest_sha256 = report.snapshot_manifest_sha256.clone().ok_or_else(|| {
        EditError::SnapshotMismatch {
            message: format!(
                "the submission decision of the candidate '{candidate_id}' pins no digest of the manifest of its snapshot: the identity of the snapshot cannot be verified"
            ),
        }
    })?;
    Ok(SnapshotIdentity {
        snapshot_id,
        aggregate_sha256,
        manifest_sha256,
        candidate_sha256: report.candidate_sha256.clone(),
        task_sha256: report.task_sha256.clone(),
        package_sha256: report.package_sha256.clone(),
        base_aggregate_sha256: report.base_aggregate_sha256.clone(),
    })
}

/// Complete an acceptance that was interrupted after the switch of the
/// pointer.
///
/// The revision the pointer refers to must be the one the acceptance
/// intended: the same evaluation and a bundle that still holds. A retry with
/// another evaluation is refused: the bundle of a published revision is never
/// rewritten.
fn complete_acceptance(
    store: &SceneStore,
    candidate_id: &str,
    by: &str,
    _clock: &dyn Clock,
    options: &AcceptOptions,
    pointer: &CurrentPointer,
) -> Result<DecisionReport, EditError> {
    let scene = load_scene(store, &pointer.scene_revision)?;
    let bundle = scene
        .verification
        .as_ref()
        .ok_or_else(|| EditError::EvidenceMismatch {
            message: format!(
            "the revision '{}' carries no verification bundle: the acceptance cannot be completed",
            pointer.scene_revision
        ),
        })?;
    let binding = gate::resolve_evaluation(store, candidate_id, options)?;
    let evaluation = bundle
        .evaluations
        .first()
        .ok_or_else(|| EditError::EvaluationMissing {
            message: format!(
                "the revision '{}' refers to no evaluation: the acceptance cannot be completed",
                pointer.scene_revision
            ),
        })?;
    if evaluation.evaluation_id != binding.evaluation_id || evaluation.sha256 != binding.sha256 {
        return Err(EditError::EvaluationStale {
            message: format!(
                "the revision '{}' is based on the evaluation '{}' (the digest {}) but the acceptance is based on '{}' (the digest {})",
                pointer.scene_revision,
                evaluation.evaluation_id,
                evaluation.sha256,
                binding.evaluation_id,
                binding.sha256
            ),
        });
    }
    verify_revision_bundle(store, &pointer.scene_revision)?;

    let mut report = DecisionReport::empty(
        candidate_id.to_string(),
        Decision::Accepted,
        pointer.accepted_at.clone(),
    );
    report.base_scene_revision = scene.base_scene_revision.clone();
    report.resulting_scene_revision = Some(pointer.scene_revision.clone());
    report.decided_by = Some(by.to_string());
    report.snapshot_id = Some(candidate_id.to_string());
    report.aggregate_sha256 = Some(bundle.geometry_digest.clone());
    report.evaluation_id = Some(binding.evaluation_id.clone());
    report.evaluation_sha256 = Some(binding.sha256.clone());
    report.checks.push(Check {
        check: "recovery".to_string(),
        status: CheckStatus::Pass,
        detail: format!(
            "the store already points to the revision '{}' of the candidate",
            pointer.scene_revision
        ),
    });
    write_decision(store, &report, DECISION_ACCEPT)?;
    record_index(store, &report, Some(pointer.scene_revision.clone()))?;
    Ok(report)
}

/// The historical acceptance of a candidate (None when the candidate was
/// never accepted).
fn historical_acceptance(
    store: &SceneStore,
    candidate_id: &str,
) -> Result<Option<DecisionReport>, EditError> {
    let index = load_index(store)?;
    let entry = match index.entry(candidate_id) {
        Some(entry) => entry,
        None => return Ok(None),
    };
    if entry.status != "accepted" {
        return Ok(None);
    }
    if let Some(report) = last_history(store, candidate_id, DECISION_ACCEPT)? {
        return Ok(Some(report));
    }
    // A legacy candidate: the index carries the result of the acceptance.
    let mut report = DecisionReport::empty(
        candidate_id.to_string(),
        Decision::Accepted,
        entry
            .decided_at
            .clone()
            .unwrap_or_else(|| entry.submitted_at.clone()),
    );
    report.resulting_scene_revision = entry.scene_revision.clone();
    Ok(Some(report))
}

/// Write the decision of a candidate into the store.
///
/// The decision is appended to the immutable history of the candidate and
/// mirrored into `decision.json`.
fn write_decision(
    store: &SceneStore,
    report: &DecisionReport,
    kind: &str,
) -> Result<(), EditError> {
    write_history(store, report, kind)?;
    let text = report.to_json()?;
    write_bytes_atomic(
        &store.decision_path(&report.candidate_id),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
}

/// Append a decision to the immutable history of a candidate.
///
/// The history is append-only: the sequence number is the next free one and
/// an existing record is never rewritten.
fn write_history(store: &SceneStore, report: &DecisionReport, kind: &str) -> Result<(), EditError> {
    let directory = store.decisions_dir(&report.candidate_id);
    std::fs::create_dir_all(&directory).map_err(|err| {
        io_error(
            &directory,
            format!("the decision history cannot be created: {err}"),
        )
    })?;
    let sequence = next_decision_sequence(&directory)?;
    let path = directory.join(format!("{sequence:04}-{kind}.json"));
    let text = report.to_json()?;
    write_bytes_new(
        &path,
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
}

/// The next free sequence number of a decision history.
fn next_decision_sequence(directory: &Path) -> Result<u32, EditError> {
    let mut highest = 0_u32;
    if !directory.is_dir() {
        return Ok(1);
    }
    let entries = std::fs::read_dir(directory).map_err(|err| {
        io_error(
            directory,
            format!("the decision history cannot be read: {err}"),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            io_error(
                directory,
                format!("the decision history cannot be read: {err}"),
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((sequence, _)) = name.split_once('-') {
            if let Ok(value) = sequence.parse::<u32>() {
                if value > highest {
                    highest = value;
                }
            }
        }
    }
    Ok(highest + 1)
}

/// The last record of a kind of the decision history of a candidate.
fn last_history(
    store: &SceneStore,
    candidate_id: &str,
    kind: &str,
) -> Result<Option<DecisionReport>, EditError> {
    let directory = store.decisions_dir(candidate_id);
    if !directory.is_dir() {
        return Ok(None);
    }
    let suffix = format!("-{kind}.json");
    let entries = std::fs::read_dir(&directory).map_err(|err| {
        io_error(
            &directory,
            format!("the decision history cannot be read: {err}"),
        )
    })?;
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in entries {
        let entry = entry.map_err(|err| {
            io_error(
                &directory,
                format!("the decision history cannot be read: {err}"),
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(&suffix) {
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
    let path = match best {
        Some((_, path)) => path,
        None => return Ok(None),
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the decision cannot be read: {err}")))?;
    let report = serde_json::from_str(&text).map_err(|err| EditError::Scene {
        message: format!("the decision '{}' is not usable: {err}", path.display()),
    })?;
    Ok(Some(report))
}

/// Record a candidate in the index of the store.
fn record_index(
    store: &SceneStore,
    report: &DecisionReport,
    scene_revision: Option<String>,
) -> Result<(), EditError> {
    let mut index = load_index(store)?;
    let status = match report.decision {
        Decision::Accepted => "accepted",
        Decision::Rejected | Decision::Error => "rejected",
        Decision::Proposed => "proposed",
    };
    let submitted_at = index
        .entry(&report.candidate_id)
        .map(|entry| entry.submitted_at.clone())
        .unwrap_or_else(|| report.decided_at.clone());
    index.upsert(CandidateIndexEntry {
        candidate_id: report.candidate_id.clone(),
        status: status.to_string(),
        scene_revision,
        submitted_at,
        decided_at: Some(report.decided_at.clone()),
        decided_by: report.decided_by.clone(),
        reason: None,
    });
    write_index(store, &index)
}

/// Set the authoritative pointer of the store.
fn set_current(
    store: &SceneStore,
    scene_revision: &str,
    candidate_id: Option<String>,
    clock: &dyn Clock,
) -> Result<(), EditError> {
    let pointer = CurrentPointer {
        scene_revision: scene_revision.to_string(),
        candidate_id,
        accepted_at: clock.now_rfc3339_utc(),
    };
    write_current(store, &pointer)
}

/// Read a candidate of the store.
fn load_stored_candidate(
    store: &SceneStore,
    candidate_id: &str,
) -> Result<EditCandidate, EditError> {
    if !is_valid_component_name(candidate_id) {
        return Err(EditError::Schema {
            message: format!("the candidate id '{candidate_id}' is not usable"),
        });
    }
    let path = store.candidate_path(candidate_id);
    let text = std::fs::read_to_string(&path).map_err(|err| {
        io_error(
            &path,
            format!("the candidate '{candidate_id}' is not in the store: {err}"),
        )
    })?;
    parse_candidate(&text).map_err(|error| EditError::Schema {
        message: format!(
            "the candidate '{candidate_id}' of the store is not usable: {}",
            error.message
        ),
    })
}

/// The identifier the candidate is stored under.
fn candidate_id_of(
    candidate_dir: &Path,
    text: &str,
    parsed: &Result<EditCandidate, crate::candidate::CandidateError>,
) -> Result<String, EditError> {
    if let Ok(candidate) = parsed {
        return Ok(candidate.candidate_id.clone());
    }
    let hint = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("candidate_id")
                .and_then(|value| value.as_str())
                .map(|text| text.to_string())
        });
    let fallback = candidate_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let candidate_id = hint.unwrap_or(fallback);
    let sanitized = sanitize_id(&candidate_id);
    match sanitized {
        Some(sanitized) => Ok(sanitized),
        None => Err(EditError::Schema {
            message: format!(
                "the candidate id '{candidate_id}' is not usable (at most 64 characters of [A-Za-z0-9_.-], the first one not a dash)"
            ),
        }),
    }
}

/// The base scene revision of an unreadable candidate (best effort).
fn base_scene_revision_hint(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("base")
                .and_then(|base| base.get("scene_revision"))
                .and_then(|value| value.as_str())
                .map(|text| text.to_string())
        })
}

/// Reduce a name to a usable candidate identifier.
fn sanitize_id(name: &str) -> Option<String> {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric()
            || character == '_'
            || character == '.'
            || character == '-'
        {
            sanitized.push(character);
        }
    }
    if sanitized.is_empty() || sanitized.len() > 64 {
        return None;
    }
    if !sanitized
        .as_bytes()
        .first()
        .map(|byte| byte.is_ascii_alphanumeric())
        .unwrap_or(false)
    {
        return None;
    }
    Some(sanitized)
}

/// Copy a directory tree.
fn copy_tree(source: &Path, destination: &Path) -> Result<(), EditError> {
    std::fs::create_dir_all(destination).map_err(|err| {
        io_error(
            destination,
            format!("the directory cannot be created: {err}"),
        )
    })?;
    let entries = std::fs::read_dir(source)
        .map_err(|err| io_error(source, format!("the directory cannot be read: {err}")))?;
    for entry in entries {
        let entry = entry
            .map_err(|err| io_error(source, format!("the directory cannot be read: {err}")))?;
        let path = entry.path();
        let name = entry.file_name();
        let target = destination.join(&name);
        if path.is_dir() {
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)
                .map_err(|err| io_error(&target, format!("the file cannot be copied: {err}")))?;
        }
    }
    Ok(())
}

/// The index schema of the store (re-exported for the callers).
pub const INDEX_SCHEMA_NAME: &str = INDEX_SCHEMA;

/// The path of a candidate geometry inside a candidate directory.
pub fn geometry_path(candidate_dir: &Path, relative: &str) -> PathBuf {
    candidate_dir.join(relative)
}
