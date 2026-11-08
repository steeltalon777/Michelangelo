//! The uncertainty policy of an input package (IT-8C).
//!
//! The policy of the package describes how the uncertainty of a build may be
//! resolved. It is the second level of the precedence of the contract
//! `docs/specs/VERIFICATION_STATE.md` §8:
//!
//! 1. the author decisions that are accepted inside the package,
//! 2. the `uncertainty_policy` of the package,
//! 3. the policy of the workspace,
//! 4. the flags of the candidate or of the task.
//!
//! A lower level can never weaken a higher one **for the author's geometric
//! intent** (supplements, the completion rules of an unknown, the acceptance
//! of a hidden choice). The security constraints — file and network access,
//! budgets, the protected scope, the sandbox and the secrets — are
//! independent: no level of the hierarchy may weaken them.
//!
//! The helper reads the policy of a manifest without touching the resources
//! of the package: the Core needs the policy of the copy of a run, not a
//! validation of the package.

use serde_json::Value;

use crate::manifest::{AcceptanceStatus, InputPackage, StubPolicy};

/// The default of `require_author_acceptance` (the schema default).
pub const DEFAULT_REQUIRE_AUTHOR_ACCEPTANCE: bool = true;

/// The prefix of the acceptance target of a supplement.
pub const SUPPLEMENT_TARGET_PREFIX: &str = "supplements:";
/// The prefix of the acceptance target of an unknown.
pub const UNKNOWN_TARGET_PREFIX: &str = "unknowns:";
/// The prefix of the acceptance target of a measurement.
pub const MEASUREMENT_TARGET_PREFIX: &str = "measurements:";
/// The prefix of the acceptance target of a part.
pub const PART_TARGET_PREFIX: &str = "parts:";
/// The prefix of the acceptance target of a view.
pub const VIEW_TARGET_PREFIX: &str = "views:";

/// The uncertainty policy of an input package.
///
/// The policy is read defensively: a package that carries no
/// `uncertainty_policy` object has no enforced policy (`policy_present` is
/// false) and keeps the Stage B behaviour (every supplement is applied, stubs
/// are allowed). A package that carries the object uses the defaults of the
/// schema for the fields it omits (`require_author_acceptance` defaults to
/// `true`); the enforcement gate is `policy_present && require_author_acceptance`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePolicy {
    /// Whether the package carries an `uncertainty_policy` object.
    pub policy_present: bool,
    /// Whether an author acceptance is required before an AI-authored
    /// supplement may be applied.
    pub require_author_acceptance: bool,
    /// The completion rules the package allows. An empty list does not
    /// restrict the applied kinds.
    pub allowed_completion_rules: Vec<String>,
    /// Whether stubs are allowed.
    pub stub_policy: StubPolicy,
    /// The targets of the accepted author decisions of the package.
    pub accepted_targets: Vec<String>,
}

impl Default for PackagePolicy {
    fn default() -> Self {
        PackagePolicy {
            policy_present: false,
            require_author_acceptance: DEFAULT_REQUIRE_AUTHOR_ACCEPTANCE,
            allowed_completion_rules: Vec::new(),
            stub_policy: StubPolicy::Allowed,
            accepted_targets: Vec::new(),
        }
    }
}

impl PackagePolicy {
    /// The policy of a parsed manifest.
    pub fn from_manifest(package: &InputPackage) -> PackagePolicy {
        let mut policy = PackagePolicy::default();
        if let Some(uncertainty) = &package.uncertainty_policy {
            policy.policy_present = true;
            policy.require_author_acceptance = uncertainty.require_author_acceptance;
            policy.allowed_completion_rules = uncertainty.allowed_completion_rules.clone();
            policy.stub_policy = uncertainty.stub_policy;
        }
        for acceptance in &package.acceptance {
            if acceptance.status == AcceptanceStatus::Accepted {
                policy.accepted_targets.push(acceptance.target.clone());
            }
        }
        policy.accepted_targets.sort();
        policy.accepted_targets.dedup();
        policy
    }

    /// The policy of a raw manifest document.
    ///
    /// The function never fails: a document that carries no policy object
    /// yields the default policy (the behaviour of the stage B solver).
    pub fn from_json(package: &Value) -> PackagePolicy {
        let mut policy = PackagePolicy::default();
        if let Some(uncertainty) = package.get("uncertainty_policy") {
            policy.policy_present = true;
            policy.require_author_acceptance = uncertainty
                .get("require_author_acceptance")
                .and_then(|value| value.as_bool())
                .unwrap_or(DEFAULT_REQUIRE_AUTHOR_ACCEPTANCE);
            if let Some(rules) = uncertainty
                .get("allowed_completion_rules")
                .and_then(|value| value.as_array())
            {
                policy.allowed_completion_rules = rules
                    .iter()
                    .filter_map(|value| value.as_str().map(|text| text.to_string()))
                    .collect();
            }
            policy.stub_policy = match uncertainty
                .get("stub_policy")
                .and_then(|value| value.as_str())
            {
                Some("forbidden") => StubPolicy::Forbidden,
                _ => StubPolicy::Allowed,
            };
        }
        if let Some(entries) = package.get("acceptance").and_then(|value| value.as_array()) {
            for entry in entries {
                if entry.get("status").and_then(|value| value.as_str()) == Some("accepted") {
                    if let Some(target) = entry.get("target").and_then(|value| value.as_str()) {
                        policy.accepted_targets.push(target.to_string());
                    }
                }
            }
        }
        policy.accepted_targets.sort();
        policy.accepted_targets.dedup();
        policy
    }

    /// Whether the package accepts the target of an author decision.
    pub fn accepts_target(&self, target: &str) -> bool {
        self.accepted_targets.iter().any(|value| value == target)
    }

    /// Whether the package carries an accepted author decision for a
    /// supplement.
    pub fn accepts_supplement(&self, supplement_id: &str) -> bool {
        self.accepts_target(&format!("{SUPPLEMENT_TARGET_PREFIX}{supplement_id}"))
    }

    /// Whether the package carries an accepted author decision for an
    /// unknown.
    pub fn accepts_unknown(&self, unknown_id: &str) -> bool {
        self.accepts_target(&format!("{UNKNOWN_TARGET_PREFIX}{unknown_id}"))
    }

    /// Whether a completion rule may be applied.
    ///
    /// An empty list of allowed rules does not restrict the applied kinds:
    /// the list restricts only when it names at least one rule.
    pub fn rule_is_allowed(&self, kind: &str) -> bool {
        if self.allowed_completion_rules.is_empty() {
            return true;
        }
        self.allowed_completion_rules
            .iter()
            .any(|rule| rule.eq_ignore_ascii_case(kind))
    }

    /// Whether stubs may be used.
    pub fn stubs_allowed(&self) -> bool {
        self.stub_policy == StubPolicy::Allowed
    }

    /// Whether an AI-authored supplement needs an accepted author decision.
    ///
    /// A package that carries no `uncertainty_policy` object expresses no
    /// requirement: the behaviour of the stage B solver is kept. Inside the
    /// object the field defaults to `true`.
    pub fn requires_author_acceptance(&self) -> bool {
        self.policy_present && self.require_author_acceptance
    }
}
