//! Negative tests mapped to the GLM-1 NEGATIVE_TEST_MATRIX rows owned by
//! GLM-4: F6 (role), F7 (scope widening), F9 (protected-role analogue),
//! F10 (arbitrary code), F12 (floating latest), F17 (parameters), plus
//! held-out leakage, socket mismatch, duplicate identity, foreign skeleton,
//! stale donor, combinatorial cap and honest insufficient-combination
//! failure. No test fakes a PASS: every expected failure asserts the exact
//! rejection code.

use std::collections::BTreeMap;

use michelangelo_family::errors::FamilyErrorCode;
use michelangelo_family::fixture;
use michelangelo_family::json::Json;
use michelangelo_family::model::{
    Constraint, ManifestKind, ManifestRef, ParamSpec, ParamValue, SourceEntry, SourceRole,
    SourceUsage,
};
use michelangelo_family::plan::{enumerate_valid_combinations, sample_requests, FamilyPlanner};
use michelangelo_family::sha256::sha256_hex;
use michelangelo_family::{BatchExecutorPort, NullBatchExecutor, VariantRequest};

/// Mutate a top-level JSON object produced by a DTO's to_json().
fn mutate(doc: &Json, f: impl FnOnce(&mut BTreeMap<String, Json>)) -> Json {
    let mut m = match doc.clone() {
        Json::Object(m) => m,
        _ => panic!("expected object"),
    };
    f(&mut m);
    Json::Object(m)
}

fn house() -> (
    michelangelo_family::SourceSet,
    michelangelo_family::FamilyRecipe,
    michelangelo_family::RecipeLock,
) {
    fixture::test_house()
}

// ---------------------------------------------------------------- F6: role

#[test]
fn f6_unknown_role_string_is_rejected() {
    let err = SourceRole::parse("co_author").unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::RoleInvalid);
    let err = SourceUsage::parse("builder_input").unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::RoleInvalid);
}

#[test]
fn f6_donor_without_allowed_parts_is_rejected() {
    let (mut sources, _, _) = house();
    let donor = sources
        .sources
        .iter_mut()
        .find(|s| s.source_id == "roof-donor")
        .unwrap();
    donor.allowed_parts.clear();
    let err = sources.validate().unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::RoleInvalid);
}

#[test]
fn f6_style_reference_must_not_be_build_input() {
    let mut sources = fixture::test_house_source_set();
    let style = sources
        .sources
        .iter_mut()
        .find(|s| s.source_id == "style-example")
        .unwrap();
    style.usage = SourceUsage::BuildInput; // reference_only required
    assert_eq!(sources.validate().unwrap_err().code, FamilyErrorCode::RoleInvalid);
}

// --------------------------------------------------- held-out leakage (F6/F9)

#[test]
fn heldout_validation_reference_with_build_usage_is_rejected() {
    let mut sources = fixture::test_house_source_set();
    let held = sources
        .sources
        .iter_mut()
        .find(|s| s.source_id == "heldout-check")
        .unwrap();
    held.usage = SourceUsage::ReferenceOnly; // must be held_out
    assert_eq!(sources.validate().unwrap_err().code, FamilyErrorCode::HeldoutLeakage);
}

#[test]
fn heldout_source_cannot_also_carry_build_role() {
    let mut sources = fixture::test_house_source_set();
    let held = sources
        .sources
        .iter_mut()
        .find(|s| s.source_id == "heldout-check")
        .unwrap();
    held.roles.push(SourceRole::ModuleDonor);
    let err = sources.validate().unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::HeldoutLeakage);
}

#[test]
fn generator_consuming_heldout_source_is_rejected_at_plan_time() {
    let (sources, mut recipe, lock) = house();
    recipe.generator.source_inputs.push("heldout-check".to_string());
    // Recipe digest changed => lock mismatch surfaces first by design; feed a
    // fresh lock so the held-out check is the one that fires.
    let fresh_lock = fixture::test_house_lock(&recipe, &sources);
    let requests = sample_requests(&recipe, "house", 1).unwrap();
    let err = FamilyPlanner::plan("b", &recipe, &sources, &fresh_lock, &requests).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::HeldoutLeakage);
    let _ = lock;
}

// ------------------------------------------------------- F7: scope widening

#[test]
fn f7_request_cannot_carry_scope_fields() {
    let (_, recipe, _) = house();
    let r = sample_requests(&recipe, "house", 1).unwrap().pop().unwrap();
    let doc = Json::Object(BTreeMap::from([
        ("schema".to_string(), Json::Str("michelangelo.variant-request/draft-0.1".to_string())),
        ("item_id".to_string(), Json::Str(r.item_id.clone())),
        ("parameters".to_string(), Json::Object(
            r.parameters.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())),
        ("purpose".to_string(), Json::Str(r.purpose.clone())),
        // Scope-widening injection on the wire:
        ("scope".to_string(), Json::Object(BTreeMap::from([
            ("allowed_roles".to_string(), Json::Array(vec![Json::Str("anything".to_string())])),
        ]))),
    ]));
    let err = michelangelo_family::wire::parse_variant_request(&doc).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ScopeWidening);
}

#[test]
fn f7_binding_targeting_role_outside_allowed_roles_is_rejected() {
    let (_, mut recipe, _) = house();
    recipe.generator.parameter_bindings.insert(
        "palette".to_string(),
        "landscaping.tree_style".to_string(),
    );
    let err = recipe.validate().unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ScopeWidening);
}

// ------------------------------------------- F9: protected role (planning analogue)

#[test]
fn f9_binding_targeting_protected_role_is_rejected() {
    let (_, mut recipe, _) = house();
    recipe.generator.parameter_bindings.insert(
        "palette".to_string(),
        "entry_anchor.palette".to_string(),
    );
    let err = recipe.validate().unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ProtectedRoleTarget);
}

#[test]
fn f9_protected_and_allowed_overlap_is_rejected() {
    let (_, mut recipe, _) = house();
    recipe.scope.allowed_roles.push("entry_anchor".to_string());
    let err = recipe.validate().unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ScopeWidening);
}

// ------------------------------------------------- F10: arbitrary code

#[test]
fn f10_recipe_with_allow_arbitrary_code_true_is_rejected() {
    let (_, recipe, _) = house();
    let doc = mutate(&recipe.to_json(), |m| {
        m.insert("allow_arbitrary_code".to_string(), Json::Bool(true));
    });
    let err = michelangelo_family::wire::parse_recipe(&doc).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ArbitraryCode);
}

#[test]
fn f10_scope_with_allow_arbitrary_code_true_is_rejected() {
    let (_, recipe, _) = house();
    let doc = mutate(&recipe.to_json(), |m| {
        let scope = mutate(
            m.get("scope").unwrap(),
            |sm| {
                sm.insert("allow_arbitrary_code".to_string(), Json::Bool(true));
            },
        );
        m.insert("scope".to_string(), scope);
    });
    let err = michelangelo_family::wire::parse_recipe(&doc).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ArbitraryCode);
}

#[test]
fn f10_script_like_binding_path_is_rejected() {
    let (_, mut recipe, _) = house();
    recipe
        .generator
        .parameter_bindings
        .insert("palette".to_string(), "exec.run_script".to_string());
    assert_eq!(recipe.validate().unwrap_err().code, FamilyErrorCode::ArbitraryCode);

    let (_, mut recipe, _) = house();
    recipe
        .generator
        .parameter_bindings
        .insert("palette".to_string(), "../../python_entry.main".to_string());
    assert_eq!(recipe.validate().unwrap_err().code, FamilyErrorCode::ArbitraryCode);
}

#[test]
fn v2_v4_generator_kinds_are_capability_denied() {
    assert_eq!(
        michelangelo_family::GeneratorKind::parse("parametric_modifier_v2").unwrap_err().code,
        FamilyErrorCode::CapabilityDenied
    );
    assert_eq!(
        michelangelo_family::GeneratorKind::parse("rule_based_construction_v3").unwrap_err().code,
        FamilyErrorCode::CapabilityDenied
    );
    assert_eq!(
        michelangelo_family::GeneratorKind::parse("agent_proposal_v4").unwrap_err().code,
        FamilyErrorCode::CapabilityDenied
    );
    assert_eq!(
        michelangelo_family::GeneratorKind::parse("registered_blender_template_v0")
            .unwrap()
            .as_str(),
        "registered_blender_template_v0"
    );
}

// ------------------------------------------------- F12: floating latest

#[test]
fn f12_latest_tool_version_is_rejected() {
    let mut lock = fixture::test_house_lock(&fixture::test_house().1, &fixture::test_house_source_set());
    lock.tool_versions.insert("blender".to_string(), "latest".to_string());
    assert_eq!(lock.validate().unwrap_err().code, FamilyErrorCode::FloatingVersion);
}

#[test]
fn f12_range_versions_are_rejected() {
    for v in ["^4.2", "~4.2", ">=4.2", "<5", "*", ""] {
        let mut lock = fixture::test_house_lock(&fixture::test_house().1, &fixture::test_house_source_set());
        lock.tool_versions.insert("blender".to_string(), v.to_string());
        assert_eq!(lock.validate().unwrap_err().code, FamilyErrorCode::FloatingVersion, "version {v}");
    }
}

#[test]
fn f12_lock_without_required_tools_is_rejected() {
    let (_, recipe, _) = house();
    let sources = fixture::test_house_source_set();
    let mut lock = fixture::test_house_lock(&recipe, &sources);
    lock.tool_versions.remove("michelangelo");
    assert_eq!(lock.validate().unwrap_err().code, FamilyErrorCode::FloatingVersion);
}

#[test]
fn f12_wire_lock_claiming_floating_allowed_is_rejected() {
    let (_, recipe, _) = house();
    let sources = fixture::test_house_source_set();
    let lock = fixture::test_house_lock(&recipe, &sources);
    let doc = mutate(&Json::Object(BTreeMap::from([
        ("schema".to_string(), Json::Str("michelangelo.recipe-lock/draft-0.1".to_string())),
        ("lock_id".to_string(), Json::Str(lock.lock_id.clone())),
        ("recipe_ref".to_string(), Json::Object(BTreeMap::from([
            ("recipe_id".to_string(), Json::Str(lock.recipe_id.clone())),
            ("recipe_revision".to_string(), Json::Int(lock.recipe_revision as i64)),
            ("recipe_sha256".to_string(), Json::Str(lock.recipe_sha256.clone())),
        ]))),
        ("source_set_sha256".to_string(), Json::Str(lock.source_set_sha256.clone())),
        ("dependencies".to_string(), Json::Array(lock.dependencies.iter().map(|d|
            Json::Object(BTreeMap::from([
                ("id".to_string(), Json::Str(d.id.clone())),
                ("revision".to_string(), Json::Int(d.revision as i64)),
                ("sha256".to_string(), Json::Str(d.sha256.clone())),
            ]))).collect())),
        ("tool_versions".to_string(), Json::Object(lock.tool_versions.iter().map(|(k, v)|
            (k.clone(), Json::Str(v.clone()))).collect())),
        ("prng".to_string(), Json::Object(BTreeMap::from([
            ("algorithm".to_string(), Json::Str(lock.prng.algorithm.clone())),
            ("derivation_version".to_string(), Json::Str(lock.prng.derivation_version.clone())),
            ("default_seed".to_string(), Json::Int(lock.prng.default_seed as i64)),
        ]))),
        ("pinned_profiles".to_string(), Json::Object(BTreeMap::new())),
        ("floating_latest_allowed".to_string(), Json::Bool(true)),
    ])), |_m| {});
    let err = michelangelo_family::wire::parse_lock(&doc).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::FloatingVersion);
}

// --------------------------------------------------- F17: parameters

#[test]
fn f17_parameter_matrix_on_wire_values() {
    let (_, recipe, _) = house();
    let full = sample_requests(&recipe, "house", 1).unwrap().pop().unwrap();
    // already covered in unit_tests; here: wrong integer type on integer param
    let mut wrong = full.clone();
    wrong
        .parameters
        .insert("floors".to_string(), ParamValue::Text("2".to_string()));
    assert_eq!(wrong.validate_against(&recipe).unwrap_err().code, FamilyErrorCode::ParameterInvalid);
}

// ------------------------------------------------- socket mismatch

#[test]
fn socket_mismatch_for_hip_roof_is_diagnosed() {
    let (_, recipe, _) = house();
    let params: BTreeMap<String, ParamValue> = BTreeMap::from([
        ("floors".to_string(), ParamValue::Int(1)),
        ("bays".to_string(), ParamValue::Int(3)),
        ("roof".to_string(), ParamValue::Text("hip".to_string())),
        ("palette".to_string(), ParamValue::Text("brick_dark".to_string())),
        ("wear_permille".to_string(), ParamValue::Text("0".to_string())),
        ("trim".to_string(), ParamValue::Text("none".to_string())),
    ]);
    let err = michelangelo_family::combination_allowed(&recipe, &params).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::SocketMismatch);
    assert!(err.detail.contains("roof_hip_socket"));
}

#[test]
fn socket_requirement_without_socket_profile_is_rejected() {
    let (_, mut recipe, _) = house();
    recipe.sockets = None;
    let params: BTreeMap<String, ParamValue> = BTreeMap::from([
        ("roof".to_string(), ParamValue::Text("flat".to_string())),
    ]);
    assert_eq!(
        michelangelo_family::combination_allowed(&recipe, &params).unwrap_err().code,
        FamilyErrorCode::SocketMismatch
    );
}

// ------------------------------------------------- duplicate identity

#[test]
fn duplicate_source_id_is_rejected() {
    let mut sources = fixture::test_house_source_set();
    let clone = sources.sources[0].clone();
    sources.sources.push(clone);
    assert_eq!(sources.validate().unwrap_err().code, FamilyErrorCode::DuplicateIdentity);
}

#[test]
fn duplicate_item_id_in_one_plan_is_rejected() {
    let (sources, recipe, lock) = house();
    let mut requests = sample_requests(&recipe, "house", 2).unwrap();
    requests[1].item_id = requests[0].item_id.clone();
    let err = FamilyPlanner::plan("b", &recipe, &sources, &lock, &requests).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::DuplicateIdentity);
}

// ------------------------------------------------- foreign skeleton

#[test]
fn foreign_skeleton_between_build_sources_is_rejected() {
    let mut sources = fixture::test_house_source_set();
    sources.sources[0].skeleton_profile_sha256 = Some(sha256_hex(b"skel-A"));
    sources.sources[0].roles = vec![SourceRole::BaseCandidate];
    sources.sources[1].skeleton_profile_sha256 = Some(sha256_hex(b"skel-B"));
    let err = sources.validate().unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ForeignSkeleton);
}

// ------------------------------------------------- stale donor / lock

#[test]
fn changed_donor_hash_makes_lock_stale() {
    let (mut sources, recipe, lock) = house();
    let donor = sources
        .sources
        .iter_mut()
        .find(|s| s.source_id == "roof-donor")
        .unwrap();
    // Donor re-export: same id, new bytes.
    donor.manifest.sha256 = sha256_hex(b"re-exported roof donor payload v2");
    let err = lock.check_against(&recipe, &sources).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::StaleDependency);
}

#[test]
fn changed_recipe_dependency_hash_makes_lock_stale() {
    let (sources, mut recipe, lock) = house();
    recipe.dependencies[0].sha256 = sha256_hex(b"rotated facade-kit v3 body");
    let err = lock.check_against(&recipe, &sources).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::StaleDependency);
}

#[test]
fn lock_for_other_recipe_revision_is_lock_mismatch() {
    let (sources, mut recipe, lock) = house();
    recipe.revision = 2;
    let err = lock.check_against(&recipe, &sources).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::LockMismatch);
}

#[test]
fn changed_source_set_digest_is_stale() {
    let (_, recipe, lock) = house();
    let mut other = fixture::test_house_source_set();
    other.revision = 2;
    let err = lock.check_against(&recipe, &other).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::StaleDependency);
}

// --------------------------------------- combinatorial cap & honest insufficiency

fn tight_recipe(source_set_digest: &str) -> michelangelo_family::FamilyRecipe {
    let mut recipe = fixture::test_house_recipe(source_set_digest);
    // Everything pinned to a single valid value: exactly one combination.
    recipe.parameters = BTreeMap::from([
        ("floors".to_string(), ParamSpec::Integer { min: 1, max: 1 }),
        ("bays".to_string(), ParamSpec::Integer { min: 2, max: 2 }),
        ("roof".to_string(), ParamSpec::Enum { values: vec!["gable".to_string()] }),
        ("palette".to_string(), ParamSpec::Enum { values: vec!["brick_dark".to_string()] }),
        ("wear_permille".to_string(), ParamSpec::Enum { values: vec!["100".to_string()] }),
        ("trim".to_string(), ParamSpec::Enum { values: vec!["none".to_string()] }),
    ]);
    recipe
}

#[test]
fn asking_for_15_when_only_1_combination_exists_fails_honestly() {
    let sources = fixture::test_house_source_set();
    let recipe = tight_recipe(&sources.digest());
    assert_eq!(enumerate_valid_combinations(&recipe).unwrap().len(), 1);
    let err = sample_requests(&recipe, "house", 15).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::InsufficientUniqueCombinations);
    assert!(err.detail.contains("refusing to fabricate"));
}

#[test]
fn combinatorial_cap_is_enforced_before_enumeration() {
    let sources = fixture::test_house_source_set();
    let mut recipe = fixture::test_house_recipe(&sources.digest());
    recipe.budget.max_combinations = 100; // product is 1944
    let err = enumerate_valid_combinations(&recipe).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::CombinatorialCapExceeded);
}

#[test]
fn plan_over_budget_max_items_is_rejected() {
    let sources = fixture::test_house_source_set();
    let mut recipe = fixture::test_house_recipe(&sources.digest());
    recipe.budget.max_items_per_plan = 5;
    let lock = fixture::test_house_lock(&recipe, &sources);
    let requests: Vec<VariantRequest> = (0..6)
        .map(|i| VariantRequest {
            item_id: format!("house-{i:03}"),
            parameters: enumerate_valid_combinations(&recipe).unwrap()[i].clone(),
            purpose: "p".to_string(),
            export_targets: vec![],
        })
        .collect();
    let err = FamilyPlanner::plan("b", &recipe, &sources, &lock, &requests).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::CombinatorialCapExceeded);
}

#[test]
fn empty_plan_is_rejected() {
    let (sources, recipe, lock) = house();
    let err = FamilyPlanner::plan("b", &recipe, &sources, &lock, &[]).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::SchemaInvalid);
}

// ------------------------------------------------- pure-planning boundary

#[test]
fn null_batch_executor_refuses_every_plan() {
    let (sources, recipe, lock) = house();
    let requests = sample_requests(&recipe, "house", 2).unwrap();
    let plan = FamilyPlanner::plan("b", &recipe, &sources, &lock, &requests).unwrap();
    let err = NullBatchExecutor.accept_plan(&plan).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::NotPlannable);
}

// ------------------------------------------------- wire strictness

#[test]
fn wire_rejects_unknown_top_level_keys() {
    let sources = fixture::test_house_source_set();
    let doc = mutate(&sources.to_json(), |m| {
        m.insert("alignment_override".to_string(), Json::Str("free-form".to_string()));
    });
    assert_eq!(
        michelangelo_family::wire::parse_source_set(&doc).unwrap_err().code,
        FamilyErrorCode::SchemaInvalid
    );
}

#[test]
fn wire_rejects_broken_json() {
    use michelangelo_family::json::parse;
    assert!(parse("{\"a\":1,\"a\":2}").is_err(), "duplicate keys");
    assert!(parse("{} trailing").is_err());
    assert!(parse("{\"a\":}").is_err());
}

#[test]
fn maximum_dimension_constraint_rejects_tall_gable() {
    let (_, recipe, _) = house();
    let params: BTreeMap<String, ParamValue> = BTreeMap::from([
        ("floors".to_string(), ParamValue::Int(3)),
        ("bays".to_string(), ParamValue::Int(3)),
        ("roof".to_string(), ParamValue::Text("gable".to_string())),
    ]);
    let err = michelangelo_family::combination_allowed(&recipe, &params).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ParameterInvalid);
    assert!(err.detail.contains("exceeds maximum_dimension"), "{}", err.detail);
}

#[test]
fn unused_helper_keeps_manifest_kind_import_honest() {
    // ManifestKind/ManifestRef/SourceEntry are used below to keep imports
    // explicit for future donor-mismatch tests; a minimal sanity check here.
    let m = ManifestRef {
        kind: ManifestKind::SourceSnapshot,
        id: "x".to_string(),
        sha256: sha256_hex(b"x"),
    };
    assert_eq!(m.kind, ManifestKind::SourceSnapshot);
    let _entry = SourceEntry {
        source_id: "x".to_string(),
        asset_id: "x".to_string(),
        manifest: m,
        roles: vec![SourceRole::BaseCandidate],
        usage: SourceUsage::BuildInput,
        allowed_parts: vec![],
        skeleton_profile_sha256: None,
    };
    // Constraint enum stays importable for downstream negative fixtures.
    let c = Constraint::MaximumDimensionZ { value_m: 1.0 };
    assert!(matches!(c, Constraint::MaximumDimensionZ { .. }));
}
