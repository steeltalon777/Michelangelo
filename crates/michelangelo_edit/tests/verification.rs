//! The failure tests of the verification contract (IT-8A).
//!
//! The tests prove the invariants of `docs/specs/VERIFICATION_STATE.md`:
//! the snapshot of the full post-change result, the exclusive lock of the
//! store, the publication from the snapshot, the append-only history of the
//! decisions and the state of the unknowns of a revision.

mod common;

use std::path::PathBuf;

use common::{
    accept, candidate, digest, lid_obj, make_run, register_evaluation, write_candidate, write_file,
};
use michelangelo_edit::apply::{
    accept_candidate, accept_candidate_with_hooks, reject_candidate, rollback_scene,
    submit_candidate, AcceptHooks, DECISION_ACCEPT, DECISION_ACCEPT_IDEMPOTENT, DECISION_REJECT,
    DECISION_REJECT_IDEMPOTENT, DECISION_REJECT_REFUSED, DECISION_SUBMIT,
};
use michelangelo_edit::error::{
    CANDIDATE_ALREADY_ACCEPTED, EVIDENCE_MISMATCH, SNAPSHOT_MISSING, STALE_BASE, STORE_BUSY,
};
use michelangelo_edit::scene::{list_revisions, load_current, load_index, load_scene, SceneStore};
use michelangelo_edit::verification::{aggregate_digest, load_snapshot, StoreLock};
use michelangelo_edit::{Decision, EditError, FixedClock};
use serde_json::json;

/// A scratch run with a candidate.
struct Fixture {
    _directory: tempfile::TempDir,
    run_dir: PathBuf,
    candidate_dir: PathBuf,
}

impl Fixture {
    fn new(overrides: &serde_json::Value, geometry: &[(&str, String)]) -> Fixture {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let run_dir = directory.path().join("run-0001");
        make_run(&run_dir);
        michelangelo_edit::scene::init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
            .expect("the scene is created");
        let candidate_dir = directory.path().join("cand-0001");
        let value = candidate(overrides);
        write_candidate(&candidate_dir, &value, geometry);
        Fixture {
            _directory: directory,
            run_dir,
            candidate_dir,
        }
    }

    fn clock(&self) -> FixedClock {
        FixedClock::new("2026-01-03T04:05:06Z")
    }

    fn store(&self) -> SceneStore {
        SceneStore::new(&self.run_dir)
    }

    fn submit(&self) -> michelangelo_edit::DecisionReport {
        submit_candidate(&self.run_dir, &self.candidate_dir, &self.clock())
            .expect("the candidate is submitted")
    }

    fn accept(&self) -> michelangelo_edit::DecisionReport {
        accept(&self.run_dir, "cand-0001", "alice", &self.clock())
            .expect("the candidate is accepted")
    }
}

fn lid_geometry() -> Vec<(&'static str, String)> {
    vec![("lid.obj", lid_obj(0.02))]
}

/// The names of the records of the decision history of a candidate.
fn history(store: &SceneStore) -> Vec<String> {
    let directory = store.decisions_dir("cand-0001");
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

/// The bytes of a record of the decision history.
fn history_bytes(store: &SceneStore, name: &str) -> Vec<u8> {
    std::fs::read(store.decisions_dir("cand-0001").join(name)).expect("the record is read")
}

// ---------------------------------------------------------------------------
// A1: the snapshot of the full post-change result
// ---------------------------------------------------------------------------

#[test]
fn the_snapshot_carries_the_full_component_set() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    assert_eq!(manifest.schema, "michelangelo.geometry-snapshot/0");
    assert_eq!(manifest.snapshot_id, "cand-0001");
    assert_eq!(manifest.base.scene_revision, "s0001");
    assert_eq!(
        manifest.files.len(),
        2,
        "the full component set is materialised"
    );
    let names: Vec<&str> = manifest
        .files
        .iter()
        .map(|file| file.component.as_str())
        .collect();
    assert_eq!(names, vec!["base", "lid"]);
    assert!(store
        .snapshot_dir("cand-0001")
        .join("geometry/base.obj")
        .is_file());
    assert!(store
        .snapshot_dir("cand-0001")
        .join("geometry/lid.obj")
        .is_file());
    assert!(store.snapshot_manifest_path("cand-0001").is_file());
}

#[test]
fn the_unchanged_component_of_the_snapshot_is_byte_identical() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let base = manifest.file("base").expect("the base is listed");
    assert_eq!(base.source.as_str(), "base");
    let from_snapshot = std::fs::read(store.snapshot_dir("cand-0001").join("geometry/base.obj"))
        .expect("the snapshot geometry is read");
    let from_base = std::fs::read(fixture.run_dir.join("scene/s0001/components/base.obj"))
        .expect("the base component is read");
    assert_eq!(from_snapshot, from_base);
    assert_eq!(
        base.sha256,
        digest(&fixture.run_dir.join("scene/s0001/components/base.obj"))
    );
}

#[test]
fn the_changed_component_of_the_snapshot_comes_from_the_candidate() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let lid = manifest.file("lid").expect("the lid is listed");
    assert_eq!(lid.source.as_str(), "candidate");
    let from_snapshot = std::fs::read(store.snapshot_dir("cand-0001").join("geometry/lid.obj"))
        .expect("the snapshot geometry is read");
    assert_eq!(from_snapshot, lid_obj(0.02).into_bytes());
}

#[test]
fn the_aggregate_digest_matches_the_canonical_definition() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let components: Vec<(String, String)> = manifest
        .files
        .iter()
        .map(|file| (file.component.clone(), file.sha256.clone()))
        .collect();
    assert_eq!(manifest.aggregate_sha256, aggregate_digest(&components));
    // The canonical definition: sha256 over "<component>\0<sha256>\n".
    let mut expected = String::new();
    for (component, sha256) in &components {
        expected.push_str(component);
        expected.push('\u{0}');
        expected.push_str(sha256);
        expected.push('\n');
    }
    assert_eq!(
        manifest.aggregate_sha256,
        michelangelo_input::sha256::sha256_hex(expected.as_bytes())
    );
}

#[test]
fn the_decision_report_carries_the_snapshot() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let report = fixture.submit();
    assert_eq!(report.snapshot_id.as_deref(), Some("cand-0001"));
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    assert_eq!(
        report.aggregate_sha256.as_deref(),
        Some(manifest.aggregate_sha256.as_str())
    );
}

#[test]
fn an_old_decision_report_still_parses() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let path = fixture.run_dir.join("candidates/cand-0001/decision.json");
    let text = std::fs::read_to_string(&path).expect("the decision is read");
    let value: serde_json::Value = serde_json::from_str(&text).expect("the decision is JSON");
    assert!(value.get("snapshot_id").is_some());
    // A decision of a legacy candidate carries no snapshot.
    let mut legacy = value.clone();
    if let Some(object) = legacy.as_object_mut() {
        object.remove("snapshot_id");
        object.remove("aggregate_sha256");
    }
    let report: michelangelo_edit::DecisionReport =
        serde_json::from_value(legacy).expect("the legacy decision parses");
    assert!(report.snapshot_id.is_none());
}

// ---------------------------------------------------------------------------
// A3: the protection against a substitution
// ---------------------------------------------------------------------------

#[test]
fn a_tampered_candidate_geometry_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let geometry = fixture
        .run_dir
        .join("candidates/cand-0001/geometry/lid.obj");
    write_file(&geometry, &lid_obj(0.03));
    let error = accept(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVIDENCE_MISMATCH);
    assert_eq!(error.exit_code(), 1);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
    let store = fixture.store();
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
}

#[test]
fn a_tampered_snapshot_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let geometry = fixture
        .run_dir
        .join("verification/snapshots/cand-0001/geometry/lid.obj");
    write_file(&geometry, &lid_obj(0.03));
    let error = accept(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVIDENCE_MISMATCH);
    assert_eq!(error.exit_code(), 1);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_refused_acceptance_is_appended_to_the_history() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let geometry = fixture
        .run_dir
        .join("candidates/cand-0001/geometry/lid.obj");
    write_file(&geometry, &lid_obj(0.03));
    let _ = accept(&fixture.run_dir, "cand-0001", "alice", &fixture.clock());
    let store = fixture.store();
    let refused: Vec<String> = history(&store)
        .into_iter()
        .filter(|name| name.contains("accept-refused"))
        .collect();
    assert_eq!(refused.len(), 1, "the refused acceptance is recorded");
    let report = std::fs::read_to_string(
        fixture
            .run_dir
            .join("candidates/cand-0001/decisions")
            .join(&refused[0]),
    )
    .expect("the refusal record is readable");
    assert!(report.contains(EVIDENCE_MISMATCH));
    let mirror =
        std::fs::read_to_string(fixture.run_dir.join("candidates/cand-0001/decision.json"))
            .expect("the mirror is readable");
    assert!(mirror.contains("\"decision\": \"proposed\""));
}

#[test]
fn the_published_geometry_comes_from_the_snapshot() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let geometry = fixture
        .run_dir
        .join("candidates/cand-0001/geometry/lid.obj");
    write_file(&geometry, &lid_obj(0.03));
    // The tampered candidate is refused, the snapshot is untouched.
    let _ = accept(&fixture.run_dir, "cand-0001", "alice", &fixture.clock());
    let snapshot = fixture
        .run_dir
        .join("verification/snapshots/cand-0001/geometry/lid.obj");
    assert_eq!(
        std::fs::read(&snapshot).expect("the snapshot is read"),
        lid_obj(0.02).into_bytes()
    );
}

// ---------------------------------------------------------------------------
// A2: the lock and the base check
// ---------------------------------------------------------------------------

#[test]
fn a_busy_lock_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    let _lock = StoreLock::acquire(&store, "test").expect("the lock is taken");
    let error = accept(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect_err("the acceptance is refused");
    assert_eq!(error.code(), STORE_BUSY);
    assert_eq!(error.exit_code(), 2);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn the_lock_is_released_when_it_is_dropped() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    {
        let _lock = StoreLock::acquire(&store, "test").expect("the lock is taken");
    }
    let report = fixture.accept();
    assert_eq!(report.decision, Decision::Accepted);
}

#[test]
fn an_injected_change_of_the_pointer_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let run_dir = fixture.run_dir.clone();
    let mut after_validation = move || {
        let pointer = json!({
            "scene_revision": "s0002",
            "candidate_id": null,
            "accepted_at": "2026-01-03T04:05:06Z",
        });
        write_file(&run_dir.join("current.json"), &pointer.to_string());
    };
    let hooks = AcceptHooks {
        after_validation: Some(&mut after_validation),
        ..Default::default()
    };
    register_evaluation(&fixture.run_dir, "cand-0001");
    let error = accept_candidate_with_hooks(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        hooks,
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), STALE_BASE);
    assert_eq!(error.exit_code(), 1);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
    let store = fixture.store();
    let index = load_index(&store).expect("the index is read");
    let entry = index.entry("cand-0001").expect("the candidate is listed");
    assert_eq!(entry.status, "proposed", "the candidate is not rejected");
}

#[test]
fn a_crash_before_the_pointer_leaves_the_old_bundle_authoritative() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let mut before_publish = || -> Result<(), EditError> {
        Err(EditError::Internal {
            message: "the process died before the pointer was switched".to_string(),
        })
    };
    let hooks = AcceptHooks {
        before_publish: Some(&mut before_publish),
        ..Default::default()
    };
    register_evaluation(&fixture.run_dir, "cand-0001");
    let error = accept_candidate_with_hooks(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        hooks,
    )
    .expect_err("the acceptance is interrupted");
    assert!(error.to_string().contains("died"));

    let store = fixture.store();
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0001",
        "the old bundle is authoritative"
    );
    assert!(
        fixture.run_dir.join("scene/s0002/scene.json").is_file(),
        "the revision was published before the crash"
    );

    // The retry succeeds and does not publish a second revision.
    let report = fixture.accept();
    assert_eq!(report.decision, Decision::Accepted);
    assert_eq!(report.resulting_scene_revision.as_deref(), Some("s0002"));
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0002");
    let revisions = list_revisions(&store).expect("the revisions are listed");
    assert_eq!(revisions, vec!["s0001".to_string(), "s0002".to_string()]);
}

// ---------------------------------------------------------------------------
// A4: the append-only history and the idempotence
// ---------------------------------------------------------------------------

#[test]
fn a_repeated_acceptance_is_idempotent() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let first = fixture.accept();
    assert_eq!(first.decision, Decision::Accepted);
    assert_eq!(first.resulting_scene_revision.as_deref(), Some("s0002"));
    let store = fixture.store();
    let accepted = history_bytes(&store, "0002-accept.json");

    let second = fixture.accept();
    assert_eq!(second.decision, Decision::Accepted);
    assert_eq!(second.resulting_scene_revision.as_deref(), Some("s0002"));
    assert_eq!(
        history(&store),
        vec![
            "0001-submit.json".to_string(),
            "0002-accept.json".to_string(),
            "0003-accept-idempotent.json".to_string(),
        ]
    );
    assert_eq!(
        accepted,
        history_bytes(&store, "0002-accept.json"),
        "the accepted history is never rewritten"
    );
    let index = load_index(&store).expect("the index is read");
    let entry = index.entry("cand-0001").expect("the candidate is listed");
    assert_eq!(entry.status, "accepted");
    assert_eq!(entry.scene_revision.as_deref(), Some("s0002"));
    let revisions = list_revisions(&store).expect("the revisions are listed");
    assert_eq!(revisions.len(), 2, "no second revision is published");
}

#[test]
fn an_acceptance_after_a_rollback_does_not_switch_the_scene_back() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    fixture.accept();
    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    let report = fixture.accept();
    assert_eq!(report.decision, Decision::Accepted);
    assert_eq!(
        report.resulting_scene_revision.as_deref(),
        Some("s0002"),
        "the historical result is returned"
    );
    let store = fixture.store();
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0001",
        "the scene is not switched back"
    );
    let index = load_index(&store).expect("the index is read");
    let entry = index.entry("cand-0001").expect("the candidate is listed");
    assert_eq!(entry.status, "accepted");
}

#[test]
fn a_rejection_of_an_accepted_candidate_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    fixture.accept();
    let store = fixture.store();
    let accepted = history_bytes(&store, "0002-accept.json");

    let report = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("too late"),
        &fixture.clock(),
    )
    .expect("the rejection is decided");
    assert_eq!(report.decision, Decision::Rejected);
    let codes: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains(&CANDIDATE_ALREADY_ACCEPTED));
    assert_eq!(
        accepted,
        history_bytes(&store, "0002-accept.json"),
        "the accepted history is never rewritten"
    );
    assert!(history(&store).contains(&"0003-reject-refused.json".to_string()));
    let index = load_index(&store).expect("the index is read");
    let entry = index.entry("cand-0001").expect("the candidate is listed");
    assert_eq!(entry.status, "accepted", "the index is not rewritten");
}

#[test]
fn a_repeated_rejection_is_idempotent() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let first = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("the lid is wrong"),
        &fixture.clock(),
    )
    .expect("the candidate is rejected");
    assert_eq!(first.decision, Decision::Rejected);
    let store = fixture.store();
    let rejected = history_bytes(&store, "0002-reject.json");

    let second = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("the lid is wrong"),
        &fixture.clock(),
    )
    .expect("the rejection is decided");
    assert_eq!(second.decision, Decision::Rejected);
    assert_eq!(
        rejected,
        history_bytes(&store, "0002-reject.json"),
        "the rejected history is never rewritten"
    );
    assert!(history(&store).contains(&"0003-reject-idempotent.json".to_string()));
    let index = load_index(&store).expect("the index is read");
    let entry = index.entry("cand-0001").expect("the candidate is listed");
    assert_eq!(entry.status, "rejected");
}

#[test]
fn the_decision_history_carries_the_expected_kinds() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    assert_eq!(history(&store), vec!["0001-submit.json".to_string()]);
    fixture.accept();
    assert_eq!(DECISION_ACCEPT, "accept");
    assert_eq!(DECISION_ACCEPT_IDEMPOTENT, "accept-idempotent");
    assert_eq!(DECISION_REJECT, "reject");
    assert_eq!(DECISION_REJECT_IDEMPOTENT, "reject-idempotent");
    assert_eq!(DECISION_REJECT_REFUSED, "reject-refused");
    assert_eq!(DECISION_SUBMIT, "submit");
}

// ---------------------------------------------------------------------------
// Legacy data
// ---------------------------------------------------------------------------

#[test]
fn a_candidate_without_a_snapshot_cannot_be_accepted() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    // A legacy candidate: it is in the store, but it carries no snapshot.
    let legacy = fixture.run_dir.join("candidates/cand-legacy");
    std::fs::create_dir_all(legacy.join("geometry")).expect("the directory is created");
    let document = candidate(&json!({"candidate_id": "cand-legacy"}));
    write_file(&legacy.join("candidate.json"), &document.to_string());
    write_file(&legacy.join("geometry/lid.obj"), &lid_obj(0.02));

    let error = accept_candidate(&fixture.run_dir, "cand-legacy", "alice", &fixture.clock())
        .expect_err("the acceptance is refused");
    assert_eq!(error.code(), SNAPSHOT_MISSING);
    assert_eq!(error.exit_code(), 1);
    assert!(!fixture.run_dir.join("scene/s0002").exists());
    let store = fixture.store();
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
}

// ---------------------------------------------------------------------------
// A5: the bundle, the unknown state and the rollback
// ---------------------------------------------------------------------------

#[test]
fn the_initialization_seeds_the_unknown_state() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let report = json!({
        "schema": "michelangelo.reconstruction-report/0",
        "package": {"package_id": "demo-package", "revision": 1},
        "build": {
            "unknowns": [
                {"id": "u_flame_shape", "status": "stub", "detail": "a stub box"},
                {"id": "u_glass_thickness", "status": "unresolved", "detail": "no answer"}
            ]
        }
    });
    write_file(&run_dir.join("report.json"), &report.to_string());
    michelangelo_edit::scene::init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
        .expect("the scene is created");

    let path = run_dir.join("verification/unknowns/revisions/s0001.json");
    assert!(path.is_file(), "the unknown state is seeded");
    let text = std::fs::read_to_string(&path).expect("the state is read");
    let value: serde_json::Value = serde_json::from_str(&text).expect("the state is JSON");
    assert_eq!(value["schema"], "michelangelo.unknown-state/0");
    assert_eq!(value["revision"], "s0001");
    let unknowns = value["unknowns"].as_array().expect("the unknowns");
    assert_eq!(unknowns.len(), 2);
    for unknown in unknowns {
        assert_eq!(unknown["status"], "open", "no unknown is closed");
    }
    let stub = unknowns
        .iter()
        .find(|unknown| unknown["unknown_id"] == "u_flame_shape")
        .expect("the stub is listed");
    assert_eq!(stub["stub"], true);
    assert_eq!(stub["solver_status"], "stub");
}

#[test]
fn the_initialization_without_a_report_writes_an_empty_state() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    common::write_file(&run_dir.join("source/base.obj"), &common::box_obj(0.1));
    michelangelo_edit::scene::init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
        .expect("the scene is created");
    let path = run_dir.join("verification/unknowns/revisions/s0001.json");
    assert!(path.is_file());
    let text = std::fs::read_to_string(&path).expect("the state is read");
    let value: serde_json::Value = serde_json::from_str(&text).expect("the state is JSON");
    assert_eq!(value["unknowns"].as_array().expect("the unknowns").len(), 0);
}

#[test]
fn the_scene_carries_the_verification_bundle() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let store = fixture.store();
    let scene = load_scene(&store, "s0001").expect("the scene is read");
    let bundle = scene.verification.expect("the bundle exists");
    assert_eq!(bundle.bindings.len(), 0);
    assert_eq!(bundle.evaluations.len(), 0);
    assert_eq!(bundle.confirmations.len(), 0);
    assert_eq!(bundle.resolutions.len(), 0);
    assert!(!bundle.geometry_digest.is_empty());
    assert!(bundle.tool_versions.contains_key("michelangelo"));
    assert!(!bundle.created_at.is_empty());
    let unknowns = bundle.unknowns.expect("the unknown state is referenced");
    assert_eq!(unknowns.revision, "s0001");
    assert_eq!(unknowns.path, "verification/unknowns/revisions/s0001.json");
    let path = fixture.run_dir.join(&unknowns.path);
    assert!(path.is_file(), "the referenced document exists");
    assert_eq!(
        unknowns.sha256,
        digest(&path),
        "the referenced digest matches the document"
    );
    let components: Vec<(String, String)> = scene
        .components
        .iter()
        .map(|component| (component.name.clone(), component.sha256.clone()))
        .collect();
    assert_eq!(bundle.geometry_digest, aggregate_digest(&components));
}

#[test]
fn every_revision_carries_its_own_unknown_state() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    fixture.accept();
    let store = fixture.store();
    let first = store.unknown_state_path("s0001");
    let second = store.unknown_state_path("s0002");
    assert!(first.is_file());
    assert!(second.is_file());
    let scene = load_scene(&store, "s0002").expect("the scene is read");
    let bundle = scene.verification.expect("the bundle exists");
    let unknowns = bundle.unknowns.expect("the unknown state is referenced");
    assert_eq!(unknowns.revision, "s0002");
    assert_eq!(unknowns.path, "verification/unknowns/revisions/s0002.json");
    assert_eq!(unknowns.sha256, digest(&second));
    // The state of the new revision is a copy of the state of its base.
    let base_text = std::fs::read_to_string(&first).expect("the state is read");
    let new_text = std::fs::read_to_string(&second).expect("the state is read");
    let base_value: serde_json::Value =
        serde_json::from_str(&base_text).expect("the state is JSON");
    let new_value: serde_json::Value = serde_json::from_str(&new_text).expect("the state is JSON");
    assert_eq!(base_value["unknowns"], new_value["unknowns"]);
    assert_eq!(new_value["revision"], "s0002");
}

#[test]
fn a_rollback_selects_the_state_of_the_target_revision() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    fixture.accept();
    let store = fixture.store();
    let second = store.unknown_state_path("s0002");
    write_file(&second, "{\"schema\": \"michelangelo.unknown-state/0\", \"revision\": \"s0002\", \"created_at\": \"2026-01-03T04:05:06Z\", \"unknowns\": []}\n");

    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
    // The authoritative bundle is the one of the target revision.
    let scene = load_scene(&store, "s0001").expect("the scene is read");
    let bundle = scene.verification.expect("the bundle exists");
    let unknowns = bundle.unknowns.expect("the unknown state is referenced");
    assert_eq!(unknowns.revision, "s0001");
    assert_eq!(unknowns.path, "verification/unknowns/revisions/s0001.json");
    assert_eq!(unknowns.sha256, digest(&store.unknown_state_path("s0001")));
    // The state of the target revision is not the state of the latest one.
    let target_text =
        std::fs::read_to_string(store.unknown_state_path("s0001")).expect("the state is read");
    let latest_text = std::fs::read_to_string(&second).expect("the state is read");
    assert_ne!(target_text, latest_text);
}

#[test]
fn the_snapshot_pins_the_copy_of_the_input_package() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    // The run carries a copy of the input package.
    let package = json!({
        "schema": "michelangelo.input-package/0",
        "package_id": "demo-package",
        "revision": 1,
        "uncertainty_policy": {
            "require_author_acceptance": true,
            "stub_policy": "allowed",
            "allowed_completion_rules": ["thickness"]
        }
    });
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &package.to_string(),
    );
    assert!(
        fixture.run_dir.join("package/package.json").is_file(),
        "the package copy exists"
    );
    fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let package_digest = digest(&fixture.run_dir.join("package/package.json"));
    assert_eq!(
        manifest.package_sha256.as_deref(),
        Some(package_digest.as_str())
    );
    assert!(
        manifest.package_policy_sha256.is_some(),
        "the policy of the package is pinned"
    );
    assert_eq!(manifest.bindings_sha256, None, "there are no bindings yet");
    // The first revision was published before the copy of the package
    // existed: its bundle pins no package.
    let scene = load_scene(&store, "s0001").expect("the scene is read");
    let bundle = scene.verification.expect("the bundle exists");
    assert_eq!(bundle.policies.package_sha256, None);
}

#[test]
fn the_lock_file_carries_its_diagnostic() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let store = fixture.store();
    {
        let _lock = StoreLock::acquire(&store, "submit").expect("the lock is taken");
    }
    // The lock file stays behind when the lock is released and carries the
    // diagnostic of the last holder.
    assert!(store.lock_path().is_file());
    let text = std::fs::read_to_string(store.lock_path()).expect("the lock file is read");
    let value: serde_json::Value = serde_json::from_str(&text).expect("the lock file is JSON");
    assert_eq!(value["operation"], "submit");
    assert!(value["pid"].as_u64().unwrap_or_default() > 0);
    assert!(!value["since"].as_str().unwrap_or_default().is_empty());
}

#[test]
fn the_historical_report_is_never_modified() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let before = digest(&fixture.run_dir.join("report.json"));
    fixture.submit();
    fixture.accept();
    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    assert_eq!(before, digest(&fixture.run_dir.join("report.json")));
}
