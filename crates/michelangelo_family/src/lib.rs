//! michelangelo_family — ADR 0005 Family Builder / **Pure Planner V0–V1**
//! (GLM-4 stream, worktree `feat/adr5-family`, base `b861547` / IT-8D).
//!
//! **PROVISIONAL until the GLM-1 contract gate lands.** The DTOs in
//! [`model`], the planner in [`plan`] and the port in [`port`] are private
//! draft shapes oriented on the GLM-1 `schemas/adr5` documents (branch
//! `feat/adr5-contracts`, commit `4597c43`); they are not the approved wire
//! protocol and do not satisfy the GLM-1 schemas. Round 2 reconciles them.
//!
//! **Own tag namespace (finding L):** the emitted documents carry the
//! private tags `michelangelo.family.<doc>/draft-0` (never the GLM-1 tags
//! `michelangelo.<doc>/draft-0.1`) plus the explicit marker field
//! `provisional: true`, so a consumer dispatching by tag cannot mistake
//! these provisional artifacts for GLM-1 documents.
//!
//! Responsibilities implemented here (ADR 0005 §§4–6, 11.1):
//! - single- and multi-source `SourceSet` with six closed roles, allowed
//!   usage, held-out separation and donor part grants;
//! - `FamilyRecipe` describing the allowed result set: typed parameters,
//!   bounds, explicit incompatibility matrix, socket requirements, combinatorial
//!   cap, V0/V1 generator kinds only (V2–V4 capability-denied);
//! - immutable `RecipeLock` over exact dependency hashes, exact tool versions
//!   (floating `latest` rejected) and a pinned PRNG derivation;
//! - `VariantRequest` validation (complete typed value sets, F17 negatives);
//! - deterministic per-item seed derivation (`family-seed-v1`, stable identity);
//! - `ResolvedBatchPlan` dry planning for up to N distinct valid requests
//!   with honest `INSUFFICIENT_UNIQUE_COMBINATIONS` failure.
//!
//! Non-goals (hard boundaries): no Blender invocation, no variant generation,
//! no publication, no acceptance, no scope widening, no self-approval.

pub mod errors;
pub mod fixture;
pub mod json;
pub mod model;
pub mod plan;
pub mod port;
pub mod sha256;
pub mod wire;

pub use errors::{FamilyError, FamilyErrorCode, FamilyResult};
pub use model::{
    Budget, Capability, ConflictPolicy, Constraint, DepRef, FamilyRecipe, Generator, GeneratorKind,
    HeightModel, ManifestKind, ManifestRef, ParamSpec, ParamValue, Randomness, RecipeLock, Scope,
    SocketProfile, SourceEntry, SourceRole, SourceSet, SourceUsage, VariantRequest,
    PROVISIONAL_MARKER_KEY, WIRE_SCHEMA_BATCH_PLAN, WIRE_SCHEMA_LOCK, WIRE_SCHEMA_RECIPE,
    WIRE_SCHEMA_REQUEST, WIRE_SCHEMA_SOURCE_SET,
};
pub use plan::{
    combination_allowed, derive_seed, enumerate_valid_combinations, sample_requests, FamilyPlanner,
    PlannedItem, ResolvedBatchPlan, SEED_DERIVATION_ALGORITHM, SEED_DERIVATION_VERSION,
};
pub use port::{BatchExecutorPort, NullBatchExecutor, PlanAck};
pub use sha256::{sha256, sha256_hex};
