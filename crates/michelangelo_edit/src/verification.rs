//! The verification store of the edit boundary (IT-8A).
//!
//! The module implements the immutable evidence of the contract
//! `docs/specs/VERIFICATION_STATE.md`:
//!
//! ```text
//! <run>/verification/
//!   snapshots/<candidate_id>/manifest.json  michelangelo.geometry-snapshot/0
//!   snapshots/<candidate_id>/geometry/*.obj # the FULL post-change component set
//!   unknowns/revisions/sNNNN.json           michelangelo.unknown-state/0
//!   unknowns/events.jsonl                   append-only audit trail
//!   run.lock                                OS advisory lock
//! ```
//!
//! The snapshot materialises the full result of a change: the unchanged
//! components are copied byte-identically from the base revision, the changed
//! and the added ones come from the candidate. The manifest pins every file
//! with its digest and carries the aggregate digest of the whole set.
//!
//! The lock is an OS advisory lock (`std::fs::File::try_lock()`): the
//! operating system releases it when the owning process dies, there is no
//! TTL based stealing.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{io_error, EditError};
use crate::scene::SceneStore;

/// The name of the verification directory inside a run directory.
pub const VERIFICATION_DIR_NAME: &str = "verification";
/// The name of the snapshots directory inside the verification directory.
pub const SNAPSHOTS_DIR_NAME: &str = "snapshots";
/// The name of the geometry directory inside a snapshot.
pub const GEOMETRY_DIR_NAME: &str = "geometry";
/// The name of the manifest of a snapshot.
pub const MANIFEST_FILE_NAME: &str = "manifest.json";
/// The name of the unknowns directory inside the verification directory.
pub const UNKNOWNS_DIR_NAME: &str = "unknowns";
/// The name of the revisions directory inside the unknowns directory.
pub const REVISIONS_DIR_NAME: &str = "revisions";
/// The name of the append-only event trail of the unknowns.
pub const EVENTS_FILE_NAME: &str = "events.jsonl";
/// The name of the lock file of the store.
pub const LOCK_FILE_NAME: &str = "run.lock";
/// The schema of a geometry snapshot.
pub const SNAPSHOT_SCHEMA: &str = "michelangelo.geometry-snapshot/0";
/// The schema of an unknown state document.
pub const UNKNOWN_STATE_SCHEMA: &str = "michelangelo.unknown-state/0";

/// The origin of a file of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    /// The file was copied byte-identically from the base revision.
    Base,
    /// The file comes from the candidate.
    Candidate,
}

impl SnapshotSource {
    /// The origin as the text used in the manifest.
    pub fn as_str(&self) -> &'static str {
        match self {
            SnapshotSource::Base => "base",
            SnapshotSource::Candidate => "candidate",
        }
    }
}

/// The base a snapshot was taken from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotBase {
    /// The identifier of the input package.
    pub package_id: String,
    /// The revision of the input package.
    pub package_revision: u32,
    /// The identifier of the run.
    pub run_id: String,
    /// The scene revision the snapshot was taken from.
    pub scene_revision: String,
    /// The aggregate digest of the components of the base revision.
    pub geometry_digest: String,
}

/// One file of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// The name of the component.
    pub component: String,
    /// The path of the file, relative to the snapshot directory.
    pub path: String,
    /// The SHA-256 of the bytes of the file.
    pub sha256: String,
    /// The origin of the file.
    pub source: SnapshotSource,
}

/// The manifest of a geometry snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the snapshot (the identifier of the candidate).
    pub snapshot_id: String,
    /// The creation time of the snapshot (RFC 3339, UTC).
    pub created_at: String,
    /// The base the snapshot was taken from.
    pub base: SnapshotBase,
    /// The SHA-256 of the bytes of the candidate document.
    pub candidate_sha256: Option<String>,
    /// The SHA-256 of the canonical JSON of the task of the candidate.
    pub task_sha256: Option<String>,
    /// The SHA-256 of the bytes of the copy of the input package (None when
    /// the run carries no copy of the package).
    pub package_sha256: Option<String>,
    /// The SHA-256 of the bindings of the candidate (None until the bindings
    /// exist).
    pub bindings_sha256: Option<String>,
    /// The SHA-256 of the canonical JSON of the uncertainty policy of the
    /// package (None when the run carries no copy of the package).
    pub package_policy_sha256: Option<String>,
    /// The files of the snapshot, sorted by the name of the component.
    pub files: Vec<SnapshotFile>,
    /// The aggregate digest of the whole component set.
    pub aggregate_sha256: String,
    /// The versions of the tools that produced the snapshot.
    pub tool_versions: BTreeMap<String, String>,
}

impl SnapshotManifest {
    /// The file of a component (None when the component is not in the set).
    pub fn file(&self, component: &str) -> Option<&SnapshotFile> {
        self.files.iter().find(|file| file.component == component)
    }
}

/// One unknown of an unknown state document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownStateEntry {
    /// The identifier of the unknown.
    pub unknown_id: String,
    /// The state of the unknown (`open` in IT-8A).
    pub status: String,
    /// The status the solver reported (traceability only).
    pub solver_status: String,
    /// Whether the unknown was a stub of the solver.
    pub stub: bool,
    /// The reason of the state (None when there is none).
    pub reason: Option<String>,
    /// The detail the solver reported.
    pub detail: Option<String>,
}

/// The immutable state of the unknowns of a scene revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownState {
    /// The schema identifier.
    pub schema: String,
    /// The revision the state belongs to.
    pub revision: String,
    /// The creation time of the document (RFC 3339, UTC).
    pub created_at: String,
    /// The unknowns of the revision, sorted by the identifier.
    pub unknowns: Vec<UnknownStateEntry>,
}

impl UnknownState {
    /// An empty state document of a revision.
    pub fn empty(revision: &str, created_at: &str) -> Self {
        UnknownState {
            schema: UNKNOWN_STATE_SCHEMA.to_string(),
            revision: revision.to_string(),
            created_at: created_at.to_string(),
            unknowns: Vec::new(),
        }
    }
}

/// The digest of the bytes of a file.
pub fn file_sha256(path: &Path) -> Result<String, EditError> {
    let bytes = std::fs::read(path)
        .map_err(|err| io_error(path, format!("the file cannot be read: {err}")))?;
    Ok(michelangelo_input::sha256::sha256_hex(&bytes))
}

/// The aggregate digest of a component set.
///
/// The digest is the SHA-256 over the lines `"<component>\u{0}<sha256>\n"`,
/// sorted by the name of the component (contract §3).
pub fn aggregate_digest(components: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = components.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let mut buffer: Vec<u8> = Vec::new();
    for (component, sha256) in sorted {
        buffer.extend_from_slice(component.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(sha256.as_bytes());
        buffer.push(b'\n');
    }
    michelangelo_input::sha256::sha256_hex(&buffer)
}

/// The canonical JSON of a value: the object keys are sorted, the text is
/// compact and the numbers keep the representation of the value.
pub fn canonical_json(value: &serde_json::Value) -> String {
    let mut text = String::new();
    write_canonical(value, &mut text);
    text
}

/// Append the canonical JSON of a value to a buffer.
fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (position, key) in keys.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(key));
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_else(|_| "null".to_string())),
    }
}

/// The JSON representation of a string.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"".to_string())
}

/// The versions of the tools that work on a run.
pub fn tool_versions(store: &SceneStore) -> BTreeMap<String, String> {
    let mut versions: BTreeMap<String, String> = BTreeMap::new();
    versions.insert(
        "michelangelo".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    if let Ok(text) = std::fs::read_to_string(store.report_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(solver) = value.get("solver") {
                let name = solver
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let version = solver
                    .get("version")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                versions.insert("solver".to_string(), format!("{name}/{version}"));
            }
        }
    }
    versions
}

/// The digest of the copy of the input package of a run (None when the run
/// carries no copy of the package).
pub fn package_digest(store: &SceneStore) -> Option<String> {
    let path = store.package_copy_path();
    if !path.is_file() {
        return None;
    }
    file_sha256(&path).ok()
}

/// The digest of the uncertainty policy of the copy of the input package
/// (None when the run carries no copy of the package or no policy).
pub fn package_policy_digest(store: &SceneStore) -> Option<String> {
    let path = store.package_copy_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let policy = value.get("uncertainty_policy")?;
    Some(michelangelo_input::sha256::sha256_hex(
        canonical_json(policy).as_bytes(),
    ))
}

/// The digest of the workspace policy that applies to a run (None when the
/// workspace carries no policy).
pub fn workspace_policy_digest(store: &SceneStore) -> Option<String> {
    let path = crate::policy::find_policy_file(&store.run_dir)?;
    file_sha256(&path).ok()
}

/// The exclusive advisory lock of a store.
///
/// The lock is taken with `std::fs::File::try_lock()`: the operating system
/// releases it when the owning process dies. A busy lock is an error
/// (`STORE_BUSY`), the boundary never waits.
pub struct StoreLock {
    file: File,
    path: PathBuf,
}

impl StoreLock {
    /// Take the lock of a store for an operation.
    pub fn acquire(store: &SceneStore, operation: &str) -> Result<StoreLock, EditError> {
        let path = store.lock_path();
        let parent = store.verification_dir();
        std::fs::create_dir_all(&parent).map_err(|err| {
            io_error(
                &parent,
                format!("the verification directory cannot be created: {err}"),
            )
        })?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|err| io_error(&path, format!("the lock file cannot be opened: {err}")))?;
        if let Err(error) = file.try_lock() {
            return Err(EditError::StoreBusy {
                message: format!(
                    "the store of the run is busy: the lock '{}' is held by another process ({error})",
                    path.display()
                ),
            });
        }
        let mut lock = StoreLock { file, path };
        lock.write_diagnostic(operation)?;
        Ok(lock)
    }

    /// The path of the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the diagnostic content of the lock file.
    fn write_diagnostic(&mut self, operation: &str) -> Result<(), EditError> {
        let diagnostic = serde_json::json!({
            "pid": std::process::id(),
            "operation": operation,
            "since": michelangelo_recon::report::now_rfc3339_utc(),
        });
        let text =
            serde_json::to_string_pretty(&diagnostic).map_err(|err| EditError::Internal {
                message: format!("the lock diagnostic cannot be serialized: {err}"),
            })?;
        self.file.set_len(0).map_err(|err| {
            io_error(
                &self.path,
                format!("the lock file cannot be truncated: {err}"),
            )
        })?;
        self.file.seek(SeekFrom::Start(0)).map_err(|err| {
            io_error(
                &self.path,
                format!("the lock file cannot be rewound: {err}"),
            )
        })?;
        self.file
            .write_all(format!("{text}\n").as_bytes())
            .map_err(|err| {
                io_error(
                    &self.path,
                    format!("the lock file cannot be written: {err}"),
                )
            })?;
        Ok(())
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// The manifest of the snapshot of a candidate (None when the candidate has
/// no snapshot).
pub fn load_snapshot(
    store: &SceneStore,
    candidate_id: &str,
) -> Result<Option<SnapshotManifest>, EditError> {
    let path = store.snapshot_manifest_path(candidate_id);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the snapshot cannot be read: {err}")))?;
    let manifest: SnapshotManifest =
        serde_json::from_str(&text).map_err(|err| EditError::Scene {
            message: format!("the snapshot '{}' is not usable: {err}", path.display()),
        })?;
    Ok(Some(manifest))
}

/// The path of a file of a snapshot.
pub fn snapshot_file_path(store: &SceneStore, candidate_id: &str, relative: &str) -> PathBuf {
    store.snapshot_dir(candidate_id).join(relative)
}

/// Verify a snapshot against its own manifest.
///
/// Every file of the manifest must exist, its bytes must match the digest of
/// the manifest and the aggregate digest of the manifest must match the
/// digests of the files. A mismatch is an `EVIDENCE_MISMATCH`.
pub fn verify_snapshot(store: &SceneStore, manifest: &SnapshotManifest) -> Result<(), EditError> {
    let mut components: Vec<(String, String)> = Vec::new();
    for file in &manifest.files {
        let path = snapshot_file_path(store, &manifest.snapshot_id, &file.path);
        if !path.is_file() {
            return Err(EditError::EvidenceMismatch {
                message: format!(
                    "the snapshot '{}' is incomplete: the file '{}' is missing",
                    manifest.snapshot_id, file.path
                ),
            });
        }
        let digest = file_sha256(&path)?;
        if digest != file.sha256 {
            return Err(EditError::EvidenceMismatch {
                message: format!(
                    "the file '{}' of the snapshot '{}' carries the digest {digest} but the manifest pins {}",
                    file.path, manifest.snapshot_id, file.sha256
                ),
            });
        }
        components.push((file.component.clone(), digest));
    }
    let aggregate = aggregate_digest(&components);
    if aggregate != manifest.aggregate_sha256 {
        return Err(EditError::EvidenceMismatch {
            message: format!(
                "the aggregate digest of the snapshot '{}' is {aggregate} but the manifest pins {}",
                manifest.snapshot_id, manifest.aggregate_sha256
            ),
        });
    }
    Ok(())
}

/// The path of an unknown state document, relative to the run directory.
pub fn unknown_state_relative_path(revision: &str) -> String {
    format!("{VERIFICATION_DIR_NAME}/{UNKNOWNS_DIR_NAME}/{REVISIONS_DIR_NAME}/{revision}.json")
}

/// The state document of the unknowns of a revision (None when the revision
/// carries no state document).
pub fn load_unknown_state(
    store: &SceneStore,
    revision: &str,
) -> Result<Option<UnknownState>, EditError> {
    let path = store.unknown_state_path(revision);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the unknown state cannot be read: {err}")))?;
    let state: UnknownState = serde_json::from_str(&text).map_err(|err| EditError::Scene {
        message: format!(
            "the unknown state '{}' is not usable: {err}",
            path.display()
        ),
    })?;
    Ok(Some(state))
}

/// Write an immutable unknown state document.
///
/// An existing document is never overwritten: the state of a revision is
/// immutable.
pub fn write_unknown_state(store: &SceneStore, state: &UnknownState) -> Result<(), EditError> {
    let path = store.unknown_state_path(&state.revision);
    if path.is_file() {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(state).map_err(|err| EditError::Internal {
        message: format!("the unknown state cannot be serialized: {err}"),
    })?;
    write_bytes_new(&path, format!("{text}\n").as_bytes())
}

/// Write a file that must not exist yet.
pub fn write_bytes_new(path: &Path, bytes: &[u8]) -> Result<(), EditError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&parent)
        .map_err(|err| io_error(&parent, format!("the directory cannot be created: {err}")))?;
    let mut file = std::fs::File::create_new(path)
        .map_err(|err| io_error(path, format!("the file cannot be created: {err}")))?;
    file.write_all(bytes)
        .map_err(|err| io_error(path, format!("the file cannot be written: {err}")))?;
    file.sync_all()
        .map_err(|err| io_error(path, format!("the file cannot be flushed: {err}")))?;
    Ok(())
}

/// Seed the unknown state of a revision from the immutable report of a run.
///
/// Every unknown of the report becomes `open`: the solver reported a stub
/// or an unresolved item, and only an explicit binding and evidence may
/// close an unknown (the resolution arrives with IT-8C). The status of the
/// solver is kept for the traceability.
pub fn seed_unknown_state(store: &SceneStore, revision: &str, created_at: &str) -> UnknownState {
    let mut state = UnknownState::empty(revision, created_at);
    let text = match std::fs::read_to_string(store.report_path()) {
        Ok(text) => text,
        Err(_) => return state,
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return state,
    };
    let unknowns = match value
        .get("build")
        .and_then(|build| build.get("unknowns"))
        .and_then(|unknowns| unknowns.as_array())
    {
        Some(unknowns) => unknowns,
        None => return state,
    };
    for unknown in unknowns {
        let unknown_id = unknown
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string();
        let solver_status = unknown
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string();
        let detail = unknown
            .get("detail")
            .and_then(|value| value.as_str())
            .map(|text| text.to_string());
        state.unknowns.push(UnknownStateEntry {
            unknown_id,
            status: "open".to_string(),
            solver_status: solver_status.clone(),
            stub: solver_status == "stub",
            reason: None,
            detail,
        });
    }
    state
        .unknowns
        .sort_by(|left, right| left.unknown_id.cmp(&right.unknown_id));
    state
}

/// Append an event to the append-only audit trail of the unknowns.
pub fn append_event(store: &SceneStore, event: &serde_json::Value) -> Result<(), EditError> {
    let path = store.events_path();
    let parent = store.unknowns_dir();
    std::fs::create_dir_all(&parent).map_err(|err| {
        io_error(
            &parent,
            format!("the unknowns directory cannot be created: {err}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| io_error(&path, format!("the event trail cannot be opened: {err}")))?;
    let text = serde_json::to_string(event).map_err(|err| EditError::Internal {
        message: format!("the event cannot be serialized: {err}"),
    })?;
    file.write_all(format!("{text}\n").as_bytes())
        .map_err(|err| io_error(&path, format!("the event cannot be written: {err}")))?;
    Ok(())
}
