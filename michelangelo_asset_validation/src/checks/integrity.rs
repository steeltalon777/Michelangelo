//! Integrity checks: payload_integrity, asset_digest_full, dependency_closure,
//! path_containment. All of them measure the bytes actually read from the
//! workspace (IT-8D D1.1/D1.2 analogs for the new bundle path).

use crate::canonical::sha256_hex;
use crate::check::{CheckId, CheckRecord, CheckStatus, DocRef};
use crate::context::ValidationContext;
use crate::error::VALIDATION_UNKNOWN_REQUIRED;
use crate::profile::{ProfileKind, ValidationProfile};

/// Reference keys understood by the evaluators (conventional names of the
/// `references.reference_digests` map).
pub mod reference_keys {
    /// Pinned expected asset digest of the bundle (pinned before generation).
    pub const EXPECTED_ASSET_DIGEST: &str = "expected_asset_digest";
    /// Pinned expected dependency digest of the bundle.
    pub const EXPECTED_DEPENDENCY_DIGEST: &str = "expected_dependency_digest";
    /// Pinned source payload digests for passthrough (source unchanged).
    pub const SOURCE_PAYLOAD_DIGESTS: &str = "source_payload_digests";
    /// Pinned protected-scene expectations (protected_roles).
    pub const PROTECTED_SCENE_LAYOUT: &str = "protected_scene_layout";
    /// Pinned connector profile (connector_compatibility).
    pub const CONNECTOR_PROFILE: &str = "connector_profile";
    /// Pinned expected per-file UV counts (uv_preservation).
    pub const EXPECTED_UV_COUNTS: &str = "expected_uv_counts";
    /// Pinned skeleton profile (rig_bindings).
    pub const SKELETON_PROFILE: &str = "skeleton_profile";
    /// Pinned silhouette reference (optional for rig-compatible kinds).
    pub const SILHOUETTE_REFERENCE: &str = "silhouette_reference";
}

/// The resolved state of a pinned reference.
pub(crate) enum PinnedRef {
    /// The reference exists and its digest matches the pinned value.
    Present(serde_json::Value, String),
    /// The profile does not pin this reference.
    Missing,
    /// The workspace content of the reference differs from the pinned digest
    /// (stale basis / substitution attempt).
    Changed(String),
}

/// Resolve a pinned reference, separating "not pinned" (Missing) from
/// "pinned but changed" (Changed) from operational failures (Err).
pub(crate) fn resolve_pinned(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
    key: &str,
) -> Result<PinnedRef, crate::error::ValidationError> {
    match context.resolve_reference(profile, key) {
        Ok((value, digest)) => Ok(PinnedRef::Present(value, digest)),
        Err(err) => match err.code() {
            crate::error::REFERENCE_MISSING => Ok(PinnedRef::Missing),
            crate::error::DEPENDENCY_CHANGED => Ok(PinnedRef::Changed(err.message().to_string())),
            _ => Err(err),
        },
    }
}

fn record(
    check: CheckId,
    result: CheckStatus,
    value: Option<f64>,
    detail: String,
    evidence: Vec<DocRef>,
) -> CheckRecord {
    CheckRecord {
        check_id: check,
        result,
        value,
        detail: Some(detail),
        applicability_ref: None,
        evidence,
    }
}

fn pass(check: CheckId, value: Option<f64>, detail: String, evidence: Vec<DocRef>) -> CheckRecord {
    record(check, CheckStatus::Pass, value, detail, evidence)
}

fn fail(check: CheckId, detail: String, evidence: Vec<DocRef>) -> CheckRecord {
    record(check, CheckStatus::Fail, None, detail, evidence)
}

fn unavailable(check: CheckId, detail: String) -> CheckRecord {
    record(check, CheckStatus::Unavailable, None, detail, Vec::new())
}

/// Every artifact: the actual bytes exist, size matches, sha256 matches the
/// declared digest. For passthrough profiles with a pinned
/// `source_payload_digests` reference, the actual bytes must additionally
/// match the pinned source digests (source unchanged).
pub fn check_payload_integrity(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, crate::error::ValidationError> {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    let mut verified = 0_usize;
    let artifacts = context.manifest().artifacts.clone();
    for artifact in &artifacts {
        let read = context.read_bytes(&artifact.path);
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(_) => {
                failures.push(format!(
                    "SNAPSHOT_MISMATCH: the artifact '{}' ('{}') is missing or unreadable",
                    artifact.artifact_id, artifact.path
                ));
                continue;
            }
        };
        let actual = sha256_hex(&bytes);
        if actual != artifact.sha256 {
            failures.push(format!(
                "SNAPSHOT_MISMATCH: the artifact '{}' declares {} but the actual bytes hash {}",
                artifact.artifact_id, artifact.sha256, actual
            ));
            continue;
        }
        if bytes.len() as u64 != artifact.size_bytes {
            failures.push(format!(
                "SNAPSHOT_MISMATCH: the artifact '{}' declares {} bytes but the actual size is {}",
                artifact.artifact_id,
                artifact.size_bytes,
                bytes.len()
            ));
            continue;
        }
        evidence.push(DocRef {
            path: artifact.path.clone(),
            sha256: actual,
        });
        verified += 1;
    }

    // Passthrough: the source payload must be unchanged.
    if profile.kind == ProfileKind::PassthroughPreservation {
        match resolve_pinned(context, profile, reference_keys::SOURCE_PAYLOAD_DIGESTS)? {
            PinnedRef::Present(value, _) => {
                if let Some(map) = value.as_object() {
                    for (path, declared) in map {
                        let pinned = declared.as_str().unwrap_or_default().to_string();
                        match context.read_bytes(path) {
                            Ok(bytes) => {
                                let actual = sha256_hex(&bytes);
                                if actual != pinned {
                                    failures.push(format!(
                                        "SNAPSHOT_MISMATCH: the source payload '{}' changed: \
                                         pinned {}, actual {}",
                                        path, pinned, actual
                                    ));
                                }
                            }
                            Err(_) => failures.push(format!(
                                "DEPENDENCY_MISSING: the source payload '{}' is missing",
                                path
                            )),
                        }
                    }
                }
            }
            PinnedRef::Changed(message) => {
                failures.push(format!("SNAPSHOT_MISMATCH: {message}"));
            }
            PinnedRef::Missing => {
                // The passthrough profile must pin the source digests; without
                // them the preservation claim cannot be verified.
                return Ok(unavailable(
                    CheckId::PayloadIntegrity,
                    "the passthrough profile does not pin the source_payload_digests \
                     reference; source preservation cannot be verified"
                        .to_string(),
                ));
            }
        }
    }

    if failures.is_empty() {
        Ok(pass(
            CheckId::PayloadIntegrity,
            Some(verified as f64),
            format!("{verified} artifacts verified against the actually read bytes"),
            evidence,
        ))
    } else {
        Ok(fail(
            CheckId::PayloadIntegrity,
            failures.join("; "),
            evidence,
        ))
    }
}

/// The recomputed asset digest over the actually read artifact bytes must
/// match the manifest declaration and the pinned expected digest. This is
/// the anti-substitution guard: replacing geometry AND repairing the manifest
/// changes the digest and no longer matches the value pinned before
/// generation (F1).
pub fn check_asset_digest_full(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, crate::error::ValidationError> {
    // The actual bytes have been read by payload_integrity (ordered before
    // this check); recompute from the recorded input digests.
    let manifest = context.manifest().clone();
    let mut entries: Vec<(String, String, String, String)> = Vec::new();
    let inputs = context.input_digests();
    let mut missing = Vec::new();
    for artifact in &manifest.artifacts {
        match inputs.get(&artifact.path) {
            Some(actual) => entries.push((
                artifact.artifact_id.clone(),
                artifact.path.clone(),
                artifact.content_kind.clone(),
                actual.clone(),
            )),
            None => missing.push(artifact.path.clone()),
        }
    }
    if !missing.is_empty() {
        return Ok(unavailable(
            CheckId::AssetDigestFull,
            format!(
                "the artifacts {:?} were not read; the asset digest cannot be recomputed",
                missing
            ),
        ));
    }
    let recomputed = crate::canonical::asset_digest(&entries);
    let mut failures = Vec::new();
    if let Some(declared) = &manifest.asset_digest {
        if declared != &recomputed {
            failures.push(format!(
                "SNAPSHOT_MISMATCH: the manifest declares asset_digest {} but the actual \
                 artifacts digest {}",
                declared, recomputed
            ));
        }
    } else {
        failures.push("SNAPSHOT_MISMATCH: the manifest does not declare an asset_digest".to_string());
    }
    let mut pinned_digest: Option<String> = None;
    match resolve_pinned(context, profile, reference_keys::EXPECTED_ASSET_DIGEST)? {
        PinnedRef::Present(value, _) => {
            if let Some(expected) = value.get("asset_digest").and_then(|v| v.as_str()) {
                pinned_digest = Some(expected.to_string());
                if expected != recomputed {
                    failures.push(format!(
                        "SNAPSHOT_MISMATCH: the pinned expected_asset_digest {} does not match \
                         the recomputed digest {} (manifest substitution or payload repair)",
                        expected, recomputed
                    ));
                }
            }
        }
        PinnedRef::Changed(message) => {
            failures.push(format!("SNAPSHOT_MISMATCH: {message}"));
        }
        PinnedRef::Missing => {
            return Ok(unavailable(
                CheckId::AssetDigestFull,
                "the profile does not pin the expected_asset_digest reference; the bundle \
                 cannot be verified against the pre-generation snapshot"
                    .to_string(),
            ));
        }
    }
    let evidence = vec![DocRef {
        path: "asset-digest".to_string(),
        sha256: recomputed.clone(),
    }];
    if failures.is_empty() {
        Ok(pass(
            CheckId::AssetDigestFull,
            None,
            format!(
                "asset digest {} recomputed from the actually read bytes and matches the \
                 pinned expectation {}",
                recomputed,
                pinned_digest.unwrap_or_default()
            ),
            evidence,
        ))
    } else {
        Ok(fail(CheckId::AssetDigestFull, failures.join("; "), evidence))
    }
}

/// The transitive dependency closure: every declared dependency exists with
/// the declared digest; the aggregate dependency digest matches the manifest
/// declaration and the pinned expectation (a changed shared material or node
/// group invalidates the candidate, F9/F19 class).
pub fn check_dependency_closure(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, crate::error::ValidationError> {
    let manifest = context.manifest().clone();
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    for dep in &manifest.dependencies {
        let bytes = match context.read_bytes(&dep.path) {
            Ok(bytes) => bytes,
            Err(_) => {
                failures.push(format!(
                    "DEPENDENCY_MISSING: the dependency '{}' ('{}') is missing or unreadable",
                    dep.dep_id, dep.path
                ));
                continue;
            }
        };
        let actual = sha256_hex(&bytes);
        if actual != dep.sha256 {
            failures.push(format!(
                "DEPENDENCY_CHANGED: the dependency '{}' declares {} but the actual bytes \
                 hash {}",
                dep.dep_id, dep.sha256, actual
            ));
            continue;
        }
        evidence.push(DocRef {
            path: dep.path.clone(),
            sha256: actual,
        });
    }

    let recomputed = manifest.recomputed_dependency_digest();
    if let Some(declared) = &manifest.dependency_digest {
        if declared != &recomputed {
            failures.push(format!(
                "DEPENDENCY_CHANGED: the manifest declares dependency_digest {} but the \
                 actual aggregate is {}",
                declared, recomputed
            ));
        }
    }
    match resolve_pinned(context, profile, reference_keys::EXPECTED_DEPENDENCY_DIGEST)? {
        PinnedRef::Present(value, _) => {
            if let Some(expected) = value.get("dependency_digest").and_then(|v| v.as_str()) {
                if expected != recomputed {
                    failures.push(format!(
                        "DEPENDENCY_CHANGED: the pinned expected_dependency_digest {} does not \
                         match the recomputed aggregate {}; the dependency set changed after \
                         the lock",
                        expected, recomputed
                    ));
                }
            }
        }
        PinnedRef::Changed(message) => {
            failures.push(format!("DEPENDENCY_CHANGED: {message}"));
        }
        PinnedRef::Missing => {
            // Optional pin: without it only manifest self-consistency holds.
        }
    }

    if failures.is_empty() {
        Ok(pass(
            CheckId::DependencyClosure,
            Some(manifest.dependencies.len() as f64),
            format!(
                "{} dependencies verified; aggregate dependency_digest {}",
                manifest.dependencies.len(),
                recomputed
            ),
            evidence,
        ))
    } else {
        Ok(fail(CheckId::DependencyClosure, failures.join("; "), evidence))
    }
}

/// Every artifact and dependency path is a contained workspace-relative path
/// (F13 class). The context already refuses an escaping manifest path at
/// open(); this check also refuses escaping artifact/dependency paths and
/// verifies containment of the resolved location.
pub fn check_path_containment(
    context: &mut ValidationContext,
) -> Result<CheckRecord, crate::error::ValidationError> {
    let manifest = context.manifest().clone();
    let mut failures = Vec::new();
    for artifact in &manifest.artifacts {
        if let Err(err) = crate::ids::validate_rel_path(&artifact.path) {
            failures.push(format!("PATH_ESCAPE: artifact '{}': {}", artifact.artifact_id, err.message()));
            continue;
        }
        if let Err(err) = crate::ids::resolve_under(context.root(), &artifact.path) {
            failures.push(format!("PATH_ESCAPE: artifact '{}': {}", artifact.artifact_id, err.message()));
        }
    }
    for dep in &manifest.dependencies {
        if let Err(err) = crate::ids::validate_rel_path(&dep.path) {
            failures.push(format!("PATH_ESCAPE: dependency '{}': {}", dep.dep_id, err.message()));
            continue;
        }
        if let Err(err) = crate::ids::resolve_under(context.root(), &dep.path) {
            failures.push(format!("PATH_ESCAPE: dependency '{}': {}", dep.dep_id, err.message()));
        }
    }
    if failures.is_empty() {
        Ok(pass(
            CheckId::PathContainment,
            Some((manifest.artifacts.len() + manifest.dependencies.len()) as f64),
            "all artifact and dependency paths stay inside the workspace container".to_string(),
            Vec::new(),
        ))
    } else {
        Ok(fail(CheckId::PathContainment, failures.join("; "), Vec::new()))
    }
}

/// Defensive helper used by the evaluator: every required check id must come
/// from the closed registry (F8 belt-and-braces guard; the profile layer
/// already refuses unknown ids at load time).
pub fn assert_registry_ids(profile: &ValidationProfile) -> Result<(), crate::error::ValidationError> {
    for check in &profile.required_checks {
        if crate::check::CheckId::ALL.iter().all(|known| known != check) {
            return Err(crate::error::ValidationError::rule(
                VALIDATION_UNKNOWN_REQUIRED,
                format!("the required check '{}' is unknown", check.as_str()),
            ));
        }
    }
    Ok(())
}
