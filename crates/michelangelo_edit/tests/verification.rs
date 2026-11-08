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
    accept_candidate, accept_candidate_with_hooks, accept_candidate_with_hooks_and_options,
    accept_candidate_with_options, reject_candidate, rollback_scene, submit_candidate, AcceptHooks,
    DECISION_ACCEPT, DECISION_ACCEPT_IDEMPOTENT, DECISION_REJECT, DECISION_REJECT_IDEMPOTENT,
    DECISION_REJECT_REFUSED, DECISION_SUBMIT,
};
use michelangelo_edit::error::{
    CANDIDATE_ALREADY_ACCEPTED, EVALUATION_STALE, EVIDENCE_MISMATCH, SNAPSHOT_MISMATCH,
    SNAPSHOT_MISSING, STALE_BASE, STORE_BUSY,
};
use michelangelo_edit::gate::AcceptOptions;
use michelangelo_edit::scene::{list_revisions, load_current, load_index, load_scene, SceneStore};
use michelangelo_edit::verification::{aggregate_digest, load_snapshot, StoreLock};
use michelangelo_edit::{Decision, EditError, FixedClock};
use serde_json::{json, Value};

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
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");
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

    // The retry succeeds with the same evaluation and does not publish a
    // second revision: the published revision is reused.
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect("the acceptance succeeds");
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
        // IT-8C: the stub of the solver waits for its resolution
        // (contract §9: the stubs are 'stub_pending'); no unknown is closed.
        assert_ne!(unknown["status"], "resolved", "no unknown is closed");
    }
    let stub = unknowns
        .iter()
        .find(|unknown| unknown["unknown_id"] == "u_flame_shape")
        .expect("the stub is listed");
    assert_eq!(stub["status"], "stub_pending");
    assert_eq!(stub["stub"], true);
    assert_eq!(stub["solver_status"], "stub");
    let open = unknowns
        .iter()
        .find(|unknown| unknown["unknown_id"] == "u_glass_thickness")
        .expect("the unresolved unknown is listed");
    assert_eq!(open["status"], "open");
    assert_eq!(open["resolution_id"], Value::Null);
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

// ---------------------------------------------------------------------------
// IT-8D1: the identity of the snapshot, the bundle and the decisions
// ---------------------------------------------------------------------------

#[test]
fn the_decision_pins_the_identity_of_the_snapshot() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let report = fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    assert_eq!(report.snapshot_id.as_deref(), Some("cand-0001"));
    assert_eq!(
        report.snapshot_manifest_sha256.as_deref(),
        Some(digest(&store.snapshot_manifest_path("cand-0001")).as_str()),
        "the decision pins the bytes of the manifest"
    );
    assert_eq!(
        report.candidate_sha256.as_deref(),
        Some(digest(&store.candidate_path("cand-0001")).as_str()),
        "the decision pins the bytes of the candidate document"
    );
    assert_eq!(
        report.aggregate_sha256.as_deref(),
        Some(manifest.aggregate_sha256.as_str())
    );
    assert!(report.task_sha256.is_some(), "the task is pinned");
    assert!(
        report.base_aggregate_sha256.is_some(),
        "the base revision is pinned"
    );
}

#[test]
fn a_coordinated_tamper_of_a_protected_component_and_the_manifest_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();

    // The protected component of the snapshot is replaced and the manifest is
    // rewritten with it: the snapshot is consistent with itself, but it is
    // not the snapshot the decision pinned any more.
    let snapshot_geometry = store
        .snapshot_dir("cand-0001")
        .join("geometry")
        .join("base.obj");
    write_file(&snapshot_geometry, &common::box_obj(0.2));
    let manifest_path = store.snapshot_manifest_path("cand-0001");
    let mut manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("the manifest is read"),
    )
    .expect("the manifest is JSON");
    let new_digest = digest(&snapshot_geometry);
    for file in manifest["files"]
        .as_array_mut()
        .expect("the files are listed")
    {
        if file["component"] == "base" {
            file["sha256"] = Value::String(new_digest.clone());
        }
    }
    let components: Vec<(String, String)> = manifest["files"]
        .as_array()
        .expect("the files are listed")
        .iter()
        .map(|file| {
            (
                file["component"].as_str().unwrap_or_default().to_string(),
                file["sha256"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    manifest["aggregate_sha256"] = Value::String(aggregate_digest(&components));
    write_file(&manifest_path, &manifest.to_string());

    // A new evaluation does not legitimise the tampered snapshot.
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");
    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert!(
        error.code() == SNAPSHOT_MISMATCH || error.code() == EVIDENCE_MISMATCH,
        "the refusal names the evidence: {}",
        error.code()
    );
    assert_eq!(error.exit_code(), 1);
    assert!(
        !fixture.run_dir.join("scene/s0002").exists(),
        "no revision is published"
    );
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001", "the pointer stays");
}

#[test]
fn a_crash_after_the_pointer_completes_the_same_operation() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");
    let mut after_pointer = || -> Result<(), EditError> {
        Err(EditError::Internal {
            message: "the process died after the pointer was switched".to_string(),
        })
    };
    let hooks = AcceptHooks {
        after_pointer: Some(&mut after_pointer),
        ..Default::default()
    };
    let error = accept_candidate_with_hooks_and_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        hooks,
        AcceptOptions {
            evaluation: Some(evaluation.clone()),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is interrupted");
    assert!(error.to_string().contains("died"));

    let store = fixture.store();
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0002",
        "the pointer is switched before the crash"
    );
    assert_eq!(
        history(&store),
        vec!["0001-submit.json".to_string()],
        "the history is not written before the crash"
    );

    // The retry completes the same operation and publishes no second
    // revision.
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(evaluation.clone()),
            ..Default::default()
        },
    )
    .expect("the acceptance succeeds");
    assert_eq!(report.decision, Decision::Accepted);
    assert_eq!(report.resulting_scene_revision.as_deref(), Some("s0002"));
    let revisions = list_revisions(&store).expect("the revisions are listed");
    assert_eq!(revisions, vec!["s0001".to_string(), "s0002".to_string()]);
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "accepted"
    );

    // After a rollback the retry keeps the idempotency: the historical
    // result is returned and the scene is not switched back.
    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    let again = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect("the acceptance succeeds");
    assert_eq!(again.decision, Decision::Accepted);
    assert_eq!(again.resulting_scene_revision.as_deref(), Some("s0002"));
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0001",
        "the scene is not switched back"
    );
}

/// Accept a candidate and crash after the switch of the pointer.
///
/// The acceptance is committed (the pointer names the revision of the
/// candidate) but its bookkeeping (the history and the index) is missing.
fn owner_review_crash_after_the_pointer(fixture: &Fixture) -> PathBuf {
    fixture.submit();
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");
    let mut after_pointer = || -> Result<(), EditError> {
        Err(EditError::Internal {
            message: "owner-review simulated crash after the commit point".to_string(),
        })
    };
    let result = accept_candidate_with_hooks_and_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptHooks {
            after_pointer: Some(&mut after_pointer),
            ..Default::default()
        },
        AcceptOptions {
            evaluation: Some(evaluation.clone()),
            ..Default::default()
        },
    );
    assert!(result.is_err(), "the acceptance is interrupted");
    let store = fixture.store();
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0002",
        "the pointer is switched before the crash"
    );
    assert_eq!(
        history(&store),
        vec!["0001-submit.json".to_string()],
        "the bookkeeping of the acceptance is missing"
    );
    evaluation
}

#[test]
fn owner_review_crash_then_rollback_before_retry_preserves_rollback() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let evaluation = owner_review_crash_after_the_pointer(&fixture);
    let store = fixture.store();

    // The rollback completes the bookkeeping of the committed acceptance
    // first and then rolls the store back.
    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "accepted",
        "the committed acceptance is recovered before the rollback"
    );
    assert!(
        history(&store)
            .iter()
            .any(|name| name.ends_with("-accept.json")),
        "the history carries the recovered acceptance: {:?}",
        history(&store)
    );

    // The retry returns the historical acceptance and never switches the
    // scene back.
    let result = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    );
    assert!(
        result.is_ok(),
        "the retry returns the historical acceptance: {:?}",
        result.err()
    );
    let current = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        current.scene_revision, "s0001",
        "a historical retry must not undo the explicit rollback"
    );
}

#[test]
fn owner_review_reject_after_committed_crash_cannot_replace_acceptance() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    owner_review_crash_after_the_pointer(&fixture);
    let store = fixture.store();

    // The rejection recovers the bookkeeping of the committed acceptance and
    // refuses itself: the history never carries a reject that contradicts
    // the commit.
    let report = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("late reject"),
        &fixture.clock(),
    )
    .expect("the rejection is refused, not applied");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == CANDIDATE_ALREADY_ACCEPTED),
        "the pointer already committed the acceptance: {:?}",
        report.diagnostics
    );
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "accepted"
    );
    let current = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(current.scene_revision, "s0002");
    assert!(
        !history(&store)
            .iter()
            .any(|name| name.ends_with("-reject.json")),
        "no reject record contradicts the commit: {:?}",
        history(&store)
    );
}

#[test]
fn a_rejection_cannot_overwrite_an_acceptance_that_won_the_race() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");

    // The rejection runs while the acceptance holds the lock of the store:
    // it cannot read a stale status and it cannot write anything.
    let (sender, receiver) = std::sync::mpsc::channel();
    let run_dir = fixture.run_dir.clone();
    let mut after_validation = move || {
        let outcome = reject_candidate(
            &run_dir,
            "cand-0001",
            "bob",
            Some("too late"),
            &FixedClock::new("2026-01-03T04:05:06Z"),
        );
        let _ = sender.send(outcome.map(|report| report.decision));
    };
    let hooks = AcceptHooks {
        after_validation: Some(&mut after_validation),
        ..Default::default()
    };
    let report = accept_candidate_with_hooks_and_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        hooks,
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect("the acceptance succeeds");
    assert_eq!(report.decision, Decision::Accepted);

    let outcome = receiver.recv().expect("the rejection answered");
    assert!(
        outcome.is_err(),
        "the rejection cannot run while the acceptance holds the lock"
    );

    let store = fixture.store();
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "accepted",
        "the acceptance is not overwritten"
    );

    // A rejection that arrives later is refused: the acceptance is never
    // rewritten.
    let refused = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("too late"),
        &fixture.clock(),
    )
    .expect("the rejection is decided");
    let codes: Vec<&str> = refused
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains(&CANDIDATE_ALREADY_ACCEPTED));
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "accepted"
    );
}

#[test]
fn a_retry_with_another_evaluation_does_not_reuse_the_old_bundle() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let first = register_evaluation(&fixture.run_dir, "cand-0001");

    // The acceptance is interrupted after the publication of the revision.
    let mut before_publish = || -> Result<(), EditError> {
        Err(EditError::Internal {
            message: "the process died before the pointer was switched".to_string(),
        })
    };
    let hooks = AcceptHooks {
        before_publish: Some(&mut before_publish),
        ..Default::default()
    };
    let _ = accept_candidate_with_hooks_and_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        hooks,
        AcceptOptions {
            evaluation: Some(first),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is interrupted");

    // The retry with another evaluation never reuses the published bundle.
    let second = register_evaluation(&fixture.run_dir, "cand-0001");
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(second),
            ..Default::default()
        },
    )
    .expect("the acceptance succeeds");
    assert_eq!(
        report.resulting_scene_revision.as_deref(),
        Some("s0003"),
        "the bundle of another evaluation is never reused"
    );
    let store = fixture.store();
    let revisions = list_revisions(&store).expect("the revisions are listed");
    assert_eq!(
        revisions,
        vec![
            "s0001".to_string(),
            "s0002".to_string(),
            "s0003".to_string()
        ]
    );
    let scene = load_scene(&store, "s0003").expect("the scene is read");
    let bundle = scene.verification.expect("the bundle exists");
    assert_eq!(bundle.evaluations[0].evaluation_id, "ev-0002");
}

// ---------------------------------------------------------------------------
// Review E: the recovery of a committed acceptance
// ---------------------------------------------------------------------------

/// Write a second candidate that is built on a revision.
fn owner_review_second_candidate(fixture: &Fixture, base_revision: &str) -> PathBuf {
    let directory = fixture._directory.path().join("cand-0002");
    write_candidate(
        &directory,
        &candidate(&json!({
            "candidate_id": "cand-0002",
            "base": {"scene_revision": base_revision},
            "task": {"allowed_components": ["lid"], "protected": []}
        })),
        &[("lid.obj", lid_obj(0.03))],
    );
    directory
}

#[test]
fn review_e_a_late_reject_recovers_the_committed_acceptance_it_contradicts() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    // The acceptance of the first candidate is committed (the pointer names
    // its revision) but its bookkeeping is missing.
    owner_review_crash_after_the_pointer(&fixture);
    let store = fixture.store();

    // A second candidate is built on the revision of the committed
    // acceptance and accepted: the pointer names the new revision.
    let directory = owner_review_second_candidate(&fixture, "s0002");
    submit_candidate(&fixture.run_dir, &directory, &fixture.clock())
        .expect("the second candidate is submitted");
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0002");
    let accepted = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0002",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect("the second candidate is accepted");
    assert_eq!(accepted.resulting_scene_revision.as_deref(), Some("s0003"));

    // The late rejection of the first candidate recovers the acceptance that
    // was committed before the crash and refuses itself.
    let report = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("late reject"),
        &fixture.clock(),
    )
    .expect("the rejection is refused, not applied");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == CANDIDATE_ALREADY_ACCEPTED),
        "the committed acceptance is never contradicted: {:?}",
        report.diagnostics
    );

    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "accepted",
        "the committed acceptance is recovered"
    );
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .scene_revision
            .as_deref(),
        Some("s0002"),
        "the recovered acceptance names the revision it published"
    );

    // The history carries the recovered acceptance and the refusal, never a
    // plain rejection.
    let names = history(&store);
    assert!(
        names.iter().any(|name| name.ends_with("-accept.json")),
        "the history carries the recovered acceptance: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.ends_with("-reject-refused.json")),
        "the history carries the refusal: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name.ends_with("-reject.json")),
        "no plain rejection contradicts the commit: {names:?}"
    );

    // The scene pointer stays on the revision of the second candidate.
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0003");
    assert_eq!(pointer.candidate_id.as_deref(), Some("cand-0002"));
}

#[test]
fn review_e_the_recovered_acceptance_names_the_recovery_as_its_author() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    owner_review_crash_after_the_pointer(&fixture);
    let store = fixture.store();

    // The rejection completes the bookkeeping of the committed acceptance.
    reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("late reject"),
        &fixture.clock(),
    )
    .expect("the rejection is refused, not applied");

    let names = history(&store);
    let accept = names
        .iter()
        .find(|name| name.ends_with("-accept.json"))
        .expect("the recovered acceptance is in the history");
    let text = history_bytes(&store, accept);
    let value: Value = serde_json::from_slice(&text).expect("the decision is JSON");
    assert_eq!(
        value["decided_by"].as_str(),
        Some("recovery:bob"),
        "the audit trail names the recovery, never the late actor"
    );
    assert!(
        value["checks"]
            .as_array()
            .expect("the decision carries checks")
            .iter()
            .any(|check| check["check"].as_str() == Some("recovery")),
        "the recovered decision explains the recovery: {value}"
    );
}

#[test]
fn review_e_a_revision_that_was_never_published_is_not_recovered() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    // The acceptance dies before the pointer is switched: the revision is
    // written but it was never published.
    let mut before_publish = || -> Result<(), EditError> {
        Err(EditError::Internal {
            message: "the process died before the pointer was switched".to_string(),
        })
    };
    let hooks = AcceptHooks {
        before_publish: Some(&mut before_publish),
        ..Default::default()
    };
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");
    let error = accept_candidate_with_hooks_and_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        hooks,
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is interrupted");
    assert!(error.to_string().contains("died"));
    let store = fixture.store();
    assert!(
        fixture.run_dir.join("scene/s0002/scene.json").is_file(),
        "the revision was written before the crash"
    );
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0001",
        "the revision was never published"
    );

    // The rejection of the candidate is a plain rejection: the revision that
    // was never published is not a committed acceptance.
    let report = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("not good enough"),
        &fixture.clock(),
    )
    .expect("the candidate is rejected");
    assert_eq!(report.decision, Decision::Rejected);
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "rejected",
        "a revision that was never published is not a committed acceptance"
    );
    let names = history(&store);
    assert!(
        !names.iter().any(|name| name.ends_with("-accept.json")),
        "no acceptance is recovered: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.ends_with("-reject.json")),
        "the rejection is recorded: {names:?}"
    );
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001", "the pointer never moved");
}

#[test]
fn review_e_a_rejected_candidate_is_never_recovered_as_accepted() {
    // A revision written by an acceptance that crashed before the pointer was
    // switched is not a committed acceptance. Selecting it with a rollback
    // makes it the current revision, but the candidate stays rejected: the
    // recovery never turns a rejection into a fabricated acceptance.
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let mut before_publish = || -> Result<(), EditError> {
        Err(EditError::Internal {
            message: "the process died before the pointer was switched".to_string(),
        })
    };
    let evaluation = register_evaluation(&fixture.run_dir, "cand-0001");
    accept_candidate_with_hooks_and_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptHooks {
            before_publish: Some(&mut before_publish),
            ..Default::default()
        },
        AcceptOptions {
            evaluation: Some(evaluation),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is interrupted");
    let store = fixture.store();
    reject_candidate(&fixture.run_dir, "cand-0001", "bob", None, &fixture.clock())
        .expect("the candidate is rejected");

    // The rollback selects the revision the crashed acceptance wrote.
    rollback_scene(&fixture.run_dir, "s0002", "bob", &fixture.clock())
        .expect("the store selects the revision");
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0002", "the revision is selected");

    // The next decision operation runs the recovery: the rejected candidate
    // must not be recovered as accepted.
    rollback_scene(&fixture.run_dir, "s0001", "bob", &fixture.clock())
        .expect("the store is rolled back again");
    let index = load_index(&store).expect("the index is read");
    assert_eq!(
        index
            .entry("cand-0001")
            .expect("the candidate is listed")
            .status,
        "rejected",
        "a rejection is never turned into an acceptance"
    );
    let names = history(&store);
    assert!(
        !names.iter().any(|name| name.ends_with("-accept.json")),
        "no acceptance is fabricated: {names:?}"
    );
}

#[test]
fn review_e_the_pins_of_the_package_fail_closed() {
    // A run without a copy of the package pins no view.
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    assert!(
        manifest.view_pins.is_empty(),
        "a run without a copy of the package pins no view"
    );

    // A copy of the package that cannot be parsed is an error: the pins
    // never fall back to the digest the package declares.
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    write_file(&fixture.run_dir.join("package/package.json"), "{not json");
    let error = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect_err("the broken copy of the package is refused");
    assert!(
        error.message().contains("package"),
        "the error names the copy of the package: {}",
        error.message()
    );

    // A mask the package declares and that cannot be read is an error too.
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &json!({
            "schema": "michelangelo.input-package/0",
            "package_id": "demo-package",
            "revision": 1,
            "views": [{
                "view_id": "front",
                "kind": "calibrated",
                "image": {"path": "images/front.png", "sha256": "00", "width_px": 4, "height_px": 4},
                "mask": {"path": "masks/front.png", "sha256": "11"},
                "projection": {
                    "type": "orthographic",
                    "location_m": [0.0, -4.0, 0.0],
                    "look_at_m": [0.0, 0.0, 0.0],
                    "up_world": [0.0, 0.0, 1.0],
                    "ortho_scale_m": 1.0
                }
            }]
        })
        .to_string(),
    );
    let error = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect_err("the missing mask of the package is refused");
    assert_eq!(
        error.code(),
        "IO_ERROR",
        "the mask that cannot be read is an error: {}",
        error.message()
    );
    assert!(
        error.message().contains("cannot be read"),
        "the error names the file that cannot be read: {}",
        error.message()
    );

    // The mask the run really carries is pinned, never the digest the
    // package declares.
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &json!({
            "schema": "michelangelo.input-package/0",
            "package_id": "demo-package",
            "revision": 1,
            "views": [{
                "view_id": "front",
                "kind": "calibrated",
                "image": {"path": "images/front.png", "sha256": "00", "width_px": 4, "height_px": 4},
                "mask": {"path": "masks/front.png", "sha256": "11"},
                "projection": {
                    "type": "orthographic",
                    "location_m": [0.0, -4.0, 0.0],
                    "look_at_m": [0.0, 0.0, 0.0],
                    "up_world": [0.0, 0.0, 1.0],
                    "ortho_scale_m": 1.0
                }
            }]
        })
        .to_string(),
    );
    write_file(&fixture.run_dir.join("package/masks/front.png"), "mask");
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Proposed);
    let store = fixture.store();
    let manifest = load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    assert_eq!(manifest.view_pins.len(), 1);
    assert_eq!(
        manifest.view_pins[0].mask_sha256,
        digest(&fixture.run_dir.join("package/masks/front.png")),
        "the pin carries the digest of the mask the run really has"
    );
}

#[test]
fn a_corrupted_registered_report_is_refused_even_with_a_valid_copy() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    fixture.submit();
    let registered = register_evaluation(&fixture.run_dir, "cand-0001");

    // A valid copy of the report: it carries the digest of the registry.
    let copy = fixture
        .run_dir
        .join("verification/evaluations/copy-of-report.json");
    std::fs::copy(&registered, &copy).expect("the copy is written");

    // The registered report is corrupted.
    std::fs::write(&registered, "{}\n").expect("the report is corrupted");

    let error = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(copy),
            ..Default::default()
        },
    )
    .expect_err("the acceptance is refused");
    assert_eq!(error.code(), EVALUATION_STALE);
    assert!(
        !fixture.run_dir.join("scene/s0002").exists(),
        "no revision is published"
    );
}
