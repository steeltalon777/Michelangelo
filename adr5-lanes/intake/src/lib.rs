//! Michelangelo asset intake / preservation PoC (ADR 0005, stream A).
//!
//! Takes an already-finished model through **Intake**: format detection,
//! dependency closure discovery, path-safety diagnostics, and an immutable
//! source snapshot in a disposable workspace. The original bytes are never
//! modified, converted, remeshed, retopologized or decimated — by design
//! there is no code path that could do so silently.
//!
//! Supported inputs (deliberately narrow, per the stream assignment):
//! * GLB (glTF 2.0 binary container, embedded BIN chunk and images);
//! * plain `.gltf` with relative local dependencies (buffers, images,
//!   `data:` URIs);
//! * OBJ with MTL libraries and local texture maps.
//!
//! Explicitly NOT supported and rejected with a dedicated error:
//! arbitrary `.blend` intake (ADR 0005 §13.2 keeps the safe `.blend`
//! adapter an open gate) and remote/network URIs (never fetched).
//!
//! Guarantees provided here:
//! * hashes are computed over bytes actually read, not over declarations;
//! * a dependency substituted between planning and execution fails with
//!   `InputChanged`, never a false success;
//! * snapshot directories are reserved with create/no-overwrite semantics
//!   keyed by content identity (idempotent reuse, collision-safe);
//! * unknown license provenance is recorded but is never a usage
//!   permission.
//!
//! What this crate does NOT do: preparation/repair, validation gates,
//! accept/publish, Asset Library integration, arbitrary `.blend` import.
//! Its manifest is a provisional private DTO (see [`dto`]) pending the
//! GLM-1 contracts stream.
//!
//! Standalone build note: this crate is intentionally **not** a member of
//! the repository root Cargo workspace (the root `Cargo.toml` must not
//! change in this stream). It has its own `Cargo.lock` and reuses the
//! in-repo SHA-256 from `michelangelo_input` via a read-only path
//! dependency.

#![forbid(unsafe_code)]

pub mod base64;
pub mod dto;
pub mod error;
pub mod export;
pub mod format;
pub mod gltf;
pub mod hash;
pub mod license;
pub mod limits;
pub mod obj;
pub mod pathsafe;
pub mod report;
pub mod snapshot;

pub use dto::{intake_report_view_from_rejection, source_snapshot_view};
pub use error::IntakeError;
pub use export::{export_passthrough, LossReport, PassthroughExport, StoredExport};
pub use format::SourceFormat;
pub use license::{LicenseDecision, LicenseStatus, ProvenanceInfo};
pub use limits::IntakeLimits;
pub use report::{Diagnostic, DiagnosticCode, ReportStatus, Severity};
pub use snapshot::{
    execute_plan, plan_intake, DependencyKind, IntakePlan, IntakeRequest, PlannedDependency,
    PlannedFile, SnapshotManifest, StoredSnapshot, SNAPSHOT_SCHEMA,
};
