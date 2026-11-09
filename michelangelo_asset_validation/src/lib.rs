//! The asset validation layer of ADR 0005 (GLM-6 stream, IT-8D base).
//!
//! A standalone contract PoC for §§5.3, 8, 11 of ADR 0005. It owns:
//!
//! * `profile` — the versioned `ValidationProfile` (applicability, required /
//!   optional checks from the closed check_id registry, thresholds,
//!   evaluator versions), pinned BEFORE generation;
//! * `check` — the NEW result model `pass / fail / unavailable / not_required`.
//!   This is NOT the IT-8 evaluation enum: `not_required` never enters the
//!   existing `michelangelo.evaluation-report/0` and never converts an
//!   `unavailable` into a `pass` (OP-7);
//! * `checks::*` — deterministic evaluators measuring the bytes actually
//!   read (payload/dependency/asset digests, effective protected transforms,
//!   socket fit, budgets, UV/material presence, rig identity);
//! * `report` — the pinned immutable `ValidationReport` (measurements only,
//!   verdict fold fail > unavailable > pass, no `not_required` verdict);
//! * `registry` — reservation before computation + no-clobber registration
//!   (a repeated report is never overwritten) + `verify` (F2/F3/F19);
//! * `acceptance` — the seam to the future canonical accept Core: the
//!   decision document is separate from measurements; this module never
//!   publishes and there is no second publisher.
//!
//! Reconstruction profiles are refused here (`UNSUPPORTED_PROFILE_KIND`):
//! the existing IT-8D reconstruction evaluator stays authoritative and is
//! not re-implemented.
//!
//! All DTOs are private provisional drafts aligned with the GLM-1 contract
//! gate `draft-0.1`; they are not an approved wire protocol (round 2 of the
//! dispatch will align them).

#![forbid(unsafe_code)]

pub mod acceptance;
pub mod canonical;
pub mod check;
pub mod checks;
pub mod context;
pub mod error;
pub mod evaluate;
pub mod ids;
pub mod profile;
pub mod registry;
pub mod report;

/// Test-support fixtures (deterministic workspace builders). Like
/// `michelangelo_evidence::testing`, this module is for tests and PoC
/// harnesses only; it is never used on production paths.
pub mod testing;

pub use acceptance::{
    AcceptanceBasis, AcceptanceRecord, AcceptanceTarget, Actor, CanonicalAcceptSeam,
    PrerequisiteReport, RegistryBackedSeam,
};
pub use check::{CheckId, CheckRecord, CheckStatus, DocRef, Verdict};
pub use context::{
    ArtifactEntry, BundleManifest, ConversionDecl, DependencyEntry, LossDecl, OutputDecls,
    ValidationContext,
};
pub use error::{
    ValidationError, ACCEPTANCE_REPLAY, NO_CLOBBER_VIOLATION, VALIDATION_INPUT_CHANGED,
    VALIDATION_UNKNOWN_REQUIRED,
};
pub use evaluate::{evaluate, EvaluationRequest};
pub use profile::{
    AppliesTo, Comparison, ProfileKind, ProfileReferences, TargetKind, Threshold,
    ValidationProfile, PROFILE_SCHEMA,
};
pub use registry::{
    load_pinned_profile, load_registry, pinned_profile_path, registry_path, store_pinned_profile,
    verify_registered_report, write_and_register, RegistryEntry, ReportRegistry, Reservation,
    VerifyOutcome, PROFILES_DIR, REGISTRY_SCHEMA,
};
pub use report::{BundleKind, BundleRef, ProfileRef, ValidationReport, REPORT_SCHEMA};
