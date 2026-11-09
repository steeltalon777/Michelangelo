//! PROVISIONAL private DTOs for ADR 0005 Family Builder (GLM-4 stream).
//!
//! **Status: draft-0, provisional until the GLM-1 contract gate lands.** These
//! types mirror the shape of `schemas/adr5/*.schema.json` (GLM-1 worktree
//! `feat/adr5-contracts`) but are NOT the approved wire protocol and do not
//! satisfy the GLM-1 schemas. Round 2 of the dispatch will reconcile them with
//! the contract gate.
//!
//! **Own tag namespace (finding L).** The documents emitted here deliberately
//! carry *private* tags of the form `michelangelo.family.<doc>/draft-0` and
//! never the GLM-1 tags `michelangelo.<doc>/draft-0.1`. The GLM-1 tags name
//! the approved contract; reusing them for documents that do not satisfy the
//! GLM-1 schemas would make a tag-dispatching consumer pick these artifacts
//! up as GLM-1 documents and fail on them. Every document additionally
//! carries the explicit marker field `provisional: true`, so "not yet
//! contracted" is visible in the payload and not only in the tag.
//!
//! Pure planning only: no Blender calls, no publication, no self-approval.

use std::collections::BTreeMap;

use crate::errors::{FamilyError, FamilyErrorCode, FamilyResult};
use crate::json::Json;
use crate::sha256::sha256_hex;

/// Provisional wire tags of THIS stream. Private namespace, never GLM-1.
pub const WIRE_SCHEMA_SOURCE_SET: &str = "michelangelo.family.source-set/draft-0";
pub const WIRE_SCHEMA_RECIPE: &str = "michelangelo.family.family-recipe/draft-0";
pub const WIRE_SCHEMA_LOCK: &str = "michelangelo.family.recipe-lock/draft-0";
pub const WIRE_SCHEMA_REQUEST: &str = "michelangelo.family.variant-request/draft-0";
pub const WIRE_SCHEMA_BATCH_PLAN: &str = "michelangelo.family.batch-plan/draft-0";

/// Key of the explicit provisional marker carried by every emitted document.
pub const PROVISIONAL_MARKER_KEY: &str = "provisional";

/// The marker value every emitted document carries: these artifacts are
/// provisional until round 2 reconciles them with the GLM-1 contract.
pub fn provisional_marker() -> (String, Json) {
    (PROVISIONAL_MARKER_KEY.to_string(), Json::Bool(true))
}

/// Six closed source roles (ADR 0005 §4.4 / GLM-1 role enum, OP-5). PROVISIONAL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceRole {
    BaseCandidate,
    ModuleDonor,
    VariationExemplar,
    StyleReference,
    GeometricConstraintReference,
    IndependentValidationReference,
}

impl SourceRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceRole::BaseCandidate => "base_candidate",
            SourceRole::ModuleDonor => "module_donor",
            SourceRole::VariationExemplar => "variation_exemplar",
            SourceRole::StyleReference => "style_reference",
            SourceRole::GeometricConstraintReference => "geometric_constraint_reference",
            SourceRole::IndependentValidationReference => "independent_validation_reference",
        }
    }

    pub fn parse(s: &str) -> FamilyResult<Self> {
        match s {
            "base_candidate" => Ok(SourceRole::BaseCandidate),
            "module_donor" => Ok(SourceRole::ModuleDonor),
            "variation_exemplar" => Ok(SourceRole::VariationExemplar),
            "style_reference" => Ok(SourceRole::StyleReference),
            "geometric_constraint_reference" => Ok(SourceRole::GeometricConstraintReference),
            "independent_validation_reference" => Ok(SourceRole::IndependentValidationReference),
            _ => Err(FamilyError::new(
                FamilyErrorCode::RoleInvalid,
                format!("unknown source role '{s}'"),
            )),
        }
    }

    /// Roles that participate in building a variant.
    pub fn is_build_role(&self) -> bool {
        matches!(
            self,
            SourceRole::BaseCandidate
                | SourceRole::ModuleDonor
                | SourceRole::VariationExemplar
                | SourceRole::GeometricConstraintReference
        )
    }
}

/// Allowed usage of one source; `HeldOut` is separated from generator inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceUsage {
    BuildInput,
    ReferenceOnly,
    HeldOut,
}

impl SourceUsage {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceUsage::BuildInput => "build_input",
            SourceUsage::ReferenceOnly => "reference_only",
            SourceUsage::HeldOut => "held_out",
        }
    }

    pub fn parse(s: &str) -> FamilyResult<Self> {
        match s {
            "build_input" => Ok(SourceUsage::BuildInput),
            "reference_only" => Ok(SourceUsage::ReferenceOnly),
            "held_out" => Ok(SourceUsage::HeldOut),
            _ => Err(FamilyError::new(
                FamilyErrorCode::RoleInvalid,
                format!("unknown source usage '{s}'"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    SourceSnapshot,
    AssetRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRef {
    pub kind: ManifestKind,
    pub id: String,
    pub sha256: String,
}

/// One immutable source revision with explicit roles and usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    pub source_id: String,
    pub asset_id: String,
    pub manifest: ManifestRef,
    pub roles: Vec<SourceRole>,
    pub usage: SourceUsage,
    /// Explicit parts a module donor may contribute; absence is not permission.
    pub allowed_parts: Vec<String>,
    /// §11.2 analogue: pinned skeleton/rest-bind digest for skinned sources.
    pub skeleton_profile_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    RequireExplicitResolution,
    RejectConflict,
}

/// Immutable list of source revisions. PROVISIONAL mirror of the GLM-1
/// `source_set` shape, carried under the private tag
/// `michelangelo.family.source-set/draft-0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSet {
    pub source_set_id: String,
    pub revision: u64,
    pub sources: Vec<SourceEntry>,
    pub conflict_policy: ConflictPolicy,
    pub alignment_profile_sha256: Option<String>,
}

impl SourceSet {
    /// Structural + role validation. Enforces the GLM-1 held-out invariant and
    /// donor `allowed_parts` rule (F6), duplicate identity and skeleton
    /// compatibility (§11.2 analogue).
    pub fn validate(&self) -> FamilyResult<()> {
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.sources {
            if !seen.insert(s.source_id.as_str()) {
                return Err(FamilyError::new(
                    FamilyErrorCode::DuplicateIdentity,
                    format!("duplicate source_id '{}'", s.source_id),
                ));
            }
            if s.roles.is_empty() {
                return Err(FamilyError::new(
                    FamilyErrorCode::RoleInvalid,
                    format!("source '{}' has no roles", s.source_id),
                ));
            }
            let donor = s.roles.contains(&SourceRole::ModuleDonor);
            // Held-out invariant (GLM-1 source_set allOf): the validation
            // reference role must be held_out so it cannot feed the generator.
            // Checked first: role/usage mixing is the core leakage.
            if s.roles
                .contains(&SourceRole::IndependentValidationReference)
                && s.usage != SourceUsage::HeldOut
            {
                return Err(FamilyError::new(
                    FamilyErrorCode::HeldoutLeakage,
                    format!(
                        "source '{}' carries independent_validation_reference but usage is not held_out (F6/F9 analogue)",
                        s.source_id
                    ),
                ));
            }
            // A held-out source must not carry any build role either.
            if s.usage == SourceUsage::HeldOut
                && s.roles
                    .iter()
                    .any(|r| *r != SourceRole::IndependentValidationReference)
            {
                return Err(FamilyError::new(
                    FamilyErrorCode::HeldoutLeakage,
                    format!(
                        "held_out source '{}' also carries non-validation role(s): held-out resources are separated from generator inputs",
                        s.source_id
                    ),
                ));
            }
            if donor && s.allowed_parts.is_empty() {
                return Err(FamilyError::new(
                    FamilyErrorCode::RoleInvalid,
                    format!(
                        "module_donor '{}' missing allowed_parts (F6): absence of a part grant is not a permission",
                        s.source_id
                    ),
                ));
            }
            // Role/usage coherence.
            for role in &s.roles {
                match role {
                    SourceRole::BaseCandidate
                    | SourceRole::ModuleDonor
                    | SourceRole::VariationExemplar
                    | SourceRole::GeometricConstraintReference => {
                        if s.usage != SourceUsage::BuildInput {
                            return Err(FamilyError::new(
                                FamilyErrorCode::RoleInvalid,
                                format!(
                                    "source '{}' role {} requires usage build_input, got {}",
                                    s.source_id,
                                    role.as_str(),
                                    s.usage.as_str()
                                ),
                            ));
                        }
                    }
                    SourceRole::StyleReference => {
                        if s.usage != SourceUsage::ReferenceOnly {
                            return Err(FamilyError::new(
                                FamilyErrorCode::RoleInvalid,
                                format!(
                                    "source '{}' style_reference must be reference_only (a reference does not license importing geometry)",
                                    s.source_id
                                ),
                            ));
                        }
                    }
                    SourceRole::IndependentValidationReference => {}
                }
            }
        }
        // Skinned build inputs must share one skeleton profile (§11.2 analogue).
        let mut skeletons: Vec<&str> = self
            .sources
            .iter()
            .filter(|s| {
                s.roles.iter().any(|r| r.is_build_role()) && s.skeleton_profile_sha256.is_some()
            })
            .filter_map(|s| s.skeleton_profile_sha256.as_deref())
            .collect();
        skeletons.sort_unstable();
        if let (Some(first), Some(last)) = (skeletons.first(), skeletons.last()) {
            if first != last {
                return Err(FamilyError::new(
                    FamilyErrorCode::ForeignSkeleton,
                    format!(
                        "skinned build inputs declare different skeleton profiles ({first} vs {last}); a foreign skeleton is not auto-compatible"
                    ),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Json {
        Json::Object(
            [
                (
                    "schema".to_string(),
                    Json::Str(WIRE_SCHEMA_SOURCE_SET.to_string()),
                ),
                provisional_marker(),
                (
                    "source_set_id".to_string(),
                    Json::Str(self.source_set_id.clone()),
                ),
                ("revision".to_string(), Json::Int(self.revision as i64)),
                (
                    "sources".to_string(),
                    Json::Array(
                        self.sources
                            .iter()
                            .map(|s| {
                                Json::Object(BTreeMap::from([
                                    ("source_id".to_string(), Json::Str(s.source_id.clone())),
                                    ("asset_id".to_string(), Json::Str(s.asset_id.clone())),
                                    (
                                        "manifest_ref".to_string(),
                                        Json::Object(BTreeMap::from([
                                            (
                                                "kind".to_string(),
                                                Json::Str(
                                                    match s.manifest.kind {
                                                        ManifestKind::SourceSnapshot => {
                                                            "source_snapshot"
                                                        }
                                                        ManifestKind::AssetRevision => {
                                                            "asset_revision"
                                                        }
                                                    }
                                                    .to_string(),
                                                ),
                                            ),
                                            ("id".to_string(), Json::Str(s.manifest.id.clone())),
                                            (
                                                "sha256".to_string(),
                                                Json::Str(s.manifest.sha256.clone()),
                                            ),
                                        ])),
                                    ),
                                    (
                                        "roles".to_string(),
                                        Json::Array(
                                            s.roles
                                                .iter()
                                                .map(|r| Json::Str(r.as_str().to_string()))
                                                .collect(),
                                        ),
                                    ),
                                    ("usage".to_string(), Json::Str(s.usage.as_str().to_string())),
                                    (
                                        "allowed_parts".to_string(),
                                        Json::Array(
                                            s.allowed_parts
                                                .iter()
                                                .map(|p| Json::Str(p.clone()))
                                                .collect(),
                                        ),
                                    ),
                                    (
                                        "skeleton_profile_sha256".to_string(),
                                        s.skeleton_profile_sha256
                                            .clone()
                                            .map(Json::Str)
                                            .unwrap_or(Json::Null),
                                    ),
                                ]))
                            })
                            .collect(),
                    ),
                ),
                (
                    "conflict_policy".to_string(),
                    Json::Str(
                        match self.conflict_policy {
                            ConflictPolicy::RequireExplicitResolution => {
                                "require_explicit_resolution"
                            }
                            ConflictPolicy::RejectConflict => "reject_conflict",
                        }
                        .to_string(),
                    ),
                ),
                (
                    "alignment_profile_sha256".to_string(),
                    self.alignment_profile_sha256
                        .clone()
                        .map(Json::Str)
                        .unwrap_or(Json::Null),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }

    /// Canonical digest of this source set (includes role/usage/hashes).
    pub fn digest(&self) -> String {
        sha256_hex(self.to_json().to_canonical().as_bytes())
    }
}

/// Typed parameter spec. `Number` carries f64 values hashed via Rust
/// shortest-round-trip decimal (determinism pinned to `derivation_version`);
/// the bundled fixture uses integer per-mille wear to avoid float hashing.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamSpec {
    Integer { min: i64, max: i64 },
    Number { min: f64, max: f64 },
    Enum { values: Vec<String> },
    Boolean,
}

impl ParamSpec {
    pub fn type_name(&self) -> &'static str {
        match self {
            ParamSpec::Integer { .. } => "integer",
            ParamSpec::Number { .. } => "number",
            ParamSpec::Enum { .. } => "enum",
            ParamSpec::Boolean => "boolean",
        }
    }

    /// Number of distinct values in the cartesian product (bounded by combinatorial cap).
    pub fn cardinality(&self) -> FamilyResult<u64> {
        match self {
            ParamSpec::Integer { min, max } => Ok((*max - *min + 1).max(0) as u64),
            ParamSpec::Enum { values } => Ok(values.len() as u64),
            ParamSpec::Boolean => Ok(2),
            // Numbers are dense; they must be stepped by the caller (fixture
            // uses per-mille integers). A dense number param is not
            // enumerable, so it caps the product.
            ParamSpec::Number { .. } => Err(FamilyError::new(
                FamilyErrorCode::CombinatorialCapExceeded,
                "dense 'number' parameter cannot be enumerated; use integer/enum/boolean parameters for combination planning",
            )),
        }
    }
}

/// A single resolved parameter value of a variant request.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Int(i64),
    Num(f64),
    Text(String),
    Bool(bool),
}

impl ParamValue {
    pub fn to_json(&self) -> Json {
        match self {
            ParamValue::Int(i) => Json::Int(*i),
            ParamValue::Num(x) => Json::Float(*x),
            ParamValue::Text(s) => Json::Str(s.clone()),
            ParamValue::Bool(b) => Json::Bool(*b),
        }
    }

    pub fn parse(v: &Json) -> FamilyResult<Self> {
        Ok(match v {
            Json::Int(i) => ParamValue::Int(*i),
            Json::Float(f) => ParamValue::Num(*f),
            Json::Str(s) => ParamValue::Text(s.clone()),
            Json::Bool(b) => ParamValue::Bool(*b),
            _ => {
                return Err(FamilyError::new(
                    FamilyErrorCode::ParameterInvalid,
                    "parameter value must be integer/number/string/boolean",
                ))
            }
        })
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            ParamValue::Int(_) => "integer",
            ParamValue::Num(_) => "number",
            ParamValue::Text(_) => "string",
            ParamValue::Bool(_) => "boolean",
        }
    }

    /// Human-readable canonical spelling (used in fingerprints and diagnostics).
    pub fn canonical(&self) -> String {
        match self {
            ParamValue::Int(i) => i.to_string(),
            ParamValue::Num(x) => {
                if *x == x.trunc() && x.abs() < 1e15 {
                    format!("{x:.1}")
                } else {
                    format!("{x}")
                }
            }
            ParamValue::Text(s) => s.clone(),
            ParamValue::Bool(b) => b.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorKind {
    /// V0/V1: registered (owner-reviewed) Blender template — material/palette/
    /// wear presets and assembly over registered modules/sockets.
    RegisteredBlenderTemplateV0,
    /// V0: a fixed enumerated set of prepared static variants.
    StaticVariantSetV0,
}

impl GeneratorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            GeneratorKind::RegisteredBlenderTemplateV0 => "registered_blender_template_v0",
            GeneratorKind::StaticVariantSetV0 => "static_variant_set_v0",
        }
    }

    /// V2–V4 kinds are capability-denied at planning time (no implementation).
    pub fn parse(s: &str) -> FamilyResult<Self> {
        match s {
            "registered_blender_template_v0" => Ok(GeneratorKind::RegisteredBlenderTemplateV0),
            "static_variant_set_v0" => Ok(GeneratorKind::StaticVariantSetV0),
            // Explicit deferred kinds make the denial honest and specific.
            "parametric_modifier_v2" | "rule_based_construction_v3" | "agent_proposal_v4" => {
                Err(FamilyError::new(
                    FamilyErrorCode::CapabilityDenied,
                    format!(
                        "generator kind '{s}' is a V2-V4 capability: capability-denied at planning, not implemented"
                    ),
                ))
            }
            _ => Err(FamilyError::new(
                FamilyErrorCode::CapabilityDenied,
                format!("unknown generator kind '{s}'"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ModuleAssembly,
    MaterialVariation,
    BoundedGeometryNodes,
    StaticVariants,
}

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::ModuleAssembly => "module_assembly",
            Capability::MaterialVariation => "material_variation",
            Capability::BoundedGeometryNodes => "bounded_geometry_nodes",
            Capability::StaticVariants => "static_variants",
        }
    }

    pub fn parse(s: &str) -> FamilyResult<Self> {
        match s {
            "module_assembly" => Ok(Capability::ModuleAssembly),
            "material_variation" => Ok(Capability::MaterialVariation),
            "bounded_geometry_nodes" => Ok(Capability::BoundedGeometryNodes),
            "static_variants" => Ok(Capability::StaticVariants),
            _ => Err(FamilyError::new(
                FamilyErrorCode::CapabilityDenied,
                format!("unknown capability '{s}'"),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Generator {
    pub kind: GeneratorKind,
    pub template_ref: String,
    pub capabilities: Vec<Capability>,
    /// Typed parameter bindings `param -> "role.path"` (role segment checked
    /// against allowed/protected roles; no property-path escape hatch).
    pub parameter_bindings: BTreeMap<String, String>,
    /// Source ids the template consumes; held-out sources are rejected at plan time.
    pub source_inputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub allowed_roles: Vec<String>,
    pub protected_roles: Vec<String>,
    // `allow_arbitrary_code` is intentionally ABSENT: it is constant false.
    // Its presence on the wire is rejected (F10), see model::wire.
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    /// Pairwise typed incompatibility from the explicit matrix; no silent averaging.
    ParamPairIncompatible {
        param: String,
        value: String,
        forbids_param: String,
        forbids_value: String,
    },
    /// A chosen module value requires a socket that must exist in the socket profile.
    RequiresSocket {
        param: String,
        value: String,
        socket_id: String,
    },
    /// Maximum Z extent computed from the declared height model.
    MaximumDimensionZ { value_m: f64 },
}

/// Test-house planning dimension model (provisional, V0/V1 static families
/// only): height = floors_param * height_per_floor_m + roof rise.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightModel {
    pub floors_param: String,
    pub height_per_floor_m: f64,
    pub roof_param: String,
    pub roof_rise_m: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Randomness {
    /// e.g. "sha256-first8be(michelangelo/family-seed-v1, lock_digest, item_id)".
    pub algorithm: String,
    pub derivation_version: String,
    pub default_seed: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    pub max_items_per_plan: u64,
    /// Hard cap on the cartesian product size considered for planning.
    pub max_combinations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepRef {
    pub id: String,
    pub revision: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SocketProfile {
    pub profile_ref: String,
    pub available_sockets: Vec<String>,
}

/// Immutable recipe describing the ALLOWED SET of results (not a program, not
/// geometry). PROVISIONAL mirror of the GLM-1 `family_recipe` shape, carried
/// under the private tag `michelangelo.family.family-recipe/draft-0`.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyRecipe {
    pub recipe_id: String,
    pub revision: u64,
    pub source_set_id: String,
    pub source_set_revision: u64,
    pub source_set_sha256: String,
    pub style_profile_sha256: Option<String>,
    pub parameters: BTreeMap<String, ParamSpec>,
    pub generator: Generator,
    pub scope: Scope,
    pub constraints: Vec<Constraint>,
    pub height_model: Option<HeightModel>,
    pub sockets: Option<SocketProfile>,
    pub randomness: Randomness,
    pub budget: Budget,
    pub dependencies: Vec<DepRef>,
}

impl FamilyRecipe {
    /// Recipe validation: scope sanity, binding role checks (F7/F9 analogue),
    /// no arbitrary code paths (F10), closed capability set.
    pub fn validate(&self) -> FamilyResult<()> {
        if self.scope.allowed_roles.is_empty() {
            return Err(FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "scope.allowed_roles must not be empty",
            ));
        }
        let allowed: std::collections::BTreeSet<&str> = self
            .scope
            .allowed_roles
            .iter()
            .map(|s| s.as_str())
            .collect();
        for p in &self.scope.protected_roles {
            if allowed.contains(p.as_str()) {
                return Err(FamilyError::new(
                    FamilyErrorCode::ScopeWidening,
                    format!(
                        "role '{p}' appears in both allowed_roles and protected_roles: a protected role must not be simultaneously modifiable"
                    ),
                ));
            }
        }
        // Typed parameter bindings: `param -> "role.path"`.
        for (param, binding) in &self.generator.parameter_bindings {
            if !self.parameters.contains_key(param) {
                return Err(FamilyError::new(
                    FamilyErrorCode::SchemaInvalid,
                    format!("parameter_bindings references unknown parameter '{param}'"),
                ));
            }
            let segments: Vec<&str> = binding.split('.').collect();
            if segments.len() != 2
                || segments.iter().any(|s| {
                    s.is_empty()
                        || !s
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                })
            {
                return Err(FamilyError::new(
                    FamilyErrorCode::ArbitraryCode,
                    format!(
                        "binding '{binding}' is not a strict 'role.token' path: free property paths, scripts and exec targets are rejected (F10)"
                    ),
                ));
            }
            // Script/exec host vocabulary never appears in typed bindings (F10).
            const FORBIDDEN: [&str; 7] = [
                "exec", "script", "python", "eval", "shell", "command", "code",
            ];
            if segments.iter().any(|s| FORBIDDEN.contains(s)) {
                return Err(FamilyError::new(
                    FamilyErrorCode::ArbitraryCode,
                    format!("binding '{binding}' names a script/exec host: recipes never execute arbitrary code (F10)"),
                ));
            }
            let role = segments[0];
            if self.scope.protected_roles.iter().any(|p| p == role) {
                return Err(FamilyError::new(
                    FamilyErrorCode::ProtectedRoleTarget,
                    format!(
                        "binding '{binding}' targets protected role '{role}' (F9 analogue): effective changes to protected parts are not plannable"
                    ),
                ));
            }
            if !allowed.contains(role) {
                return Err(FamilyError::new(
                    FamilyErrorCode::ScopeWidening,
                    format!(
                        "binding '{binding}' targets role '{role}' outside scope.allowed_roles (F7)"
                    ),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Json {
        let mut params = BTreeMap::new();
        for (name, spec) in &self.parameters {
            let spec_json = match spec {
                ParamSpec::Integer { min, max } => Json::Object(BTreeMap::from([
                    ("type".to_string(), Json::Str("integer".to_string())),
                    ("min".to_string(), Json::Int(*min)),
                    ("max".to_string(), Json::Int(*max)),
                ])),
                ParamSpec::Number { min, max } => Json::Object(BTreeMap::from([
                    ("type".to_string(), Json::Str("number".to_string())),
                    ("min".to_string(), Json::Float(*min)),
                    ("max".to_string(), Json::Float(*max)),
                ])),
                ParamSpec::Enum { values } => Json::Object(BTreeMap::from([
                    ("type".to_string(), Json::Str("enum".to_string())),
                    (
                        "values".to_string(),
                        Json::Array(values.iter().map(|v| Json::Str(v.clone())).collect()),
                    ),
                ])),
                ParamSpec::Boolean => Json::Object(BTreeMap::from([(
                    "type".to_string(),
                    Json::Str("boolean".to_string()),
                )])),
            };
            params.insert(name.clone(), spec_json);
        }

        let constraints = self
            .constraints
            .iter()
            .map(|c| match c {
                Constraint::ParamPairIncompatible {
                    param,
                    value,
                    forbids_param,
                    forbids_value,
                } => Json::Object(BTreeMap::from([
                    (
                        "kind".to_string(),
                        Json::Str("param_pair_incompatible".to_string()),
                    ),
                    ("param".to_string(), Json::Str(param.clone())),
                    ("value".to_string(), Json::Str(value.clone())),
                    (
                        "forbids_param".to_string(),
                        Json::Str(forbids_param.clone()),
                    ),
                    (
                        "forbids_value".to_string(),
                        Json::Str(forbids_value.clone()),
                    ),
                ])),
                Constraint::RequiresSocket {
                    param,
                    value,
                    socket_id,
                } => Json::Object(BTreeMap::from([
                    ("kind".to_string(), Json::Str("requires_socket".to_string())),
                    ("param".to_string(), Json::Str(param.clone())),
                    ("value".to_string(), Json::Str(value.clone())),
                    ("socket_id".to_string(), Json::Str(socket_id.clone())),
                ])),
                Constraint::MaximumDimensionZ { value_m } => Json::Object(BTreeMap::from([
                    (
                        "kind".to_string(),
                        Json::Str("maximum_dimension".to_string()),
                    ),
                    ("axis".to_string(), Json::Str("Z".to_string())),
                    ("value_m".to_string(), Json::Float(*value_m)),
                ])),
            })
            .collect();

        Json::Object(BTreeMap::from([
            (
                "schema".to_string(),
                Json::Str(WIRE_SCHEMA_RECIPE.to_string()),
            ),
            provisional_marker(),
            ("recipe_id".to_string(), Json::Str(self.recipe_id.clone())),
            ("revision".to_string(), Json::Int(self.revision as i64)),
            (
                "source_set_ref".to_string(),
                Json::Object(BTreeMap::from([
                    (
                        "source_set_id".to_string(),
                        Json::Str(self.source_set_id.clone()),
                    ),
                    (
                        "revision".to_string(),
                        Json::Int(self.source_set_revision as i64),
                    ),
                    (
                        "sha256".to_string(),
                        Json::Str(self.source_set_sha256.clone()),
                    ),
                ])),
            ),
            (
                "style_profile_sha256".to_string(),
                self.style_profile_sha256
                    .clone()
                    .map(Json::Str)
                    .unwrap_or(Json::Null),
            ),
            ("parameters".to_string(), Json::Object(params)),
            (
                "generator".to_string(),
                Json::Object(BTreeMap::from([
                    (
                        "kind".to_string(),
                        Json::Str(self.generator.kind.as_str().to_string()),
                    ),
                    (
                        "template_ref".to_string(),
                        Json::Str(self.generator.template_ref.clone()),
                    ),
                    (
                        "capabilities".to_string(),
                        Json::Array(
                            self.generator
                                .capabilities
                                .iter()
                                .map(|c| Json::Str(c.as_str().to_string()))
                                .collect(),
                        ),
                    ),
                    (
                        "parameter_bindings".to_string(),
                        Json::Object(
                            self.generator
                                .parameter_bindings
                                .iter()
                                .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
                                .collect(),
                        ),
                    ),
                    (
                        "source_inputs".to_string(),
                        Json::Array(
                            self.generator
                                .source_inputs
                                .iter()
                                .map(|s| Json::Str(s.clone()))
                                .collect(),
                        ),
                    ),
                ])),
            ),
            (
                "scope".to_string(),
                Json::Object(BTreeMap::from([
                    (
                        "allowed_roles".to_string(),
                        Json::Array(
                            self.scope
                                .allowed_roles
                                .iter()
                                .map(|r| Json::Str(r.clone()))
                                .collect(),
                        ),
                    ),
                    (
                        "protected_roles".to_string(),
                        Json::Array(
                            self.scope
                                .protected_roles
                                .iter()
                                .map(|r| Json::Str(r.clone()))
                                .collect(),
                        ),
                    ),
                    // Constant false by construction; wire-level `true` is rejected.
                    ("allow_arbitrary_code".to_string(), Json::Bool(false)),
                ])),
            ),
            ("constraints".to_string(), Json::Array(constraints)),
            (
                "height_model".to_string(),
                match &self.height_model {
                    Some(hm) => Json::Object(BTreeMap::from([
                        (
                            "floors_param".to_string(),
                            Json::Str(hm.floors_param.clone()),
                        ),
                        (
                            "height_per_floor_m".to_string(),
                            Json::Float(hm.height_per_floor_m),
                        ),
                        ("roof_param".to_string(), Json::Str(hm.roof_param.clone())),
                        (
                            "roof_rise_m".to_string(),
                            Json::Object(
                                hm.roof_rise_m
                                    .iter()
                                    .map(|(k, v)| (k.clone(), Json::Float(*v)))
                                    .collect(),
                            ),
                        ),
                    ])),
                    None => Json::Null,
                },
            ),
            (
                "randomness".to_string(),
                Json::Object(BTreeMap::from([
                    (
                        "algorithm".to_string(),
                        Json::Str(self.randomness.algorithm.clone()),
                    ),
                    (
                        "derivation_version".to_string(),
                        Json::Str(self.randomness.derivation_version.clone()),
                    ),
                    (
                        "default_seed".to_string(),
                        Json::Int(self.randomness.default_seed as i64),
                    ),
                ])),
            ),
            (
                "budget".to_string(),
                Json::Object(BTreeMap::from([
                    (
                        "max_items_per_plan".to_string(),
                        Json::Int(self.budget.max_items_per_plan as i64),
                    ),
                    (
                        "max_combinations".to_string(),
                        Json::Int(self.budget.max_combinations as i64),
                    ),
                ])),
            ),
            (
                "dependencies".to_string(),
                Json::Array(
                    self.dependencies
                        .iter()
                        .map(|d| {
                            Json::Object(BTreeMap::from([
                                ("id".to_string(), Json::Str(d.id.clone())),
                                ("revision".to_string(), Json::Int(d.revision as i64)),
                                ("sha256".to_string(), Json::Str(d.sha256.clone())),
                            ]))
                        })
                        .collect(),
                ),
            ),
            (
                "sockets".to_string(),
                match &self.sockets {
                    Some(sp) => Json::Object(BTreeMap::from([
                        ("profile_ref".to_string(), Json::Str(sp.profile_ref.clone())),
                        (
                            "available_sockets".to_string(),
                            Json::Array(
                                sp.available_sockets
                                    .iter()
                                    .map(|s| Json::Str(s.clone()))
                                    .collect(),
                            ),
                        ),
                    ])),
                    None => Json::Null,
                },
            ),
        ]))
    }

    pub fn digest(&self) -> String {
        sha256_hex(self.to_json().to_canonical().as_bytes())
    }
}

/// Immutable lock over exact dependency hashes, tool versions and PRNG
/// algorithm. Floating `latest` never enters an executable plan (F12).
/// PROVISIONAL mirror of the GLM-1 `recipe_lock` shape, carried under the
/// private tag `michelangelo.family.recipe-lock/draft-0`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeLock {
    pub lock_id: String,
    pub recipe_id: String,
    pub recipe_revision: u64,
    pub recipe_sha256: String,
    pub source_set_sha256: String,
    /// Exact dependency hashes incl. donor manifests recorded at lock time.
    pub dependencies: Vec<DepRef>,
    /// At least "blender" and "michelangelo"; exact versions, no ranges.
    pub tool_versions: BTreeMap<String, String>,
    pub prng: Randomness,
    /// Pinned profile digests (style/validation/approval/skeleton) — optional.
    pub pinned_profiles: BTreeMap<String, String>,
}

impl RecipeLock {
    /// A tool version is exact only: rejects `latest`, `*`, ranges (`^ ~ > < =`)
    /// and empties (F12).
    pub fn check_tool_version(tool: &str, version: &str) -> FamilyResult<()> {
        let v = version.trim();
        let floating = v.is_empty()
            || v.eq_ignore_ascii_case("latest")
            || v == "*"
            || v.starts_with('^')
            || v.starts_with('~')
            || v.starts_with('>')
            || v.starts_with('<')
            || v.starts_with(">=")
            || v.starts_with("<=");
        if floating {
            return Err(FamilyError::new(
                FamilyErrorCode::FloatingVersion,
                format!("tool '{tool}' has floating/placeholder version '{v}': floating latest never enters an executable plan (F12)"),
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> FamilyResult<()> {
        for required in ["blender", "michelangelo"] {
            if !self.tool_versions.contains_key(required) {
                return Err(FamilyError::new(
                    FamilyErrorCode::FloatingVersion,
                    format!("lock is missing required exact tool version '{required}'"),
                ));
            }
        }
        for (tool, version) in &self.tool_versions {
            Self::check_tool_version(tool, version)?;
        }
        Ok(())
    }

    /// Cross-check against the current recipe + source set:
    /// - recipe identity/revision/hash must match the lock's recipe_ref,
    /// - every recipe dependency hash must equal the locked hash,
    /// - every source manifest hash must equal the locked hash; a changed
    ///   donor hash makes the lock stale.
    pub fn check_against(&self, recipe: &FamilyRecipe, sources: &SourceSet) -> FamilyResult<()> {
        if self.recipe_id != recipe.recipe_id || self.recipe_revision != recipe.revision {
            return Err(FamilyError::new(
                FamilyErrorCode::LockMismatch,
                format!(
                    "lock targets {}@{} but recipe is {}@{}",
                    self.recipe_id, self.recipe_revision, recipe.recipe_id, recipe.revision
                ),
            ));
        }
        // Dependency-level staleness first: a changed dependency hash is the
        // more precise diagnosis than a wholesale digest mismatch.
        for dep in &recipe.dependencies {
            let locked = self
                .dependencies
                .iter()
                .find(|d| d.id == dep.id && d.revision == dep.revision)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::StaleDependency,
                        format!("recipe dependency '{}' absent from lock", dep.id),
                    )
                })?;
            if locked.sha256 != dep.sha256 {
                return Err(FamilyError::new(
                    FamilyErrorCode::StaleDependency,
                    format!(
                        "dependency '{}' hash changed since lock: lock is stale",
                        dep.id
                    ),
                ));
            }
        }
        let current_recipe_digest = recipe.digest();
        if self.recipe_sha256 != current_recipe_digest {
            return Err(FamilyError::new(
                FamilyErrorCode::LockMismatch,
                "locked recipe_sha256 does not match the current recipe digest",
            ));
        }
        if self.source_set_sha256 != sources.digest() {
            return Err(FamilyError::new(
                FamilyErrorCode::StaleDependency,
                "locked source_set digest does not match the current SourceSet (stale donor/source)",
            ));
        }
        // Donor manifests are also locked: a donor re-export invalidates the lock.
        for source in &sources.sources {
            let expected_key = format!("source:{}", source.source_id);
            let locked = self
                .dependencies
                .iter()
                .find(|d| d.id == expected_key)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::StaleDependency,
                        format!("source '{}' not pinned in lock", source.source_id),
                    )
                })?;
            if locked.sha256 != source.manifest.sha256 {
                return Err(FamilyError::new(
                    FamilyErrorCode::StaleDependency,
                    format!(
                        "donor/source '{}' manifest hash changed since lock: lock is stale, dependent results are invalidated",
                        source.source_id
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Canonical wire document of the lock (provisional private tag).
    /// `floating_latest_allowed` is constant false by construction.
    pub fn to_json(&self) -> Json {
        Json::Object(BTreeMap::from([
            (
                "schema".to_string(),
                Json::Str(WIRE_SCHEMA_LOCK.to_string()),
            ),
            provisional_marker(),
            ("lock_id".to_string(), Json::Str(self.lock_id.clone())),
            (
                "recipe_ref".to_string(),
                Json::Object(BTreeMap::from([
                    ("recipe_id".to_string(), Json::Str(self.recipe_id.clone())),
                    (
                        "recipe_revision".to_string(),
                        Json::Int(self.recipe_revision as i64),
                    ),
                    (
                        "recipe_sha256".to_string(),
                        Json::Str(self.recipe_sha256.clone()),
                    ),
                ])),
            ),
            (
                "source_set_sha256".to_string(),
                Json::Str(self.source_set_sha256.clone()),
            ),
            (
                "dependencies".to_string(),
                Json::Array(
                    self.dependencies
                        .iter()
                        .map(|d| {
                            Json::Object(BTreeMap::from([
                                ("id".to_string(), Json::Str(d.id.clone())),
                                ("revision".to_string(), Json::Int(d.revision as i64)),
                                ("sha256".to_string(), Json::Str(d.sha256.clone())),
                            ]))
                        })
                        .collect(),
                ),
            ),
            (
                "tool_versions".to_string(),
                Json::Object(
                    self.tool_versions
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
                        .collect(),
                ),
            ),
            (
                "prng".to_string(),
                Json::Object(BTreeMap::from([
                    (
                        "algorithm".to_string(),
                        Json::Str(self.prng.algorithm.clone()),
                    ),
                    (
                        "derivation_version".to_string(),
                        Json::Str(self.prng.derivation_version.clone()),
                    ),
                    (
                        "default_seed".to_string(),
                        Json::Int(self.prng.default_seed as i64),
                    ),
                ])),
            ),
            (
                "pinned_profiles".to_string(),
                Json::Object(
                    self.pinned_profiles
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
                        .collect(),
                ),
            ),
            ("floating_latest_allowed".to_string(), Json::Bool(false)),
        ]))
    }

    /// Canonical digest over the lock content (excluding the digest itself).
    /// Equal to `sha256(canonical wire document)`: the lock digest pins
    /// exactly the bytes that are published.
    pub fn digest(&self) -> String {
        sha256_hex(self.to_json().to_canonical().as_bytes())
    }
}

/// One stable family item: full resolved parameter set, no scope/capability/
/// validation fields by construction. PROVISIONAL mirror of the GLM-1
/// `variant_request` shape, carried under the private tag
/// `michelangelo.family.variant-request/draft-0`.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantRequest {
    pub item_id: String,
    pub parameters: BTreeMap<String, ParamValue>,
    pub purpose: String,
    pub export_targets: Vec<String>,
}

impl VariantRequest {
    /// Canonical wire document of the request (provisional private tag plus
    /// the explicit `provisional: true` marker).
    pub fn to_json(&self) -> Json {
        Json::Object(BTreeMap::from([
            (
                "schema".to_string(),
                Json::Str(WIRE_SCHEMA_REQUEST.to_string()),
            ),
            provisional_marker(),
            ("item_id".to_string(), Json::Str(self.item_id.clone())),
            (
                "parameters".to_string(),
                Json::Object(
                    self.parameters
                        .iter()
                        .map(|(k, v)| (k.clone(), v.to_json()))
                        .collect(),
                ),
            ),
            ("purpose".to_string(), Json::Str(self.purpose.clone())),
            (
                "export_targets".to_string(),
                Json::Array(
                    self.export_targets
                        .iter()
                        .map(|t| Json::Str(t.clone()))
                        .collect(),
                ),
            ),
        ]))
    }

    /// Completeness + typed validation against the recipe (F17). Unknown
    /// parameters, missing values, out-of-range values, wrong types and
    /// enum misses are all explicit rejections, never silent ignores.
    pub fn validate_against(&self, recipe: &FamilyRecipe) -> FamilyResult<()> {
        for (name, value) in &self.parameters {
            let spec = recipe.parameters.get(name).ok_or_else(|| {
                FamilyError::new(
                    FamilyErrorCode::ParameterInvalid,
                    format!("unknown parameter '{name}' (F17): the request cannot add parameters beyond the recipe"),
                )
            })?;
            match spec {
                ParamSpec::Integer { min, max } => match value {
                    ParamValue::Int(i) if *i >= *min && *i <= *max => {}
                    ParamValue::Int(i) => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!("parameter '{name}' value {i} outside [{min},{max}] (F17)"),
                        ))
                    }
                    other => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!(
                                "parameter '{name}' expects integer, got {} (F17)",
                                other.type_name()
                            ),
                        ))
                    }
                },
                ParamSpec::Number { min, max } => match value {
                    ParamValue::Num(x) if *x >= *min && *x <= *max => {}
                    ParamValue::Num(x) => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!("parameter '{name}' value {x} outside [{min},{max}] (F17)"),
                        ))
                    }
                    other => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!(
                                "parameter '{name}' expects number, got {} (F17)",
                                other.type_name()
                            ),
                        ))
                    }
                },
                ParamSpec::Enum { values } => match value {
                    ParamValue::Text(s) if values.iter().any(|v| v == s) => {}
                    ParamValue::Text(s) => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!("parameter '{name}' value '{s}' not in enum {values:?} (F17)"),
                        ))
                    }
                    other => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!(
                                "parameter '{name}' expects enum string, got {} (F17)",
                                other.type_name()
                            ),
                        ))
                    }
                },
                ParamSpec::Boolean => match value {
                    ParamValue::Bool(_) => {}
                    other => {
                        return Err(FamilyError::new(
                            FamilyErrorCode::ParameterInvalid,
                            format!(
                                "parameter '{name}' expects boolean, got {} (F17)",
                                other.type_name()
                            ),
                        ))
                    }
                },
            }
        }
        for name in recipe.parameters.keys() {
            if !self.parameters.contains_key(name) {
                return Err(FamilyError::new(
                    FamilyErrorCode::ParameterInvalid,
                    format!("parameter '{name}' missing: the request must carry the complete allowed value set (F17)"),
                ));
            }
        }
        Ok(())
    }
}
