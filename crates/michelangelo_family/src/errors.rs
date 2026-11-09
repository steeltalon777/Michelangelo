//! Typed planning errors. Codes are provisional and aligned with the GLM-1
//! draft NEGATIVE_TEST_MATRIX (F6/F7/F9/F10/F12/F17) — see INTEGRATION_NOTES.md.
//! None of these codes are approved wire values until the contract gate lands.

use std::fmt;

/// Stable machine-readable planning rejection codes (PROVISIONAL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyErrorCode {
    /// F6: role outside the closed set, or donor missing `allowed_parts`.
    RoleInvalid,
    /// F7: attempt to widen recipe/workspace scope via a request or recipe field.
    ScopeWidening,
    /// F9 (planning-level analogue): parameter binding or donor part targeting a
    /// protected role. Full effective-result F9 evidence is GLM-5/GLM-6 scope.
    ProtectedRoleTarget,
    /// F10: `allow_arbitrary_code: true`, script/eval payloads, free script paths.
    ArbitraryCode,
    /// F12: floating `latest` or range/version placeholder in a lock.
    FloatingVersion,
    /// F17: unknown / out-of-range / incomplete / wrong-typed parameter value.
    ParameterInvalid,
    /// Held-out source feeding the generator, or held-out/build role mixing.
    HeldoutLeakage,
    /// Required socket for a chosen module value is absent from the socket profile.
    SocketMismatch,
    /// Same source_id or item_id twice within one identity scope.
    DuplicateIdentity,
    /// Skinned donors carrying different skeleton profile digests (§11.2 analogue).
    ForeignSkeleton,
    /// V2–V4 capability or unknown generator kind requested at plan time.
    CapabilityDenied,
    /// Lock dependency hash no longer matches the recipe/source-set digests.
    StaleDependency,
    /// Lock references a recipe revision it was not derived from.
    LockMismatch,
    /// Fewer unique valid combinations exist than distinct requests demanded —
    /// honest failure; no fabricated variants.
    InsufficientUniqueCombinations,
    /// Cartesian product size exceeds the recipe's combinatorial cap.
    CombinatorialCapExceeded,
    /// Wire document shape invalid (unknown/missing fields, bad tokens).
    SchemaInvalid,
    /// Execution-time boundary: this crate is pure planning only.
    NotPlannable,
}

impl FamilyErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            FamilyErrorCode::RoleInvalid => "ROLE_INVALID",
            FamilyErrorCode::ScopeWidening => "SCOPE_WIDENING",
            FamilyErrorCode::ProtectedRoleTarget => "PROTECTED_ROLE_TARGET",
            FamilyErrorCode::ArbitraryCode => "ARBITRARY_CODE",
            FamilyErrorCode::FloatingVersion => "FLOATING_VERSION",
            FamilyErrorCode::ParameterInvalid => "PARAMETER_INVALID",
            FamilyErrorCode::HeldoutLeakage => "HELDOUT_LEAKAGE",
            FamilyErrorCode::SocketMismatch => "SOCKET_MISMATCH",
            FamilyErrorCode::DuplicateIdentity => "DUPLICATE_IDENTITY",
            FamilyErrorCode::ForeignSkeleton => "FOREIGN_SKELETON",
            FamilyErrorCode::CapabilityDenied => "CAPABILITY_DENIED",
            FamilyErrorCode::StaleDependency => "STALE_DEPENDENCY",
            FamilyErrorCode::LockMismatch => "LOCK_MISMATCH",
            FamilyErrorCode::InsufficientUniqueCombinations => "INSUFFICIENT_UNIQUE_COMBINATIONS",
            FamilyErrorCode::CombinatorialCapExceeded => "COMBINATORIAL_CAP_EXCEEDED",
            FamilyErrorCode::SchemaInvalid => "SCHEMA_INVALID",
            FamilyErrorCode::NotPlannable => "NOT_PLANNABLE",
        }
    }
}

impl fmt::Display for FamilyErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A planning diagnostic / rejection with stable code and a human detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyError {
    pub code: FamilyErrorCode,
    pub detail: String,
}

impl FamilyError {
    pub fn new(code: FamilyErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for FamilyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for FamilyError {}

pub type FamilyResult<T> = Result<T, FamilyError>;
