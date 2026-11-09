//! Resource limits for a single intake operation.
//!
//! Limits are recorded in the snapshot manifest so that a snapshot records
//! the exact policy that was in force when it was created.

use serde::{Deserialize, Serialize};

/// Hard limits for one intake. Defaults are conservative for a local PoC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeLimits {
    /// Maximum size of the source file itself.
    pub max_source_bytes: u64,
    /// Maximum size of a single dependency file.
    pub max_dependency_bytes: u64,
    /// Maximum combined size of all dependencies.
    pub max_total_dependency_bytes: u64,
    /// Maximum number of dependencies.
    pub max_dependency_count: usize,
    /// Maximum length of a single declared reference string.
    pub max_reference_len: usize,
}

impl Default for IntakeLimits {
    fn default() -> Self {
        const MIB: u64 = 1024 * 1024;
        IntakeLimits {
            max_source_bytes: 512 * MIB,
            max_dependency_bytes: 256 * MIB,
            max_total_dependency_bytes: 1024 * MIB,
            max_dependency_count: 4096,
            max_reference_len: 1024,
        }
    }
}
