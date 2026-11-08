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
    EVALUATION_UNREGISTERED, UNKNOWN_NOT_FOUND, UNKNOWN_POLICY_VIOLATION,
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

    // The bundle of the later revision keeps the references of the
    // resolution that survived: the resolution, the confirmation and the
    // binding it used.
    let scene = load_scene(&fixture.store(), "s0003").expect("the scene is read");
    let bundle = scene.verification.expect("the scene carries a bundle");
    assert_eq!(
        bundle.resolutions.len(),
        2,
        "both resolutions are referenced: {:?}",
        bundle.resolutions
    );
    assert_eq!(
        bundle.confirmations.len(),
        2,
        "both confirmations are referenced: {:?}",
        bundle.confirmations
    );
    assert!(
        !bundle.bindings.is_empty(),
        "the bundle references the bindings of the resolutions"
    );
    assert_bundle_documents(&fixture, &bundle);
}

#[test]
fn a_later_revision_keeps_the_evidence_of_a_surviving_resolution() {
    let fixture = Fixture::new();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");

    // The bundle of the first revision references the evidence of the
    // closure: the resolution, the confirmation and the binding.
    let scene = load_scene(&fixture.store(), "s0002").expect("the scene is read");
    let bundle = scene.verification.expect("the scene carries a bundle");
    assert_eq!(bundle.resolutions.len(), 1);
    assert_eq!(bundle.confirmations.len(), 1);
    assert_eq!(bundle.bindings.len(), 1);
    let resolution_id = bundle.resolutions[0].resolution_id.clone();
    let confirmation_id = bundle.confirmations[0].confirmation_id.clone();
    let binding_version = bundle.bindings[0].version.clone();

    // A second binding of another unknown writes a new version of the
    // binding document: the resolution of the first unknown stays valid.
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

    // A later revision changes another component: the resolution of the lid
    // survives and the new bundle keeps its references.
    let directory = fixture.write_second_candidate("s0002", "base", 0.1);
    fixture.submit_directory(&directory);
    let (report_path, _) = fixture.evaluate_second();
    let revision = fixture.accept_candidate("cand-0002", report_path, None);
    assert_eq!(revision, "s0003");
    assert_eq!(
        fixture.status(Some("s0003"), "u_glass_thickness"),
        "resolved"
    );

    let scene = load_scene(&fixture.store(), "s0003").expect("the scene is read");
    let bundle = scene.verification.expect("the scene carries a bundle");
    assert!(
        bundle
            .resolutions
            .iter()
            .any(|reference| reference.resolution_id == resolution_id),
        "the bundle of the later revision references the surviving resolution"
    );
    assert!(
        bundle
            .confirmations
            .iter()
            .any(|reference| reference.confirmation_id == confirmation_id),
        "the bundle of the later revision references the confirmation"
    );
    assert!(
        bundle
            .bindings
            .iter()
            .any(|reference| reference.version == binding_version),
        "the bundle of the later revision references the binding of the resolution"
    );
    // Every distinct binding version the resolutions refer to is referenced.
    assert!(
        bundle.bindings.len() >= 2,
        "the bundle references every binding version: {:?}",
        bundle.bindings
    );
    assert_bundle_documents(&fixture, &bundle);
}

#[test]
fn the_view_of_a_revision_reads_the_binding_of_its_bundle() {
    let fixture = Fixture::new();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    assert!(report_path.is_file(), "the report of the evaluation exists");
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");

    // The author rebinds the unknown with another claim and another scope.
    let other_region = Region {
        min: [-0.1, -0.1, -0.1],
        max: [0.1, 0.1, 0.1],
    };
    bind_unknown(
        &fixture.store(),
        "u_glass_thickness",
        &["lid".to_string()],
        other_region,
        "bob",
        Some("the glass is 3 mm"),
        &fixture.clock(),
    )
    .expect("the unknown is rebound");

    // A later revision is published with the new binding.
    let directory = fixture.write_second_candidate("s0002", "base", 0.1);
    fixture.submit_directory(&directory);
    let (report_path, _) = fixture.evaluate_second();
    let revision = fixture.accept_candidate("cand-0002", report_path, None);
    assert_eq!(revision, "s0003");

    // The historical revision shows the binding of its own bundle: the old
    // claim and the old scope.
    let entry = binding_of(&fixture, Some("s0002"), "u_glass_thickness");
    assert_eq!(
        entry.0.and_then(|binding| binding.claim.clone()),
        Some("the glass is 2 mm".to_string()),
        "the historical revision shows the claim of its own binding"
    );
    assert_eq!(entry.1, Some(region()), "the historical scope is shown");
    assert_eq!(entry.2.as_deref(), Some("b0001"));

    // The current revision shows the binding of its own bundle.
    let entry = binding_of(&fixture, None, "u_glass_thickness");
    assert_eq!(
        entry.0.and_then(|binding| binding.claim.clone()),
        Some("the glass is 3 mm".to_string()),
        "the current revision shows the claim of its own binding"
    );
    assert_eq!(entry.1, Some(other_region), "the current scope is shown");

    // The rollback selects the historical bundle again.
    rollback_scene(&fixture.run_dir, "s0002", "alice", &fixture.clock())
        .expect("the scene is rolled back");
    let entry = binding_of(&fixture, None, "u_glass_thickness");
    assert_eq!(
        entry.0.and_then(|binding| binding.claim.clone()),
        Some("the glass is 2 mm".to_string()),
        "the rolled back scene shows the claim of the historical binding"
    );
    assert_eq!(entry.1, Some(region()), "the rolled back scope is shown");
}

/// Check that every document the bundle of a revision refers to exists and
/// carries the digest the bundle pins.
fn assert_bundle_documents(
    fixture: &Fixture,
    bundle: &michelangelo_edit::scene::SceneVerification,
) {
    let mut references: Vec<(String, String)> = Vec::new();
    for reference in &bundle.bindings {
        references.push((reference.path.clone(), reference.sha256.clone()));
    }
    for reference in &bundle.confirmations {
        references.push((reference.path.clone(), reference.sha256.clone()));
    }
    for reference in &bundle.resolutions {
        references.push((reference.path.clone(), reference.sha256.clone()));
    }
    for (path, sha256) in references {
        let file = fixture.run_dir.join(&path);
        assert!(file.is_file(), "the document '{path}' exists");
        assert_eq!(
            common::digest(&file),
            sha256,
            "the digest of '{path}' matches the bundle"
        );
    }
}

/// The binding of an unknown in the view of a revision: the binding, its
/// region and its version.
fn binding_of(
    fixture: &Fixture,
    revision: Option<&str>,
    unknown_id: &str,
) -> (
    Option<michelangelo_edit::unknowns::UnknownBinding>,
    Option<Region>,
    Option<String>,
) {
    let view = show_unknowns(&fixture.store(), revision).expect("the view is read");
    let entry = view
        .unknowns
        .iter()
        .find(|entry| entry.unknown_id == unknown_id)
        .expect("the unknown is listed");
    (
        entry.binding.clone(),
        entry.binding.as_ref().map(|binding| binding.region),
        entry.binding_version.clone(),
    )
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

/// The report of a run whose supplement closed an unknown.
fn report_with_a_resolved_unknown() -> String {
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
            "supplements": [
                {"supplement_id": "sup-0001", "status": "applied", "rule": "thickness"}
            ],
            "unknowns": [
                {"id": "u_glass_thickness", "status": "resolved",
                 "detail": "the supplement closed the unknown"}
            ],
            "measurements": []
        }
    })
    .to_string()
}

/// The copy of a package whose policy does not allow the rule of the
/// supplement that closed the unknown.
///
/// The parameter tells whether the package carries an accepted author
/// decision for the supplement: an accepted decision has the highest
/// precedence and satisfies the restriction of the rules.
fn package_with_a_forbidden_rule(accepted: bool) -> String {
    let mut acceptance: Vec<serde_json::Value> = Vec::new();
    if accepted {
        acceptance.push(json!({
            "target": "supplements:sup-0001",
            "status": "accepted",
            "by": "owner",
            "at": "2026-01-01T00:00:00Z"
        }));
    }
    json!({
        "uncertainty_policy": {
            "require_author_acceptance": true,
            "allowed_completion_rules": ["cavity"],
            "stub_policy": "allowed"
        },
        "unknowns": [
            {"id": "u_glass_thickness", "description": "the thickness is unknown",
             "affected_parts": ["lid-part"], "policy": "rule_allowed"}
        ],
        "supplements": [
            {"supplement_id": "sup-0001", "part_id": "lid-part", "kind": "thickness",
             "geometry": {"value_m": 0.02},
             "provenance": {"origin": "ai_author", "detail": "the solver"}}
        ],
        "acceptance": acceptance
    })
    .to_string()
}

/// The candidate, the snapshot and the evaluation of a resolution.
fn resolution_inputs(
    fixture: &Fixture,
    candidate_id: &str,
    evaluation_id: &str,
) -> (
    michelangelo_edit::candidate::EditCandidate,
    michelangelo_edit::verification::SnapshotManifest,
    michelangelo_edit::gate::EvaluationBinding,
) {
    let store = fixture.store();
    let evaluation =
        resolve_evaluation_by_id(&store, evaluation_id).expect("the evaluation is known");
    let manifest = michelangelo_edit::verification::load_snapshot(&store, candidate_id)
        .expect("the snapshot is read")
        .expect("the snapshot exists");
    let document: michelangelo_edit::candidate::EditCandidate = serde_json::from_str(
        &std::fs::read_to_string(store.candidate_dir(candidate_id).join("candidate.json"))
            .expect("the candidate is read"),
    )
    .expect("the candidate parses");
    (document, manifest, evaluation)
}

#[test]
fn an_accepted_author_decision_satisfies_the_completion_rule_of_the_package() {
    let fixture = Fixture::new();
    write_file(
        &fixture.run_dir.join("report.json"),
        &report_with_a_resolved_unknown(),
    );
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &package_with_a_forbidden_rule(true),
    );
    fixture.submit();
    let (_report_path, evaluation_id) = fixture.evaluate();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let (document, manifest, evaluation) = resolution_inputs(&fixture, "cand-0001", &evaluation_id);
    let resolution = propose_resolution(
        &fixture.store(),
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect("the accepted decision of the author satisfies the rule");
    let check = resolution
        .checks
        .iter()
        .find(|check| check.id == "package_policy")
        .expect("the policy of the package is checked");
    assert!(
        check.detail.contains("thickness"),
        "the check names the rule the decision overrides: {}",
        check.detail
    );
}

// ---------------------------------------------------------------------------
// IT-8E: the carried evidence of a surviving resolution
// ---------------------------------------------------------------------------

/// A store with a resolved unknown: the candidate is bound, confirmed and
/// accepted with the resolution of the unknown.
fn owner_review_resolved_fixture() -> (Fixture, PathBuf, String) {
    let fixture = Fixture::new();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.submit();
    let (path, evaluation_id) = fixture.evaluate();
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    assert_eq!(
        fixture.accept_candidate("cand-0001", path.clone(), Some("u_glass_thickness")),
        "s0002"
    );
    (fixture, path, evaluation_id)
}

/// A second candidate that changes another component, submitted and
/// evaluated: its acceptance carries the resolution of the first one.
fn owner_review_prepare_other_edit(fixture: &Fixture) -> PathBuf {
    let directory = fixture.write_second_candidate("s0002", "base", 0.1);
    write_file(&directory.join("geometry/base.obj"), &common::box_obj(0.09));
    fixture.submit_directory(&directory);
    fixture.evaluate_second().0
}

#[test]
fn owner_review_carried_resolution_keeps_original_evaluation_reference() {
    let (fixture, _old_report, old_id) = owner_review_resolved_fixture();
    let path = owner_review_prepare_other_edit(&fixture);
    fixture.accept_candidate("cand-0002", path, None);
    assert_eq!(fixture.status(None, "u_glass_thickness"), "resolved");
    let bundle = load_scene(&fixture.store(), "s0003")
        .expect("the scene is read")
        .verification
        .expect("the scene carries a bundle");
    let ids: Vec<&str> = bundle
        .evaluations
        .iter()
        .map(|reference| reference.evaluation_id.as_str())
        .collect();
    assert!(
        bundle
            .evaluations
            .iter()
            .any(|reference| reference.evaluation_id == old_id),
        "a retained resolution must retain its evaluation reference: {ids:?}"
    );
}

#[test]
fn owner_review_corrupt_old_evaluation_blocks_carried_resolution_publication() {
    let (fixture, old_report, _old_id) = owner_review_resolved_fixture();
    let path = owner_review_prepare_other_edit(&fixture);
    let mut old: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&old_report).expect("the report is read"))
            .expect("the report is JSON");
    old["results"]["projection_conformance"] = json!("fail");
    write_file(&old_report, &old.to_string());
    let result = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0002",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            resolve: None,
        },
    );
    assert!(
        result.is_err(),
        "the old resolution evaluation is corrupt: publishing it as still resolved must fail"
    );
    let pointer = michelangelo_edit::scene::load_current(&fixture.store())
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(
        pointer.scene_revision, "s0002",
        "the refused acceptance publishes no revision"
    );
}

#[test]
fn owner_review_corrupt_old_confirmation_is_not_rehashed_as_new_trust() {
    let (fixture, _old_report, _evaluation_id) = owner_review_resolved_fixture();
    let original = load_scene(&fixture.store(), "s0002")
        .expect("the scene is read")
        .verification
        .expect("the scene carries a bundle");
    let confirmation = &original.confirmations[0];
    let confirmation_path = fixture.run_dir.join(&confirmation.path);
    let path = owner_review_prepare_other_edit(&fixture);
    let mut document: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&confirmation_path).expect("the confirmation is read"),
    )
    .expect("the confirmation is JSON");
    document["claim"] = json!("TAMPERED: a different hidden shape was approved");
    write_file(&confirmation_path, &document.to_string());
    let result = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0002",
        "alice",
        &fixture.clock(),
        AcceptOptions {
            evaluation: Some(path),
            resolve: None,
        },
    );
    assert!(
        result.is_err(),
        "a carried confirmation must match its previously accepted digest, not a newly computed replacement digest"
    );
}

#[test]
fn a_completion_rule_the_package_does_not_allow_is_refused_without_a_decision() {
    let fixture = Fixture::new();
    write_file(
        &fixture.run_dir.join("report.json"),
        &report_with_a_resolved_unknown(),
    );
    write_file(
        &fixture.run_dir.join("package/package.json"),
        &package_with_a_forbidden_rule(false),
    );
    fixture.submit();
    let (_report_path, evaluation_id) = fixture.evaluate();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let (document, manifest, evaluation) = resolution_inputs(&fixture, "cand-0001", &evaluation_id);
    let error = propose_resolution(
        &fixture.store(),
        "u_glass_thickness",
        &document,
        &manifest,
        &evaluation,
        "alice",
        &fixture.clock(),
    )
    .expect_err("the rule the package does not allow is refused");
    assert_eq!(error.code(), UNKNOWN_POLICY_VIOLATION);
}

// ---------------------------------------------------------------------------
// Review E: a binding a rollback discarded is never resurrected
// ---------------------------------------------------------------------------

/// A region that differs from the region of the fixture.
fn other_region() -> Region {
    Region {
        min: [-0.3, -0.3, -0.3],
        max: [0.3, 0.3, 0.3],
    }
}

/// The latest binding document of a store.
fn latest_document(fixture: &Fixture) -> michelangelo_edit::unknowns::BindingDocument {
    michelangelo_edit::unknowns::latest_binding_document(&fixture.store())
        .expect("the bindings are read")
        .expect("the bindings exist")
}

impl Fixture {
    /// A clock that is later than the clock of the fixture.
    ///
    /// The rollback and the binding that follows it must not carry the same
    /// timestamp: the binding that a rollback discarded and the binding that
    /// was written after it are told apart by their time.
    fn later_clock(&self) -> FixedClock {
        FixedClock::new("2026-01-04T05:06:07Z")
    }
}

#[test]
fn review_e_a_binding_after_a_rollback_never_resurrects_a_discarded_binding() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");
    assert_eq!(
        fixture.status(Some("s0002"), "u_glass_thickness"),
        "resolved"
    );

    // The author rebinds the unknown with another scope: the document b0002
    // is the one the rollback discards.
    bind_unknown(
        &fixture.store(),
        "u_glass_thickness",
        &["base".to_string()],
        other_region(),
        "bob",
        Some("the glass is 3 mm"),
        &fixture.clock(),
    )
    .expect("the unknown is rebound");
    assert_eq!(latest_document(&fixture).version, "b0002");

    // The rollback selects the revision s0002 again.
    rollback_scene(&fixture.run_dir, "s0002", "alice", &fixture.clock())
        .expect("the scene is rolled back");

    // Another unknown is bound after the rollback: the new document must not
    // carry the discarded binding forward.
    let later = fixture.later_clock();
    bind_unknown(
        &fixture.store(),
        "u_flame_shape",
        &["lid".to_string()],
        region(),
        "carol",
        None,
        &later,
    )
    .expect("the unknown is bound");
    let document = latest_document(&fixture);
    assert_eq!(document.version, "b0003");
    let binding = document
        .binding("u_glass_thickness")
        .expect("the unknown is bound in the new document");
    assert_eq!(
        binding.components,
        vec!["lid".to_string()],
        "the new document carries the binding of the selected revision"
    );
    assert_eq!(
        binding.region,
        region(),
        "the new document carries the scope of the selected revision"
    );
    assert_eq!(
        binding.claim.as_deref(),
        Some("the glass is 2 mm"),
        "the new document carries the claim of the selected revision"
    );
    assert!(
        document.binding("u_flame_shape").is_some(),
        "the new document carries the binding of the new unknown"
    );

    // The view of the current revision still reports the selected binding.
    let entry = binding_of(&fixture, None, "u_glass_thickness");
    assert_eq!(entry.1, Some(region()), "the rolled back scope is shown");
    assert_eq!(entry.2.as_deref(), Some("b0001"));

    // The resolution that survived the rollback is not invalidated by the
    // unrelated bind.
    let directory = fixture.write_second_candidate("s0002", "base", 0.1);
    fixture.submit_directory(&directory);
    let (report_path, _) = fixture.evaluate_second();
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0002",
        "alice",
        &later,
        AcceptOptions {
            evaluation: Some(report_path),
            resolve: None,
        },
    )
    .expect("the candidate is accepted");
    let revision = report
        .resulting_scene_revision
        .expect("the acceptance publishes a revision");
    assert_eq!(revision, "s0003");
    assert_eq!(
        fixture.status(Some("s0003"), "u_glass_thickness"),
        "resolved",
        "the resolution of the selected revision survives the unrelated bind"
    );
}

#[test]
fn review_e_an_explicit_rebind_after_a_rollback_is_honoured() {
    let fixture = Fixture::new();
    fixture.submit();
    let (report_path, evaluation_id) = fixture.evaluate();
    fixture.bind("u_glass_thickness", Some("the glass is 2 mm"));
    fixture.confirm("u_glass_thickness", &evaluation_id, "alice");
    let revision = fixture.accept_candidate("cand-0001", report_path, Some("u_glass_thickness"));
    assert_eq!(revision, "s0002");

    // A first rebinding is written and discarded by the rollback.
    bind_unknown(
        &fixture.store(),
        "u_glass_thickness",
        &["base".to_string()],
        other_region(),
        "bob",
        Some("the glass is 3 mm"),
        &fixture.clock(),
    )
    .expect("the unknown is rebound");
    rollback_scene(&fixture.run_dir, "s0002", "alice", &fixture.clock())
        .expect("the scene is rolled back");

    // The author binds the unknown again, with another scope: the explicit
    // decision is honoured, the bindings are not immutable.
    let new_region = Region {
        min: [-0.1, -0.1, -0.1],
        max: [0.1, 0.1, 0.1],
    };
    let later = fixture.later_clock();
    bind_unknown(
        &fixture.store(),
        "u_glass_thickness",
        &["lid".to_string()],
        new_region,
        "carol",
        Some("the glass is 4 mm"),
        &later,
    )
    .expect("the unknown is bound again");
    let document = latest_document(&fixture);
    assert_eq!(document.version, "b0003");
    let binding = document
        .binding("u_glass_thickness")
        .expect("the unknown is bound in the new document");
    assert_eq!(binding.region, new_region, "the new scope is carried");
    assert_eq!(
        binding.claim.as_deref(),
        Some("the glass is 4 mm"),
        "the new claim is carried"
    );
    assert_eq!(binding.by, "carol", "the author of the binding is carried");

    // The resolution whose binding changed is invalidated by the next
    // revision.
    let directory = fixture.write_second_candidate("s0002", "base", 0.1);
    fixture.submit_directory(&directory);
    let (report_path, _) = fixture.evaluate_second();
    let report = accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0002",
        "alice",
        &later,
        AcceptOptions {
            evaluation: Some(report_path),
            resolve: None,
        },
    )
    .expect("the candidate is accepted");
    let revision = report
        .resulting_scene_revision
        .expect("the acceptance publishes a revision");
    assert_eq!(revision, "s0003");
    assert_eq!(
        fixture.status(Some("s0003"), "u_glass_thickness"),
        "open",
        "the resolution whose binding changed is invalidated"
    );
    assert_eq!(
        fixture.reason(Some("s0003"), "u_glass_thickness"),
        Some("evidence_stale".to_string())
    );
}
