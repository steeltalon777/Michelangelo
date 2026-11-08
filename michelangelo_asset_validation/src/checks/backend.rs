//! Backend-dependent checks and the verifying-backend traits.
//!
//! Without a verifying backend the results are `unavailable` with an
//! explicit reason — never a pass (ADR 0005 §8.1: missing GT/animation test
//! is unavailable; a Workbench render does not check PBR; successful import
//! without playing clips is not rig compatibility). The traits are the
//! integration seams for the future Blender worker roundtrip / clip
//! playback backends.

use crate::check::{CheckId, CheckRecord, CheckStatus, DocRef};
use crate::context::ValidationContext;
use crate::error::ValidationError;
use crate::profile::ValidationProfile;

/// The outcome of a verifying backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendOutcome {
    /// The backend verified the property.
    Verified {
        detail: String,
        evidence: Vec<DocRef>,
    },
    /// The backend measured a violation.
    Failed { detail: String },
    /// The backend could not verify (its own honest unavailability).
    Unavailable { reason: String },
}

/// A request for the export roundtrip verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundtripRequest {
    /// The workspace-relative path of the exported exchange artifact.
    pub artifact_path: String,
    /// The expected digest of the artifact bytes.
    pub artifact_sha256: String,
    /// The target format ("glb").
    pub format: String,
}

/// The export roundtrip verifying backend (future Blender worker op; the
/// current worker has no such op — BLOCKED, see INTEGRATION_NOTES.md).
pub trait ExportRoundtripBackend {
    fn roundtrip(&self, request: &RoundtripRequest) -> BackendOutcome;
}

/// A request for clip playback verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipPlaybackRequest {
    /// The workspace-relative path of the clip.
    pub clip_path: String,
    /// The expected digest of the clip bytes.
    pub clip_sha256: String,
}

/// The clip playback verifying backend (future worker/animation integration).
pub trait ClipPlaybackBackend {
    fn play(&self, request: &ClipPlaybackRequest) -> BackendOutcome;
}

/// The honest default: no verifying backend is integrated in this module.
pub struct NoBackend;

impl ExportRoundtripBackend for NoBackend {
    fn roundtrip(&self, _request: &RoundtripRequest) -> BackendOutcome {
        BackendOutcome::Unavailable {
            reason: "no export roundtrip verifying backend is integrated (BLOCKED: the \
                     current Blender worker has no roundtrip op; ADR 0005 §2)"
                .to_string(),
        }
    }
}

impl ClipPlaybackBackend for NoBackend {
    fn play(&self, _request: &ClipPlaybackRequest) -> BackendOutcome {
        BackendOutcome::Unavailable {
            reason: "no clip playback verifying backend is integrated (BLOCKED: animation \
                     playback needs the future worker integration)"
                .to_string(),
        }
    }
}

/// Export roundtrip: only meaningful when the bundle declares an export
/// output. Required-but-not-declared is a profile/bundle conflict (fail);
/// declared-but-unverifiable is unavailable.
pub fn check_export_roundtrip(
    context: &mut ValidationContext,
    backend: &dyn ExportRoundtripBackend,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    if !manifest.outputs.export_glb {
        return Ok(CheckRecord {
            check_id: CheckId::ExportRoundtrip,
            result: CheckStatus::Fail,
            value: None,
            detail: Some(
                "PROFILE_BUNDLE_CONFLICT: the profile requires export_roundtrip but the \
                 bundle declares no export_glb output"
                    .to_string(),
            ),
            applicability_ref: None,
            evidence: Vec::new(),
        });
    }
    let export_artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(".glb"))
        .cloned();
    let Some(export_artifact) = export_artifact else {
        return Ok(CheckRecord {
            check_id: CheckId::ExportRoundtrip,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(
                "the bundle declares export_glb but contains no .glb artifact to verify"
                    .to_string(),
            ),
            applicability_ref: None,
            evidence: Vec::new(),
        });
    };
    let outcome = backend.roundtrip(&RoundtripRequest {
        artifact_path: export_artifact.path.clone(),
        artifact_sha256: export_artifact.sha256.clone(),
        format: "glb".to_string(),
    });
    Ok(match outcome {
        BackendOutcome::Verified { detail, evidence } => CheckRecord {
            check_id: CheckId::ExportRoundtrip,
            result: CheckStatus::Pass,
            value: None,
            detail: Some(detail),
            applicability_ref: None,
            evidence,
        },
        BackendOutcome::Failed { detail } => CheckRecord {
            check_id: CheckId::ExportRoundtrip,
            result: CheckStatus::Fail,
            value: None,
            detail: Some(detail),
            applicability_ref: None,
            evidence: Vec::new(),
        },
        BackendOutcome::Unavailable { reason } => CheckRecord {
            check_id: CheckId::ExportRoundtrip,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(reason),
            applicability_ref: None,
            evidence: Vec::new(),
        },
    })
}

/// Animation clips: the pinned clips must be present in the bundle
/// dependencies with matching digests, and playback must be verified by a
/// backend. Digest match alone is NOT a pass without playback (ADR §4.2/§8.1).
pub fn check_animation_clips(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
    backend: &dyn ClipPlaybackBackend,
) -> Result<CheckRecord, ValidationError> {
    let pinned = &profile.references.animation_clips;
    if pinned.is_empty() {
        return Ok(CheckRecord {
            check_id: CheckId::AnimationClips,
            result: CheckStatus::Unavailable,
            value: None,
            detail: Some(
                "the profile pins no animation clips; the coverage is empty and a clip set \
                 cannot be verified"
                    .to_string(),
            ),
            applicability_ref: None,
            evidence: Vec::new(),
        });
    }
    let manifest = context.manifest().clone();
    let mut failures = Vec::new();
    let mut verified = Vec::new();
    for clip in pinned {
        match manifest.dependencies.iter().find(|dep| dep.path == clip.path) {
            Some(dep) if dep.sha256 == clip.sha256 => {
                verified.push(clip.clone());
            }
            Some(dep) => failures.push(format!(
                "CLIP_CHANGED: the pinned clip '{}' hashes {} but the bundle dependency \
                 hashes {}",
                clip.path, clip.sha256, dep.sha256
            )),
            None => failures.push(format!(
                "CLIP_MISSING: the pinned clip '{}' is not part of the dependency closure",
                clip.path
            )),
        }
    }
    if !failures.is_empty() {
        return Ok(CheckRecord {
            check_id: CheckId::AnimationClips,
            result: CheckStatus::Fail,
            value: None,
            detail: Some(failures.join("; ")),
            applicability_ref: None,
            evidence: Vec::new(),
        });
    }
    // Playback verification over the digest-verified clips.
    let mut playback_failures = Vec::new();
    let mut played = 0_usize;
    for clip in &verified {
        match backend.play(&ClipPlaybackRequest {
            clip_path: clip.path.clone(),
            clip_sha256: clip.sha256.clone(),
        }) {
            BackendOutcome::Verified { .. } => played += 1,
            BackendOutcome::Failed { detail } => playback_failures.push(format!(
                "CLIP_PLAYBACK_FAILED: '{}': {detail}",
                clip.path
            )),
            BackendOutcome::Unavailable { reason } => {
                return Ok(CheckRecord {
                    check_id: CheckId::AnimationClips,
                    result: CheckStatus::Unavailable,
                    value: None,
                    detail: Some(format!(
                        "the pinned clips' digests are verified, but playback verification is \
                         unavailable: {reason}"
                    )),
                    applicability_ref: None,
                    evidence: verified.clone(),
                });
            }
        }
    }
    if playback_failures.is_empty() {
        Ok(CheckRecord {
            check_id: CheckId::AnimationClips,
            result: CheckStatus::Pass,
            value: Some(played as f64),
            detail: Some(format!("{played} pinned clips verified and played")),
            applicability_ref: None,
            evidence: verified,
        })
    } else {
        Ok(CheckRecord {
            check_id: CheckId::AnimationClips,
            result: CheckStatus::Fail,
            value: None,
            detail: Some(playback_failures.join("; ")),
            applicability_ref: None,
            evidence: verified,
        })
    }
}
