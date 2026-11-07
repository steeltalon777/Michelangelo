//! Michelangelo input package (Stage A) - strict manifest parsing and validation.
//!
//! This crate implements the normative contract of
//! docs/tz/TZ_INPUT_PACKAGE_STAGE_A.md section 3:
//!
//! * a strict, versioned JSON manifest (michelangelo.input-package/0),
//! * level 1 validation (format / safety) and level 2 validation (sufficiency),
//! * resource path safety (no traversal, containment inside the package root),
//! * PNG container checks and SHA-256 hash verification,
//! * a stable, deterministic report structure.
//!
//! Everything is implemented in-repo: SHA-256, CRC-32 and the PNG header
//! parser live in this crate, so the only external dependencies are serde
//! and serde_json.
//!
//! # Example
//!
//!     use michelangelo_input::{validate_package, ValidateOptions};
//!     use std::path::Path;
//!
//!     let report = validate_package(Path::new("package_dir"), &ValidateOptions::default());
//!     println!("{}", serde_json::to_string_pretty(&report).unwrap());

#![forbid(unsafe_code)]

pub mod crc32;
pub mod error;
pub mod manifest;
pub mod png;
pub mod report;
pub mod sha256;
pub mod validate;

pub use error::InputError;
pub use manifest::{InputPackage, MANIFEST_FILE_NAME, MANIFEST_SCHEMA};
pub use report::{
    Completeness, CompletenessStatus, Diagnostic, InputPackageReport, LevelSummary, ReportStatus,
    ResourceReport, Severity, ValidationLevels,
};
pub use validate::{load_manifest, validate_package, ValidateOptions};
