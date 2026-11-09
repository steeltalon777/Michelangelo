//! The pinned "adr5 test house" fixture: a small single+multi-source family
//! (urban house, ADR §11.1 analogue) used by unit tests, negative tests and
//! the 15-variant dry plan example. All digests are computed from fixture
//! content at runtime; nothing here is an approved wire document.

use std::collections::BTreeMap;

use crate::model::{
    Budget, Capability, ConflictPolicy, DepRef, FamilyRecipe, Generator, GeneratorKind,
    HeightModel, ManifestKind, ManifestRef, ParamSpec, Randomness, RecipeLock, Scope,
    SocketProfile, SourceEntry, SourceRole, SourceSet, SourceUsage,
};
use crate::sha256::sha256_hex;

/// Fixture dependency documents (placeholder bodies hashed at runtime).
pub const DEP_TEMPLATE: &str = "fixture:house-generator template v2 body";
pub const DEP_ALIGNMENT: &str = "fixture:source-alignment correspondence v1 body";
pub const DEP_SOCKETS: &str = "fixture:wall-sockets profile v1 body";
pub const DEP_VALIDATION: &str = "fixture:house-validation profile v1 body";
pub const DEP_POLICY: &str = "fixture:family-owner-policy v1 body";
pub const DEP_STYLE: &str = "fixture:northern-port style profile v2 body";

/// Source manifest digests stand in for hashes a future Intake stream would
/// produce for the fixture payloads.
pub const PAYLOAD_MASTER: &str = "fixture:urban-house-master payload";
pub const PAYLOAD_ROOFS: &str = "fixture:port-roofs donor payload";
pub const PAYLOAD_STYLE_EXAMPLE: &str = "fixture:port-house-reference payload";
pub const PAYLOAD_HELDOUT: &str = "fixture:independent-check-model payload";

pub const SOURCE_SET_ID: &str = "urban-house-sources";
pub const RECIPE_ID: &str = "urban-house-family";

pub fn test_house_source_set() -> SourceSet {
    let mk = |_source_id: &str, asset_id: &str, payload: &str, kind: ManifestKind| ManifestRef {
        kind,
        id: asset_id.to_string(),
        sha256: sha256_hex(payload.as_bytes()),
    };
    SourceSet {
        source_set_id: SOURCE_SET_ID.to_string(),
        revision: 1,
        sources: vec![
            SourceEntry {
                source_id: "master".to_string(),
                asset_id: "urban-house-master".to_string(),
                manifest: mk(
                    "master",
                    "urban-house-master",
                    PAYLOAD_MASTER,
                    ManifestKind::AssetRevision,
                ),
                roles: vec![SourceRole::BaseCandidate],
                usage: SourceUsage::BuildInput,
                allowed_parts: vec![],
                skeleton_profile_sha256: None,
            },
            SourceEntry {
                source_id: "roof-donor".to_string(),
                asset_id: "port-roofs".to_string(),
                manifest: mk(
                    "roof-donor",
                    "port-roofs",
                    PAYLOAD_ROOFS,
                    ManifestKind::AssetRevision,
                ),
                roles: vec![SourceRole::ModuleDonor],
                usage: SourceUsage::BuildInput,
                allowed_parts: vec!["roof".to_string()],
                skeleton_profile_sha256: None,
            },
            SourceEntry {
                source_id: "style-example".to_string(),
                asset_id: "port-house-reference".to_string(),
                manifest: mk(
                    "style-example",
                    "port-house-reference",
                    PAYLOAD_STYLE_EXAMPLE,
                    ManifestKind::SourceSnapshot,
                ),
                roles: vec![SourceRole::StyleReference],
                usage: SourceUsage::ReferenceOnly,
                allowed_parts: vec![],
                skeleton_profile_sha256: None,
            },
            SourceEntry {
                source_id: "heldout-check".to_string(),
                asset_id: "independent-check-model".to_string(),
                manifest: mk(
                    "heldout-check",
                    "independent-check-model",
                    PAYLOAD_HELDOUT,
                    ManifestKind::SourceSnapshot,
                ),
                roles: vec![SourceRole::IndependentValidationReference],
                usage: SourceUsage::HeldOut,
                allowed_parts: vec![],
                skeleton_profile_sha256: None,
            },
        ],
        conflict_policy: ConflictPolicy::RequireExplicitResolution,
        alignment_profile_sha256: Some(sha256_hex(DEP_ALIGNMENT.as_bytes())),
    }
}

/// Test-house recipe:
/// - parameters: floors 1..3, bays 2..4, roof {gable,hip,flat},
///   palette {brick_dark,plaster_warm,timber_grey}, wear_permille 8 steps,
///   trim {none,carved,iron}
/// - incompatibilities: flat roof forbids carved trim; timber_grey palette
///   forbids flat roof; hip roof above 2 floors exceeds the Z budget.
/// - sockets: flat roof requires `roof_flat_socket`, hip requires
///   `roof_hip_socket`; the fixture profile deliberately does NOT contain
///   `roof_hip_socket` so that hip combinations are socket-mismatched and the
///   honest 15-combination demo exercises the matrix.
pub fn test_house_recipe(source_set_digest: &str) -> FamilyRecipe {
    FamilyRecipe {
        recipe_id: RECIPE_ID.to_string(),
        revision: 1,
        source_set_id: SOURCE_SET_ID.to_string(),
        source_set_revision: 1,
        source_set_sha256: source_set_digest.to_string(),
        style_profile_sha256: Some(sha256_hex(DEP_STYLE.as_bytes())),
        parameters: BTreeMap::from([
            ("floors".to_string(), ParamSpec::Integer { min: 1, max: 3 }),
            ("bays".to_string(), ParamSpec::Integer { min: 2, max: 4 }),
            (
                "roof".to_string(),
                ParamSpec::Enum {
                    values: vec!["gable".to_string(), "hip".to_string(), "flat".to_string()],
                },
            ),
            (
                "palette".to_string(),
                ParamSpec::Enum {
                    values: vec![
                        "brick_dark".to_string(),
                        "plaster_warm".to_string(),
                        "timber_grey".to_string(),
                    ],
                },
            ),
            // Wear as 8 discrete steps keeps the cartesian product enumerable
            // (3*3*3*3*8*3 = 1944 <= budget.max_combinations = 2000) and the
            // plan fingerprint float-free.
            (
                "wear_permille".to_string(),
                ParamSpec::Enum {
                    values: vec![
                        "0".to_string(),
                        "50".to_string(),
                        "100".to_string(),
                        "150".to_string(),
                        "200".to_string(),
                        "250".to_string(),
                        "300".to_string(),
                        "350".to_string(),
                    ],
                },
            ),
            (
                "trim".to_string(),
                ParamSpec::Enum {
                    values: vec!["none".to_string(), "carved".to_string(), "iron".to_string()],
                },
            ),
        ]),
        generator: Generator {
            kind: GeneratorKind::RegisteredBlenderTemplateV0,
            template_ref: "house-generator".to_string(),
            capabilities: vec![Capability::ModuleAssembly, Capability::MaterialVariation],
            parameter_bindings: BTreeMap::from([
                ("floors".to_string(), "facade.floor_count".to_string()),
                ("bays".to_string(), "facade.bay_count".to_string()),
                ("roof".to_string(), "roof.module_choice".to_string()),
                (
                    "palette".to_string(),
                    "materials.palette_choice".to_string(),
                ),
                (
                    "wear_permille".to_string(),
                    "materials.wear_amount".to_string(),
                ),
                ("trim".to_string(), "ornaments.trim_choice".to_string()),
            ]),
            source_inputs: vec![
                "master".to_string(),
                "roof-donor".to_string(),
                "style-example".to_string(),
            ],
        },
        scope: Scope {
            allowed_roles: vec![
                "facade".to_string(),
                "roof".to_string(),
                "ornaments".to_string(),
                "materials".to_string(),
            ],
            protected_roles: vec!["entry_anchor".to_string()],
        },
        constraints: vec![
            crate::model::Constraint::ParamPairIncompatible {
                param: "roof".to_string(),
                value: "flat".to_string(),
                forbids_param: "trim".to_string(),
                forbids_value: "carved".to_string(),
            },
            crate::model::Constraint::ParamPairIncompatible {
                param: "palette".to_string(),
                value: "timber_grey".to_string(),
                forbids_param: "roof".to_string(),
                forbids_value: "flat".to_string(),
            },
            crate::model::Constraint::RequiresSocket {
                param: "roof".to_string(),
                value: "flat".to_string(),
                socket_id: "roof_flat_socket".to_string(),
            },
            crate::model::Constraint::RequiresSocket {
                param: "roof".to_string(),
                value: "hip".to_string(),
                socket_id: "roof_hip_socket".to_string(),
            },
            crate::model::Constraint::MaximumDimensionZ { value_m: 10.5 },
        ],
        height_model: Some(HeightModel {
            floors_param: "floors".to_string(),
            height_per_floor_m: 3.2,
            roof_param: "roof".to_string(),
            roof_rise_m: BTreeMap::from([
                ("gable".to_string(), 1.8),
                ("hip".to_string(), 2.2),
                ("flat".to_string(), 0.4),
            ]),
        }),
        sockets: Some(SocketProfile {
            profile_ref: "wall-sockets".to_string(),
            // Deliberately absent: roof_hip_socket (see module docs).
            available_sockets: vec!["roof_flat_socket".to_string()],
        }),
        randomness: Randomness {
            algorithm: crate::plan::SEED_DERIVATION_ALGORITHM.to_string(),
            derivation_version: crate::plan::SEED_DERIVATION_VERSION.to_string(),
            default_seed: 1742,
        },
        budget: Budget {
            max_items_per_plan: 30,
            max_combinations: 2000,
        },
        dependencies: vec![
            DepRef {
                id: "facade-kit".to_string(),
                revision: 3,
                sha256: sha256_hex(b"fixture:facade-kit v3 body"),
            },
            DepRef {
                id: "house-generator".to_string(),
                revision: 2,
                sha256: sha256_hex(DEP_TEMPLATE.as_bytes()),
            },
            DepRef {
                id: "house-validation".to_string(),
                revision: 1,
                sha256: sha256_hex(DEP_VALIDATION.as_bytes()),
            },
            DepRef {
                id: "family-owner-policy".to_string(),
                revision: 1,
                sha256: sha256_hex(DEP_POLICY.as_bytes()),
            },
        ],
    }
}

pub fn test_house_lock(recipe: &FamilyRecipe, sources: &SourceSet) -> RecipeLock {
    let mut dependencies: Vec<DepRef> = recipe.dependencies.clone();
    for s in &sources.sources {
        dependencies.push(DepRef {
            id: format!("source:{}", s.source_id),
            revision: 1,
            sha256: s.manifest.sha256.clone(),
        });
    }
    RecipeLock {
        lock_id: "urban-house-family-lock-r1".to_string(),
        recipe_id: recipe.recipe_id.clone(),
        recipe_revision: recipe.revision,
        recipe_sha256: recipe.digest(),
        source_set_sha256: sources.digest(),
        dependencies,
        tool_versions: BTreeMap::from([
            ("blender".to_string(), "4.2.3".to_string()),
            ("michelangelo".to_string(), "0.1.0".to_string()),
        ]),
        prng: recipe.randomness.clone(),
        pinned_profiles: BTreeMap::from([(
            "style_profile_sha256".to_string(),
            recipe.style_profile_sha256.clone().unwrap_or_default(),
        )]),
    }
}

/// Full consistent triple, digests cross-linked.
pub fn test_house() -> (SourceSet, FamilyRecipe, RecipeLock) {
    let sources = test_house_source_set();
    let recipe = test_house_recipe(&sources.digest());
    let lock = test_house_lock(&recipe, &sources);
    (sources, recipe, lock)
}
