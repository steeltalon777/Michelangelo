//! The evidence of the verification contract (IT-8B).
//!
//! The crate implements the immutable evidence of
//! `docs/specs/VERIFICATION_STATE.md`:
//!
//! * `digest.rs` — the digests of the contract §3 (file, geometry
//!   aggregate, camera digest, inputs digest),
//! * `report.rs` — the data transfer objects of the evaluation report
//!   (`michelangelo.evaluation-report/0`),
//! * `registry.rs` — the append-only registry of the evaluation digests
//!   (`verification/evaluations/index.json`),
//! * `gate.rs` — the pure checks of the accept gate (contract §7),
//! * `error.rs` — the typed error and its diagnostic codes.
//!
//! The crate depends on nothing but the serializer and the SHA-256
//! implementation: it never renders, never imports the edit boundary and
//! never imports the evaluator, so the dependency graph stays acyclic.

#![forbid(unsafe_code)]

pub mod digest;
pub mod error;
pub mod gate;
pub mod registry;
pub mod report;
pub mod testing;

pub use digest::{
    camera_digest, canonical_json, canonical_json_of, file_sha256, geometry_aggregate,
    inputs_canonical_json, inputs_digest, sha256_hex,
};
pub use error::{
    EvidenceError, AUTHOR_CONDITION_FAILED, EVALUATION_ALREADY_REGISTERED, EVALUATION_MISSING,
    EVALUATION_STALE, EVALUATION_UNREGISTERED, EVIDENCE_INVALID, INPUT_CHANGED, NO_GROUND_TRUTH,
    REGRESSION, VISIBLE_FAILED,
};
pub use gate::{verify_report, GateContext, AUTHOR_CONDITION_KIND};
pub use registry::{
    evaluation_dir, load_registry, registered_evaluation_bytes_for_target,
    registered_evaluations_for_target, registry_path, report_path, report_relative_path,
    write_and_register, write_registry, EvaluationRegistry, RegistryEntry,
    EVALUATION_REGISTRY_SCHEMA,
};
pub use report::{
    package_measurement_ids, package_unknown_ids, package_views, AuthorDecision, CameraDigest,
    ClaimStatus, EvaluationCheck, EvaluationInputs, EvaluationReport, EvaluationResults,
    EvaluationTarget, EvaluationThresholds, GroundTruthState, HiddenClaim, HiddenSection,
    PackageView, ReportIntegrity, TargetKind, TriState, ViewPin, VisibleSection, VisibleSet,
    VisibleView, EVALUATION_REPORT_SCHEMA,
};
