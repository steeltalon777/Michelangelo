//! Positive unit tests: digests, seed derivation, combination engine,
//! deterministic dry plan, fixture/wire round-trip consistency.

use std::collections::BTreeMap;

use michelangelo_family::errors::FamilyResult;
use michelangelo_family::fixture;
use michelangelo_family::json::{parse, Json};
use michelangelo_family::plan::{
    derive_seed, enumerate_valid_combinations, sample_requests, FamilyPlanner,
    SEED_DERIVATION_ALGORITHM, SEED_DERIVATION_VERSION,
};
use michelangelo_family::sha256::sha256_hex;
use michelangelo_family::wire::{parse_lock, parse_recipe, parse_source_set};
use michelangelo_family::FamilyErrorCode;
use michelangelo_family::{
    PROVISIONAL_MARKER_KEY, WIRE_SCHEMA_BATCH_PLAN, WIRE_SCHEMA_LOCK, WIRE_SCHEMA_RECIPE,
    WIRE_SCHEMA_REQUEST, WIRE_SCHEMA_SOURCE_SET,
};

fn house() -> (
    michelangelo_family::SourceSet,
    michelangelo_family::FamilyRecipe,
    michelangelo_family::RecipeLock,
) {
    fixture::test_house()
}

#[test]
fn sha256_known_vector() {
    // NIST FIPS 180-4 test vector (empty input).
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn seed_derivation_is_deterministic_and_identity_bound() {
    let (_, recipe, lock) = house();
    let _ = recipe;
    let d = lock.digest();
    let s1 = derive_seed(&d, "house-000");
    let s2 = derive_seed(&d, "house-000");
    assert_eq!(s1, s2, "same identity must give the same seed");
    assert_ne!(
        derive_seed(&d, "house-001"),
        s1,
        "different item_id => different seed"
    );
    assert_ne!(
        derive_seed(&d, "house-000"),
        derive_seed("0".repeat(64).as_str(), "house-000"),
        "different lock digest => different seed"
    );
    // Order independence: derivation is per-item, never positional.
    let ids = ["a", "b", "c"];
    let forward: Vec<u64> = ids.iter().map(|i| derive_seed(&d, i)).collect();
    let mut rev = ids
        .iter()
        .rev()
        .map(|i| derive_seed(&d, i))
        .collect::<Vec<u64>>();
    rev.reverse();
    assert_eq!(forward, rev);
    // 53-bit JSON-safe mask (GLM-1 draft variant_request range).
    assert!(s1 <= 9007199254740991);
    assert_eq!(SEED_DERIVATION_VERSION, "family-seed-v1");
    assert!(SEED_DERIVATION_ALGORITHM.contains("53bit"));
}

#[test]
fn test_house_fixture_is_valid() {
    let (sources, recipe, lock) = house();
    sources.validate().unwrap();
    recipe.validate().unwrap();
    lock.validate().unwrap();
    lock.check_against(&recipe, &sources).unwrap();
}

#[test]
fn combination_engine_counts_and_filters() {
    let (_, recipe, _) = house();
    let valid = enumerate_valid_combinations(&recipe).unwrap();
    // Product = 3*3*3*3*8*3 = 1944; invalid: roof=hip (socket) plus
    // height-constrained and pairwise-incompatible combinations.
    let total: u64 = recipe
        .parameters
        .values()
        .map(|s| match s {
            michelangelo_family::ParamSpec::Integer { min, max } => (max - min + 1) as u64,
            michelangelo_family::ParamSpec::Enum { values } => values.len() as u64,
            michelangelo_family::ParamSpec::Boolean => 2,
            michelangelo_family::ParamSpec::Number { .. } => 0,
        })
        .product();
    assert_eq!(total, 1944);
    assert_eq!(valid.len(), 720, "documented fixture arithmetic");
    let ok = |p: &[(&str, &str)]| {
        let m: BTreeMap<String, michelangelo_family::ParamValue> = p
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    michelangelo_family::ParamValue::Text(v.to_string()),
                )
            })
            .chain([(
                "floors".to_string(),
                michelangelo_family::ParamValue::Int(1),
            )])
            .chain([("bays".to_string(), michelangelo_family::ParamValue::Int(2))])
            .chain([(
                "wear_permille".to_string(),
                michelangelo_family::ParamValue::Text("100".to_string()),
            )])
            .collect();
        michelangelo_family::combination_allowed(&recipe, &m).is_ok()
    };
    assert!(ok(&[
        ("roof", "gable"),
        ("palette", "brick_dark"),
        ("trim", "carved")
    ]));
    assert!(
        !ok(&[
            ("roof", "flat"),
            ("palette", "brick_dark"),
            ("trim", "carved")
        ]),
        "flat forbids carved"
    );
    assert!(
        !ok(&[
            ("roof", "flat"),
            ("palette", "timber_grey"),
            ("trim", "none")
        ]),
        "timber forbids flat"
    );
    assert!(
        !ok(&[("roof", "hip"), ("palette", "brick_dark"), ("trim", "none")]),
        "hip socket missing"
    );
}

#[test]
fn sampling_gives_15_distinct_deterministic_requests() {
    let (_, recipe, _) = house();
    let a = sample_requests(&recipe, "house", 15).unwrap();
    let b = sample_requests(&recipe, "house", 15).unwrap();
    assert_eq!(a, b, "sampling is deterministic");
    assert_eq!(a.len(), 15);
    let params: std::collections::BTreeSet<String> = a
        .iter()
        .map(|r| {
            r.parameters
                .iter()
                .map(|(k, v)| format!("{k}={}", v.canonical()))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    assert_eq!(
        params.len(),
        15,
        "all 15 resolved parameter sets must be distinct"
    );
    let ids: std::collections::BTreeSet<_> = a.iter().map(|r| r.item_id.clone()).collect();
    assert_eq!(ids.len(), 15);
    for r in &a {
        r.validate_against(&recipe).unwrap();
    }
}

#[test]
fn dry_plan_is_immutable_byte_identical() {
    let (sources, recipe, lock) = house();
    let requests = sample_requests(&recipe, "house", 15).unwrap();
    let p1 =
        FamilyPlanner::plan("urban-house-dry-15", &recipe, &sources, &lock, &requests).unwrap();
    let p2 =
        FamilyPlanner::plan("urban-house-dry-15", &recipe, &sources, &lock, &requests).unwrap();
    assert_eq!(p1.to_canonical_json(), p2.to_canonical_json());
    assert_eq!(p1.items.len(), 15);
    assert!(p1.cycle_check_acyclic);
    assert_eq!(p1.max_parallel_heavy, 1);
    // Per-item namespaces are private and distinct.
    let ns: std::collections::BTreeSet<_> = p1
        .items
        .iter()
        .map(|i| i.output_namespace.clone())
        .collect();
    assert_eq!(ns.len(), 15);
    assert!(p1
        .items
        .iter()
        .all(|i| i.output_namespace.starts_with("family/urban-house-dry-15/")));
    // Fingerprints react to parameters.
    let f0 = p1.items[0]
        .input_fingerprints
        .get("resolved_parameters")
        .unwrap();
    let f1 = p1.items[1]
        .input_fingerprints
        .get("resolved_parameters")
        .unwrap();
    assert_ne!(f0, f1);
}

#[test]
fn wire_fixtures_on_disk_match_code_fixture_and_reproduce_plan() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let fixture_dir = std::path::Path::new(manifest).join("fixtures");

    let load = |name: &str| -> FamilyResult<Json> {
        let text = std::fs::read_to_string(fixture_dir.join(name)).unwrap();
        parse(&text).map_err(|e| {
            michelangelo_family::FamilyError::new(FamilyErrorCode::SchemaInvalid, format!("{e}"))
        })
    };
    let wire_sources = parse_source_set(&load("source_set.json").unwrap()).unwrap();
    let wire_recipe = parse_recipe(&load("family_recipe.json").unwrap()).unwrap();
    let wire_lock = parse_lock(&load("recipe_lock.json").unwrap()).unwrap();

    let (sources, recipe, lock) = house();
    assert_eq!(
        wire_sources, sources,
        "committed source_set fixture drifted"
    );
    assert_eq!(wire_recipe, recipe, "committed recipe fixture drifted");
    assert_eq!(wire_lock, lock, "committed lock fixture drifted");

    let requests = sample_requests(&wire_recipe, "house", 15).unwrap();
    let plan = FamilyPlanner::plan(
        "urban-house-dry-15",
        &wire_recipe,
        &wire_sources,
        &wire_lock,
        &requests,
    )
    .unwrap();
    // Cross-check the committed example digest lines still line up.
    let example = std::fs::read_to_string(
        std::path::Path::new(manifest)
            .join("../../docs/adr5-streams/family/examples/dry_plan_15.json"),
    )
    .unwrap();
    let example_plan = parse(&example).unwrap();
    assert_eq!(
        plan.to_canonical_json(),
        example_plan.to_canonical(),
        "committed dry_plan_15.json drifted from the deterministic planner output"
    );
}

#[test]
fn full_request_roundtrip_through_wire() {
    let (_, recipe, _) = house();
    let mut requests = sample_requests(&recipe, "house", 15).unwrap();
    // wire-shaped variant request document for one sampled item
    let r = &requests[0];
    let doc = r.to_json();
    let parsed = michelangelo_family::wire::parse_variant_request(&doc).unwrap();
    assert_eq!(parsed, *r);
    parsed.validate_against(&recipe).unwrap();
    requests.clear(); // silence unused-mut warning path
}

#[test]
fn variant_request_rejects_incomplete_and_unknown() {
    let (_, recipe, _) = house();
    let full = sample_requests(&recipe, "house", 1).unwrap().pop().unwrap();

    // Missing parameter
    let mut missing = full.clone();
    missing.parameters.remove("trim");
    assert_eq!(
        missing.validate_against(&recipe).unwrap_err().code,
        FamilyErrorCode::ParameterInvalid
    );
    // Unknown parameter (attempted scope widening through parameters)
    let mut extra = full.clone();
    extra.parameters.insert(
        "secret_backdoor".to_string(),
        michelangelo_family::ParamValue::Bool(true),
    );
    assert_eq!(
        extra.validate_against(&recipe).unwrap_err().code,
        FamilyErrorCode::ParameterInvalid
    );
    // Out of range
    let mut oob = full.clone();
    oob.parameters.insert(
        "floors".to_string(),
        michelangelo_family::ParamValue::Int(9),
    );
    assert_eq!(
        oob.validate_against(&recipe).unwrap_err().code,
        FamilyErrorCode::ParameterInvalid
    );
    // Wrong enum value
    let mut bad_enum = full.clone();
    bad_enum.parameters.insert(
        "roof".to_string(),
        michelangelo_family::ParamValue::Text("dome".to_string()),
    );
    assert_eq!(
        bad_enum.validate_against(&recipe).unwrap_err().code,
        FamilyErrorCode::ParameterInvalid
    );
    // Wrong type (bool where enum expected)
    let mut wrong_type = full.clone();
    wrong_type.parameters.insert(
        "roof".to_string(),
        michelangelo_family::ParamValue::Bool(true),
    );
    assert_eq!(
        wrong_type.validate_against(&recipe).unwrap_err().code,
        FamilyErrorCode::ParameterInvalid
    );
}

#[test]
fn provisional_documents_use_private_tags_and_explicit_marker() {
    let (sources, recipe, lock) = house();
    let requests = sample_requests(&recipe, "house", 2).unwrap();
    let plan =
        FamilyPlanner::plan("urban-house-dry-15", &recipe, &sources, &lock, &requests).unwrap();

    // Every emitted tag lives in the PRIVATE namespace of this stream: the
    // GLM-1 tags name the approved contract, and these documents do not
    // satisfy the GLM-1 schemas (review finding L).
    for tag in [
        WIRE_SCHEMA_SOURCE_SET,
        WIRE_SCHEMA_RECIPE,
        WIRE_SCHEMA_LOCK,
        WIRE_SCHEMA_REQUEST,
        WIRE_SCHEMA_BATCH_PLAN,
    ] {
        assert!(
            tag.starts_with("michelangelo.family."),
            "{tag} must stay in the private namespace"
        );
        assert!(tag.ends_with("/draft-0"), "{tag} must be a draft-0 tag");
        assert!(
            !tag.contains("/draft-0.1"),
            "{tag} must not reuse a GLM-1 draft-0.1 tag"
        );
    }

    // Every emitted document declares the provisional status explicitly.
    for doc in [
        sources.to_json(),
        recipe.to_json(),
        lock.to_json(),
        requests[0].to_json(),
        plan.to_json(),
    ] {
        let m = doc.as_object().expect("document must be an object");
        assert_eq!(
            m.get(PROVISIONAL_MARKER_KEY),
            Some(&Json::Bool(true)),
            "every provisional document must carry 'provisional: true'"
        );
    }

    // The marker is part of the hashed content: the published bytes and the
    // pinned digests agree.
    assert_eq!(
        lock.digest(),
        sha256_hex(lock.to_json().to_canonical().as_bytes()),
        "lock digest must cover exactly the published lock document"
    );
}

#[test]
fn planner_validates_requests_against_recipe() {
    let (sources, recipe, lock) = house();
    let mut req = sample_requests(&recipe, "house", 1).unwrap().pop().unwrap();
    req.parameters.insert(
        "floors".to_string(),
        michelangelo_family::ParamValue::Int(7),
    );
    let err = FamilyPlanner::plan("b", &recipe, &sources, &lock, &[req]).unwrap_err();
    assert_eq!(err.code, FamilyErrorCode::ParameterInvalid);
}
