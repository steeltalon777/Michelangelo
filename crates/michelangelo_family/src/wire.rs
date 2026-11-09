//! Strict wire adapters from JSON documents to provisional DTOs.
//!
//! PROVISIONAL: accepts documents shaped like the GLM-1 `schemas/adr5`
//! documents (as understood by this stream) and maps them onto private DTOs.
//! The accepted tag namespace is the private one of this stream
//! (`michelangelo.family.<doc>/draft-0`); the GLM-1 tags
//! (`michelangelo.<doc>/draft-0.1`) are rejected, because these documents do
//! not satisfy the GLM-1 schemas and must not be dispatched as GLM-1
//! artifacts. Every accepted document must additionally declare the explicit
//! marker `provisional: true`.
//!
//! Unknown top-level keys are rejected (mirrors `additionalProperties:
//! false`), which gives wire-level negative coverage for F7 (scope-field
//! injection) and F10 (`allow_arbitrary_code: true`). This is NOT an approved
//! protocol parser until the contract gate lands.

use std::collections::BTreeMap;

use crate::errors::{FamilyError, FamilyErrorCode, FamilyResult};
use crate::json::Json;
use crate::model::{
    Capability, ConflictPolicy, DepRef, FamilyRecipe, Generator, GeneratorKind, HeightModel,
    ManifestKind, ManifestRef, ParamSpec, ParamValue, Randomness, RecipeLock, Scope, SocketProfile,
    SourceEntry, SourceRole, SourceSet, SourceUsage, VariantRequest, PROVISIONAL_MARKER_KEY,
};

fn obj<'a>(v: &'a Json, what: &str) -> FamilyResult<&'a BTreeMap<String, Json>> {
    v.as_object().ok_or_else(|| {
        FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            format!("{what} must be a JSON object"),
        )
    })
}

fn str_field(v: &Json, key: &str, what: &str) -> FamilyResult<String> {
    let m = obj(v, what)?;
    match m.get(key) {
        Some(Json::Str(s)) => Ok(s.clone()),
        Some(Json::Int(i)) => Ok(i.to_string()),
        _ => Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            format!("{what}: missing/invalid string field '{key}'"),
        )),
    }
}

fn int_field(v: &Json, key: &str, what: &str) -> FamilyResult<i64> {
    obj(v, what)?
        .get(key)
        .and_then(Json::as_i64)
        .ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                format!("{what}: missing/invalid integer field '{key}'"),
            )
        })
}

/// Rejects any key outside `allowed` — the behavioural analogue of
/// `additionalProperties: false`. Used to catch scope-widening injections
/// (`scope`, `validation`, `capabilities`, `allow_arbitrary_code`, ...) on
/// documents that must not carry them.
fn reject_unknown_keys(
    m: &BTreeMap<String, Json>,
    allowed: &[&str],
    what: &str,
) -> FamilyResult<()> {
    for key in m.keys() {
        if !allowed.contains(&key.as_str()) {
            // A document that tries to attach scope/capability/validation
            // controls is scope widening (F7), not merely malformed.
            let widening = matches!(
                key.as_str(),
                "scope"
                    | "validation"
                    | "capabilities"
                    | "allowed_roles"
                    | "protected_roles"
                    | "approval"
                    | "policy"
                    | "budget"
                    | "generator"
                    | "constraints"
            );
            return Err(FamilyError::new(
                if widening {
                    FamilyErrorCode::ScopeWidening
                } else {
                    FamilyErrorCode::SchemaInvalid
                },
                format!("{what}: unexpected field '{key}'"),
            ));
        }
    }
    Ok(())
}

fn is_lower_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The explicit provisional marker must be present and `true`.
///
/// A document without the marker does not declare its provisional status; a
/// document with `provisional: false` contradicts the provisional tag it
/// carries. Both are rejected — "not yet contracted" is a property of the
/// payload, not a comment in the docs.
fn require_provisional(m: &BTreeMap<String, Json>, what: &str) -> FamilyResult<()> {
    match m.get(PROVISIONAL_MARKER_KEY) {
        Some(Json::Bool(true)) => Ok(()),
        Some(Json::Bool(false)) => Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            format!(
                "{what}: '{PROVISIONAL_MARKER_KEY}=false' contradicts the provisional tag: this document is not an approved GLM-1 artifact"
            ),
        )),
        Some(_) => Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            format!("{what}: '{PROVISIONAL_MARKER_KEY}' must be a boolean"),
        )),
        None => Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            format!(
                "{what}: missing '{PROVISIONAL_MARKER_KEY}' marker: provisional documents must declare it explicitly"
            ),
        )),
    }
}

fn parse_sha256(v: &Json, what: &str) -> FamilyResult<String> {
    let s = str_field(v, "sha256", what)?;
    if is_lower_hex64(&s) {
        Ok(s)
    } else {
        Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            format!("{what}: 'sha256' must be 64 lowercase hex chars"),
        ))
    }
}

pub fn parse_source_set(doc: &Json) -> FamilyResult<SourceSet> {
    let m = obj(doc, "source_set")?;
    if m.get("schema").and_then(Json::as_str) != Some(crate::model::WIRE_SCHEMA_SOURCE_SET) {
        return Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "source_set: schema constant mismatch",
        ));
    }
    reject_unknown_keys(
        m,
        &[
            "schema",
            PROVISIONAL_MARKER_KEY,
            "source_set_id",
            "revision",
            "sources",
            "conflict_policy",
            "alignment_profile_sha256",
        ],
        "source_set",
    )?;
    require_provisional(m, "source_set")?;
    let sources_json = obj(doc, "source_set")?
        .get("sources")
        .and_then(Json::as_array)
        .ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "source_set: 'sources' must be an array",
            )
        })?
        .to_vec();

    let mut sources = Vec::new();
    for s in &sources_json {
        let sm = obj(s, "source entry")?;
        reject_unknown_keys(
            sm,
            &[
                "source_id",
                "asset_id",
                "manifest_ref",
                "roles",
                "usage",
                "allowed_parts",
                "skeleton_profile_sha256",
            ],
            "source entry",
        )?;
        let roles_json = sm.get("roles").and_then(Json::as_array).ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "source entry: 'roles' must be an array",
            )
        })?;
        let mut roles = Vec::new();
        for r in roles_json {
            let rs = r.as_str().ok_or_else(|| {
                FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    "source entry: role must be a string",
                )
            })?;
            roles.push(SourceRole::parse(rs)?); // unknown role string => ROLE_INVALID (F6)
        }
        let usage = SourceUsage::parse(str_field(s, "usage", "source entry")?.as_str())?;
        let manifest = obj(s, "source entry")?.get("manifest_ref").ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "source entry: missing 'manifest_ref'",
            )
        })?;
        let kind_str = str_field(manifest, "kind", "manifest_ref")?;
        let kind = match kind_str.as_str() {
            "source_snapshot" => ManifestKind::SourceSnapshot,
            "asset_revision" => ManifestKind::AssetRevision,
            other => {
                return Err(FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    format!("manifest_ref: unknown kind '{other}'"),
                ))
            }
        };
        let manifest_ref = ManifestRef {
            kind,
            id: str_field(manifest, "id", "manifest_ref")?,
            sha256: parse_sha256(manifest, "manifest_ref")?,
        };
        let allowed_parts = sm
            .get("allowed_parts")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str().map(|x| x.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        sources.push(SourceEntry {
            source_id: str_field(s, "source_id", "source entry")?,
            asset_id: str_field(s, "asset_id", "source entry")?,
            manifest: manifest_ref,
            roles,
            usage,
            allowed_parts,
            skeleton_profile_sha256: sm
                .get("skeleton_profile_sha256")
                .and_then(Json::as_str)
                .map(|x| x.to_string()),
        });
    }

    let conflict_str = str_field(doc, "conflict_policy", "source_set")?;
    let conflict_policy = match conflict_str.as_str() {
        "require_explicit_resolution" => ConflictPolicy::RequireExplicitResolution,
        "reject_conflict" => ConflictPolicy::RejectConflict,
        other => {
            return Err(FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                format!("unknown conflict_policy '{other}'"),
            ))
        }
    };

    let set = SourceSet {
        source_set_id: str_field(doc, "source_set_id", "source_set")?,
        revision: int_field(doc, "revision", "source_set")? as u64,
        sources,
        conflict_policy,
        alignment_profile_sha256: m
            .get("alignment_profile_sha256")
            .and_then(Json::as_str)
            .map(|x| x.to_string()),
    };
    set.validate()?;
    Ok(set)
}

pub fn parse_recipe(doc: &Json) -> FamilyResult<FamilyRecipe> {
    let m = obj(doc, "recipe")?;
    if m.get("schema").and_then(Json::as_str) != Some(crate::model::WIRE_SCHEMA_RECIPE) {
        return Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "recipe: schema constant mismatch",
        ));
    }
    // F10: explicit arbitrary-code opt-in is rejected before any other field
    // analysis so the diagnosis is unambiguous.
    if m.contains_key("allow_arbitrary_code") {
        let v = m.get("allow_arbitrary_code").and_then(Json::as_bool);
        if v == Some(true) {
            return Err(FamilyError::new(
                FamilyErrorCode::ArbitraryCode,
                "recipe declares allow_arbitrary_code=true (F10): arbitrary code is never plannable",
            ));
        }
        return Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "recipe: 'allow_arbitrary_code' is a constant-false field and must not be authored",
        ));
    }
    reject_unknown_keys(
        m,
        &[
            "schema",
            PROVISIONAL_MARKER_KEY,
            "recipe_id",
            "revision",
            "source_set_ref",
            "style_profile_sha256",
            "parameters",
            "generator",
            "scope",
            "constraints",
            "height_model",
            "sockets",
            "randomness",
            "budget",
            "dependencies",
        ],
        "recipe",
    )?;
    require_provisional(m, "recipe")?;

    let ss_ref = m.get("source_set_ref").ok_or_else(|| {
        FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "recipe: missing 'source_set_ref'",
        )
    })?;
    let params_json = m
        .get("parameters")
        .and_then(Json::as_object)
        .ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "recipe: missing 'parameters'",
            )
        })?;
    let mut parameters = BTreeMap::new();
    for (name, spec) in params_json {
        let sm = obj(spec, "parameter spec")?;
        let t = sm.get("type").and_then(Json::as_str).ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "parameter spec: missing 'type'",
            )
        })?;
        let spec = match t {
            "integer" => ParamSpec::Integer {
                min: int_field(spec, "min", "parameter spec")?,
                max: int_field(spec, "max", "parameter spec")?,
            },
            "number" => ParamSpec::Number {
                min: obj(spec, "parameter spec")?
                    .get("min")
                    .and_then(Json::as_f64)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "parameter spec: missing 'min'",
                        )
                    })?,
                max: obj(spec, "parameter spec")?
                    .get("max")
                    .and_then(Json::as_f64)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "parameter spec: missing 'max'",
                        )
                    })?,
            },
            "enum" => ParamSpec::Enum {
                values: sm
                    .get("values")
                    .and_then(Json::as_array)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "enum parameter: missing 'values'",
                        )
                    })?
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
            },
            "boolean" => ParamSpec::Boolean,
            other => {
                return Err(FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    format!("parameter '{name}': unknown type '{other}'"),
                ))
            }
        };
        parameters.insert(name.clone(), spec);
    }

    let gen = m.get("generator").ok_or_else(|| {
        FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "recipe: missing 'generator'",
        )
    })?;
    let gm = obj(gen, "generator")?;
    reject_unknown_keys(
        gm,
        &[
            "kind",
            "template_ref",
            "capabilities",
            "parameter_bindings",
            "source_inputs",
        ],
        "generator",
    )?;
    // V2–V4 kinds and unknown kinds are capability-denied here (F10-family
    // boundary: a generator is never a script host).
    let kind = GeneratorKind::parse(&str_field(gen, "kind", "generator")?)?;
    let mut capabilities = Vec::new();
    for c in gm
        .get("capabilities")
        .and_then(Json::as_array)
        .ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "generator: missing 'capabilities'",
            )
        })?
    {
        capabilities.push(Capability::parse(c.as_str().ok_or_else(|| {
            FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "capability must be a string",
            )
        })?)?);
    }
    let mut parameter_bindings = BTreeMap::new();
    if let Some(bindings) = gm.get("parameter_bindings").and_then(Json::as_object) {
        for (k, v) in bindings {
            parameter_bindings.insert(
                k.clone(),
                v.as_str()
                    .ok_or_else(|| {
                        FamilyError::new(FamilyErrorCode::SchemaInvalid, "binding must be a string")
                    })?
                    .to_string(),
            );
        }
    }
    let generator = Generator {
        kind,
        template_ref: str_field(gen, "template_ref", "generator")?,
        capabilities,
        parameter_bindings,
        source_inputs: gm
            .get("source_inputs")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    };

    let scope_json = m.get("scope").ok_or_else(|| {
        FamilyError::new(FamilyErrorCode::SchemaInvalid, "recipe: missing 'scope'")
    })?;
    let sm = obj(scope_json, "scope")?;
    reject_unknown_keys(
        sm,
        &["allowed_roles", "protected_roles", "allow_arbitrary_code"],
        "scope",
    )?;
    if let Some(v) = sm.get("allow_arbitrary_code") {
        if v.as_bool() == Some(true) {
            return Err(FamilyError::new(
                FamilyErrorCode::ArbitraryCode,
                "scope.allow_arbitrary_code=true (F10)",
            ));
        }
    }
    let scope = Scope {
        allowed_roles: sm
            .get("allowed_roles")
            .and_then(Json::as_array)
            .ok_or_else(|| {
                FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    "scope: missing 'allowed_roles'",
                )
            })?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        protected_roles: sm
            .get("protected_roles")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    };

    let mut constraints = Vec::new();
    if let Some(cs) = m.get("constraints").and_then(Json::as_array) {
        for c in cs {
            let cm = obj(c, "constraint")?;
            let kind_str = cm.get("kind").and_then(Json::as_str).ok_or_else(|| {
                FamilyError::new(FamilyErrorCode::SchemaInvalid, "constraint: missing 'kind'")
            })?;
            match kind_str {
                "param_pair_incompatible" => constraints.push(Constraint::from_wire(cm)?),
                "requires_socket" => constraints.push(Constraint::from_wire(cm)?),
                "maximum_dimension" => constraints.push(Constraint::from_wire(cm)?),
                other => {
                    return Err(FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        format!("unknown constraint kind '{other}'"),
                    ))
                }
            }
        }
    }

    let height_model = match m.get("height_model") {
        Some(Json::Object(hm)) => Some(HeightModel {
            floors_param: hm
                .get("floors_param")
                .and_then(Json::as_str)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        "height_model: missing 'floors_param'",
                    )
                })?
                .to_string(),
            height_per_floor_m: hm
                .get("height_per_floor_m")
                .and_then(Json::as_f64)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        "height_model: missing 'height_per_floor_m'",
                    )
                })?,
            roof_param: hm
                .get("roof_param")
                .and_then(Json::as_str)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        "height_model: missing 'roof_param'",
                    )
                })?
                .to_string(),
            roof_rise_m: hm
                .get("roof_rise_m")
                .and_then(Json::as_object)
                .map(|rm| {
                    rm.iter()
                        .filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f)))
                        .collect::<BTreeMap<String, f64>>()
                })
                .unwrap_or_default(),
        }),
        _ => None,
    };

    let sockets = match m.get("sockets") {
        Some(Json::Object(sp)) => Some(SocketProfile {
            profile_ref: sp
                .get("profile_ref")
                .and_then(Json::as_str)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        "sockets: missing 'profile_ref'",
                    )
                })?
                .to_string(),
            available_sockets: sp
                .get("available_sockets")
                .and_then(Json::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
        }),
        _ => None,
    };

    let rand = m.get("randomness").ok_or_else(|| {
        FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "recipe: missing 'randomness'",
        )
    })?;
    let randomness = Randomness {
        algorithm: str_field(rand, "algorithm", "randomness")?,
        derivation_version: str_field(rand, "derivation_version", "randomness")?,
        default_seed: int_field(rand, "default_seed", "randomness")? as u64,
    };

    let budget = m.get("budget").ok_or_else(|| {
        FamilyError::new(FamilyErrorCode::SchemaInvalid, "recipe: missing 'budget'")
    })?;
    let budget_val = crate::model::Budget {
        max_items_per_plan: int_field(budget, "max_items_per_plan", "budget")? as u64,
        max_combinations: int_field(budget, "max_combinations", "budget")? as u64,
    };

    let mut dependencies = Vec::new();
    if let Some(ds) = m.get("dependencies").and_then(Json::as_array) {
        for d in ds {
            dependencies.push(DepRef {
                id: str_field(d, "id", "dependency")?,
                revision: int_field(d, "revision", "dependency")? as u64,
                sha256: parse_sha256(d, "dependency")?,
            });
        }
    }

    let recipe = FamilyRecipe {
        recipe_id: str_field(doc, "recipe_id", "recipe")?,
        revision: int_field(doc, "revision", "recipe")? as u64,
        source_set_id: str_field(ss_ref, "source_set_id", "source_set_ref")?,
        source_set_revision: int_field(ss_ref, "revision", "source_set_ref")? as u64,
        source_set_sha256: parse_sha256(ss_ref, "source_set_ref")?,
        style_profile_sha256: m
            .get("style_profile_sha256")
            .and_then(Json::as_str)
            .map(|x| x.to_string()),
        parameters,
        generator,
        scope,
        constraints,
        height_model,
        sockets,
        randomness,
        budget: budget_val,
        dependencies,
    };
    recipe.validate()?;
    Ok(recipe)
}

// Helper enum imported for from_wire below.
use crate::model::Constraint;

trait ConstraintWire {
    fn from_wire(m: &BTreeMap<String, Json>) -> FamilyResult<Constraint>;
}

impl ConstraintWire for Constraint {
    fn from_wire(m: &BTreeMap<String, Json>) -> FamilyResult<Constraint> {
        let kind = m.get("kind").and_then(Json::as_str).unwrap_or("");
        Ok(match kind {
            "param_pair_incompatible" => Constraint::ParamPairIncompatible {
                param: m
                    .get("param")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'param'",
                        )
                    })?
                    .to_string(),
                value: m
                    .get("value")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'value'",
                        )
                    })?
                    .to_string(),
                forbids_param: m
                    .get("forbids_param")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'forbids_param'",
                        )
                    })?
                    .to_string(),
                forbids_value: m
                    .get("forbids_value")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'forbids_value'",
                        )
                    })?
                    .to_string(),
            },
            "requires_socket" => Constraint::RequiresSocket {
                param: m
                    .get("param")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'param'",
                        )
                    })?
                    .to_string(),
                value: m
                    .get("value")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'value'",
                        )
                    })?
                    .to_string(),
                socket_id: m
                    .get("socket_id")
                    .and_then(Json::as_str)
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "constraint: missing 'socket_id'",
                        )
                    })?
                    .to_string(),
            },
            "maximum_dimension" => Constraint::MaximumDimensionZ {
                value_m: m.get("value_m").and_then(Json::as_f64).ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        "constraint: missing 'value_m'",
                    )
                })?,
            },
            _ => {
                return Err(FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    format!("unknown constraint kind '{kind}'"),
                ))
            }
        })
    }
}

pub fn parse_lock(doc: &Json) -> FamilyResult<RecipeLock> {
    let m = obj(doc, "lock")?;
    if m.get("schema").and_then(Json::as_str) != Some(crate::model::WIRE_SCHEMA_LOCK) {
        return Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "lock: schema constant mismatch",
        ));
    }
    reject_unknown_keys(
        m,
        &[
            "schema",
            PROVISIONAL_MARKER_KEY,
            "lock_id",
            "recipe_ref",
            "source_set_sha256",
            "dependencies",
            "tool_versions",
            "prng",
            "pinned_profiles",
            "floating_latest_allowed",
        ],
        "lock",
    )?;
    require_provisional(m, "lock")?;
    // F12 on the wire: a lock that claims floating versions are allowed is invalid.
    if m.get("floating_latest_allowed").and_then(Json::as_bool) == Some(true) {
        return Err(FamilyError::new(
            FamilyErrorCode::FloatingVersion,
            "lock declares floating_latest_allowed=true (F12)",
        ));
    }
    let mut tool_versions = BTreeMap::new();
    if let Some(tv) = m.get("tool_versions").and_then(Json::as_object) {
        for (k, v) in tv {
            let version = v.as_str().ok_or_else(|| {
                FamilyError::new(
                    FamilyErrorCode::FloatingVersion,
                    "tool version must be a string",
                )
            })?;
            RecipeLock::check_tool_version(k, version)?; // rejects latest/*/ranges (F12)
            tool_versions.insert(k.clone(), version.to_string());
        }
    }
    let mut dependencies = Vec::new();
    if let Some(ds) = m.get("dependencies").and_then(Json::as_array) {
        for d in ds {
            dependencies.push(DepRef {
                id: str_field(d, "id", "lock dependency")?,
                revision: int_field(d, "revision", "lock dependency")? as u64,
                sha256: parse_sha256(d, "lock dependency")?,
            });
        }
    }
    let recipe_ref = m.get("recipe_ref").ok_or_else(|| {
        FamilyError::new(FamilyErrorCode::SchemaInvalid, "lock: missing 'recipe_ref'")
    })?;
    let prng = m
        .get("prng")
        .ok_or_else(|| FamilyError::new(FamilyErrorCode::SchemaInvalid, "lock: missing 'prng'"))?;
    let mut pinned_profiles = BTreeMap::new();
    if let Some(pp) = m.get("pinned_profiles").and_then(Json::as_object) {
        for (k, v) in pp {
            pinned_profiles.insert(
                k.clone(),
                v.as_str()
                    .ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SchemaInvalid,
                            "pinned profile must be a string",
                        )
                    })?
                    .to_string(),
            );
        }
    }
    let lock = RecipeLock {
        lock_id: str_field(doc, "lock_id", "lock")?,
        recipe_id: str_field(recipe_ref, "recipe_id", "recipe_ref")?,
        recipe_revision: int_field(recipe_ref, "recipe_revision", "recipe_ref")? as u64,
        recipe_sha256: {
            let s = str_field(recipe_ref, "recipe_sha256", "recipe_ref")?;
            if is_lower_hex64(&s) {
                s
            } else {
                return Err(FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    "lock: 'recipe_ref.recipe_sha256' must be 64 lowercase hex chars",
                ));
            }
        },
        source_set_sha256: {
            let s = str_field(doc, "source_set_sha256", "lock")?;
            if is_lower_hex64(&s) {
                s
            } else {
                return Err(FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    "lock: 'source_set_sha256' must be 64 lowercase hex chars",
                ));
            }
        },
        dependencies,
        tool_versions,
        prng: Randomness {
            algorithm: str_field(prng, "algorithm", "prng")?,
            derivation_version: str_field(prng, "derivation_version", "prng")?,
            default_seed: prng
                .as_object()
                .map(|pm| pm.get("default_seed").and_then(Json::as_i64).unwrap_or(0))
                .unwrap_or(0) as u64,
        },
        pinned_profiles,
    };
    lock.validate()?;
    Ok(lock)
}

pub fn parse_variant_request(doc: &Json) -> FamilyResult<VariantRequest> {
    let m = obj(doc, "variant_request")?;
    if m.get("schema").and_then(Json::as_str) != Some(crate::model::WIRE_SCHEMA_REQUEST) {
        return Err(FamilyError::new(
            FamilyErrorCode::SchemaInvalid,
            "variant_request: schema constant mismatch",
        ));
    }
    // F7: a variant request cannot carry scope/capability/validation controls.
    reject_unknown_keys(
        m,
        &[
            "schema",
            PROVISIONAL_MARKER_KEY,
            "item_id",
            "parameters",
            "purpose",
            "export_targets",
        ],
        "variant_request",
    )?;
    require_provisional(m, "variant_request")?;
    let mut parameters = BTreeMap::new();
    if let Some(pm) = m.get("parameters").and_then(Json::as_object) {
        for (k, v) in pm {
            parameters.insert(k.clone(), ParamValue::parse(v)?);
        }
    }
    Ok(VariantRequest {
        item_id: str_field(doc, "item_id", "variant_request")?,
        parameters,
        purpose: str_field(doc, "purpose", "variant_request")?,
        export_targets: m
            .get("export_targets")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    })
}
