//! Rig binding checks (fixture level). Real `.gltf` skins and skin weights
//! are NOT parsed by this module; the check compares the provisional
//! rig_definition artifact against the pinned skeleton profile.

use std::collections::{BTreeMap, BTreeSet};

use crate::check::{CheckId, CheckRecord, CheckStatus, DocRef};
use crate::checks::parsing::parse_rig;
use crate::checks::integrity::{resolve_pinned, PinnedRef, reference_keys};
use crate::context::ValidationContext;
use crate::error::ValidationError;
use crate::profile::ValidationProfile;

/// The skeleton identity of the bundle rig (bone ids, parents, rest
/// transforms) must match the pinned skeleton profile exactly. Coinciding
/// bone NAMES alone are not proof (ADR §4.2): the rest transforms and the
/// parent structure are part of the comparison, and the pinned profile
/// digest is verified before use.
pub fn check_rig_bindings(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let rig_artifact = match manifest
        .artifacts
        .iter()
        .find(|a| a.content_kind == "rig_definition")
        .cloned()
    {
        Some(artifact) => artifact,
        None => {
            return Ok(CheckRecord {
                check_id: CheckId::RigBindings,
                result: CheckStatus::Unavailable,
                value: None,
                detail: Some(
                    "the bundle has no rig_definition artifact; a rig parser/worker backend \
                     integration is pending"
                        .to_string(),
                ),
                applicability_ref: None,
                evidence: Vec::new(),
            });
        }
    };
    let rig_bytes = context.read_bytes(&rig_artifact.path)?;
    let rig = match parse_rig(&rig_bytes) {
        Ok(rig) => rig,
        Err(err) => {
            return Ok(CheckRecord {
                check_id: CheckId::RigBindings,
                result: CheckStatus::Unavailable,
                value: None,
                detail: Some(format!("the rig_definition artifact is unusable: {}", err.message())),
                applicability_ref: None,
                evidence: Vec::new(),
            });
        }
    };
    let (pinned_value, pinned_digest) = match resolve_pinned(context, profile, reference_keys::SKELETON_PROFILE)? {
        PinnedRef::Present(value, digest) => (value, digest),
        PinnedRef::Changed(message) => {
            return Ok(CheckRecord {
                check_id: CheckId::RigBindings,
                result: CheckStatus::Fail,
                value: None,
                detail: Some(format!("RIG_MISMATCH: {message}")),
                applicability_ref: None,
                evidence: Vec::new(),
            });
        }
        PinnedRef::Missing => {
            return Ok(CheckRecord {
                check_id: CheckId::RigBindings,
                result: CheckStatus::Unavailable,
                value: None,
                detail: Some(
                    "the profile does not pin the skeleton_profile reference; the rig \
                     identity cannot be verified"
                        .to_string(),
                ),
                applicability_ref: None,
                evidence: Vec::new(),
            });
        }
    };
    let pinned: crate::checks::parsing::SkeletonProfile = match serde_json::from_value(pinned_value) {
        Ok(value) => value,
        Err(err) => {
            return Ok(CheckRecord {
                check_id: CheckId::RigBindings,
                result: CheckStatus::Unavailable,
                value: None,
                detail: Some(format!("the skeleton profile reference is unusable: {err}")),
                applicability_ref: None,
                evidence: Vec::new(),
            });
        }
    };

    let mut failures = Vec::new();
    // Skeleton id, if the rig declares one.
    if let Some(skeleton_ref) = &rig.skeleton_ref {
        if skeleton_ref != &pinned.skeleton_id {
            failures.push(format!(
                "RIG_MISMATCH: the rig references skeleton '{}' but the pinned profile is '{}'",
                skeleton_ref, pinned.skeleton_id
            ));
        }
    }
    // Bone identity: ids, parents, rest transforms (map comparison; f64
    // transforms are compared exactly, see transforms_equal semantics).
    let pinned_by_id: BTreeMap<&str, &crate::checks::parsing::BoneDef> = pinned
        .bones
        .iter()
        .map(|bone| (bone.bone_id.as_str(), bone))
        .collect();
    let rig_by_id: BTreeMap<&str, &crate::checks::parsing::BoneDef> = rig
        .bones
        .iter()
        .map(|bone| (bone.bone_id.as_str(), bone))
        .collect();
    for (bone_id, pinned_bone) in &pinned_by_id {
        match rig_by_id.get(bone_id) {
            None => failures.push(format!(
                "RIG_MISMATCH: the bone '{}' of the pinned skeleton is missing in the rig \
                 definition (bone names alone are not compatibility)",
                bone_id
            )),
            Some(rig_bone) => {
                if rig_bone.parent != pinned_bone.parent
                    || !crate::checks::parsing::transforms_equal(&rig_bone.rest, &pinned_bone.rest)
                {
                    failures.push(format!(
                        "RIG_MISMATCH: the bone '{}' altered its parent or rest/bind transform",
                        bone_id
                    ));
                }
            }
        }
    }
    for bone_id in rig_by_id.keys() {
        if !pinned_by_id.contains_key(*bone_id) {
            failures.push(format!(
                "RIG_MISMATCH: the rig defines an unexpected bone '{}' (new bones do not \
                 pass as a compatible variant)",
                bone_id
            ));
        }
    }
    // Bindings: every bound component points at a bone of the pinned skeleton.
    let pinned_ids: BTreeSet<&str> = pinned.bones.iter().map(|b| b.bone_id.as_str()).collect();
    for binding in &rig.bindings {
        if !pinned_ids.contains(binding.bone_id.as_str()) {
            failures.push(format!(
                "RIG_MISMATCH: the component '{}' is bound to the unknown bone '{}'",
                binding.component_id, binding.bone_id
            ));
        }
    }

    let evidence = vec![DocRef {
        path: rig_artifact.path.clone(),
        sha256: pinned_digest,
    }];
    if failures.is_empty() {
        Ok(CheckRecord {
            check_id: CheckId::RigBindings,
            result: CheckStatus::Pass,
            value: Some(rig.bones.len() as f64),
            detail: Some(format!(
                "{} bones and {} bindings match the pinned skeleton profile '{}'",
                rig.bones.len(),
                rig.bindings.len(),
                pinned.skeleton_id
            )),
            applicability_ref: None,
            evidence,
        })
    } else {
        Ok(CheckRecord {
            check_id: CheckId::RigBindings,
            result: CheckStatus::Fail,
            value: None,
            detail: Some(failures.join("; ")),
            applicability_ref: None,
            evidence,
        })
    }
}
