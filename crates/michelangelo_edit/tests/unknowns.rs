//! The failure tests of the unknown state machine (IT-8C).
//!
//! The tests prove the invariants of `docs/specs/VERIFICATION_STATE.md`
//! sections 7 and 9 and of `docs/specs/UNKNOWNS.md`: a binding is
//! required, a confirmation is bound to the claim, the geometry and the
//! evaluation, the closure happens only in the bundle of a revision, a
//! partial acceptance keeps the unknown open, a changed dependency
//! invalidates a resolution and a rollback restores the state of the target
//! bundle.

mod common;

use std::path::PathBuf;

use common::{candidate, lid_obj, make_run, register_evaluation, write_candidate, write_file};
use michelangelo_edit::apply::{accept_candidate_with_options, rollback_scene, submit_candidate};
use michelangelo_edit::candidate::Region;
use michelangelo_edit::error::{
    AGENT_CONFIRMATION_REFUSED, BINDING_MISMATCH, BINDING_MISSING, CONFIRMATION_MISSING,
    EVALUATION_UNREGISTERED, UNKNOWN_NOT_FOUND,
};
use michelangelo_edit::gate::{resolve_evaluation_by_id, AcceptOptions};
use michelangelo_edit::scene::{load_scene, SceneStore};
use michelangelo_edit::unknowns::{
    bind_unknown, confirm_unknown, propose_resolution, show_unknowns, sync_unknowns,
};
use michelangelo_edit::FixedClock;
use serde_json::json;

/// The region of the bindings of the tests.
fn region() -> Region {
    Region {
        min: [-0.2, -0.2, -0.2],
        max: [0.2, 0.2, 0.2],
    }
}

/// The report of a run that declares two unknowns.
fn report_with_unknowns() -> String {
    json!({
        "schema": "michelangelo.reconstruction-report/0",
        "run": {
            "run_id": "recon_0001_0001",
            "run_dir": "recon/r0001/run-0001",
            "created_at": "2026-01-01T00:00:00Z"
        },
        "package": {
            "package_id": "demo-package",
            "revision": 1,
            "validation_status": "valid_complete",
            "calibrated_views": ["front"]
        },
        "build": {
            "components": [],
            "supplements": [],
            "unknowns": [
                {"id": "u_flame_shape", "status": "stub", "detail": "a stub box"},
                {"id": "u_glass_thickness", "status": "unresolved",
                 "detail": "the thickness is not observable"}
            ],
            "measurements": []
        }
    })
    .to_string()
}

/// A scratch run with a candidate and two unknowns.
struct Fixture {
    _directory: tempfile::TempDir,
    run_dir: PathBuf,
    candidate_dir: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let run_dir = directory.path().join("run-0001");
        make_run(&run_dir);
        write_file(&run_dir.join("report.json"), &report_with_unknowns());
        michelangelo_edit::scene::init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
            .expect("the scene is created");
        let candidate_dir = directory.path().join("cand-0001");
        write_candidate(
            &candidate_dir,
            &candidate(&json!({})),
            &[("lid.obj", lid_obj(0.02))],
        );
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

    fn submit(&self) {
        submit_candidate(&self.run_dir, &self.candidate_dir, &self.clock())
            .expect("the candidate is submitted");
    }

    /// Register an evaluation of the snapshot of the candidate.
    fn evaluate(&self) -> (PathBuf, String) {
        let path = register_evaluation(&self.run_dir, "cand-0001");
        let text = std::fs::read_to_string(&path).expect("the report is read");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the report is JSON");
        let evaluation_id = value["evaluation_id"]
            .as_str()
            .expect("the report carries an identifier")
            .to_string();
        (path, evaluation_id)
    }

    /// Bind an unknown to the lid.
    fn bind(&self, unknown_id: &str, claim: Option<&str>) {
        bind_unknown(
            &self.store(),
            unknown_id,
            &["lid".to_string()],
            region(),
            "alice",
            claim,
            &self.clock(),
        )
        .expect("the unknown is bound");
    }

    /// Confirm an unknown for the registered evaluation.
    fn confirm(&self, unknown_id: &str, evaluation_id: &str, by: &str) {
        let evaluation = resolve_evaluation_by_id(&self.store(), evaluation_id)
            .expect("the evaluation is known");
        confirm_unknown(
            &self.store(),
            unknown_id,
            by,
            &evaluation,
            None,
            None,
            &self.clock(),
        )
        .expect("the unknown is confirmed");
    }

    /// The state of an unknown of a revision.
    fn status(&self, revision: Option<&str>, unknown_id: &str) -> String {
        let view = show_unknowns(&self.store(), revision).expect("the view is read");
        view.unknowns
            .iter()
            .find(|entry| entry.unknown_id == unknown_id)
            .expect("the unknown is listed")
            .status
            .clone()
    }

    /// The reason of an unknown of a revision.
    fn reason(&self, revision: Option<&str>, unknown_id: &str) -> Option<String> {
        let view = show_unknowns(&self.store(), revision).expect("the view is read");
        view.unknowns
            .iter()
            .find(|entry| entry.unknown_id == unknown_id)
            .expect("the unknown is listed")
            .reason
            .clone()
    }
}

// ---------------------------------------------------------------------------
// C1: the registry and the versions
// ---------------------------------------------------------------------------

#[test]
fn the_sync_imports_the_unknowns_of_the_report() {
    let fixture = Fixture::new();
    let report = sync_unknowns(&fixture.store(), &fixture.clock()).expect("the sync runs");
    assert_eq!(report.imported, 2);
    assert!(report.missing.is_empty());
    let view = show_unknowns(&fixture.store(), None).expect("the view is read");
    assert_eq!(view.unresolved, vec!["u_flame_shape", "u_glass_thickness"]);
    let stub = view
        .unknowns
        .iter()
        .find(|entry| entry.unknown_id == "u_flame_shape")
        .expect("the stub is listed");
    assert_eq!(stub.status, "stub_pending");
    assert!(stub.stub);
    let open = view
        .unknowns
        .iter()
        .find(|entry| entry.unknown_id == "u_glass_thickness")
        .expect("the unknown is listed");
    assert_eq!(open.status, "open");
}

#[test]
fn the_sync_is_idempotent() {
    let fixture = Fixture::new();
    let first = sync_unknowns(&fixture.store(), &fixture.clock()).expect("the sync runs");
    let second = sync_unknowns(&fixture.store(), &fixture.clock()).expect("the sync runs");
    assert_eq!(first.imported, second.imported);
    assert!(second.state_existed);
    assert!(second.missing.is_empty());
}

#[test]
fn a_binding_of_an_unknown_the_run_does_not_know_is_refused() {
    let fixture = Fixture::new();
    let error = bind_unknown(
        &fixture.store(),
        "u_nothing",
        &["lid".to_string()],
        region(),
        "alice",
        None,
        &fixture.clock(),
    )
    .expect_err("the binding is refused");
    assert_eq!(error.code(), UNKNOWN_NOT_FOUND);
}

#[test]
fn the_bindings_are_versioned() {
    let fixture = Fixture::new();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    let document = michelangelo_edit::unknowns::latest_binding_document(&fixture.store())
        .expect("the bindings are read")
        .expect("the bindings exist");
    assert_eq!(document.version, "b0001");
    assert_eq!(document.bindings.len(), 1);
    fixture.bind("u_flame_shape", None);
    let document = michelangelo_edit::unknowns::latest_binding_document(&fixture.store())
        .expect("the bindings are read")
        .expect("the bindings exist");
    assert_eq!(document.version, "b0002");
    assert_eq!(document.bindings.len(), 2);
}

// ---------------------------------------------------------------------------
// C2: the confirmation of the author
// ---------------------------------------------------------------------------

#[test]
fn an_unregistered_evaluation_is_refused() {
    let fixture = Fixture::new();
    fixture.submit();
    let error = resolve_evaluation_by_id(&fixture.store(), "ev-9999")
        .expect_err("the evaluation is unknown");
    assert_eq!(error.code(), EVALUATION_UNREGISTERED);
}

#[test]
fn an_agent_may_not_confirm_its_own_work() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    let evaluation = resolve_evaluation_by_id(&fixture.store(), &evaluation_id)
        .expect("the evaluation is known");
    let error = confirm_unknown(
        &fixture.store(),
        "u_glass_thickness",
        "agent",
        &evaluation,
        None,
        None,
        &fixture.clock(),
    )
    .expect_err("the confirmation is refused");
    assert_eq!(error.code(), AGENT_CONFIRMATION_REFUSED);
}

// ---------------------------------------------------------------------------
// C3: the resolution
// ---------------------------------------------------------------------------

#[test]
fn a_resolution_needs_a_binding() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    let evaluation = resolve_evaluation_by_id(&fixture.store(), &evaluation_id)
        .expect("the evaluation is known");
    let store = fixture.store();
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0001").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    let error = propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect_err("the resolution is refused");
    assert_eq!(error.code(), BINDING_MISSING);
}

#[test]
fn a_resolution_needs_a_confirmation() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, &evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0001").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    let error = propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect_err("the resolution is refused");
    assert_eq!(error.code(), CONFIRMATION_MISSING);
}

#[test]
fn a_binding_without_a_claim_uses_the_claim_of_the_confirmation() {
    let fixture = Fixture::new();
    fixture.submit();
    let (_report_path, evaluation_id) = fixture.evaluate();
    // The binding describes the components/region only; the author confirmation
    // carries the claim. This must still allow the resolution.
    fixture.bind("u_glass_thickness", None);
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, &evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0001").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    let resolution = propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect("the resolution is proposed");
    assert_eq!(resolution.unknown_id, "u_glass_thickness");
    assert!(!resolution.claim_digest.is_empty());
}

#[test]
fn a_proposal_does_not_close_the_unknown() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, &evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0001").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect("the proposal is written");
    // The scene is not modified: the unknown stays open.
    assert_eq!(fixture.status(None, "u_glass_thickness"), "open");
    let pointer = michelangelo_edit::scene::load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
}

#[test]
fn an_unrelated_change_cannot_close_the_unknown() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    // The binding covers the base, the candidate changes the lid.
    bind_unknown(
        &fixture.store(),
        "u_glass_thickness",
        &["base".to_string()],
        region(),
        "alice",
        Some("the glass is 2 mm"),
        &fixture.clock(),
    )
    .expect("the unknown is bound");
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, &evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0001").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    let error = propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect_err("the resolution is refused");
    assert_eq!(error.code(), BINDING_MISMATCH);
}

// ---------------------------------------------------------------------------
// C4: the partial acceptance and the closure in the bundle
// ---------------------------------------------------------------------------

#[test]
fn an_acceptance_without_resolve_keeps_the_unknown_open() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, _) = fixture.evaluate();
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(report_path),
            resolve: None,
        },
    )
    .expect("the candidate is accepted");
    assert_eq!(report.resulting_scene_revision.as_deref(), Some("s0002"));
    assert_eq!(fixture.status(Some("s0002"), "u_glass_thickness"), "open");
    let scene = load_scene(&fixture.store(), "s0002").expect("the scene is read");
    let verification = scene.verification.expect("the scene carries a bundle");
    assert!(verification.resolutions.is_empty());
    assert!(verification.confirmations.is_empty());
}

#[test]
fn an_acceptance_with_resolve_closes_the_unknown_in_the_bundle() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(report_path),
            resolve: Some("u_glass_thickness".to_string()),
        },
    )
    .expect("the candidate is accepted");
    assert_eq!(report.resulting_scene_revision.as_deref(), Some("s0002"));
    assert_eq!(
        fixture.status(Some("s0002"), "u_glass_thickness"),
        "resolved"
    );
    // The other unknown stays open: the closure is per unknown.
    assert_eq!(
        fixture.status(Some("s0002"), "u_flame_shape"),
        "stub_pending"
    );

    // The bundle of the revision references the documents of the closure.
    let scene = load_scene(&fixture.store(), "s0002").expect("the scene is read");
    let verification = scene.verification.expect("the scene carries a bundle");
    assert_eq!(verification.resolutions.len(), 1);
    assert_eq!(verification.confirmations.len(), 1);
    assert_eq!(verification.bindings.len(), 1);
    let resolution = &verification.resolutions[0];
    let path = fixture.run_dir.join(&resolution.path);
    assert!(path.is_file(), "the resolution of the bundle exists");
    assert_eq!(
        common::digest(&path),
        resolution.sha256,
        "the digest of the bundle matches the document"
    );
    let confirmation = &verification.confirmations[0];
    let path = fixture.run_dir.join(&confirmation.path);
    assert!(path.is_file(), "the confirmation of the bundle exists");
    let binding = &verification.bindings[0];
    let path = fixture.run_dir.join(&binding.path);
    assert!(path.is_file(), "the binding of the bundle exists");
}

/// The candidate of a later revision.
fn second_candidate(base_revision: &str, component: &str) -> serde_json::Value {
    candidate(&json!({
        "candidate_id": "cand-0002",
        "base": {"scene_revision": base_revision},
        "task": {"allowed_components": [component], "protected": []},
        "changes": [{"component": component, "action": "replace",
                     "obj": format!("geometry/{component}.obj")}],
    }))
}

impl Fixture {
    /// Write a second candidate that changes one component.
    fn write_second_candidate(&self, base_revision: &str, component: &str, height: f64) -> PathBuf {
        let value = second_candidate(base_revision, component);
        let geometry = if component == "lid" {
            lid_obj(height)
        } else {
            common::box_obj(0.1)
        };
        let directory = self._directory.path().join("cand-0002");
        let name = format!("{component}.obj");
        write_candidate(&directory, &value, &[(name.as_str(), geometry)]);
        directory
    }

    /// Register an evaluation of the second candidate.
    fn evaluate_second(&self) -> (PathBuf, String) {
        let path = register_evaluation(&self.run_dir, "cand-0002");
        let text = std::fs::read_to_string(&path).expect("the report is read");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the report is JSON");
        let evaluation_id = value["evaluation_id"]
            .as_str()
            .expect("the report carries an identifier")
            .to_string();
        (path, evaluation_id)
    }

    /// Accept a candidate with a registered evaluation, optionally
    /// resolving an unknown.
    fn accept_candidate(
        &self,
        candidate_id: &str,
        report_path: PathBuf,
        resolve: Option<&str>,
    ) -> String {
        let report = accept_candidate_with_options(
            &self.run_dir,
            candidate_id,
            "alice",
            &self.clock(),
            AcceptOptions {
                evaluation: Some(report_path),
                resolve: resolve.map(|value| value.to_string()),
            },
        )
        .expect("the candidate is accepted");
        report
            .resulting_scene_revision
            .expect("the acceptance publishes a revision")
    }

    /// Submit a candidate directory and check that it is proposed.
    fn submit_directory(&self, directory: &std::path::Path) {
        let decision = submit_candidate(&self.run_dir, directory, &self.clock())
            .expect("the candidate is submitted");
        assert_eq!(
            decision.decision,
            michelangelo_edit::Decision::Proposed,
            "the candidate is refused: {:?}",
            decision.diagnostics
        );
    }
}

#[test]
fn a_later_revision_that_changes_the_bound_geometry_invalidates_the_resolution() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");
    assert_eq!(
        fixture.status(Some("s0002"), "u_glass_thickness"),
        "resolved"
    );

    // A later revision changes the bound component: the evidence is stale.
    let directory = fixture.write_second_candidate("s0002", "lid", 0.03);
    fixture.submit_directory(&directory);
    let (report_path, _) = fixture.evaluate_second();
    let revision = fixture.accept_candidate("cand-0002", report_path, None);
    assert_eq!(revision, "s0003");
    assert_eq!(fixture.status(Some("s0003"), "u_glass_thickness"), "open");
    assert_eq!(
        fixture.reason(Some("s0003"), "u_glass_thickness"),
        Some("evidence_stale".to_string())
    );
}

#[test]
fn an_untouched_resolution_survives_a_later_revision() {
    let fixture = Fixture::new();
    // The first unknown is bound to the lid, the second one to the base.
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    bind_unknown(
        &fixture.store(),
        "u_flame_shape",
        &["base".to_string()],
        region(),
        "alice",
        None,
        &fixture.clock(),
    )
    .expect("the unknown is bound");
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");

    // The second revision changes the base: the resolution of the lid stays
    // valid and the resolution of the base is closed by the new revision.
    let directory = fixture.write_second_candidate("s0002", "base", 0.1);
    fixture.submit_directory(&directory);
    let (report_path, evaluation_id) = fixture.evaluate_second();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.confirm("u_flame_shape", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0002", report_path, Some("u_flame_shape"));
    assert_eq!(revision, "s0003");
    assert_eq!(
        fixture.status(Some("s0003"), "u_glass_thickness"),
        "resolved"
    );
    assert_eq!(fixture.status(Some("s0003"), "u_flame_shape"), "resolved");
}

#[test]
fn a_rollback_restores_the_state_of_the_target_revision() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");
    assert_eq!(
        fixture.status(Some("s0002"), "u_glass_thickness"),
        "resolved"
    );

    // A later revision changes the bound component.
    let directory = fixture.write_second_candidate("s0002", "lid", 0.03);
    fixture.submit_directory(&directory);
    let (report_path, _) = fixture.evaluate_second();
    let revision = fixture.accept_candidate("cand-0002", report_path, None);
    assert_eq!(revision, "s0003");
    assert_eq!(fixture.status(Some("s0003"), "u_glass_thickness"), "open");

    // The rollback selects the state document of the target revision.
    rollback_scene(&fixture.run_dir, "s0002", "alice", &fixture.clock())
        .expect("the scene is rolled back");
    let pointer = michelangelo_edit::scene::load_current(&fixture.store())
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0002");
    assert_eq!(fixture.status(None, "u_glass_thickness"), "resolved");
}

#[test]
fn a_confirmation_of_another_geometry_cannot_be_reused() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");

    // A second candidate with another geometry and another evaluation.
    let directory = fixture.write_second_candidate("s0001", "lid", 0.03);
    fixture.submit_directory(&directory);
    let (report_path, evaluation_id) = fixture.evaluate_second();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, &evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0002")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0002").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    let error = propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect_err("the resolution is refused");
    assert_eq!(error.code(), CONFIRMATION_MISSING);
}

// ---------------------------------------------------------------------------
// C6: the precedence of the policy of the package
// ---------------------------------------------------------------------------

#[test]
fn a_candidate_cannot_weaken_the_policy_of_the_package() {
    let fixture = Fixture::new();
    // The run carries a package that requires an author acceptance.
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &json!({"uncertainty_policy": {"require_author_acceptance": true}}).to_string(),
    );
    // The workspace allows an automatic acceptance.
    write_file(
        &fixture
            ._directory
            .path()
            .join(".michelangelo/edit_policy.json"),
        &json!({"schema": "michelangelo.edit-policy/0", "auto_accept": true}).to_string(),
    );
    // The candidate claims that no author acceptance is required.
    write_candidate(
        &fixture.candidate_dir,
        &candidate(&json!({"task": {"require_author_acceptance": false}})),
        &[("lid.obj", lid_obj(0.02))],
    );
    let report = submit_candidate(&fixture.run_dir, &fixture.candidate_dir, &fixture.clock())
        .expect("the candidate is submitted");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PACKAGE_POLICY_STRICTER"),
        "the candidate cannot weaken the policy of the package: {:?}",
        report.diagnostics
    );
}

#[test]
fn the_policy_of_the_package_keeps_the_confirmation_mandatory() {
    let fixture = Fixture::new();
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &json!({"uncertainty_policy": {"require_author_acceptance": true}}).to_string(),
    );
    write_candidate(
        &fixture.candidate_dir,
        &candidate(&json!({"task": {"require_author_acceptance": false}})),
        &[("lid.obj", lid_obj(0.02))],
    );
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, &evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, "cand-0001")
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir("cand-0001").join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    let error = propose_resolution(
        &store,
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect_err("the resolution is refused");
    assert_eq!(error.code(), CONFIRMATION_MISSING);
}
