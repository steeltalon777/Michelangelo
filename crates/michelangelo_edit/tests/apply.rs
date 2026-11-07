//! Tests of the applier: submit, accept, reject, rollback and show.

mod common;

use std::path::PathBuf;

use common::{candidate, far_lid_obj, lid_obj, make_run, write_candidate};
use michelangelo_edit::apply::{
    accept_candidate, reject_candidate, rollback_scene, show_scene, submit_candidate,
};
use michelangelo_edit::scene::{load_current, load_index, load_scene, SceneStore};
use michelangelo_edit::{Decision, FixedClock};
use serde_json::json;

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
}

fn lid_geometry() -> Vec<(&'static str, String)> {
    vec![("lid.obj", lid_obj(0.02))]
}

#[test]
fn a_submitted_candidate_is_proposed() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Proposed);
    assert_eq!(report.candidate_id, "cand-0001");
    assert_eq!(report.base_scene_revision, Some("s0001".to_string()));
    assert!(report.resulting_scene_revision.is_none());
}

#[test]
fn a_submitted_candidate_is_stored_with_its_decision() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert!(fixture
        .run_dir
        .join("candidates/cand-0001/candidate.json")
        .is_file());
    assert!(fixture
        .run_dir
        .join("candidates/cand-0001/decision.json")
        .is_file());
    assert!(fixture
        .run_dir
        .join("candidates/cand-0001/geometry/lid.obj")
        .is_file());
}

#[test]
fn the_index_records_the_candidate() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let store = SceneStore::new(&fixture.run_dir);
    let index = load_index(&store).expect("the index is read");
    assert_eq!(index.candidates.len(), 1);
    assert_eq!(index.candidates[0].candidate_id, "cand-0001");
    assert_eq!(index.candidates[0].status, "proposed");
}

#[test]
fn an_accepted_candidate_publishes_a_new_revision() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let report = accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    assert_eq!(report.decision, Decision::Accepted);
    assert_eq!(report.resulting_scene_revision, Some("s0002".to_string()));
    assert!(fixture.run_dir.join("scene/s0002/scene.json").is_file());
    let store = SceneStore::new(&fixture.run_dir);
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0002");
    assert_eq!(pointer.candidate_id, Some("cand-0001".to_string()));
}

#[test]
fn the_untouched_components_stay_byte_identical() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let before = std::fs::read(fixture.run_dir.join("scene/s0001/components/base.obj"))
        .expect("the base component is read");
    let after = std::fs::read(fixture.run_dir.join("scene/s0002/components/base.obj"))
        .expect("the base component is read");
    assert_eq!(before, after);
}

#[test]
fn the_changed_component_is_replaced() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let before = std::fs::read(fixture.run_dir.join("scene/s0001/components/lid.obj"))
        .expect("the lid is read");
    let after = std::fs::read(fixture.run_dir.join("scene/s0002/components/lid.obj"))
        .expect("the lid is read");
    assert_eq!(after, lid_obj(0.02).into_bytes());
    assert_eq!(before, lid_obj(0.02).into_bytes());
}

#[test]
fn the_new_revision_records_its_lineage() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let store = SceneStore::new(&fixture.run_dir);
    let scene = load_scene(&store, "s0002").expect("the scene is read");
    assert_eq!(scene.base_scene_revision, Some("s0001".to_string()));
    assert_eq!(scene.parent_candidate_id, Some("cand-0001".to_string()));
    assert_eq!(scene.components.len(), 2);
}

#[test]
fn a_stale_candidate_is_rejected_after_an_acceptance() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let report = accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is decided");
    assert_eq!(report.decision, Decision::Rejected);
    let codes: Vec<String> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"STALE_BASE".to_string()));
}

#[test]
fn a_protected_tamper_is_rejected() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["lid", "base"], "protected": ["base"]},
            "changes": [{"component": "base", "action": "replace", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Rejected);
    let codes: Vec<String> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"PROTECTED_MODIFIED".to_string()));
}

#[test]
fn a_region_violation_is_rejected() {
    let fixture = Fixture::new(&json!({}), &[("lid.obj", far_lid_obj())]);
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Rejected);
    let codes: Vec<String> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"REGION_VIOLATION".to_string()));
}

#[test]
fn a_rejected_candidate_does_not_change_the_scene() {
    let fixture = Fixture::new(&json!({}), &[("lid.obj", far_lid_obj())]);
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let store = SceneStore::new(&fixture.run_dir);
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_failing_publication_leaves_the_pointer_untouched() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    // The temporary revision directory is blocked: the publication must fail.
    let blocked = fixture.run_dir.join("scene/s0002.tmp");
    std::fs::write(&blocked, b"blocked").expect("the file is written");
    let error = accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect_err("the publication fails");
    assert!(error.to_string().contains("temporary"));
    let store = SceneStore::new(&fixture.run_dir);
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
    assert!(!fixture.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_rejected_candidate_is_recorded() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let report = reject_candidate(
        &fixture.run_dir,
        "cand-0001",
        "bob",
        Some("the lid is wrong"),
        &fixture.clock(),
    )
    .expect("the candidate is rejected");
    assert_eq!(report.decision, Decision::Rejected);
    assert_eq!(report.decided_by, Some("bob".to_string()));
    assert!(report.checks[0].detail.contains("the lid is wrong"));
    let store = SceneStore::new(&fixture.run_dir);
    let index = load_index(&store).expect("the index is read");
    assert_eq!(index.candidates[0].status, "rejected");
}

#[test]
fn a_rollback_points_to_the_previous_revision() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let report = rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    assert_eq!(report.to_scene_revision, "s0001");
    assert_eq!(report.from_scene_revision, Some("s0002".to_string()));
    let store = SceneStore::new(&fixture.run_dir);
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
}

#[test]
fn a_rollback_is_recorded_in_the_index() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    let store = SceneStore::new(&fixture.run_dir);
    let index = load_index(&store).expect("the index is read");
    assert_eq!(index.rollbacks.len(), 1);
    assert_eq!(index.rollbacks[0].to_scene_revision, "s0001");
}

#[test]
fn a_rollback_to_an_unknown_revision_fails() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let error = rollback_scene(&fixture.run_dir, "s0009", "alice", &fixture.clock())
        .expect_err("the rollback fails");
    assert!(error.to_string().contains("s0009"));
}

#[test]
fn the_view_shows_the_revisions_and_the_candidates() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let view = show_scene(&fixture.run_dir, None).expect("the view is built");
    assert_eq!(view.revisions.len(), 1);
    assert_eq!(view.candidates.len(), 1);
    assert_eq!(
        view.current.expect("the pointer exists").scene_revision,
        "s0001"
    );
}

#[test]
fn the_view_shows_a_candidate() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let view = show_scene(&fixture.run_dir, Some("cand-0001")).expect("the view is built");
    let shown = view.candidate.expect("the candidate is shown");
    assert_eq!(shown.candidate_id, "cand-0001");
    assert_eq!(shown.status, "proposed");
    assert_eq!(
        shown.decision.expect("the decision is shown").decision,
        Decision::Proposed
    );
}

#[test]
fn a_policy_allows_an_automatic_acceptance() {
    let fixture = Fixture::new(
        &json!({"task": {"require_author_acceptance": false}}),
        &lid_geometry(),
    );
    common::write_file(
        &fixture.run_dir.join(".michelangelo/edit_policy.json"),
        r#"{"schema": "michelangelo.edit-policy/0", "auto_accept": true}"#,
    );
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Accepted);
    assert_eq!(report.resulting_scene_revision, Some("s0002".to_string()));
}

#[test]
fn a_policy_that_denies_keeps_the_candidate_proposed() {
    let fixture = Fixture::new(
        &json!({"task": {"require_author_acceptance": false}}),
        &lid_geometry(),
    );
    common::write_file(
        &fixture.run_dir.join(".michelangelo/edit_policy.json"),
        r#"{"schema": "michelangelo.edit-policy/0", "auto_accept": false}"#,
    );
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Proposed);
}

#[test]
fn a_broken_policy_is_an_error() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    common::write_file(
        &fixture.run_dir.join(".michelangelo/edit_policy.json"),
        "{ broken",
    );
    let error = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect_err("the submission fails");
    assert!(error.to_string().contains("policy"));
}

#[test]
fn a_candidate_that_is_already_in_the_store_is_refused() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let error = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect_err("the second submission fails");
    assert!(error.to_string().contains("already"));
}

#[test]
fn a_broken_candidate_document_is_an_error() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    common::write_file(
        &fixture.candidate_dir.join("candidate.json"),
        r#"{"schema": "michelangelo.edit-candidate/0", "candidate_id": "cand-0001", "unknown": 1}"#,
    );
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Error);
    let codes: Vec<String> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"INTERNAL_ERROR".to_string()));
}

#[test]
fn a_candidate_with_a_broken_budget_is_rejected() {
    let fixture = Fixture::new(&json!({"budget": {"max_calls": 0}}), &lid_geometry());
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert_eq!(report.decision, Decision::Rejected);
    let codes: Vec<String> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"BUDGET_INVALID".to_string()));
}

#[test]
fn the_store_never_writes_into_the_source_directory() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let before = common::digest(&fixture.run_dir.join("source/base.obj"));
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    assert_eq!(
        before,
        common::digest(&fixture.run_dir.join("source/base.obj"))
    );
}

#[test]
fn the_decision_report_carries_no_absolute_path() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let text = report.to_json().expect("the report is serializable");
    assert!(!text.contains(&fixture.run_dir.display().to_string()));
    assert!(!text.contains("C:\\"));
}

#[test]
fn the_decision_report_is_deterministic() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let first = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    let second = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect_err("the second submission fails");
    assert_eq!(first.decision, Decision::Proposed);
    assert_eq!(
        second.to_string(),
        "the candidate 'cand-0001' is already in the store"
    );
}

#[test]
fn an_added_component_joins_the_new_revision() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["lid", "handle"], "protected": []},
            "changes": [{"component": "handle", "action": "add", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let store = SceneStore::new(&fixture.run_dir);
    let scene = load_scene(&store, "s0002").expect("the scene is read");
    assert_eq!(scene.components.len(), 3);
    assert!(scene.component("handle").is_some());
}

#[test]
fn the_scene_revision_is_never_modified_after_its_publication() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    accept_candidate(&fixture.run_dir, "cand-0001", "alice", &fixture.clock())
        .expect("the candidate is accepted");
    let digest = common::digest(&fixture.run_dir.join("scene/s0001/scene.json"));
    rollback_scene(&fixture.run_dir, "s0001", "alice", &fixture.clock())
        .expect("the store is rolled back");
    assert_eq!(
        digest,
        common::digest(&fixture.run_dir.join("scene/s0001/scene.json"))
    );
}
