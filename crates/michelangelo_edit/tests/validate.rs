//! Tests of the validator and of the decision report.

mod common;

use std::path::{Path, PathBuf};

use common::{box_obj, candidate, far_lid_obj, lid_obj, make_run, write_candidate};
use michelangelo_edit::candidate::parse_candidate;
use michelangelo_edit::policy::EditPolicy;
use michelangelo_edit::scene::{init_scene, load_current, load_scene, SceneStore};
use michelangelo_edit::validate::{
    validate, Decision, DecisionReport, ValidationContext, ValidationMode,
};
use michelangelo_edit::FixedClock;
use serde_json::json;

/// A store with an initialized scene and a candidate directory.
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
        init_scene(&run_dir, &FixedClock::new("2026-01-02T03:04:05Z"))
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

    fn outcome(
        &self,
        mode: ValidationMode,
        policy: Option<&EditPolicy>,
    ) -> michelangelo_edit::validate::ValidationOutcome {
        let store = SceneStore::new(&self.run_dir);
        let pointer = load_current(&store)
            .expect("the pointer is read")
            .expect("the pointer exists");
        let scene = load_scene(&store, &pointer.scene_revision).expect("the scene is read");
        let text = std::fs::read_to_string(self.candidate_dir.join("candidate.json"))
            .expect("the candidate is readable");
        let parsed = parse_candidate(&text).expect("the candidate is parsed");
        let context = ValidationContext {
            candidate: &parsed,
            candidate_dir: &self.candidate_dir,
            base_scene: &scene,
            current_scene_revision: &pointer.scene_revision,
            mode,
            decided_by: match mode {
                ValidationMode::Accept => Some("alice"),
                ValidationMode::Submit => None,
            },
            package_policy: None,
        };
        validate(&context, policy)
    }
}

fn lid_geometry() -> Vec<(&'static str, String)> {
    vec![("lid.obj", lid_obj(0.02))]
}

#[test]
fn a_valid_candidate_passes_every_check() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(outcome.valid, "{:?}", outcome.error_codes());
    assert!(outcome.error_codes().is_empty());
    assert_eq!(outcome.checks.len(), 11);
    assert_eq!(outcome.checks[0].check, "candidate_schema");
    assert_eq!(outcome.checks[10].check, "policy");
}

#[test]
fn a_stale_base_is_reported() {
    let fixture = Fixture::new(
        &json!({"base": {"scene_revision": "s0000"}}),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"STALE_BASE".to_string()));
}

#[test]
fn an_unknown_component_is_reported() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["lid", "handle"]},
            "changes": [{"component": "handle", "action": "replace", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"COMPONENT_UNKNOWN".to_string()));
}

#[test]
fn a_component_outside_the_allowed_set_is_reported() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["base"]},
            "changes": [{"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"NOT_ALLOWED".to_string()));
}

#[test]
fn a_protected_component_is_reported() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["lid", "base"], "protected": ["base", "lid"]},
            "changes": [{"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"PROTECTED_MODIFIED".to_string()));
}

#[test]
fn a_geometry_outside_the_region_is_reported() {
    let fixture = Fixture::new(&json!({}), &[("lid.obj", far_lid_obj())]);
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"REGION_VIOLATION".to_string()));
}

#[test]
fn a_broken_geometry_is_reported_as_a_parse_error() {
    let fixture = Fixture::new(&json!({}), &[("lid.obj", "v 0 0 0\nl 1 2\n".to_string())]);
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"OBJ_PARSE".to_string()));
}

#[test]
fn a_degenerate_geometry_is_reported_as_an_invalid_geometry() {
    let fixture = Fixture::new(
        &json!({}),
        &[(
            "lid.obj",
            "v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n".to_string(),
        )],
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"GEOMETRY_INVALID".to_string()));
}

#[test]
fn a_wrong_identity_is_reported() {
    let fixture = Fixture::new(
        &json!({"base": {"package_id": "other-package"}}),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"IDENTITY_MISMATCH".to_string()));
}

#[test]
fn a_delete_action_is_reported_as_unsupported() {
    let fixture = Fixture::new(
        &json!({"changes": [{"component": "lid", "action": "delete", "obj": "geometry/lid.obj"}]}),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"UNSUPPORTED_ACTION".to_string()));
}

#[test]
fn a_broken_vertex_budget_is_reported() {
    let fixture = Fixture::new(
        &json!({"task": {"max_changed_vertices": 2}}),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome
        .error_codes()
        .contains(&"BUDGET_INVALID".to_string()));
}

#[test]
fn an_empty_allowed_set_is_reported() {
    let fixture = Fixture::new(
        &json!({"task": {"allowed_components": []}}),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"NOT_ALLOWED".to_string()));
}

#[test]
fn a_protected_and_allowed_component_is_reported() {
    let fixture = Fixture::new(
        &json!({"task": {"allowed_components": ["lid", "base"], "protected": ["lid"]}}),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"NOT_ALLOWED".to_string()));
}

#[test]
fn a_submission_needs_an_author_acceptance() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(outcome.valid);
    assert!(!outcome.auto_accept);
    let codes: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"ACCEPTANCE_REQUIRED".to_string()));
}

#[test]
fn a_policy_that_denies_the_acceptance_is_reported() {
    let fixture = Fixture::new(
        &json!({"task": {"require_author_acceptance": false}}),
        &lid_geometry(),
    );
    let policy = EditPolicy {
        schema: michelangelo_edit::policy::POLICY_SCHEMA.to_string(),
        auto_accept: false,
        allowed_origins: None,
    };
    let outcome = fixture.outcome(ValidationMode::Submit, Some(&policy));
    assert!(outcome.valid);
    assert!(!outcome.auto_accept);
    let codes: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    assert!(codes.contains(&"POLICY_DENIED".to_string()));
}

#[test]
fn a_policy_that_allows_the_acceptance_enables_it() {
    let fixture = Fixture::new(
        &json!({"task": {"require_author_acceptance": false}}),
        &lid_geometry(),
    );
    let policy = EditPolicy {
        schema: michelangelo_edit::policy::POLICY_SCHEMA.to_string(),
        auto_accept: true,
        allowed_origins: None,
    };
    let outcome = fixture.outcome(ValidationMode::Submit, Some(&policy));
    assert!(outcome.valid);
    assert!(outcome.auto_accept);
}

#[test]
fn a_policy_that_refuses_the_origin_denies_the_acceptance() {
    let fixture = Fixture::new(
        &json!({"task": {"require_author_acceptance": false}}),
        &lid_geometry(),
    );
    let policy = EditPolicy {
        schema: michelangelo_edit::policy::POLICY_SCHEMA.to_string(),
        auto_accept: true,
        allowed_origins: Some(vec![
            michelangelo_edit::candidate::CandidateOrigin::AiAuthor,
        ]),
    };
    let outcome = fixture.outcome(ValidationMode::Submit, Some(&policy));
    assert!(outcome.valid);
    assert!(!outcome.auto_accept);
}

#[test]
fn the_checks_are_ordered() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    let names: Vec<String> = outcome
        .checks
        .iter()
        .map(|check| check.check.clone())
        .collect();
    assert_eq!(
        names,
        vec![
            "candidate_schema",
            "identity",
            "base_revision",
            "task_scope",
            "components",
            "protected",
            "geometry",
            "region",
            "budget",
            "acceptance",
            "policy",
        ]
    );
}

#[test]
fn the_decision_report_is_deterministic() {
    let first = DecisionReport::empty("cand-0001", Decision::Rejected, "2026-01-02T03:04:05Z");
    let second = DecisionReport::empty("cand-0001", Decision::Rejected, "2026-01-02T03:04:05Z");
    assert_eq!(
        first.to_json().expect("the report is serializable"),
        second.to_json().expect("the report is serializable")
    );
}

#[test]
fn the_decision_report_maps_to_the_exit_codes() {
    assert_eq!(Decision::Accepted.exit_code(), 0);
    assert_eq!(Decision::Proposed.exit_code(), 0);
    assert_eq!(Decision::Rejected.exit_code(), 1);
    assert_eq!(Decision::Error.exit_code(), 2);
}

#[test]
fn an_add_action_is_valid_for_a_new_component() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["lid", "handle"], "protected": []},
            "changes": [{"component": "handle", "action": "add", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(outcome.valid, "{:?}", outcome.error_codes());
    assert_eq!(outcome.prepared.len(), 1);
    assert_eq!(
        outcome.prepared[0].action,
        michelangelo_edit::candidate::CandidateAction::Add
    );
}

#[test]
fn an_add_action_for_an_existing_component_is_reported() {
    let fixture = Fixture::new(
        &json!({
            "task": {"allowed_components": ["lid"]},
            "changes": [{"component": "lid", "action": "add", "obj": "geometry/lid.obj"}]
        }),
        &lid_geometry(),
    );
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"NOT_ALLOWED".to_string()));
}

#[test]
fn a_missing_geometry_is_reported() {
    let fixture = Fixture::new(&json!({}), &[]);
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert!(!outcome.valid);
    assert!(outcome.error_codes().contains(&"OBJ_PARSE".to_string()));
}

#[test]
fn the_geometry_of_a_change_is_prepared() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    assert_eq!(outcome.prepared.len(), 1);
    assert_eq!(outcome.prepared[0].component, "lid");
    assert_eq!(outcome.prepared[0].vertex_count, 8);
    assert!(outcome.prepared[0].source.ends_with("lid.obj"));
}

#[test]
fn the_policy_file_is_optional() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let policy = michelangelo_edit::policy::load_policy(&run_dir).expect("the policy is read");
    assert!(policy.is_none());
}

#[test]
fn a_broken_policy_is_an_error() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    common::write_file(
        &run_dir.join(".michelangelo/edit_policy.json"),
        "{ not json",
    );
    let error =
        michelangelo_edit::policy::load_policy(&run_dir).expect_err("the policy is refused");
    assert!(error.to_string().contains("policy"));
}

#[test]
fn a_policy_is_found_in_the_workspace() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("recon/r0001/run-0001");
    make_run(&run_dir);
    common::write_file(
        &directory.path().join(".michelangelo/edit_policy.json"),
        r#"{"schema": "michelangelo.edit-policy/0", "auto_accept": true}"#,
    );
    let policy = michelangelo_edit::policy::load_policy(&run_dir).expect("the policy is read");
    let policy = policy.expect("the policy exists");
    assert!(policy.auto_accept);
}

#[test]
fn the_source_of_a_change_is_inside_the_candidate_directory() {
    let fixture = Fixture::new(&json!({}), &lid_geometry());
    let outcome = fixture.outcome(ValidationMode::Submit, None);
    let source: &Path = &outcome.prepared[0].source;
    assert!(source.starts_with(&fixture.candidate_dir));
}

#[test]
fn a_box_is_not_empty() {
    assert!(!box_obj(0.1).is_empty());
}
