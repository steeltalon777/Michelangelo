//! The evaluator of the local edit boundary (IT-8B).
//!
//! The crate measures the actual geometry of a candidate (its snapshot) or
//! of a published scene revision against the pinned masks of the run:
//!
//! * `render.rs` — the renderer (the real one runs the Blender worker, the
//!   tests use a deterministic fake),
//! * `compare.rs` — the comparator (the real one runs the pinned
//!   `tools/solver/compare.py`, the tests use a deterministic fake),
//! * `evaluate.rs` — the evaluation of a candidate or of a revision,
//! * `assemble.rs` — the assembly of the immutable report,
//! * `helpers.rs` — the shared helpers.
//!
//! The evaluator writes the report into the store of the run and registers
//! its digest in the registry of the Core (under the lock of the store).

#![forbid(unsafe_code)]

pub mod assemble;
pub mod compare;
pub mod error;
pub mod evaluate;
pub mod helpers;
pub mod render;

pub use compare::{
    Comparator, CompareMeasurement, CompareOutcome, CompareRequest, CompareSet, CompareView,
    ScriptComparator,
};
pub use error::{
    EvalError, COMPARE_FAILED, EVALUATION_INPUT_INVALID, EVALUATION_INPUT_MISSING, EVIDENCE_BROKEN,
    RENDER_FAILED,
};
pub use evaluate::{
    evaluate_candidate, evaluate_candidate_with, evaluate_revision, evaluate_revision_with,
    verdict_of, EvaluationOptions, HELD_OUT_MAX_DOT, PER_VIEW_REGRESSION_MAX,
};
pub use render::{BlenderRenderer, GeometrySet, RenderRequest, Renderer, ViewSet, ViewSpec};
