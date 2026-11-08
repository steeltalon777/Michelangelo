//! Pure planner: seed derivation, combination engine and ResolvedBatchPlan
//! assembly. PROVISIONAL shapes; no execution, no Blender, no publication.

use std::collections::BTreeMap;

use crate::errors::{FamilyError, FamilyErrorCode, FamilyResult};
use crate::json::Json;
use crate::model::{
    Constraint, FamilyRecipe, ParamSpec, ParamValue, RecipeLock, SourceRole, SourceSet,
    VariantRequest,
};
use crate::sha256::sha256_hex;

/// Exact per-item seed derivation, pinned as `family-seed-v1`:
///
/// ```text
/// seed = ( first 8 bytes (big-endian u64) of
///          SHA-256( "michelangelo/family-seed-v1" || 0x00
///                   || lock_digest_hex || 0x00 || item_id_utf8 ) )
///        & (2^53 - 1)
/// ```
///
/// The 53-bit mask keeps the seed inside the JSON-safe integer range of the
/// GLM-1 draft variant_request schema (0..=9007199254740991). The seed
/// depends only on the locked recipe identity and the stable item_id — never
/// on launch order, worker number or retry count (ADR §6). The derivation
/// version string is recorded in `RecipeLock.prng`.
pub const SEED_DERIVATION_VERSION: &str = "family-seed-v1";
pub const SEED_DERIVATION_ALGORITHM: &str =
    "sha256-first8be(prefix||0x00||lock_digest_hex||0x00||item_id)-masked-53bit";

pub fn derive_seed(lock_digest: &str, item_id: &str) -> u64 {
    let mut input = Vec::new();
    input.extend_from_slice(b"michelangelo/family-seed-v1");
    input.push(0x00);
    input.extend_from_slice(lock_digest.as_bytes());
    input.push(0x00);
    input.extend_from_slice(item_id.as_bytes());
    let digest = crate::sha256::sha256(&input);
    let mut be = [0u8; 8];
    be.copy_from_slice(&digest[0..8]);
    u64::from_be_bytes(be) & ((1u64 << 53) - 1)
}

/// One fully resolved family item inside a dry plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedItem {
    pub item_id: String,
    pub resolved_parameters: BTreeMap<String, ParamValue>,
    pub seed: u64,
    /// recipe/lock/source-set/parameter fingerprints (provisional keys).
    pub input_fingerprints: BTreeMap<String, String>,
    /// Private per-item output namespace (never shared writable between items).
    pub output_namespace: String,
    pub max_attempts: u8,
}

/// Immutable dry plan: shows what WOULD be created; approves nothing and
/// executes nothing. PROVISIONAL mirror of `michelangelo.batch-plan/draft-0.1`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedBatchPlan {
    pub batch_id: String,
    pub recipe_id: String,
    pub recipe_revision: u64,
    pub recipe_sha256: String,
    pub recipe_lock_digest: String,
    pub items: Vec<PlannedItem>,
    /// Constant `single_heavy_process_v0` planning note: one heavy Blender
    /// process at a time on the RTX 3050 8 GB / 32 GB RAM bench (ADR §7.3).
    pub admission_profile: &'static str,
    pub max_parallel_heavy: u8,
    pub cycle_check_acyclic: bool,
    /// Honest capability notes for deferred levels (V2–V4) and limits.
    pub notes: Vec<String>,
}

impl ResolvedBatchPlan {
    pub fn to_json(&self) -> Json {
        Json::Object(BTreeMap::from([
            ("schema".to_string(), Json::Str("michelangelo.batch-plan/draft-0.1".to_string())),
            ("batch_id".to_string(), Json::Str(self.batch_id.clone())),
            (
                "recipe_ref".to_string(),
                Json::Object(BTreeMap::from([
                    ("recipe_id".to_string(), Json::Str(self.recipe_id.clone())),
                    ("revision".to_string(), Json::Int(self.recipe_revision as i64)),
                    ("recipe_sha256".to_string(), Json::Str(self.recipe_sha256.clone())),
                    ("recipe_lock_digest".to_string(), Json::Str(self.recipe_lock_digest.clone())),
                ])),
            ),
            (
                "items".to_string(),
                Json::Array(
                    self.items
                        .iter()
                        .map(|it| {
                            Json::Object(BTreeMap::from([
                                ("item_id".to_string(), Json::Str(it.item_id.clone())),
                                (
                                    "resolved_parameters".to_string(),
                                    Json::Object(
                                        it.resolved_parameters
                                            .iter()
                                            .map(|(k, v)| (k.clone(), v.to_json()))
                                            .collect(),
                                    ),
                                ),
                                ("seed".to_string(), Json::Int(it.seed as i64)),
                                (
                                    "input_fingerprints".to_string(),
                                    Json::Object(
                                        it.input_fingerprints
                                            .iter()
                                            .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
                                            .collect(),
                                    ),
                                ),
                                (
                                    "output_namespace".to_string(),
                                    Json::Str(it.output_namespace.clone()),
                                ),
                                ("max_attempts".to_string(), Json::Int(it.max_attempts as i64)),
                                ("source_kind".to_string(), Json::Str("variant_request".to_string())),
                                ("depends_on_items".to_string(), Json::Array(vec![])),
                            ]))
                        })
                        .collect(),
                ),
            ),
            (
                "limits".to_string(),
                Json::Object(BTreeMap::from([
                    ("admission_profile".to_string(), Json::Str(self.admission_profile.to_string())),
                    ("max_parallel_heavy".to_string(), Json::Int(self.max_parallel_heavy as i64)),
                ])),
            ),
            (
                "cycle_check".to_string(),
                Json::Object(BTreeMap::from([
                    ("checked".to_string(), Json::Bool(true)),
                    ("acyclic".to_string(), Json::Bool(self.cycle_check_acyclic)),
                ])),
            ),
            (
                "notes".to_string(),
                Json::Array(self.notes.iter().map(|n| Json::Str(n.clone())).collect()),
            ),
        ]))
    }

    pub fn to_canonical_json(&self) -> String {
        self.to_json().to_canonical()
    }
}

/// All distinct values of a parameter in canonical order (integers ascend,
/// enum values follow the declared recipe order, booleans false < true).
fn value_domain(spec: &ParamSpec) -> Vec<ParamValue> {
    match spec {
        ParamSpec::Integer { min, max } => (*min..=*max).map(ParamValue::Int).collect(),
        ParamSpec::Enum { values } => values.iter().map(|v| ParamValue::Text(v.clone())).collect(),
        ParamSpec::Boolean => vec![ParamValue::Bool(false), ParamValue::Bool(true)],
        ParamSpec::Number { .. } => vec![],
    }
}

/// Static planning checks that a resolved combination satisfies the explicit
/// incompatibility matrix, socket requirements and the height model.
/// Contradictions are rejections — never silently averaged away.
pub fn combination_allowed(
    recipe: &FamilyRecipe,
    params: &BTreeMap<String, ParamValue>,
) -> FamilyResult<()> {
    for c in &recipe.constraints {
        match c {
            Constraint::ParamPairIncompatible { param, value, forbids_param, forbids_value } => {
                let a = params.get(param).and_then(|v| match v {
                    ParamValue::Text(s) => Some(s.as_str()),
                    _ => None,
                });
                let b = params.get(forbids_param).and_then(|v| match v {
                    ParamValue::Text(s) => Some(s.as_str()),
                    _ => None,
                });
                if a == Some(value.as_str()) && b == Some(forbids_value.as_str()) {
                    return Err(FamilyError::new(
                        FamilyErrorCode::ParameterInvalid,
                        format!("incompatible combination: {param}={value} forbids {forbids_param}={forbids_value}"),
                    ));
                }
            }
            Constraint::RequiresSocket { param, value, socket_id } => {
                let a = params.get(param).and_then(|v| match v {
                    ParamValue::Text(s) => Some(s.as_str()),
                    _ => None,
                });
                if a == Some(value.as_str()) {
                    let profile = recipe.sockets.as_ref().ok_or_else(|| {
                        FamilyError::new(
                            FamilyErrorCode::SocketMismatch,
                            format!("combination requires socket '{socket_id}' but the recipe declares no socket profile"),
                        )
                    })?;
                    if !profile.available_sockets.iter().any(|s| s == socket_id) {
                        return Err(FamilyError::new(
                            FamilyErrorCode::SocketMismatch,
                            format!("socket '{socket_id}' required by {param}={value} is absent from socket profile '{}' (socket mismatch)", profile.profile_ref),
                        ));
                    }
                }
            }
            Constraint::MaximumDimensionZ { value_m } => {
                if let Some(hm) = &recipe.height_model {
                    let floors = params.get(&hm.floors_param).and_then(|v| match v {
                        ParamValue::Int(i) => Some(*i as f64),
                        _ => None,
                    });
                    let roof = params.get(&hm.roof_param).and_then(|v| match v {
                        ParamValue::Text(s) => Some(s.clone()),
                        _ => None,
                    });
                    if let (Some(floors), Some(roof)) = (floors, roof) {
                        let rise = hm.roof_rise_m.get(&roof).copied().unwrap_or(0.0);
                        let height = floors * hm.height_per_floor_m + rise;
                        if height > *value_m {
                            return Err(FamilyError::new(
                                FamilyErrorCode::ParameterInvalid,
                                format!(
                                    "estimated height {height:.1}m exceeds maximum_dimension Z={value_m}m for {}={} with {}={roof}",
                                    hm.floors_param, floors, hm.roof_param
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Deterministically enumerates every valid combination of the recipe's typed
/// parameters (canonical order), filtered by `combination_allowed`. The
/// cartesian product size is capped by `budget.max_combinations`; exceeding it
/// is an honest rejection, not a truncated silent plan.
pub fn enumerate_valid_combinations(
    recipe: &FamilyRecipe,
) -> FamilyResult<Vec<BTreeMap<String, ParamValue>>> {
    // Product size first (combinatorial cap check before enumeration).
    let mut product: u64 = 1;
    for (name, spec) in &recipe.parameters {
        let card = spec.cardinality()?;
        product = product.saturating_mul(card);
        if product > recipe.budget.max_combinations {
            return Err(FamilyError::new(
                FamilyErrorCode::CombinatorialCapExceeded,
                format!(
                    "cartesian product reaches {product} at parameter '{name}', above cap {}",
                    recipe.budget.max_combinations
                ),
            ));
        }
    }

    // Enumerate in canonical parameter-name order with odometer stepping.
    let names: Vec<&String> = recipe.parameters.keys().collect();
    let domains: Vec<Vec<ParamValue>> = names
        .iter()
        .map(|n| value_domain(&recipe.parameters[*n]))
        .collect();

    let mut valid = Vec::new();
    let total: u64 = domains.iter().map(|d| d.len() as u64).product();
    for index in 0..total {
        let mut rest = index;
        let mut params = BTreeMap::new();
        for (i, domain) in domains.iter().enumerate().rev() {
            let base = domain.len() as u64;
            let digit = rest % base;
            rest /= base;
            params.insert(names[i].clone(), domain[digit as usize].clone());
        }
        if combination_allowed(recipe, &params).is_ok() {
            valid.push(params);
        }
    }
    Ok(valid)
}

/// Builds deterministic variant requests from the enumerated valid
/// combinations: stride sampling `i * floor(N/n)` spreads n distinct items
/// across the canonical order. Distinct indices guarantee distinct requests.
/// If fewer than n unique valid combinations exist, fails honestly with
/// `INSUFFICIENT_UNIQUE_COMBINATIONS` — no fabricated variants (ADR §9.3).
pub fn sample_requests(
    recipe: &FamilyRecipe,
    batch_prefix: &str,
    count: usize,
) -> FamilyResult<Vec<VariantRequest>> {
    let valid = enumerate_valid_combinations(recipe)?;
    if (valid.len() as u64) < count as u64 {
        return Err(FamilyError::new(
            FamilyErrorCode::InsufficientUniqueCombinations,
            format!(
                "only {} unique valid combinations exist for this recipe; refusing to fabricate {} distinct requests (ADR §9.3: count does not guarantee diversity)",
                valid.len(),
                count
            ),
        ));
    }
    let stride = valid.len() / count;
    let mut requests = Vec::new();
    for k in 0..count {
        let combo = &valid[k * stride];
        // Canonical order label, e.g. house-000 — stable identity, independent
        // of launch order.
        requests.push(VariantRequest {
            item_id: format!("{batch_prefix}-{k:03}"),
            parameters: combo.clone(),
            purpose: "family_variant_static_review".to_string(),
            export_targets: vec!["blend".to_string(), "glb".to_string()],
        });
    }
    Ok(requests)
}

pub struct FamilyPlanner;

impl FamilyPlanner {
    /// `family.plan(recipe, lock, requests)` (CONTRACT_GATE §7.3, provisional
    /// name). Validates everything cross-wise and produces an immutable dry
    /// plan. Executes nothing.
    pub fn plan(
        batch_id: &str,
        recipe: &FamilyRecipe,
        sources: &SourceSet,
        lock: &RecipeLock,
        requests: &[VariantRequest],
    ) -> FamilyResult<ResolvedBatchPlan> {
        recipe.validate()?;
        sources.validate()?;
        lock.validate()?;
        lock.check_against(recipe, sources)?;

        // Held-out leakage at plan time: the generator may only consume
        // build/reference inputs; held-out sources never feed generation.
        for input_id in &recipe.generator.source_inputs {
            let source = sources
                .sources
                .iter()
                .find(|s| &s.source_id == input_id)
                .ok_or_else(|| {
                    FamilyError::new(
                        FamilyErrorCode::SchemaInvalid,
                        format!("generator.source_inputs references unknown source '{input_id}'"),
                    )
                })?;
            if source.usage == crate::model::SourceUsage::HeldOut {
                return Err(FamilyError::new(
                    FamilyErrorCode::HeldoutLeakage,
                    format!(
                        "generator consumes held-out source '{input_id}': independent validation reference must not feed generation"
                    ),
                ));
            }
            // Donor provenance: a module_donor input must have explicit part grants.
            if source.roles.contains(&SourceRole::ModuleDonor) && source.allowed_parts.is_empty() {
                return Err(FamilyError::new(
                    FamilyErrorCode::RoleInvalid,
                    format!("donor source '{input_id}' contributes without allowed_parts"),
                ));
            }
        }

        if requests.is_empty() {
            return Err(FamilyError::new(
                FamilyErrorCode::SchemaInvalid,
                "plan requires at least one variant request",
            ));
        }
        if requests.len() as u64 > recipe.budget.max_items_per_plan {
            return Err(FamilyError::new(
                FamilyErrorCode::CombinatorialCapExceeded,
                format!(
                    "{} requests exceed budget.max_items_per_plan={}",
                    requests.len(),
                    recipe.budget.max_items_per_plan
                ),
            ));
        }

        // Distinct stable identities (duplicate identity is a plan-level failure).
        let mut seen = std::collections::BTreeSet::new();
        for r in requests {
            if !seen.insert(r.item_id.as_str()) {
                return Err(FamilyError::new(
                    FamilyErrorCode::DuplicateIdentity,
                    format!("duplicate item_id '{}' in one plan", r.item_id),
                ));
            }
            r.validate_against(recipe)?;
        }

        let lock_digest = lock.digest();
        let recipe_digest = recipe.digest();
        let source_set_digest = sources.digest();

        let mut items = Vec::new();
        for r in requests {
            let mut fingerprints = BTreeMap::new();
            fingerprints.insert("recipe_sha256".to_string(), recipe_digest.clone());
            fingerprints.insert("recipe_lock_digest".to_string(), lock_digest.clone());
            fingerprints.insert("source_set_sha256".to_string(), source_set_digest.clone());
            let param_canonical = Json::Object(
                r.parameters
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect(),
            )
            .to_canonical();
            fingerprints.insert("resolved_parameters".to_string(), sha256_hex(param_canonical.as_bytes()));
            items.push(PlannedItem {
                item_id: r.item_id.clone(),
                resolved_parameters: r.parameters.clone(),
                seed: derive_seed(&lock_digest, &r.item_id),
                input_fingerprints: fingerprints,
                output_namespace: format!("family/{batch_id}/{}", r.item_id),
                max_attempts: 1,
            });
        }

        Ok(ResolvedBatchPlan {
            batch_id: batch_id.to_string(),
            recipe_id: recipe.recipe_id.clone(),
            recipe_revision: recipe.revision,
            recipe_sha256: recipe_digest,
            recipe_lock_digest: lock_digest,
            items,
            admission_profile: "single_heavy_process_v0",
            max_parallel_heavy: 1,
            // Family items are mutually independent: no inter-item edges, so
            // the dependency graph is trivially acyclic after construction.
            cycle_check_acyclic: true,
            notes: vec![
                "pure dry plan: no Blender invocation, no publication, no acceptance".to_string(),
                "V2-V4 capabilities are capability-denied at planning level".to_string(),
            ],
        })
    }
}
