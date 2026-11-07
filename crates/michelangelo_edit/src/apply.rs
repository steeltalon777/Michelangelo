//! The applier of the edit boundary.
//!
//! The applier implements the four operations of the boundary:
//!
//! * `submit` — a candidate enters the store and is validated; it becomes a
//!   `proposed` candidate or, when the workspace policy allows it, an
//!   `accepted` one,
//! * `accept` — an author accepts a candidate and a new scene revision is
//!   published,
//! * `reject` — an author refuses a candidate,
//! * `rollback` — the store points to a previous revision again.
//!
//! Publishing a revision never modifies the base revision, never modifies the
//! `source/` directory of the run and never modifies a component that the
//! candidate does not change: the bytes of an untouched component are copied
//! into the new revision verbatim.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate::{is_valid_component_name, parse_candidate, CandidateAction, EditCandidate};
use crate::error::{io_error, EditError};
use crate::policy::load_policy;
use crate::scene::{
    component_bytes, list_revisions, load_current, load_index, load_scene, next_revision,
    publish_revision, write_current, write_index, CandidateIndexEntry, CurrentPointer,
    RollbackRecord, Scene, SceneComponent, SceneStore,
};
use crate::validate::{
    validate, Check, CheckStatus, Decision, DecisionReport, Diagnostic, Severity,
    ValidationContext, ValidationMode,
};
use crate::{Clock, INDEX_SCHEMA};

/// The schema of the rollback report.
pub const ROLLBACK_SCHEMA: &str = "michelangelo.edit-rollback/0";
/// The schema of the scene view.
pub const VIEW_SCHEMA: &str = "michelangelo.edit-scene-view/0";

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

/// Submit a candidate into the store of a run.
///
/// The candidate is copied into the store, validated against the current scene
/// revision and decided: it becomes `proposed` (an explicit author
/// acceptance is required) or, when the workspace policy allows it,
/// `accepted`.
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
    let candidate = match parsed {
        Ok(candidate) => candidate,
        Err(error) => {
            let mut report =
                DecisionReport::empty(candidate_id.clone(), Decision::Error, &decided_at);
            report.base_scene_revision = base_scene_revision_hint(&text);
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
            write_decision(&store, &report)?;
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

    let context = ValidationContext {
        candidate: &candidate,
        candidate_dir: &store_candidate_dir,
        base_scene: &base_scene,
        current_scene_revision: &pointer.scene_revision,
        mode: ValidationMode::Submit,
        decided_by: None,
    };
    let outcome = validate(&context, policy.as_ref());

    let mut report = DecisionReport::empty(candidate_id.clone(), Decision::Proposed, &decided_at);
    report.base_scene_revision = Some(pointer.scene_revision.clone());
    report.checks = outcome.checks.clone();
    report.diagnostics = outcome.diagnostics.clone();

    if !outcome.valid {
        report.decision = Decision::Rejected;
        write_decision(&store, &report)?;
        record_index(&store, &report, None)?;
        return Ok(report);
    }

    if outcome.auto_accept {
        let revision = publish_candidate(&store, &candidate, &base_scene, &outcome, clock)?;
        report.decision = Decision::Accepted;
        report.resulting_scene_revision = Some(revision.clone());
        write_decision(&store, &report)?;
        record_index(&store, &report, Some(revision.clone()))?;
        set_current(&store, &revision, Some(candidate_id.clone()), clock)?;
        return Ok(report);
    }

    report.decision = Decision::Proposed;
    write_decision(&store, &report)?;
    record_index(&store, &report, None)?;
    Ok(report)
}

/// Accept a candidate of the store and publish a new scene revision.
pub fn accept_candidate(
    run_dir: &Path,
    candidate_id: &str,
    by: &str,
    clock: &dyn Clock,
) -> Result<DecisionReport, EditError> {
    let store = SceneStore::new(run_dir);
    let candidate = load_stored_candidate(&store, candidate_id)?;
    let pointer = load_current(&store)?.ok_or_else(|| EditError::Scene {
        message: "the scene store is not initialized: run 'michelangelo edit init' first"
            .to_string(),
    })?;
    let base_scene = load_scene(&store, &pointer.scene_revision)?;
    let policy = load_policy(run_dir)?;

    let candidate_dir = store.candidate_dir(candidate_id);
    let context = ValidationContext {
        candidate: &candidate,
        candidate_dir: &candidate_dir,
        base_scene: &base_scene,
        current_scene_revision: &pointer.scene_revision,
        mode: ValidationMode::Accept,
        decided_by: Some(by),
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
        write_decision(&store, &report)?;
        record_index(&store, &report, None)?;
        return Ok(report);
    }

    let revision = publish_candidate(&store, &candidate, &base_scene, &outcome, clock)?;
    report.decision = Decision::Accepted;
    report.resulting_scene_revision = Some(revision.clone());
    write_decision(&store, &report)?;
    record_index(&store, &report, Some(revision.clone()))?;
    set_current(&store, &revision, Some(candidate_id.to_string()), clock)?;
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
    let decided_at = clock.now_rfc3339_utc();
    let mut report =
        DecisionReport::empty(candidate_id.to_string(), Decision::Rejected, &decided_at);
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
    write_decision(&store, &report)?;
    record_index(&store, &report, None)?;
    Ok(report)
}

/// Roll the store back to a previous scene revision.
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

/// Publish a new scene revision for an accepted candidate.
fn publish_candidate(
    store: &SceneStore,
    candidate: &EditCandidate,
    base_scene: &Scene,
    outcome: &crate::validate::ValidationOutcome,
    clock: &dyn Clock,
) -> Result<String, EditError> {
    let revision = next_revision(store)?;
    let mut components: Vec<SceneComponent> = Vec::new();
    let mut payloads: Vec<(String, Vec<u8>)> = Vec::new();

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
                components.push(SceneComponent {
                    name: component.name.clone(),
                    part_id: component.part_id.clone(),
                    obj: format!(
                        "{}/{}.obj",
                        crate::scene::COMPONENTS_DIR_NAME,
                        component.name
                    ),
                    sha256: change.sha256.clone(),
                    bbox_min: change.bbox.min,
                    bbox_max: change.bbox.max,
                    protected: component.protected,
                });
                payloads.push((component.name.clone(), bytes));
            }
            None => {
                // The component is untouched: its bytes are copied verbatim.
                let bytes = component_bytes(store, &base_scene.scene_revision, &component.name)?;
                components.push(component.clone());
                payloads.push((component.name.clone(), bytes));
            }
        }
    }

    for change in &outcome.prepared {
        if change.action != CandidateAction::Add {
            continue;
        }
        if components
            .iter()
            .any(|component| component.name == change.component)
        {
            continue;
        }
        let bytes = std::fs::read(&change.source).map_err(|err| {
            io_error(
                &change.source,
                format!("the geometry cannot be read: {err}"),
            )
        })?;
        components.push(SceneComponent {
            name: change.component.clone(),
            part_id: None,
            obj: format!(
                "{}/{}.obj",
                crate::scene::COMPONENTS_DIR_NAME,
                change.component
            ),
            sha256: change.sha256.clone(),
            bbox_min: change.bbox.min,
            bbox_max: change.bbox.max,
            protected: false,
        });
        payloads.push((change.component.clone(), bytes));
    }

    components.sort_by(|left, right| left.name.cmp(&right.name));
    payloads.sort_by(|left, right| left.0.cmp(&right.0));

    let scene = Scene {
        schema: crate::SCENE_SCHEMA.to_string(),
        run_id: base_scene.run_id.clone(),
        scene_revision: revision.clone(),
        base_scene_revision: Some(base_scene.scene_revision.clone()),
        parent_candidate_id: Some(candidate.candidate_id.clone()),
        created_at: clock.now_rfc3339_utc(),
        components,
        source: base_scene.source.clone(),
        notes: base_scene.notes.clone(),
    };
    publish_revision(store, &scene, &payloads)?;
    Ok(revision)
}

/// Write the decision report of a candidate into the store.
fn write_decision(store: &SceneStore, report: &DecisionReport) -> Result<(), EditError> {
    let text = report.to_json()?;
    crate::scene::write_bytes_atomic(
        &store.decision_path(&report.candidate_id),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
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
