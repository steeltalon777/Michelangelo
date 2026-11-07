//! Typed errors of the reconstruction pipeline.
//!
//! Every failure mode of the crate is one variant of [`ReconError`]. The
//! variants map to CLI exit codes:
//!
//! * `2` — invalid input or a broken setup (missing package, missing tool,
//!   invalid job spec, failing solver, failing Blender job),
//! * `1` — the pipeline ran but the quality thresholds were not met.

use std::fmt;
use std::path::PathBuf;

/// Error type of the reconstruction pipeline.
#[derive(Debug)]
pub enum ReconError {
    /// The input package is invalid or unusable for metric reconstruction.
    InvalidPackage {
        /// Package identifier (empty when the manifest could not be read).
        package_id: String,
        /// Human readable summary of the validation failures.
        message: String,
    },
    /// A required file or directory does not exist.
    MissingPath {
        /// What was missing (e.g. "run directory").
        what: String,
        /// The path that was expected.
        path: PathBuf,
    },
    /// An I/O operation failed.
    Io {
        /// What was being done.
        context: String,
        /// The underlying error message.
        message: String,
    },
    /// A required tool (Python interpreter, solver script, worker script)
    /// could not be found.
    ToolNotFound {
        /// What was looked for.
        what: String,
        /// The path that was expected.
        path: PathBuf,
    },
    /// A tool could not be started.
    ToolLaunch {
        /// The tool that could not be started.
        what: String,
        /// The underlying error message.
        message: String,
    },
    /// The solver rejected the package (exit code 2).
    SolverRejected {
        /// The solver diagnostics (tail).
        message: String,
        /// The stdout tail of the solver.
        stdout: String,
    },
    /// The solver failed (exit code 3 or any other non-zero code).
    SolverFailed {
        /// The exit code of the solver (None when it was killed).
        exit_code: Option<i32>,
        /// The stderr tail of the solver.
        stderr: String,
        /// The stdout tail of the solver.
        stdout: String,
    },
    /// The solver exceeded its timeout.
    SolverTimeout {
        /// The effective timeout in milliseconds.
        timeout_ms: u64,
        /// The stderr tail collected before the kill.
        stderr: String,
        /// The stdout tail collected before the kill.
        stdout: String,
    },
    /// The solver output is missing or malformed.
    SolverOutput {
        /// What went wrong.
        message: String,
    },
    /// The comparator failed (usage or I/O error).
    CompareFailed {
        /// What went wrong.
        message: String,
    },
    /// A view uses a lens shift, which the v0 pipeline does not support.
    UnsupportedShift {
        /// The offending view identifier.
        view_id: String,
    },
    /// The camera of a view is degenerate or otherwise unusable.
    InvalidCamera {
        /// The offending view identifier.
        view_id: String,
        /// What went wrong.
        message: String,
    },
    /// A numeric value is not finite.
    NonFinite {
        /// The field that carries the value.
        field: String,
    },
    /// The Blender job specification is invalid.
    InvalidSpec {
        /// The validation message of the protocol validator.
        message: String,
    },
    /// The Blender job did not complete successfully.
    Blender {
        /// The worker status or error message.
        message: String,
    },
    /// A required artifact is missing after a successful job.
    MissingArtifact {
        /// The workspace-relative path of the artifact.
        path: String,
    },
    /// The report could not be written.
    ReportWrite {
        /// The path of the report.
        path: String,
        /// The underlying error message.
        message: String,
    },
}

impl ReconError {
    /// The CLI exit code of this error.
    ///
    /// `1` is reserved for a completed pipeline whose quality thresholds
    /// were not met; every setup or input problem maps to `2`.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidPackage { .. }
            | Self::MissingPath { .. }
            | Self::Io { .. }
            | Self::ToolNotFound { .. }
            | Self::ToolLaunch { .. }
            | Self::SolverRejected { .. }
            | Self::SolverFailed { .. }
            | Self::SolverTimeout { .. }
            | Self::SolverOutput { .. }
            | Self::CompareFailed { .. }
            | Self::UnsupportedShift { .. }
            | Self::InvalidCamera { .. }
            | Self::NonFinite { .. }
            | Self::InvalidSpec { .. }
            | Self::Blender { .. }
            | Self::MissingArtifact { .. }
            | Self::ReportWrite { .. } => 2,
        }
    }
}

impl fmt::Display for ReconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPackage {
                package_id,
                message,
            } => {
                if package_id.is_empty() {
                    write!(f, "the input package is invalid: {message}")
                } else {
                    write!(f, "the input package '{package_id}' is invalid: {message}")
                }
            }
            Self::MissingPath { what, path } => {
                write!(f, "{what} does not exist: '{}'", path.display())
            }
            Self::Io { context, message } => write!(f, "{context}: {message}"),
            Self::ToolNotFound { what, path } => {
                write!(f, "{what} not found: '{}'", path.display())
            }
            Self::ToolLaunch { what, message } => {
                write!(f, "{what} could not be started: {message}")
            }
            Self::SolverRejected { message, .. } => {
                write!(f, "the solver rejected the input package: {message}")
            }
            Self::SolverFailed {
                exit_code, stderr, ..
            } => {
                let code = exit_code.map_or_else(|| "unknown".to_string(), |c| c.to_string());
                if stderr.trim().is_empty() {
                    write!(f, "the solver failed with exit code {code}")
                } else {
                    write!(
                        f,
                        "the solver failed with exit code {code}: {}",
                        stderr.trim()
                    )
                }
            }
            Self::SolverTimeout {
                timeout_ms, stderr, ..
            } => {
                if stderr.trim().is_empty() {
                    write!(f, "the solver timed out after {timeout_ms} ms")
                } else {
                    write!(
                        f,
                        "the solver timed out after {timeout_ms} ms: {}",
                        stderr.trim()
                    )
                }
            }
            Self::SolverOutput { message } => write!(f, "the solver output is unusable: {message}"),
            Self::CompareFailed { message } => write!(f, "the comparator failed: {message}"),
            Self::UnsupportedShift { view_id } => write!(
                f,
                "view '{view_id}' uses a non-zero shift_x/shift_y, which the v0 \
                 reconstruction pipeline does not support (UNSUPPORTED_SHIFT)"
            ),
            Self::InvalidCamera { view_id, message } => {
                write!(f, "the camera of view '{view_id}' is unusable: {message}")
            }
            Self::NonFinite { field } => write!(f, "'{field}' is not a finite number"),
            Self::InvalidSpec { message } => {
                write!(f, "the Blender job specification is invalid: {message}")
            }
            Self::Blender { message } => write!(f, "the Blender job failed: {message}"),
            Self::MissingArtifact { path } => {
                write!(f, "the expected artifact '{path}' was not produced")
            }
            Self::ReportWrite { path, message } => {
                write!(f, "the report '{path}' cannot be written: {message}")
            }
        }
    }
}

impl std::error::Error for ReconError {}

impl From<std::io::Error> for ReconError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            context: "an I/O operation failed".to_string(),
            message: err.to_string(),
        }
    }
}
