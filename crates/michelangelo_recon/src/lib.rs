//! Michelangelo reconstruction (Stage B) — Core-side orchestration.
//!
//! This crate implements the Rust side of the Stage B reconstruction PoC
//! (docs/tz/TZ_RECON_POC_STAGE_B.md section 3.2):
//!
//! * `camera.rs` — the calibrated orthographic camera basis and its
//!   conversion to the Blender Euler-XYZ convention,
//! * `units.rs` — package unit (m/cm/mm) to metre conversion,
//! * `solver.rs` — discovery and bounded execution of the Python solver and
//!   comparator (`tools/solver/solve.py`, `tools/solver/compare.py`),
//! * `plan.rs` — the `BlenderJobSpec` builder for a reconstruction run,
//! * `store.rs` — the run store (`<workspace>/recon/rNNNN/run-NNNN/`),
//! * `report.rs` — the `michelangelo.reconstruction-report/0` report,
//! * `pipeline.rs` — the `run` and `score` pipelines.
//!
//! The crate never performs network access, never mutates an input package
//! and never panics on invalid input: every failure is a typed
//! [`ReconError`].
//!
//! # Example
//!
//! ```no_run
//! use michelangelo_recon::{run, ReconOptions};
//! use std::path::Path;
//!
//! let report = run(
//!     Path::new("examples/input_packages/valid_full"),
//!     Path::new("/tmp/workspace"),
//!     &ReconOptions::default(),
//! )?;
//! println!("{}", report.status.as_str());
//! # Ok::<(), michelangelo_recon::ReconError>(())
//! ```

#![forbid(unsafe_code)]

pub mod camera;
pub mod error;
pub mod pipeline;
pub mod plan;
pub mod report;
pub mod solver;
pub mod store;
pub mod testing;
pub mod units;

pub use error::ReconError;
pub use pipeline::{run, score, ReconOptions, ScoreOptions};
pub use report::{ReconstructionReport, RunStatus, REPORT_SCHEMA};
