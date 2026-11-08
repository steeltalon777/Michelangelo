//! Michelangelo local edit boundary (Stage C).
//!
//! This crate implements the normative contract of
//! `docs/tz/TZ_LOCAL_EDIT_STAGE_C.md`:
//!
//! * `obj.rs` — a strict OBJ parser (vertices, faces, bounding box),
//! * `scene.rs` — the immutable scene revision store of a reconstruction run,
//! * `candidate.rs` — the typed edit candidate (`michelangelo.edit-candidate/0`),
//! * `validate.rs` — the deterministic validator and the decision report
//!   (`michelangelo.edit-decision/0`),
//! * `apply.rs` — the applier (accept, reject, rollback, show),
//! * `policy.rs` — the optional workspace edit policy.
//!
//! The crate performs no network access, never mutates the run's `source/`
//! directory and never mutates a published scene revision. Every failure is a
//! typed [`EditError`].
//!
//! # Example
//!
//! ```no_run
//! use michelangelo_edit::{init_scene, SystemClock};
//! use std::path::Path;
//!
//! let scene = init_scene(Path::new("/tmp/workspace/recon/r0001/run-0001"), &SystemClock)?;
//! println!("{}", scene.scene_revision);
//! # Ok::<(), michelangelo_edit::EditError>(())
//! ```

#![forbid(unsafe_code)]

pub mod apply;
pub mod candidate;
pub mod error;
pub mod gate;
pub mod obj;
pub mod policy;
pub mod scene;
pub mod unknowns;
pub mod validate;
pub mod verification;

pub use apply::{
    accept_candidate, accept_candidate_with_hooks, accept_candidate_with_hooks_and_options,
    accept_candidate_with_options, reject_candidate, rollback_scene, show_scene, submit_candidate,
    AcceptHooks, ShowReport,
};
pub use candidate::{CandidateAction, CandidateOrigin, EditCandidate};
pub use error::{io_error, EditError};
pub use gate::{AcceptOptions, EvaluationBinding};
pub use obj::{parse_obj, Bbox, ObjError, ObjMesh};
pub use policy::{EditPolicy, PolicyDecision};
pub use scene::{
    init_scene, load_current, load_index, load_scene, CurrentPointer, Scene, SceneComponent,
    SceneStore, SceneVerification,
};
pub use unknowns::{
    bind_unknown, confirm_unknown, list_unknowns, materialize_resolution, propose_resolution,
    show_unknowns, sync_unknowns, BindingDocument, Confirmation, Resolution, ResolutionMaterial,
    UnknownBinding, UnknownEntryView, UnknownView,
};
pub use validate::{Check, CheckStatus, Decision, DecisionReport, Diagnostic, Severity};
pub use verification::{
    SnapshotManifest, StoreLock, UnknownState, SNAPSHOT_SCHEMA, UNKNOWN_STATE_SCHEMA,
};

/// The schema of a scene document.
pub const SCENE_SCHEMA: &str = "michelangelo.scene/0";
/// The schema of an edit candidate.
pub const CANDIDATE_SCHEMA: &str = "michelangelo.edit-candidate/0";
/// The schema of a decision report.
pub const DECISION_SCHEMA: &str = "michelangelo.edit-decision/0";
/// The schema of the candidate index.
pub const INDEX_SCHEMA: &str = "michelangelo.edit-candidate-index/0";

/// A clock that produces the RFC 3339 UTC timestamps of the store.
///
/// The clock is injected so that the tests are deterministic.
pub trait Clock {
    /// The current time as an RFC 3339 timestamp in UTC.
    fn now_rfc3339_utc(&self) -> String;
}

/// The clock of the production code (the wall clock).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_rfc3339_utc(&self) -> String {
        michelangelo_recon::report::now_rfc3339_utc()
    }
}

/// A clock that always returns the same timestamp (used by the tests).
#[derive(Debug, Clone)]
pub struct FixedClock {
    /// The timestamp that the clock returns.
    pub timestamp: String,
}

impl FixedClock {
    /// A clock that returns the given timestamp.
    pub fn new(timestamp: impl Into<String>) -> Self {
        FixedClock {
            timestamp: timestamp.into(),
        }
    }
}

impl Clock for FixedClock {
    fn now_rfc3339_utc(&self) -> String {
        self.timestamp.clone()
    }
}
