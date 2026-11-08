//! The unknown state machine of the local edit boundary (IT-8C).
//!
//! The module implements the state documents of the contract
//! `docs/specs/VERIFICATION_STATE.md` §2 and §9:
//!
//! ```text
//! <run>/verification/bindings/bNNNN.json        michelangelo.unknown-bindings/0
//! <run>/verification/confirmations/cfNNNN.json  michelangelo.confirmation/0
//! <run>/verification/resolutions/rNNNN.json     michelangelo.resolution/0
//! <run>/verification/unknowns/revisions/sNNNN.json  michelangelo.unknown-state/0
//! <run>/verification/unknowns/events.jsonl      append-only audit trail
//! ```
//!
//! Every document is immutable: a binding, a confirmation and a resolution
//! are written once and are never rewritten. The unknown state of a revision
//! is an immutable snapshot too, so a revision always refers to the state it
//! was published with.
//!
//! The rules of the module:
//!
//! * a bare `part_id` match never closes an unknown: only an explicit
//!   binding, an author confirmation and a published resolution do,
//! * the closure happens only in a scene revision whose bundle references the
//!   resolution,
//! * every new bundle recomputes the state: a resolution whose dependencies
//!   changed is not carried forward (the unknown becomes `open` with the
//!   reason `evidence_stale`),
//! * the historical documents stay in the history but are not current.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use michelangelo_input::policy::PackagePolicy;

use crate::candidate::{EditCandidate, Region};
use crate::error::{io_error, EditError};
use crate::gate::EvaluationBinding;
use crate::scene::{load_current, Scene, SceneStore};
use crate::verification::{
    aggregate_digest, canonical_json, file_sha256, load_unknown_state, write_bytes_new,
    write_unknown_state, SnapshotManifest, StoreLock, UnknownState, UnknownStateEntry,
};
use crate::Clock;

/// The schema of a binding document.
pub const BINDINGS_SCHEMA: &str = "michelangelo.unknown-bindings/0";
/// The schema of an author confirmation.
pub const CONFIRMATION_SCHEMA: &str = "michelangelo.confirmation/0";
/// The schema of a resolution.
pub const RESOLUTION_SCHEMA: &str = "michelangelo.resolution/0";
/// The schema of the view of the unknowns.
pub const UNKNOWN_VIEW_SCHEMA: &str = "michelangelo.unknown-view/0";

/// The name of the directory of the binding documents.
pub const BINDINGS_DIR_NAME: &str = "bindings";
/// The name of the directory of the author confirmations.
pub const CONFIRMATIONS_DIR_NAME: &str = "confirmations";
/// The name of the directory of the resolutions.
pub const RESOLUTIONS_DIR_NAME: &str = "resolutions";

/// The state of an unknown that waits for a decision.
pub const STATUS_OPEN: &str = "open";
/// The state of an unknown that the solver represented with a stub.
pub const STATUS_STUB_PENDING: &str = "stub_pending";
/// The state of an unknown that a published resolution closed.
pub const STATUS_RESOLVED: &str = "resolved";
/// The reason of an unknown whose evidence became stale.
pub const REASON_EVIDENCE_STALE: &str = "evidence_stale";

/// The names that may not be used as the author of a confirmation.
///
/// The `by` value of a confirmation is the authorization of the owner (or of
/// an operator the owner delegated). An agent may not confirm its own work:
/// the assistant surface never offers the confirmation and the CLI refuses
/// the reserved identities of an agent.
pub const RESERVED_AUTHOR_IDENTITIES: [&str; 10] = [
    "agent",
    "assistant",
    "ai",
    "ai_author",
    "autonomous",
    "bot",
    "dsh",
    "llm",
    "michelangelo",
    "model",
];

/// One binding of an unknown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnknownBinding {
    /// The identifier of the unknown.
    pub unknown_id: String,
    /// The components the binding covers.
    pub components: Vec<String>,
    /// The region the binding covers.
    pub region: Region,
    /// The author that created the binding.
    pub by: String,
    /// The claim of the author about the unknown (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<String>,
}

/// An immutable, versioned document of bindings.
///
/// Every version carries the whole set of the bindings of the store: a
/// revision that refers to a version sees exactly the bindings that were
/// declared when the version was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingDocument {
    /// The schema identifier.
    pub schema: String,
    /// The version of the bindings (`bNNNN`).
    pub version: String,
    /// The creation time of the document (RFC 3339, UTC).
    pub created_at: String,
    /// The bindings, sorted by the identifier of the unknown.
    pub bindings: Vec<UnknownBinding>,
}

impl BindingDocument {
    /// The binding of an unknown (None when the unknown is not bound).
    pub fn binding(&self, unknown_id: &str) -> Option<&UnknownBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.unknown_id == unknown_id)
    }
}

/// An immutable author confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the confirmation (`cf-NNNN`).
    pub confirmation_id: String,
    /// The identifier of the unknown.
    pub unknown_id: String,
    /// The claim the author accepted.
    pub claim: String,
    /// The digest of the claim (contract §3).
    pub claim_digest: String,
    /// The aggregate digest of the geometry the confirmation is about.
    pub target_geometry_digest: String,
    /// The identifier of the evaluation the confirmation is bound to.
    pub evaluation_id: String,
    /// The digest of the report of the evaluation.
    pub evaluation_sha256: String,
    /// The digest of the evidence file of the author (None when the author
    /// confirmed without a file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_sha256: Option<String>,
    /// The author that confirmed.
    pub by: String,
    /// The creation time of the confirmation (RFC 3339, UTC).
    pub created_at: String,
}

/// One check of a resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionCheck {
    /// The identifier of the check.
    pub id: String,
    /// The status of the check (`pass` or `unavailable`).
    pub status: String,
    /// The human readable detail of the check.
    pub detail: String,
}

/// An immutable resolution of an unknown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the resolution (`r-NNNN`).
    pub resolution_id: String,
    /// The identifier of the unknown.
    pub unknown_id: String,
    /// The version of the binding the resolution is based on.
    pub binding_version: String,
    /// The candidate whose snapshot resolves the unknown.
    pub candidate_id: String,
    /// The identifier of the evaluation of the resolving geometry.
    pub evaluation_id: String,
    /// The digest of the report of the evaluation.
    pub evaluation_sha256: String,
    /// The aggregate digest of the resolving geometry.
    pub target_geometry_digest: String,
    /// The digest of the claim of the author.
    pub claim_digest: String,
    /// The identifier of the author confirmation.
    pub confirmation_id: String,
    /// The digest of the evidence of the author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_sha256: Option<String>,
    /// The author that decided the resolution.
    pub by: String,
    /// The time of the decision (RFC 3339, UTC).
    pub decided_at: String,
    /// The checks the resolution is based on.
    pub checks: Vec<ResolutionCheck>,
}

/// The documents a published revision refers to when it resolves an unknown.
#[derive(Debug, Clone)]
pub struct ResolutionMaterial {
    /// The resolution itself.
    pub resolution: Resolution,
    /// The confirmation the resolution is based on.
    pub confirmation: Confirmation,
    /// The version of the bindings the resolution is based on.
    pub binding_version: String,
}

/// The state of one unknown, as the view reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnknownEntryView {
    /// The identifier of the unknown.
    pub unknown_id: String,
    /// The state of the unknown.
    pub status: String,
    /// The reason of the state (None when there is none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Whether the unknown was a stub of the solver.
    pub stub: bool,
    /// The status the solver reported.
    pub solver_status: String,
    /// The detail the solver reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The binding of the unknown (None when it is not bound).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<UnknownBinding>,
    /// The version of the binding document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_version: Option<String>,
    /// The resolution of the unknown (None when it is not resolved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    /// The confirmations of the unknown.
    pub confirmations: Vec<Confirmation>,
    /// The references to the evidence of the unknown.
    pub evidence_refs: Vec<String>,
}

/// The view of the unknowns of a revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnknownView {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the run.
    pub run_id: String,
    /// The revision the view describes.
    pub scene_revision: String,
    /// The version of the binding document that is current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_version: Option<String>,
    /// The unknowns of the revision, sorted by the identifier.
    pub unknowns: Vec<UnknownEntryView>,
    /// The identifiers of the unknowns that are not resolved.
    pub unresolved: Vec<String>,
}

/// The report of a synchronisation of the unknown state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncReport {
    /// The schema identifier.
    pub schema: String,
    /// The revision the synchronisation applies to.
    pub scene_revision: String,
    /// The number of the unknowns of the immutable report.
    pub imported: usize,
    /// The number of the unknowns that are already in the state.
    pub unchanged: usize,
    /// The number of the unknowns the state does not know yet.
    pub added: usize,
    /// The identifiers of the unknowns the state does not know yet.
    pub missing: Vec<String>,
    /// True when the state document already existed (the sync is idempotent).
    pub state_existed: bool,
}

/// The digest of a claim (contract §3).
pub fn claim_digest(
    unknown_id: &str,
    claim: &str,
    author_conditions: &[String],
    target_geometry_digest: &str,
) -> String {
    let value = serde_json::json!({
        "unknown_id": unknown_id,
        "claim": claim,
        "author_conditions": author_conditions,
        "target_geometry_digest": target_geometry_digest,
    });
    michelangelo_input::sha256::sha256_hex(canonical_json(&value).as_bytes())
}

/// The path of a binding document, relative to the run directory.
pub fn binding_relative_path(version: &str) -> String {
    format!("verification/bindings/{version}.json")
}

/// The path of a confirmation, relative to the run directory.
pub fn confirmation_relative_path(confirmation_id: &str) -> String {
    format!("verification/confirmations/{confirmation_id}.json")
}

/// The path of a resolution, relative to the run directory.
pub fn resolution_relative_path(resolution_id: &str) -> String {
    format!("verification/resolutions/{resolution_id}.json")
}

/// Read a binding document (None when the version does not exist).
pub fn load_binding_document(
    store: &SceneStore,
    version: &str,
) -> Result<Option<BindingDocument>, EditError> {
    let path = store.binding_path(version);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the binding cannot be read: {err}")))?;
    let document: BindingDocument =
        serde_json::from_str(&text).map_err(|err| EditError::Scene {
            message: format!("the binding '{}' is not usable: {err}", path.display()),
        })?;
    Ok(Some(document))
}

/// The versions of the binding documents of the store, sorted by name.
pub fn list_binding_versions(store: &SceneStore) -> Result<Vec<String>, EditError> {
    let directory = store.bindings_dir();
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(&directory)
        .map_err(|err| io_error(&directory, format!("the bindings cannot be read: {err}")))?;
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|err| io_error(&directory, format!("the bindings cannot be read: {err}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".json") {
            if stem.starts_with('b') && stem[1..].chars().all(|c| c.is_ascii_digit()) {
                versions.push(stem.to_string());
            }
        }
    }
    versions.sort();
    Ok(versions)
}

/// The current binding document of the store (None when there is none).
pub fn latest_binding_document(store: &SceneStore) -> Result<Option<BindingDocument>, EditError> {
    let versions = list_binding_versions(store)?;
    match versions.last() {
        Some(version) => load_binding_document(store, version),
        None => Ok(None),
    }
}

/// The next free version of the binding documents.
fn next_binding_version(store: &SceneStore) -> Result<String, EditError> {
    let versions = list_binding_versions(store)?;
    let mut highest = 0_u32;
    for version in &versions {
        if let Ok(number) = version[1..].parse::<u32>() {
            if number > highest {
                highest = number;
            }
        }
    }
    Ok(format!("b{:04}", highest + 1))
}

/// The next free identifier of the confirmations.
fn next_confirmation_id(store: &SceneStore) -> Result<String, EditError> {
    Ok(format!(
        "cf-{:04}",
        next_number(&store.confirmations_dir(), "cf")? + 1
    ))
}

/// The next free identifier of the resolutions.
fn next_resolution_id(store: &SceneStore) -> Result<String, EditError> {
    Ok(format!(
        "r-{:04}",
        next_number(&store.resolutions_dir(), "r")? + 1
    ))
}

/// The next free number of the files of a directory with a prefix.
fn next_number(directory: &Path, prefix: &str) -> Result<u32, EditError> {
    if !directory.is_dir() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(directory)
        .map_err(|err| io_error(directory, format!("the directory cannot be read: {err}")))?;
    let mut highest = 0_u32;
    for entry in entries {
        let entry = entry
            .map_err(|err| io_error(directory, format!("the directory cannot be read: {err}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let stem = match name.strip_suffix(".json") {
            Some(stem) => stem,
            None => continue,
        };
        let rest = match stem.strip_prefix(prefix) {
            Some(rest) => rest,
            None => continue,
        };
        // The identifiers carry a dash ("cf-0001"): the number follows it.
        let rest = rest.trim_start_matches('-');
        if let Ok(number) = rest.parse::<u32>() {
            if number > highest {
                highest = number;
            }
        }
    }
    Ok(highest)
}

/// Bind an unknown to a set of components and to a region.
///
/// The operation writes the next version of the binding document: the
/// document carries the whole set of the bindings, so a revision that refers
/// to a version sees exactly the bindings of that moment.
pub fn bind_unknown(
    store: &SceneStore,
    unknown_id: &str,
    components: &[String],
    region: Region,
    by: &str,
    claim: Option<&str>,
    clock: &dyn Clock,
) -> Result<BindingDocument, EditError> {
    if unknown_id.trim().is_empty() {
        return Err(EditError::Usage {
            message: "the identifier of the unknown must not be empty".to_string(),
        });
    }
    if by.trim().is_empty() {
        return Err(EditError::Usage {
            message: "the author of the binding must not be empty".to_string(),
        });
    }
    if components.is_empty() {
        return Err(EditError::Usage {
            message: "a binding needs at least one component".to_string(),
        });
    }
    if !region.is_valid() {
        return Err(EditError::Usage {
            message: "the region of the binding is empty or inverted".to_string(),
        });
    }
    let _lock = StoreLock::acquire(store, "unknown-bind")?;

    // The unknown must exist: a binding of an unknown that the run does not
    // know is refused.
    let state = current_state(store)?;
    if !state
        .unknowns
        .iter()
        .any(|entry| entry.unknown_id == unknown_id)
    {
        return Err(EditError::UnknownNotFound {
            message: format!(
                "the run does not know the unknown '{unknown_id}': run 'michelangelo unknown sync' first"
            ),
        });
    }

    let mut bindings = match latest_binding_document(store)? {
        Some(document) => document.bindings,
        None => Vec::new(),
    };
    bindings.retain(|binding| binding.unknown_id != unknown_id);
    bindings.push(UnknownBinding {
        unknown_id: unknown_id.to_string(),
        components: components.to_vec(),
        region,
        by: by.to_string(),
        claim: claim.map(|text| text.to_string()),
    });
    bindings.sort_by(|left, right| left.unknown_id.cmp(&right.unknown_id));

    let document = BindingDocument {
        schema: BINDINGS_SCHEMA.to_string(),
        version: next_binding_version(store)?,
        created_at: clock.now_rfc3339_utc(),
        bindings,
    };
    let text = serde_json::to_string_pretty(&document).map_err(|err| EditError::Internal {
        message: format!("the binding cannot be serialized: {err}"),
    })?;
    write_bytes_new(
        &store.binding_path(&document.version),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )?;
    crate::verification::append_event(
        store,
        &serde_json::json!({
            "event": "bind",
            "at": document.created_at,
            "by": by,
            "unknown_id": unknown_id,
            "binding_version": document.version,
            "components": components,
        }),
    )?;
    Ok(document)
}

/// Read the confirmations of the store, sorted by the identifier.
pub fn list_confirmations(store: &SceneStore) -> Result<Vec<Confirmation>, EditError> {
    let directory = store.confirmations_dir();
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(&directory).map_err(|err| {
        io_error(
            &directory,
            format!("the confirmations cannot be read: {err}"),
        )
    })?;
    let mut confirmations = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            io_error(
                &directory,
                format!("the confirmations cannot be read: {err}"),
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| io_error(&path, format!("the confirmation cannot be read: {err}")))?;
        let confirmation: Confirmation =
            serde_json::from_str(&text).map_err(|err| EditError::Scene {
                message: format!("the confirmation '{}' is not usable: {err}", path.display()),
            })?;
        confirmations.push(confirmation);
    }
    confirmations.sort_by(|left, right| left.confirmation_id.cmp(&right.confirmation_id));
    Ok(confirmations)
}

/// Read a confirmation (None when it does not exist).
pub fn load_confirmation(
    store: &SceneStore,
    confirmation_id: &str,
) -> Result<Option<Confirmation>, EditError> {
    Ok(list_confirmations(store)?
        .into_iter()
        .find(|confirmation| confirmation.confirmation_id == confirmation_id))
}

/// Read the resolutions of the store, sorted by the identifier.
pub fn list_resolutions(store: &SceneStore) -> Result<Vec<Resolution>, EditError> {
    let directory = store.resolutions_dir();
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(&directory)
        .map_err(|err| io_error(&directory, format!("the resolutions cannot be read: {err}")))?;
    let mut resolutions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            io_error(&directory, format!("the resolutions cannot be read: {err}"))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| io_error(&path, format!("the resolution cannot be read: {err}")))?;
        let resolution: Resolution =
            serde_json::from_str(&text).map_err(|err| EditError::Scene {
                message: format!("the resolution '{}' is not usable: {err}", path.display()),
            })?;
        resolutions.push(resolution);
    }
    resolutions.sort_by(|left, right| left.resolution_id.cmp(&right.resolution_id));
    Ok(resolutions)
}

/// The latest resolution of an unknown (None when there is none).
pub fn latest_resolution(
    store: &SceneStore,
    unknown_id: &str,
) -> Result<Option<Resolution>, EditError> {
    Ok(list_resolutions(store)?
        .into_iter()
        .rfind(|resolution| resolution.unknown_id == unknown_id))
}

/// The policy of the package copy of the run (the default when the run
/// carries no copy).
pub fn package_policy(store: &SceneStore) -> PackagePolicy {
    let path = store.package_copy_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return PackagePolicy::default(),
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => PackagePolicy::from_json(&value),
        Err(_) => PackagePolicy::default(),
    }
}

/// The state of the unknowns of the current revision.
pub fn current_state(store: &SceneStore) -> Result<UnknownState, EditError> {
    let pointer = load_current(store)?.ok_or_else(|| EditError::Scene {
        message: "the scene store is not initialized: run 'michelangelo edit init' first"
            .to_string(),
    })?;
    load_unknown_state(store, &pointer.scene_revision)?.ok_or_else(|| EditError::Scene {
        message: format!(
            "the revision '{}' carries no state document of the unknowns",
            pointer.scene_revision
        ),
    })
}

/// The state of the unknowns of a revision (the current one by default).
pub fn state_of(
    store: &SceneStore,
    revision: Option<&str>,
) -> Result<(String, UnknownState), EditError> {
    let revision = match revision {
        Some(revision) => revision.to_string(),
        None => {
            load_current(store)?
                .ok_or_else(|| EditError::Scene {
                    message:
                        "the scene store is not initialized: run 'michelangelo edit init' first"
                            .to_string(),
                })?
                .scene_revision
        }
    };
    let state = load_unknown_state(store, &revision)?.ok_or_else(|| EditError::Scene {
        message: format!("the revision '{revision}' carries no state document of the unknowns"),
    })?;
    Ok((revision, state))
}

/// The list of the unknowns of a revision.
pub fn list_unknowns(
    store: &SceneStore,
    revision: Option<&str>,
) -> Result<Vec<UnknownStateEntry>, EditError> {
    Ok(state_of(store, revision)?.1.unknowns)
}

/// The view of the unknowns of a revision.
pub fn show_unknowns(store: &SceneStore, revision: Option<&str>) -> Result<UnknownView, EditError> {
    let (revision, state) = state_of(store, revision)?;
    let binding_document = latest_binding_document(store)?;
    let confirmations = list_confirmations(store)?;
    let resolutions = list_resolutions(store)?;

    let mut unknowns = Vec::new();
    let mut unresolved = Vec::new();
    for entry in &state.unknowns {
        let binding = binding_document
            .as_ref()
            .and_then(|document| document.binding(&entry.unknown_id))
            .cloned();
        let resolution = entry
            .resolution_id
            .as_ref()
            .and_then(|id| resolutions.iter().find(|item| &item.resolution_id == id))
            .cloned();
        let own: Vec<Confirmation> = confirmations
            .iter()
            .filter(|confirmation| confirmation.unknown_id == entry.unknown_id)
            .cloned()
            .collect();
        let mut evidence_refs: Vec<String> = own
            .iter()
            .filter_map(|confirmation| confirmation.evidence_sha256.clone())
            .collect();
        if let Some(resolution) = &resolution {
            if let Some(evidence) = &resolution.evidence_sha256 {
                if !evidence_refs.contains(evidence) {
                    evidence_refs.push(evidence.clone());
                }
            }
            evidence_refs.push(resolution.evaluation_sha256.clone());
        }
        if entry.status != STATUS_RESOLVED {
            unresolved.push(entry.unknown_id.clone());
        }
        unknowns.push(UnknownEntryView {
            unknown_id: entry.unknown_id.clone(),
            status: entry.status.clone(),
            reason: entry.reason.clone(),
            stub: entry.stub,
            solver_status: entry.solver_status.clone(),
            detail: entry.detail.clone(),
            binding,
            binding_version: entry.binding_version.clone(),
            resolution,
            confirmations: own,
            evidence_refs,
        });
    }
    unresolved.sort();

    Ok(UnknownView {
        schema: UNKNOWN_VIEW_SCHEMA.to_string(),
        run_id: crate::scene::run_id_of(&store.run_dir),
        scene_revision: revision,
        binding_version: binding_document.map(|document| document.version),
        unknowns,
        unresolved,
    })
}

/// Re-import the unknowns of the immutable report of the run.
///
/// The operation is idempotent: the state document of a revision is
/// immutable, so a repeated synchronisation never rewrites it. The report
/// shows what the state does not know yet.
pub fn sync_unknowns(store: &SceneStore, clock: &dyn Clock) -> Result<SyncReport, EditError> {
    let _lock = StoreLock::acquire(store, "unknown-sync")?;
    let pointer = load_current(store)?.ok_or_else(|| EditError::Scene {
        message: "the scene store is not initialized: run 'michelangelo edit init' first"
            .to_string(),
    })?;
    let revision = pointer.scene_revision;
    let seeded =
        crate::verification::seed_unknown_state(store, &revision, &clock.now_rfc3339_utc());
    let existing = load_unknown_state(store, &revision)?;
    let state_existed = existing.is_some();
    let state = match &existing {
        Some(state) => state.clone(),
        None => seeded.clone(),
    };

    let known: Vec<String> = state
        .unknowns
        .iter()
        .map(|entry| entry.unknown_id.clone())
        .collect();
    let missing: Vec<String> = seeded
        .unknowns
        .iter()
        .map(|entry| entry.unknown_id.clone())
        .filter(|id| !known.contains(id))
        .collect();

    if existing.is_none() {
        write_unknown_state(store, &seeded)?;
    }

    crate::verification::append_event(
        store,
        &serde_json::json!({
            "event": "sync",
            "at": clock.now_rfc3339_utc(),
            "scene_revision": revision,
            "imported": seeded.unknowns.len(),
            "state_existed": state_existed,
        }),
    )?;

    Ok(SyncReport {
        schema: String::from("michelangelo.unknown-sync/0"),
        scene_revision: revision,
        imported: seeded.unknowns.len(),
        unchanged: state.unknowns.len(),
        added: missing.len(),
        missing,
        state_existed,
    })
}

/// The claim of an unknown: the claim of its binding, if it carries one.
fn claim_of(store: &SceneStore, unknown_id: &str) -> Result<Option<String>, EditError> {
    Ok(latest_binding_document(store)?
        .and_then(|document| document.binding(unknown_id).and_then(|b| b.claim.clone())))
}

/// The confirmation of an unknown for a claim, a geometry and an evaluation.
fn find_confirmation(
    store: &SceneStore,
    unknown_id: &str,
    claim: Option<&str>,
    target_geometry_digest: &str,
    evaluation_id: &str,
    evaluation_sha256: &str,
) -> Result<Option<Confirmation>, EditError> {
    let expected = claim.map(|claim| claim_digest(unknown_id, claim, &[], target_geometry_digest));
    for confirmation in list_confirmations(store)? {
        if confirmation.unknown_id != unknown_id {
            continue;
        }
        if confirmation.evaluation_id != evaluation_id {
            continue;
        }
        if confirmation.evaluation_sha256 != evaluation_sha256 {
            continue;
        }
        if confirmation.target_geometry_digest != target_geometry_digest {
            continue;
        }
        if let Some(expected) = &expected {
            if confirmation.claim_digest != *expected {
                continue;
            }
        }
        return Ok(Some(confirmation));
    }
    Ok(None)
}

/// Confirm an unknown on behalf of the author.
///
/// The confirmation is bound to the claim, to the geometry the evaluation
/// measured, to the evaluation and to the evidence of the author. It can
/// never be reused for another geometry or for another evaluation.
pub fn confirm_unknown(
    store: &SceneStore,
    unknown_id: &str,
    by: &str,
    evaluation: &EvaluationBinding,
    evidence: Option<&Path>,
    claim: Option<&str>,
    clock: &dyn Clock,
) -> Result<Confirmation, EditError> {
    if by.trim().is_empty() {
        return Err(EditError::Usage {
            message: "the author of the confirmation must not be empty".to_string(),
        });
    }
    let identity = by.trim().to_ascii_lowercase();
    if RESERVED_AUTHOR_IDENTITIES.contains(&identity.as_str()) {
        return Err(EditError::AgentConfirmationRefused {
            message: format!(
                "'{by}' is not an author: an agent may not confirm its own work as the owner"
            ),
        });
    }

    let _lock = StoreLock::acquire(store, "unknown-confirm")?;

    // The unknown must exist and must not be resolved yet.
    let state = current_state(store)?;
    let entry = state
        .unknowns
        .iter()
        .find(|entry| entry.unknown_id == unknown_id)
        .ok_or_else(|| EditError::UnknownNotFound {
            message: format!("the run does not know the unknown '{unknown_id}'"),
        })?;
    if entry.status == STATUS_RESOLVED {
        return Err(EditError::ResolutionRefused {
            message: format!("the unknown '{unknown_id}' is already resolved"),
        });
    }

    // The evaluation must be registered and its digest must be the one the
    // registry holds.
    let registry =
        michelangelo_evidence::registry::load_registry(&store.run_dir).map_err(EditError::from)?;
    let registered = registry.entry(&evaluation.evaluation_id).ok_or_else(|| {
        EditError::EvaluationUnregistered {
            message: format!(
                "the evaluation '{}' is not registered in the registry of the run",
                evaluation.evaluation_id
            ),
        }
    })?;
    if registered.sha256 != evaluation.sha256 {
        return Err(EditError::EvaluationStale {
            message: format!(
                "the evaluation report of '{}' carries the digest {} but the registry pins {}",
                evaluation.evaluation_id, evaluation.sha256, registered.sha256
            ),
        });
    }

    // The claim of the author: the claim of the binding wins, the claim of
    // the confirmation may not contradict it.
    let bound_claim = claim_of(store, unknown_id)?;
    let claim_text = match (&bound_claim, claim) {
        (Some(bound), Some(given)) if bound != given => {
            return Err(EditError::ConfirmationMissing {
                message: format!(
                    "the confirmation claims '{given}' but the binding of '{unknown_id}' claims '{bound}'"
                ),
            })
        }
        (Some(bound), _) => bound.clone(),
        (None, Some(given)) => given.to_string(),
        (None, None) => String::new(),
    };

    let target_geometry_digest = evaluation.report.target.aggregate_sha256.clone();
    let evidence_sha256 = match evidence {
        Some(path) => {
            if !path.is_file() {
                return Err(io_error(path, "the evidence file does not exist"));
            }
            Some(file_sha256(path)?)
        }
        None => None,
    };

    let confirmation = Confirmation {
        schema: CONFIRMATION_SCHEMA.to_string(),
        confirmation_id: next_confirmation_id(store)?,
        unknown_id: unknown_id.to_string(),
        claim: claim_text.clone(),
        claim_digest: claim_digest(unknown_id, &claim_text, &[], &target_geometry_digest),
        target_geometry_digest,
        evaluation_id: evaluation.evaluation_id.clone(),
        evaluation_sha256: evaluation.sha256.clone(),
        evidence_sha256,
        by: by.to_string(),
        created_at: clock.now_rfc3339_utc(),
    };
    let text = serde_json::to_string_pretty(&confirmation).map_err(|err| EditError::Internal {
        message: format!("the confirmation cannot be serialized: {err}"),
    })?;
    write_bytes_new(
        &store.confirmation_path(&confirmation.confirmation_id),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )?;
    crate::verification::append_event(
        store,
        &serde_json::json!({
            "event": "confirm",
            "at": confirmation.created_at,
            "by": by,
            "unknown_id": unknown_id,
            "confirmation_id": confirmation.confirmation_id,
            "evaluation_id": confirmation.evaluation_id,
            "target_geometry_digest": confirmation.target_geometry_digest,
        }),
    )?;
    Ok(confirmation)
}

/// The inputs of the checks of a resolution.
pub struct ResolutionInputs<'a> {
    /// The store of the run.
    pub store: &'a SceneStore,
    /// The identifier of the unknown.
    pub unknown_id: &'a str,
    /// The candidate whose snapshot resolves the unknown.
    pub candidate: &'a EditCandidate,
    /// The snapshot of the candidate.
    pub manifest: &'a SnapshotManifest,
    /// The evaluation of the snapshot.
    pub evaluation: &'a EvaluationBinding,
}

/// Check every condition of a resolution and build its document.
///
/// The function never writes: the caller decides whether the resolution is
/// proposed (written, not published) or materialized (written and published
/// in a bundle).
pub fn check_resolution(
    inputs: &ResolutionInputs<'_>,
    by: &str,
    decided_at: &str,
) -> Result<(Resolution, Confirmation), EditError> {
    let store = inputs.store;
    let unknown_id = inputs.unknown_id;
    let mut checks: Vec<ResolutionCheck> = Vec::new();

    // 1. A binding must exist: a part id match never closes an unknown.
    let binding_document =
        latest_binding_document(store)?.ok_or_else(|| EditError::BindingMissing {
            message: format!(
            "the unknown '{unknown_id}' carries no binding: run 'michelangelo unknown bind' first"
        ),
        })?;
    let binding = binding_document
        .binding(unknown_id)
        .cloned()
        .ok_or_else(|| EditError::BindingMissing {
            message: format!(
                "the unknown '{unknown_id}' carries no binding in the version '{}'",
                binding_document.version
            ),
        })?;
    checks.push(ResolutionCheck {
        id: String::from("binding"),
        status: String::from("pass"),
        detail: format!(
            "the unknown '{unknown_id}' is bound to the components {} in the version '{}'",
            binding.components.join(", "),
            binding_document.version
        ),
    });

    // 2. The snapshot must be the geometry the evaluation measured.
    let target = &inputs.evaluation.report.target;
    if target.aggregate_sha256 != inputs.manifest.aggregate_sha256 {
        return Err(EditError::EvaluationStale {
            message: format!(
                "the evaluation '{}' measured the geometry {} but the snapshot of the candidate is {}",
                inputs.evaluation.evaluation_id,
                target.aggregate_sha256,
                inputs.manifest.aggregate_sha256
            ),
        });
    }
    if target.id != inputs.candidate.candidate_id {
        return Err(EditError::EvaluationStale {
            message: format!(
                "the evaluation '{}' measured the target '{}' but the resolution is about the candidate '{}'",
                inputs.evaluation.evaluation_id,
                target.id,
                inputs.candidate.candidate_id
            ),
        });
    }
    checks.push(ResolutionCheck {
        id: String::from("geometry"),
        status: String::from("pass"),
        detail: format!(
            "the snapshot of the candidate '{}' is the geometry the evaluation '{}' measured ({})",
            inputs.candidate.candidate_id,
            inputs.evaluation.evaluation_id,
            inputs.manifest.aggregate_sha256
        ),
    });

    // 3. The projection conformance must pass.
    if inputs.evaluation.report.results.projection_conformance
        != michelangelo_evidence::report::TriState::Pass
    {
        return Err(EditError::VisibleFailed {
            message: format!(
                "the projection conformance of the evaluation '{}' is {}",
                inputs.evaluation.evaluation_id,
                inputs
                    .evaluation
                    .report
                    .results
                    .projection_conformance
                    .as_str()
            ),
        });
    }
    checks.push(ResolutionCheck {
        id: String::from("projection_conformance"),
        status: String::from("pass"),
        detail: String::from("the projection conformance of the evaluation passes"),
    });

    // 4. Every required author geometric condition must pass. An optional
    // condition may stay unverified, but only with an explicit note.
    let mut optional_notes: Vec<String> = Vec::new();
    for check in &inputs.evaluation.report.checks {
        if check.kind != michelangelo_evidence::gate::AUTHOR_CONDITION_KIND {
            continue;
        }
        if check.required {
            if check.status != michelangelo_evidence::report::TriState::Pass {
                return Err(EditError::AuthorConditionFailed {
                    message: format!(
                        "the required author condition '{}' of the evaluation '{}' is {}: {}",
                        check.id,
                        inputs.evaluation.evaluation_id,
                        check.status.as_str(),
                        check.detail
                    ),
                });
            }
        } else if check.status == michelangelo_evidence::report::TriState::Unavailable {
            optional_notes.push(format!("{}: {}", check.id, check.detail));
        }
    }
    checks.push(ResolutionCheck {
        id: String::from("author_conditions"),
        status: if optional_notes.is_empty() {
            String::from("pass")
        } else {
            String::from("unavailable")
        },
        detail: if optional_notes.is_empty() {
            String::from("every required author geometric condition of the evaluation passes")
        } else {
            format!(
                "every required author geometric condition passes; the optional conditions stay unverified: {}",
                optional_notes.join("; ")
            )
        },
    });

    // 5. The author confirmation of this claim, this geometry and this
    // evaluation must exist. A binding without a claim lets the confirmation
    // carry the claim (the binding is about the components/region, the author
    // confirmation is about the claim).
    let claim_text = binding.claim.clone();
    let confirmation = find_confirmation(
        store,
        unknown_id,
        claim_text.as_deref(),
        &inputs.manifest.aggregate_sha256,
        &inputs.evaluation.evaluation_id,
        &inputs.evaluation.sha256,
    )?
    .ok_or_else(|| EditError::ConfirmationMissing {
        message: format!(
            "the unknown '{unknown_id}' carries no author confirmation for the claim, the geometry {} and the evaluation '{}'",
            inputs.manifest.aggregate_sha256,
            inputs.evaluation.evaluation_id
        ),
    })?;
    checks.push(ResolutionCheck {
        id: String::from("confirmation"),
        status: String::from("pass"),
        detail: format!(
            "the author '{}' confirmed the unknown with the confirmation '{}'",
            confirmation.by, confirmation.confirmation_id
        ),
    });

    // 6. The uncertainty policy of the package must be satisfied.
    let policy = package_policy(store);
    check_package_policy(store, unknown_id, &policy, &mut checks)?;

    // 7. The change must touch the binding.
    let changed: Vec<String> = inputs
        .candidate
        .changes
        .iter()
        .map(|change| change.component.clone())
        .collect();
    let touches = changed
        .iter()
        .any(|component| binding.components.contains(component));
    if !touches {
        return Err(EditError::BindingMismatch {
            message: format!(
                "the candidate changes the components {} but the binding of '{unknown_id}' covers {}",
                changed.join(", "),
                binding.components.join(", ")
            ),
        });
    }
    let mut intersects = false;
    for change in &inputs.candidate.changes {
        if !binding.components.contains(&change.component) {
            continue;
        }
        let path = crate::verification::snapshot_file_path(
            store,
            &inputs.manifest.snapshot_id,
            &format!(
                "{}/{}.obj",
                crate::verification::GEOMETRY_DIR_NAME,
                change.component
            ),
        );
        let parsed = crate::obj::parse_obj_file(&path).map_err(|err: crate::obj::ObjError| {
            EditError::Scene {
                message: format!(
                    "the component '{}' of the snapshot is not usable: {err}",
                    change.component
                ),
            }
        })?;
        if region_intersects(&binding.region, &parsed.mesh.bbox) {
            intersects = true;
        }
    }
    if !intersects {
        return Err(EditError::BindingMismatch {
            message: format!(
                "the geometry of the candidate does not intersect the region of the binding of '{unknown_id}'"
            ),
        });
    }
    checks.push(ResolutionCheck {
        id: String::from("binding_intersection"),
        status: String::from("pass"),
        detail: format!(
            "the change touches the components {} of the binding and intersects its region",
            binding.components.join(", ")
        ),
    });

    let resolution = Resolution {
        schema: RESOLUTION_SCHEMA.to_string(),
        resolution_id: String::new(),
        unknown_id: unknown_id.to_string(),
        binding_version: binding_document.version.clone(),
        candidate_id: inputs.candidate.candidate_id.clone(),
        evaluation_id: inputs.evaluation.evaluation_id.clone(),
        evaluation_sha256: inputs.evaluation.sha256.clone(),
        target_geometry_digest: inputs.manifest.aggregate_sha256.clone(),
        claim_digest: confirmation.claim_digest.clone(),
        confirmation_id: confirmation.confirmation_id.clone(),
        evidence_sha256: confirmation.evidence_sha256.clone(),
        by: by.to_string(),
        decided_at: decided_at.to_string(),
        checks,
    };
    Ok((resolution, confirmation))
}

/// Check the uncertainty policy of the package for a resolution.
fn check_package_policy(
    store: &SceneStore,
    unknown_id: &str,
    policy: &PackagePolicy,
    checks: &mut Vec<ResolutionCheck>,
) -> Result<(), EditError> {
    // A stub may not be the basis of a resolution when the package forbids
    // the stubs.
    let solver_status = solver_status_of(store, unknown_id);
    if solver_status.as_deref() == Some("stub") && !policy.stubs_allowed() {
        return Err(EditError::StubForbidden {
            message: format!(
                "the package forbids the stubs: the unknown '{unknown_id}' is a stub and cannot be resolved by a stub"
            ),
        });
    }

    // The completion rules of the package limit the kinds that may resolve an
    // unknown.
    if solver_status.as_deref() == Some("resolved") {
        let rules = applied_rules_for(store, unknown_id);
        for rule in rules {
            if !policy.rule_is_allowed(&rule) {
                return Err(EditError::UnknownPolicyViolation {
                    message: format!(
                        "the unknown '{unknown_id}' was resolved by the rule '{rule}' which the package does not allow"
                    ),
                });
            }
        }
    }

    let mut detail = String::from(
        "the uncertainty policy of the package is satisfied (require_author_acceptance: ",
    );
    detail.push_str(if policy.requires_author_acceptance() {
        "the author confirmation of the unknown is the basis of the resolution"
    } else {
        "not required by the package"
    });
    detail.push_str("; allowed_completion_rules: ");
    if policy.allowed_completion_rules.is_empty() {
        detail.push_str("no restriction");
    } else {
        detail.push_str(&policy.allowed_completion_rules.join(", "));
    }
    detail.push_str("; stub_policy: ");
    detail.push_str(if policy.stubs_allowed() {
        "allowed"
    } else {
        "forbidden"
    });
    checks.push(ResolutionCheck {
        id: String::from("package_policy"),
        status: String::from("pass"),
        detail,
    });
    Ok(())
}

/// The status the solver reported for an unknown.
fn solver_status_of(store: &SceneStore, unknown_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(store.report_path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("build")?
        .get("unknowns")?
        .as_array()?
        .iter()
        .find(|unknown| unknown.get("id").and_then(|value| value.as_str()) == Some(unknown_id))
        .and_then(|unknown| {
            unknown
                .get("status")
                .and_then(|value| value.as_str())
                .map(|text| text.to_string())
        })
}

/// The rules of the supplements that the solver applied to the parts of an
/// unknown.
fn applied_rules_for(store: &SceneStore, unknown_id: &str) -> Vec<String> {
    let text = match std::fs::read_to_string(store.report_path()) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    let report: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let package = match std::fs::read_to_string(store.package_copy_path())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    {
        Some(value) => value,
        None => return Vec::new(),
    };
    let parts: Vec<String> = package
        .get("unknowns")
        .and_then(|value| value.as_array())
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.get("id").and_then(|value| value.as_str()) == Some(unknown_id))
        })
        .and_then(|entry| entry.get("affected_parts"))
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|value| value.as_str().map(|text| text.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() {
        return Vec::new();
    }
    let supplements = match package
        .get("supplements")
        .and_then(|value| value.as_array())
    {
        Some(entries) => entries,
        None => return Vec::new(),
    };
    let mut rules = Vec::new();
    for supplement in supplements {
        let supplement_id = supplement
            .get("supplement_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let part_id = supplement
            .get("part_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !parts.iter().any(|part| part == part_id) {
            continue;
        }
        let applied = report
            .get("build")
            .and_then(|build| build.get("supplements"))
            .and_then(|value| value.as_array())
            .map(|entries| {
                entries.iter().any(|entry| {
                    entry.get("supplement_id").and_then(|value| value.as_str())
                        == Some(supplement_id)
                        && entry.get("status").and_then(|value| value.as_str()) == Some("applied")
                })
            })
            .unwrap_or(false);
        if !applied {
            continue;
        }
        if let Some(rule) = supplement.get("kind").and_then(|value| value.as_str()) {
            rules.push(rule.to_string());
        }
    }
    rules.sort();
    rules.dedup();
    rules
}

/// True when a region and a bounding box overlap.
fn region_intersects(region: &Region, bbox: &crate::obj::Bbox) -> bool {
    (0..3).all(|axis| region.min[axis] <= bbox.max[axis] && bbox.min[axis] <= region.max[axis])
}

/// Propose the resolution of an unknown.
///
/// The proposal is an immutable document: it does not change the scene. The
/// unknown stays open until a revision that references the resolution is
/// published.
pub fn propose_resolution(
    store: &SceneStore,
    unknown_id: &str,
    candidate: &EditCandidate,
    manifest: &SnapshotManifest,
    evaluation: &EvaluationBinding,
    by: &str,
    clock: &dyn Clock,
) -> Result<Resolution, EditError> {
    let _lock = StoreLock::acquire(store, "unknown-propose")?;
    let (mut resolution, _confirmation) = check_resolution(
        &ResolutionInputs {
            store,
            unknown_id,
            candidate,
            manifest,
            evaluation,
        },
        by,
        &clock.now_rfc3339_utc(),
    )?;

    // The proposal is idempotent: an identical resolution is reused.
    for existing in list_resolutions(store)? {
        if existing.unknown_id == resolution.unknown_id
            && existing.candidate_id == resolution.candidate_id
            && existing.evaluation_sha256 == resolution.evaluation_sha256
            && existing.target_geometry_digest == resolution.target_geometry_digest
            && existing.binding_version == resolution.binding_version
            && existing.claim_digest == resolution.claim_digest
        {
            return Ok(existing);
        }
    }

    resolution.resolution_id = next_resolution_id(store)?;
    write_resolution(store, &resolution)?;
    crate::verification::append_event(
        store,
        &serde_json::json!({
            "event": "propose",
            "at": resolution.decided_at,
            "by": by,
            "unknown_id": resolution.unknown_id,
            "resolution_id": resolution.resolution_id,
            "candidate_id": resolution.candidate_id,
            "evaluation_id": resolution.evaluation_id,
        }),
    )?;
    Ok(resolution)
}

/// Write an immutable resolution document.
pub fn write_resolution(store: &SceneStore, resolution: &Resolution) -> Result<(), EditError> {
    let text = serde_json::to_string_pretty(resolution).map_err(|err| EditError::Internal {
        message: format!("the resolution cannot be serialized: {err}"),
    })?;
    write_bytes_new(
        &store.resolution_path(&resolution.resolution_id),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
}

/// Materialize the resolution of an unknown for a candidate that is about to
/// be accepted.
///
/// The function checks every condition of the resolution and writes the
/// immutable resolution document when it does not exist yet. The caller
/// publishes the revision that references it.
pub fn materialize_resolution(
    store: &SceneStore,
    unknown_id: &str,
    candidate: &EditCandidate,
    manifest: &SnapshotManifest,
    evaluation: &EvaluationBinding,
    by: &str,
    clock: &dyn Clock,
) -> Result<ResolutionMaterial, EditError> {
    let _lock = StoreLock::acquire(store, "unknown-resolve")?;
    materialize_resolution_locked(
        store, unknown_id, candidate, manifest, evaluation, by, clock,
    )
}

/// Materialize the resolution of an unknown for a candidate that is about to
/// be accepted.
///
/// The caller must hold the lock of the store: the acceptance of a candidate
/// holds it while it publishes the revision, so the lock is not taken twice.
pub fn materialize_resolution_locked(
    store: &SceneStore,
    unknown_id: &str,
    candidate: &EditCandidate,
    manifest: &SnapshotManifest,
    evaluation: &EvaluationBinding,
    by: &str,
    clock: &dyn Clock,
) -> Result<ResolutionMaterial, EditError> {
    let (mut resolution, confirmation) = check_resolution(
        &ResolutionInputs {
            store,
            unknown_id,
            candidate,
            manifest,
            evaluation,
        },
        by,
        &clock.now_rfc3339_utc(),
    )?;

    // The resolution of this candidate and this evaluation already exists:
    // it is reused (the documents are immutable).
    for existing in list_resolutions(store)? {
        if existing.unknown_id == resolution.unknown_id
            && existing.candidate_id == resolution.candidate_id
            && existing.evaluation_sha256 == resolution.evaluation_sha256
            && existing.target_geometry_digest == resolution.target_geometry_digest
            && existing.binding_version == resolution.binding_version
            && existing.claim_digest == resolution.claim_digest
        {
            let confirmation =
                load_confirmation(store, &existing.confirmation_id)?.unwrap_or(confirmation);
            let binding_version = existing.binding_version.clone();
            return Ok(ResolutionMaterial {
                resolution: existing,
                confirmation,
                binding_version,
            });
        }
    }

    resolution.resolution_id = next_resolution_id(store)?;
    write_resolution(store, &resolution)?;
    crate::verification::append_event(
        store,
        &serde_json::json!({
            "event": "resolve",
            "at": resolution.decided_at,
            "by": by,
            "unknown_id": resolution.unknown_id,
            "resolution_id": resolution.resolution_id,
            "candidate_id": resolution.candidate_id,
            "evaluation_id": resolution.evaluation_id,
        }),
    )?;
    Ok(ResolutionMaterial {
        binding_version: resolution.binding_version.clone(),
        resolution,
        confirmation,
    })
}

/// Reopen an unknown whose resolution is no longer current.
///
/// The solver detail is kept for traceability but qualified with the cause:
/// the state must not claim that nothing happened while the evidence is stale.
fn invalidate_entry(entry: &mut UnknownStateEntry, cause: &str) {
    entry.status = STATUS_OPEN.to_string();
    entry.reason = Some(REASON_EVIDENCE_STALE.to_string());
    entry.resolution_id = None;
    let previous = entry.detail.clone().unwrap_or_default();
    entry.detail = Some(format!(
        "{previous} (invalidated: {cause}; the previous resolution is not current)"
    ));
}

/// Recompute the state of the unknowns for a new revision.
///
/// The state of the base revision is carried forward. A resolution is carried
/// forward only when all of its dependencies still hold: the bound
/// components, the binding and the claim. When a dependency changed the
/// unknown becomes `open` with the reason `evidence_stale`; the historical
/// documents stay in the history but are not current.
pub fn recompute_state(
    store: &SceneStore,
    base_revision: &str,
    new_revision: &str,
    base_scene: &Scene,
    new_components: &[(String, String)],
    resolution: Option<&Resolution>,
    created_at: &str,
) -> Result<UnknownState, EditError> {
    let base = load_unknown_state(store, base_revision)?
        .unwrap_or_else(|| UnknownState::empty(new_revision, created_at));
    let mut state = base.clone();
    state.revision = new_revision.to_string();
    state.created_at = created_at.to_string();

    let latest_bindings = latest_binding_document(store)?;

    for entry in state.unknowns.iter_mut() {
        if entry.status != STATUS_RESOLVED {
            continue;
        }
        let resolution_id = match &entry.resolution_id {
            Some(id) => id.clone(),
            None => {
                invalidate_entry(entry, "the resolution is missing");
                continue;
            }
        };
        let document = list_resolutions(store)?
            .into_iter()
            .find(|item| item.resolution_id == resolution_id);
        let document = match document {
            Some(document) => document,
            None => {
                invalidate_entry(entry, "the evidence of the resolution no longer matches");
                continue;
            }
        };
        let bound = load_binding_document(store, &document.binding_version)
            .ok()
            .flatten()
            .and_then(|binding| binding.binding(&entry.unknown_id).cloned());
        let bound = match bound {
            Some(binding) => binding,
            None => {
                invalidate_entry(entry, "the evidence of the resolution no longer matches");
                continue;
            }
        };
        // The binding must not have changed.
        let current = latest_bindings
            .as_ref()
            .and_then(|document| document.binding(&entry.unknown_id).cloned());
        if current.as_ref() != Some(&bound) {
            invalidate_entry(entry, "the binding changed");
            continue;
        }
        // The bound components must not have changed.
        let mut changed = false;
        for component in &bound.components {
            let before = base_scene
                .component(component)
                .map(|component| component.sha256.clone());
            let after = new_components
                .iter()
                .find(|(name, _)| name == component)
                .map(|(_, digest)| digest.clone());
            if before != after {
                changed = true;
            }
        }
        if changed {
            invalidate_entry(entry, "the bound geometry changed");
        }
    }

    // The unknown that the new resolution closes becomes resolved.
    if let Some(resolution) = resolution {
        for entry in state.unknowns.iter_mut() {
            if entry.unknown_id != resolution.unknown_id {
                continue;
            }
            entry.status = STATUS_RESOLVED.to_string();
            entry.reason = None;
            entry.binding_version = Some(resolution.binding_version.clone());
            entry.resolution_id = Some(resolution.resolution_id.clone());
        }
    }

    state
        .unknowns
        .sort_by(|left, right| left.unknown_id.cmp(&right.unknown_id));
    Ok(state)
}

/// The aggregate digest of a component set (helper of the state machine).
pub fn component_aggregate(components: &[(String, String)]) -> String {
    aggregate_digest(components)
}

/// The digests of the components of a revision, sorted by the name.
pub fn component_digests(scene: &Scene) -> Vec<(String, String)> {
    let mut digests: Vec<(String, String)> = scene
        .components
        .iter()
        .map(|component| (component.name.clone(), component.sha256.clone()))
        .collect();
    digests.sort_by(|left, right| left.0.cmp(&right.0));
    digests
}

/// The map of the components of a revision (used by the view).
pub fn component_map(scene: &Scene) -> BTreeMap<String, String> {
    scene
        .components
        .iter()
        .map(|component| (component.name.clone(), component.sha256.clone()))
        .collect()
}
