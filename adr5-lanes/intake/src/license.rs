//! Provenance and license status of a source asset.
//!
//! Rule (hard requirement of this stream): an unknown license is never
//! treated as commercial permission. This PoC does not interpret SPDX terms
//! either — a declared license is recorded as `declared_not_evaluated`, and
//! only the owner can establish usage rights outside this module.

use serde::{Deserialize, Serialize};

/// License status as supplied with the intake request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LicenseStatus {
    /// No license information available. Never a permission.
    #[default]
    Unknown,
    /// A license identifier (e.g. an SPDX id) was declared by the supplier.
    /// Recorded verbatim; terms are NOT evaluated by this crate.
    Declared { spdx: String },
}

/// What the intake pipeline may conclude about the license.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum LicenseDecision {
    /// A license was declared; its terms were not evaluated here.
    DeclaredNotEvaluated { spdx: String },
    /// License unknown. Explicitly not a permission of any kind.
    UnknownNoPermission,
}

/// Provenance metadata supplied with the request. All fields are optional;
/// missing supplier information is recorded as-is, never invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProvenanceInfo {
    pub supplier: Option<String>,
    pub external_id: Option<String>,
    pub license: LicenseStatus,
    pub notes: Option<String>,
}

impl ProvenanceInfo {
    pub fn unknown() -> Self {
        ProvenanceInfo {
            license: LicenseStatus::Unknown,
            ..Default::default()
        }
    }
}

pub fn license_decision(status: &LicenseStatus) -> LicenseDecision {
    match status {
        LicenseStatus::Unknown => LicenseDecision::UnknownNoPermission,
        LicenseStatus::Declared { spdx } => {
            LicenseDecision::DeclaredNotEvaluated { spdx: spdx.clone() }
        }
    }
}

/// The only usage-rights rule enforced by intake: unknown provenance does not
/// authorize anything. Returns `false` for `Unknown`; returns `false` for
/// declared licenses too, because term evaluation is out of scope for this
/// PoC and must not be faked.
pub fn grants_commercial_permission(status: &LicenseStatus) -> bool {
    let _ = status;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_license_is_never_a_permission() {
        assert!(!grants_commercial_permission(&LicenseStatus::Unknown));
        assert!(!grants_commercial_permission(&LicenseStatus::Declared {
            spdx: "CC0-1.0".into()
        }));
        assert_eq!(
            license_decision(&LicenseStatus::Unknown),
            LicenseDecision::UnknownNoPermission
        );
    }
}
