//! Error type for loading an input package manifest.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Errors returned when an input package manifest cannot be loaded.
#[derive(Debug)]
pub enum InputError {
    /// The package directory could not be resolved.
    PackageRoot {
        /// The package root that failed to resolve.
        path: PathBuf,
        /// Human readable reason.
        message: String,
    },
    /// The manifest file could not be read.
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying IO error.
        source: io::Error,
    },
    /// The manifest exceeds the maximum permitted size.
    ManifestTooLarge {
        /// The manifest file.
        path: PathBuf,
        /// Actual size in bytes.
        bytes: u64,
    },
    /// The manifest is not valid JSON or does not match the manifest schema.
    ManifestInvalid {
        /// The manifest file.
        path: PathBuf,
        /// Human readable reason.
        message: String,
    },
}

impl InputError {
    /// Build an IO error for the given path.
    pub fn io(path: &Path, source: io::Error) -> Self {
        InputError::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// Build a manifest-invalid error for the given path.
    pub fn invalid(path: &Path, message: impl Into<String>) -> Self {
        InputError::ManifestInvalid {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::PackageRoot { path, message } => {
                write!(f, "package root '{}': {message}", path.display())
            }
            InputError::Io { path, source } => {
                write!(f, "'{}': {source}", path.display())
            }
            InputError::ManifestTooLarge { path, bytes } => write!(
                f,
                "manifest '{}' is {bytes} bytes, which exceeds the maximum manifest size",
                path.display()
            ),
            InputError::ManifestInvalid { path, message } => {
                write!(f, "manifest '{}': {message}", path.display())
            }
        }
    }
}

impl std::error::Error for InputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InputError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
