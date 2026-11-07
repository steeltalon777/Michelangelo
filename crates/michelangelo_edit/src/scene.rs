//! The immutable scene revision store of a reconstruction run.
//!
//! The store lives next to the run artifacts:
//!
//! ```text
//! <run_dir>/scene/s0001/scene.json
//! <run_dir>/scene/s0001/components/<name>.obj
//! <run_dir>/candidates/cand-0001/candidate.json
//! <run_dir>/candidates/cand-0001/decision.json
//! <run_dir>/candidates/cand-0001/geometry/<name>.obj
//! <run_dir>/current.json
//! <run_dir>/candidates/index.json
//! ```
//!
//! A published revision is immutable: it is written into `<revision>.tmp`
//! and renamed into place, so a reader never observes a half written
//! revision. Only `current.json` and `candidates/index.json` are mutable,
//! and both are published with a write-to-temp + rename. A failure between
//! the two leaves the previous pointer authoritative.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate::{is_valid_component_name, COMPONENT_NAME_MAX_LEN};
use crate::error::{io_error, EditError};
use crate::obj::{parse_obj_file, ObjError};
use crate::{Clock, SCENE_SCHEMA};

/// The name of the scene directory inside a run directory.
pub const SCENE_DIR_NAME: &str = "scene";
/// The name of the components directory inside a revision.
pub const COMPONENTS_DIR_NAME: &str = "components";
/// The name of the candidates directory inside a run directory.
pub const CANDIDATES_DIR_NAME: &str = "candidates";
/// The name of the geometry directory inside a candidate.
pub const GEOMETRY_DIR_NAME: &str = "geometry";
/// The name of the scene document inside a revision.
pub const SCENE_FILE_NAME: &str = "scene.json";
/// The name of the candidate document inside a candidate.
pub const CANDIDATE_FILE_NAME: &str = "candidate.json";
/// The name of the decision document inside a candidate.
pub const DECISION_FILE_NAME: &str = "decision.json";
/// The name of the authoritative pointer of the store.
pub const CURRENT_FILE_NAME: &str = "current.json";
/// The name of the candidate index of the store.
pub const INDEX_FILE_NAME: &str = "index.json";
/// The name of the source directory of a reconstruction run.
pub const SOURCE_DIR_NAME: &str = "source";
/// The name of the reconstruction report of a run.
pub const REPORT_FILE_NAME: &str = "report.json";

/// One component of a scene revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneComponent {
    /// The component name (the stem of the OBJ file).
    pub name: String,
    /// The identifier of the part the component belongs to (None when the
    /// component is not bound to a part).
    pub part_id: Option<String>,
    /// The OBJ of the component, relative to the revision directory.
    pub obj: String,
    /// The SHA-256 of the bytes of the OBJ.
    pub sha256: String,
    /// The minimum corner of the bounding box.
    pub bbox_min: [f64; 3],
    /// The maximum corner of the bounding box.
    pub bbox_max: [f64; 3],
    /// Whether the component is protected against modification.
    pub protected: bool,
}

/// The package a scene revision was built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneSource {
    /// The identifier of the input package.
    pub package_id: String,
    /// The revision of the input package.
    pub package_revision: u32,
}

/// An immutable scene revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the run the scene belongs to.
    pub run_id: String,
    /// The identifier of the revision (for example `s0002`).
    pub scene_revision: String,
    /// The revision this revision was derived from (None for the first one).
    pub base_scene_revision: Option<String>,
    /// The candidate that produced this revision (None for the first one).
    pub parent_candidate_id: Option<String>,
    /// The creation time of the revision (RFC 3339, UTC).
    pub created_at: String,
    /// The components of the revision, sorted by name.
    pub components: Vec<SceneComponent>,
    /// The package the revision was built from.
    pub source: SceneSource,
    /// Free form notes.
    pub notes: Vec<String>,
}

impl Scene {
    /// The component with the given name.
    pub fn component(&self, name: &str) -> Option<&SceneComponent> {
        self.components
            .iter()
            .find(|component| component.name == name)
    }

    /// The names of the components, in the order of the document.
    pub fn component_names(&self) -> Vec<String> {
        self.components
            .iter()
            .map(|component| component.name.clone())
            .collect()
    }
}

/// The authoritative pointer of a scene store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentPointer {
    /// The revision the pointer refers to.
    pub scene_revision: String,
    /// The candidate that produced the revision (None for the first revision
    /// or after a rollback).
    pub candidate_id: Option<String>,
    /// The time the revision became current (RFC 3339, UTC).
    pub accepted_at: String,
}

/// One entry of the candidate index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateIndexEntry {
    /// The identifier of the candidate.
    pub candidate_id: String,
    /// The status of the candidate (`proposed`, `accepted` or `rejected`).
    pub status: String,
    /// The revision the candidate produced (None when it produced none).
    pub scene_revision: Option<String>,
    /// The time the candidate was submitted (RFC 3339, UTC).
    pub submitted_at: String,
    /// The time the candidate was decided (RFC 3339, UTC).
    pub decided_at: Option<String>,
    /// The author that decided the candidate.
    pub decided_by: Option<String>,
    /// The reason of the decision.
    pub reason: Option<String>,
}

/// One rollback of a scene store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollbackRecord {
    /// The revision that was current before the rollback.
    pub from_scene_revision: Option<String>,
    /// The revision the store was rolled back to.
    pub to_scene_revision: String,
    /// The author that performed the rollback.
    pub by: String,
    /// The time of the rollback (RFC 3339, UTC).
    pub at: String,
}

/// The ordered index of the candidates of a store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateIndex {
    /// The schema identifier.
    pub schema: String,
    /// The candidates, ordered by identifier.
    pub candidates: Vec<CandidateIndexEntry>,
    /// The rollbacks, ordered by time.
    pub rollbacks: Vec<RollbackRecord>,
}

impl CandidateIndex {
    /// An empty index.
    pub fn empty() -> Self {
        CandidateIndex {
            schema: crate::INDEX_SCHEMA.to_string(),
            candidates: Vec::new(),
            rollbacks: Vec::new(),
        }
    }

    /// The entry of a candidate (None when the candidate is unknown).
    pub fn entry(&self, candidate_id: &str) -> Option<&CandidateIndexEntry> {
        self.candidates
            .iter()
            .find(|entry| entry.candidate_id == candidate_id)
    }

    /// Insert or replace the entry of a candidate.
    pub fn upsert(&mut self, entry: CandidateIndexEntry) {
        match self
            .candidates
            .iter_mut()
            .find(|existing| existing.candidate_id == entry.candidate_id)
        {
            Some(existing) => *existing = entry,
            None => {
                self.candidates.push(entry);
                self.candidates
                    .sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
            }
        }
    }
}

/// The store of the scene revisions of a reconstruction run.
#[derive(Debug, Clone)]
pub struct SceneStore {
    /// The directory of the reconstruction run.
    pub run_dir: PathBuf,
}

impl SceneStore {
    /// The store of a run directory.
    pub fn new(run_dir: &Path) -> Self {
        SceneStore {
            run_dir: run_dir.to_path_buf(),
        }
    }

    /// The scene directory of the store.
    pub fn scene_dir(&self) -> PathBuf {
        self.run_dir.join(SCENE_DIR_NAME)
    }

    /// The directory of a revision.
    pub fn revision_dir(&self, scene_revision: &str) -> PathBuf {
        self.scene_dir().join(scene_revision)
    }

    /// The components directory of a revision.
    pub fn components_dir(&self, scene_revision: &str) -> PathBuf {
        self.revision_dir(scene_revision).join(COMPONENTS_DIR_NAME)
    }

    /// The scene document of a revision.
    pub fn scene_path(&self, scene_revision: &str) -> PathBuf {
        self.revision_dir(scene_revision).join(SCENE_FILE_NAME)
    }

    /// The candidates directory of the store.
    pub fn candidates_dir(&self) -> PathBuf {
        self.run_dir.join(CANDIDATES_DIR_NAME)
    }

    /// The directory of a candidate.
    pub fn candidate_dir(&self, candidate_id: &str) -> PathBuf {
        self.candidates_dir().join(candidate_id)
    }

    /// The candidate document of a candidate.
    pub fn candidate_path(&self, candidate_id: &str) -> PathBuf {
        self.candidate_dir(candidate_id).join(CANDIDATE_FILE_NAME)
    }

    /// The decision document of a candidate.
    pub fn decision_path(&self, candidate_id: &str) -> PathBuf {
        self.candidate_dir(candidate_id).join(DECISION_FILE_NAME)
    }

    /// The geometry directory of a candidate.
    pub fn geometry_dir(&self, candidate_id: &str) -> PathBuf {
        self.candidate_dir(candidate_id).join(GEOMETRY_DIR_NAME)
    }

    /// The authoritative pointer of the store.
    pub fn current_path(&self) -> PathBuf {
        self.run_dir.join(CURRENT_FILE_NAME)
    }

    /// The candidate index of the store.
    pub fn index_path(&self) -> PathBuf {
        self.candidates_dir().join(INDEX_FILE_NAME)
    }

    /// The source directory of the reconstruction run.
    pub fn source_dir(&self) -> PathBuf {
        self.run_dir.join(SOURCE_DIR_NAME)
    }

    /// The reconstruction report of the run.
    pub fn report_path(&self) -> PathBuf {
        self.run_dir.join(REPORT_FILE_NAME)
    }
}

/// The name of a scene revision (`sNNNN`).
pub fn revision_name(index: u32) -> String {
    format!("s{index:04}")
}

/// True when the name is a valid scene revision name.
pub fn is_valid_revision_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 5 && bytes[0] == b's' && bytes[1..].iter().all(|byte| byte.is_ascii_digit())
}

/// The revisions of the store, sorted by name.
pub fn list_revisions(store: &SceneStore) -> Result<Vec<String>, EditError> {
    let scene_dir = store.scene_dir();
    if !scene_dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(&scene_dir).map_err(|err| {
        io_error(
            &scene_dir,
            format!("the scene directory cannot be read: {err}"),
        )
    })?;
    let mut revisions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            io_error(
                &scene_dir,
                format!("the scene directory cannot be read: {err}"),
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
            && is_valid_revision_name(&name)
        {
            revisions.push(name);
        }
    }
    revisions.sort();
    Ok(revisions)
}

/// The next free revision name of the store.
pub fn next_revision(store: &SceneStore) -> Result<String, EditError> {
    let revisions = list_revisions(store)?;
    let mut highest = 0_u32;
    for revision in &revisions {
        if let Ok(index) = revision[1..].parse::<u32>() {
            if index > highest {
                highest = index;
            }
        }
    }
    Ok(revision_name(highest + 1))
}

/// Read the authoritative pointer of the store (None when it does not exist).
pub fn load_current(store: &SceneStore) -> Result<Option<CurrentPointer>, EditError> {
    let path = store.current_path();
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the pointer cannot be read: {err}")))?;
    let pointer = serde_json::from_str(&text).map_err(|err| EditError::Scene {
        message: format!("the pointer '{}' is not usable: {err}", path.display()),
    })?;
    Ok(Some(pointer))
}

/// Publish the authoritative pointer of the store (write-to-temp + rename).
pub fn write_current(store: &SceneStore, pointer: &CurrentPointer) -> Result<(), EditError> {
    let text = serde_json::to_string_pretty(pointer).map_err(|err| EditError::Internal {
        message: format!("the pointer cannot be serialized: {err}"),
    })?;
    write_bytes_atomic(
        &store.current_path(),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
}

/// Read the candidate index of the store (an empty index when it is missing).
pub fn load_index(store: &SceneStore) -> Result<CandidateIndex, EditError> {
    let path = store.index_path();
    if !path.is_file() {
        return Ok(CandidateIndex::empty());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the index cannot be read: {err}")))?;
    serde_json::from_str(&text).map_err(|err| EditError::Scene {
        message: format!("the index '{}' is not usable: {err}", path.display()),
    })
}

/// Publish the candidate index of the store (write-to-temp + rename).
pub fn write_index(store: &SceneStore, index: &CandidateIndex) -> Result<(), EditError> {
    let text = serde_json::to_string_pretty(index).map_err(|err| EditError::Internal {
        message: format!("the index cannot be serialized: {err}"),
    })?;
    write_bytes_atomic(
        &store.index_path(),
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
}

/// Read a scene revision.
pub fn load_scene(store: &SceneStore, scene_revision: &str) -> Result<Scene, EditError> {
    let path = store.scene_path(scene_revision);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| io_error(&path, format!("the scene cannot be read: {err}")))?;
    let scene: Scene = serde_json::from_str(&text).map_err(|err| EditError::Scene {
        message: format!("the scene '{}' is not usable: {err}", path.display()),
    })?;
    Ok(scene)
}

/// Write a document atomically (write-to-temp + rename).
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), EditError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&parent)
        .map_err(|err| io_error(&parent, format!("the directory cannot be created: {err}")))?;

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document.json".to_string());
    let temporary = parent.join(format!("{file_name}.tmp"));

    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)
            .map_err(|err| io_error(&temporary, format!("the file cannot be created: {err}")))?;
        file.write_all(bytes)
            .map_err(|err| io_error(&temporary, format!("the file cannot be written: {err}")))?;
        file.sync_all()
            .map_err(|err| io_error(&temporary, format!("the file cannot be flushed: {err}")))?;
    }

    std::fs::rename(&temporary, path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        io_error(path, format!("the file cannot be published: {err}"))
    })?;
    Ok(())
}

/// The bytes of the OBJ of a component of a revision.
pub fn component_bytes(
    store: &SceneStore,
    scene_revision: &str,
    name: &str,
) -> Result<Vec<u8>, EditError> {
    let path = store
        .components_dir(scene_revision)
        .join(format!("{name}.obj"));
    std::fs::read(&path)
        .map_err(|err| io_error(&path, format!("the component cannot be read: {err}")))
}

/// Create the first scene revision of a run from the OBJs of `source/`.
///
/// The operation is idempotent: when the revision `s0001` already exists it
/// is returned unchanged and never overwritten.
pub fn init_scene(run_dir: &Path, clock: &dyn Clock) -> Result<Scene, EditError> {
    init_scene_with_protection(run_dir, &[], clock)
}

/// Create the first scene revision of a run, marking the given components as
/// protected.
///
/// Protection is established at initialization time (it is an author decision,
/// not something a candidate may redefine). An existing `s0001` is returned
/// unchanged: revisions are immutable, so protection cannot be retrofitted.
pub fn init_scene_with_protection(
    run_dir: &Path,
    protected: &[String],
    clock: &dyn Clock,
) -> Result<Scene, EditError> {
    let store = SceneStore::new(run_dir);
    if !run_dir.is_dir() {
        return Err(io_error(run_dir, "the run directory does not exist"));
    }
    let source_dir = store.source_dir();
    if !source_dir.is_dir() {
        return Err(io_error(
            &source_dir,
            "the run directory has no 'source' directory",
        ));
    }

    let first = revision_name(1);
    if store.revision_dir(&first).is_dir() {
        // The scene already exists: never overwrite it.
        let scene = load_scene(&store, &first)?;
        ensure_current(&store, &first, clock)?;
        ensure_index(&store)?;
        return Ok(scene);
    }

    let sourced = read_source_components(&source_dir)?;
    if sourced.is_empty() {
        return Err(io_error(
            &source_dir,
            "the source directory carries no OBJ file",
        ));
    }

    let mut protected_names: Vec<String> = Vec::new();
    for name in protected {
        if !sourced.iter().any(|(component, _)| &component.name == name) {
            return Err(EditError::Schema {
                message: format!("the protected component '{name}' is not part of the run"),
            });
        }
        protected_names.push(name.clone());
    }

    let mut components: Vec<SceneComponent> = Vec::with_capacity(sourced.len());
    let mut payloads: Vec<(String, Vec<u8>)> = Vec::with_capacity(sourced.len());
    for (component, path) in &sourced {
        let bytes = std::fs::read(path)
            .map_err(|err| io_error(path, format!("the component cannot be read: {err}")))?;
        let mut component = component.clone();
        if protected_names.contains(&component.name) {
            component.protected = true;
        }
        payloads.push((component.name.clone(), bytes));
        components.push(component);
    }

    let created_at = clock.now_rfc3339_utc();
    let scene = Scene {
        schema: SCENE_SCHEMA.to_string(),
        run_id: run_id_of(run_dir),
        scene_revision: first.clone(),
        base_scene_revision: None,
        parent_candidate_id: None,
        created_at: created_at.clone(),
        components,
        source: read_source_identity(&store),
        notes: Vec::new(),
    };

    publish_revision(&store, &scene, &payloads)?;
    ensure_current(&store, &first, clock)?;
    ensure_index(&store)?;
    Ok(scene)
}

/// The identifier of a run (the name of its directory).
pub fn run_id_of(run_dir: &Path) -> String {
    run_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "run".to_string())
}

/// The components of the `source/` directory of a run.
fn read_source_components(source_dir: &Path) -> Result<Vec<(SceneComponent, PathBuf)>, EditError> {
    let entries = std::fs::read_dir(source_dir).map_err(|err| {
        io_error(
            source_dir,
            format!("the source directory cannot be read: {err}"),
        )
    })?;
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            io_error(
                source_dir,
                format!("the source directory cannot be read: {err}"),
            )
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_obj = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase() == "obj")
            .unwrap_or(false);
        if is_obj {
            files.push(path);
        }
    }
    files.sort();

    let mut components: BTreeMap<String, (SceneComponent, PathBuf)> = BTreeMap::new();
    for path in &files {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !is_valid_component_name(&name) {
            return Err(EditError::Scene {
                message: format!(
                    "the component name '{name}' of '{}' is not usable (at most {COMPONENT_NAME_MAX_LEN} characters of [A-Za-z0-9_.-], the first one not a dash)",
                    path.display()
                ),
            });
        }
        let file = parse_obj_file(path).map_err(|err: ObjError| EditError::Scene {
            message: format!("the component '{}' is not usable: {err}", path.display()),
        })?;
        let component = SceneComponent {
            name: name.clone(),
            part_id: None,
            obj: format!("{COMPONENTS_DIR_NAME}/{name}.obj"),
            sha256: file.sha256,
            bbox_min: file.mesh.bbox.min,
            bbox_max: file.mesh.bbox.max,
            protected: false,
        };
        components.insert(name.clone(), (component, path.clone()));
    }
    Ok(components.into_values().collect())
}

/// The identity of the package a run was built from.
fn read_source_identity(store: &SceneStore) -> SceneSource {
    let path = store.report_path();
    let fallback = SceneSource {
        package_id: "unknown".to_string(),
        package_revision: 0,
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return fallback,
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return fallback,
    };
    let package = match value.get("package") {
        Some(package) => package,
        None => return fallback,
    };
    let package_id = package
        .get("package_id")
        .and_then(|value| value.as_str())
        .map(|text| text.to_string())
        .unwrap_or_else(|| fallback.package_id.clone());
    let package_revision = package
        .get("revision")
        .and_then(|value| value.as_u64())
        .map(|value| value as u32)
        .unwrap_or(fallback.package_revision);
    SceneSource {
        package_id,
        package_revision,
    }
}

/// Publish a scene revision atomically.
///
/// The revision is written into `<revision>.tmp` and renamed into place; a
/// published revision is never modified and an existing revision is never
/// overwritten.
pub fn publish_revision(
    store: &SceneStore,
    scene: &Scene,
    payloads: &[(String, Vec<u8>)],
) -> Result<(), EditError> {
    let target = store.revision_dir(&scene.scene_revision);
    if target.exists() {
        return Err(io_error(
            &target,
            "the scene revision already exists and is never overwritten",
        ));
    }
    let scene_dir = store.scene_dir();
    std::fs::create_dir_all(&scene_dir).map_err(|err| {
        io_error(
            &scene_dir,
            format!("the scene directory cannot be created: {err}"),
        )
    })?;

    let temporary = store.revision_dir(&format!("{}.tmp", scene.scene_revision));
    if temporary.exists() {
        return Err(io_error(
            &temporary,
            "a temporary revision directory already exists",
        ));
    }
    std::fs::create_dir_all(&temporary).map_err(|err| {
        io_error(
            &temporary,
            format!("the temporary revision directory cannot be created: {err}"),
        )
    })?;

    let result = write_revision_content(store, &temporary, scene, payloads);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&temporary);
        return result;
    }

    std::fs::rename(&temporary, &target).map_err(|err| {
        let _ = std::fs::remove_dir_all(&temporary);
        io_error(
            &target,
            format!("the scene revision cannot be published: {err}"),
        )
    })?;
    Ok(())
}

/// Write the content of a revision into a directory.
fn write_revision_content(
    _store: &SceneStore,
    directory: &Path,
    scene: &Scene,
    payloads: &[(String, Vec<u8>)],
) -> Result<(), EditError> {
    let components_dir = directory.join(COMPONENTS_DIR_NAME);
    std::fs::create_dir_all(&components_dir).map_err(|err| {
        io_error(
            &components_dir,
            format!("the components directory cannot be created: {err}"),
        )
    })?;
    for (name, bytes) in payloads {
        let path = components_dir.join(format!("{name}.obj"));
        std::fs::write(&path, bytes)
            .map_err(|err| io_error(&path, format!("the component cannot be written: {err}")))?;
    }
    let text = serde_json::to_string_pretty(scene).map_err(|err| EditError::Internal {
        message: format!("the scene cannot be serialized: {err}"),
    })?;
    let path = directory.join(SCENE_FILE_NAME);
    write_bytes_atomic(
        &path,
        format!(
            "{text}
"
        )
        .as_bytes(),
    )
}

/// Make sure the store has a pointer (an existing pointer is never changed).
fn ensure_current(
    store: &SceneStore,
    scene_revision: &str,
    clock: &dyn Clock,
) -> Result<(), EditError> {
    if store.current_path().is_file() {
        return Ok(());
    }
    let pointer = CurrentPointer {
        scene_revision: scene_revision.to_string(),
        candidate_id: None,
        accepted_at: clock.now_rfc3339_utc(),
    };
    write_current(store, &pointer)
}

/// Make sure the store has a candidate index.
fn ensure_index(store: &SceneStore) -> Result<(), EditError> {
    if store.index_path().is_file() {
        return Ok(());
    }
    write_index(store, &CandidateIndex::empty())
}
