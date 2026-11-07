//! Tests of the edit candidate parser.

mod common;

use common::candidate;
use michelangelo_edit::candidate::{
    is_safe_relative_obj_path, is_valid_component_name, parse_candidate, CandidateAction,
    CandidateOrigin,
};
use serde_json::json;

#[test]
fn a_valid_candidate_is_parsed() {
    let value = candidate(&json!({}));
    let parsed = parse_candidate(&value.to_string()).expect("the candidate is parsed");
    assert_eq!(parsed.schema, "michelangelo.edit-candidate/0");
    assert_eq!(parsed.candidate_id, "cand-0001");
    assert_eq!(parsed.base.scene_revision, "s0001");
    assert_eq!(parsed.provenance.origin, CandidateOrigin::Human);
    assert_eq!(parsed.changes[0].action, CandidateAction::Replace);
    assert_eq!(parsed.changes[0].obj, "geometry/lid.obj");
    assert_eq!(parsed.budget.max_calls, 1);
}

#[test]
fn an_unknown_field_is_rejected() {
    let value = candidate(&json!({"extra": true}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn an_unknown_schema_is_rejected() {
    let value = candidate(&json!({"schema": "michelangelo.edit-candidate/1"}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
    assert!(error.message.contains("not supported"));
}

#[test]
fn an_unknown_origin_is_rejected() {
    let value = candidate(&json!({"provenance": {"origin": "robot"}}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn an_unknown_action_is_rejected() {
    let value = candidate(
        &json!({"changes": [{"component": "lid", "action": "move", "obj": "geometry/lid.obj"}]}),
    );
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn a_budget_without_calls_is_rejected() {
    let value = candidate(&json!({"budget": {"max_calls": 0}}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "BUDGET_INVALID");
}

#[test]
fn a_negative_budget_is_rejected() {
    let value = candidate(&json!({"budget": {"max_seconds": -1}}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "BUDGET_INVALID");
}

#[test]
fn a_fractional_budget_is_rejected() {
    let value = candidate(&json!({"budget": {"max_calls": 1.5}}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "BUDGET_INVALID");
}

#[test]
fn a_negative_cost_is_rejected() {
    let value = candidate(&json!({"budget": {"max_cost_usd": -0.5}}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "BUDGET_INVALID");
}

#[test]
fn an_invalid_component_name_is_rejected() {
    let value = candidate(
        &json!({"changes": [{"component": "-lid", "action": "replace", "obj": "geometry/lid.obj"}]}),
    );
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn a_traversing_obj_path_is_rejected() {
    let value = candidate(
        &json!({"changes": [{"component": "lid", "action": "replace", "obj": "../lid.obj"}]}),
    );
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn an_absolute_obj_path_is_rejected() {
    let value = candidate(
        &json!({"changes": [{"component": "lid", "action": "replace", "obj": "C:\\lid.obj"}]}),
    );
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn a_twice_changed_component_is_rejected() {
    let value = candidate(&json!({"changes": [
        {"component": "lid", "action": "replace", "obj": "geometry/lid.obj"},
        {"component": "lid", "action": "add", "obj": "geometry/lid.obj"}
    ]}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn an_inverted_region_is_rejected() {
    let value = candidate(
        &json!({"task": {"allowed_region": {"min": [0.2, 0.2, 0.2], "max": [-0.2, -0.2, -0.2]}}}),
    );
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn a_candidate_without_change_is_rejected() {
    let value = candidate(&json!({"changes": []}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn an_invalid_base_revision_is_rejected() {
    let value = candidate(&json!({"base": {"scene_revision": "scene-1"}}));
    let error = parse_candidate(&value.to_string()).expect_err("the candidate is refused");
    assert_eq!(error.code, "INTERNAL_ERROR");
}

#[test]
fn the_delete_action_is_parsed_and_refused_by_the_validator() {
    let value = candidate(
        &json!({"changes": [{"component": "lid", "action": "delete", "obj": "geometry/lid.obj"}]}),
    );
    let parsed = parse_candidate(&value.to_string()).expect("the candidate is parsed");
    assert_eq!(parsed.changes[0].action, CandidateAction::Delete);
}

#[test]
fn the_component_name_rule_is_enforced() {
    assert!(is_valid_component_name("lid"));
    assert!(is_valid_component_name("supplement_bracket_arm_connection"));
    assert!(!is_valid_component_name(""));
    assert!(!is_valid_component_name("-lid"));
    assert!(!is_valid_component_name("a/b"));
    assert!(!is_valid_component_name(&"a".repeat(65)));
    assert!(is_valid_component_name(&"a".repeat(64)));
}

#[test]
fn the_obj_path_rule_is_enforced() {
    assert!(is_safe_relative_obj_path("geometry/lid.obj"));
    assert!(!is_safe_relative_obj_path("/geometry/lid.obj"));
    assert!(!is_safe_relative_obj_path("geometry/../lid.obj"));
    assert!(!is_safe_relative_obj_path("geometry/lid.txt"));
    assert!(!is_safe_relative_obj_path(""));
}

#[test]
fn the_candidate_round_trips() {
    let value = candidate(&json!({"provenance": {"origin": "ai_author"}}));
    let parsed = parse_candidate(&value.to_string()).expect("the candidate is parsed");
    let text = serde_json::to_string(&parsed).expect("the candidate is serializable");
    let again = parse_candidate(&text).expect("the candidate is parsed again");
    assert_eq!(parsed, again);
    assert_eq!(again.provenance.origin, CandidateOrigin::AiAuthor);
}
