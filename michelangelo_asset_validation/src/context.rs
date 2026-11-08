//! The validation context: the private provisional bundle manifest DTO, the
//! workspace reader that records the digests of the bytes actually read, and
//! the pinned-reference resolver.
//!
//! The manifest mirrors the field names of the GLM-1
//! `candidate_bundle.schema.json` draft but is a PRIVATE DTO of this module
//! (round 2 will align the DTOs; nothing here is an approved wire protocol).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::canonical::{dependency_digest, parse_json, sha256_hex};
use crate::error::ValidationError;
use crate::ids::{validate_rel_path, resolve_under};
use crate::profile::{ProfileKind, ValidationProfile};

/// One artifact of the measured bundle (private provisional DTO).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactEntry {
    pub artifact_id: String,
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub content_kind: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub protected: bool,
}

/// One dependency of the transitive closure (private provisional DTO).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyEntry {
    pub dep_id: String,
    pub path: String,
    pub sha256: String,
    pub revision: u64,
}

/// A declared format conversion (private provisional DTO).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionDecl {
    pub from: String,
    pub to: String,
}

/// A declared conversion loss (private provisional DTO).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossDecl {
    pub kind: String,
    pub detail: String,
}

/// Output declarations of the candidate (private provisional DTO).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputDecls {
    #[serde(default)]
    pub export_glb: bool,
    #[serde(default)]
    pub editable_blend: bool,
}

/// The measured candidate bundle manifest (private provisional DTO,
/// aligned with `michelangelo.candidate-asset-bundle/draft-0.1` field names).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema: String,
    pub candidate_id: String,
    pub artifacts: Vec<ArtifactEntry>,
    #[serde(default)]
    pub dependencies: Vec<DependencyEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_digest: Option<String>,
    #[serde(default)]
    pub conversions: Vec<ConversionDecl>,
    #[serde(default)]
    pub declared_losses: Vec<LossDecl>,
    #[serde(default)]
    pub outputs: OutputDecls,
}

impl BundleManifest {
    /// Parse the manifest from bytes; the schema tag is the private draft tag.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<BundleManifest, ValidationError> {
        let value = parse_json(bytes, "the bundle manifest")?;
        let manifest: BundleManifest = serde_json::from_value(value).map_err(|err| {
            ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!("the bundle manifest is not usable: {err}"),
            )
        })?;
        if manifest.schema != MANIFEST_SCHEMA {
            return Err(ValidationError::rule(
                crate::error::DOCUMENT_INVALID,
                format!(
                    "the manifest schema '{}' is not '{}'",
                    manifest.schema, MANIFEST_SCHEMA
                ),
            ));
        }
        Ok(manifest)
    }

    /// The recomputed asset digest over the declared artifact entries.
    pub fn recomputed_asset_digest(&self) -> String {
        let entries: Vec<(String, String, String, String)> = self
            .artifacts
            .iter()
            .map(|artifact| {
                (
                    artifact.artifact_id.clone(),
                    artifact.path.clone(),
                    artifact.content_kind.clone(),
                    artifact.sha256.clone(),
                )
            })
            .collect();
        crate::canonical::asset_digest(&entries)
    }

    /// The recomputed dependency digest over the declared dependency entries.
    pub fn recomputed_dependency_digest(&self) -> String {
        let entries: Vec<(String, String, String)> = self
            .dependencies
            .iter()
            .map(|dep| (dep.dep_id.clone(), dep.revision.to_string(), dep.sha256.clone()))
            .collect();
        dependency_digest(&entries)
    }
}

/// The private manifest schema tag (provisional mirror of the GLM-1 draft).
pub const MANIFEST_SCHEMA: &str = "michelangelo.candidate-asset-bundle/draft-0.1";

/// The bundle reference for the report (kind + id + manifest bytes digest).
#[derive(Debug, Clone)]
pub struct BundleIdentity {
    pub kind: crate::report::BundleKind,
    pub id: String,
    pub manifest_sha256: String,
}

/// The context of one evaluation: the workspace root, the manifest location,
/// the parsed manifest, and the log of the bytes actually read.
pub struct ValidationContext {
    root: PathBuf,
    manifest_path: String,
    manifest: BundleManifest,
    identity: BundleIdentity,
    inputs: BTreeMap<String, String>,
}

impl ValidationContext {
    /// Open a context: read the manifest bytes (recording their digest),
    /// parse the manifest, and validate its location containment.
    pub fn open(root: &std::path::Path, manifest_path: &str) -> Result<ValidationContext, ValidationError> {
        let joined = resolve_under(root, manifest_path)?;
        let bytes = std::fs::read(&joined)
            .map_err(|err| ValidationError::io(&joined, format!("the manifest cannot be read: {err}")))?;
        let manifest = BundleManifest::from_json_bytes(&bytes)?;
        crate::ids::validate_id(&manifest.candidate_id, "candidate_id")?;
        for artifact in &manifest.artifacts {
            crate::ids::validate_id(&artifact.artifact_id, "artifact_id")?;
            validate_rel_path(&artifact.path)?;
            crate::ids::validate_sha256(&artifact.sha256, "artifact digest")?;
        }
        for dep in &manifest.dependencies {
            crate::ids::validate_id(&dep.dep_id, "dep_id")?;
            validate_rel_path(&dep.path)?;
            crate::ids::validate_sha256(&dep.sha256, "dependency digest")?;
        }
        let manifest_sha256 = sha256_hex(&bytes);
        let identity = BundleIdentity {
            kind: crate::report::BundleKind::CandidateAssetBundle,
            id: manifest.candidate_id.clone(),
            manifest_sha256: manifest_sha256.clone(),
        };
        let mut inputs = BTreeMap::new();
        inputs.insert(manifest_path.to_string(), manifest_sha256);
        Ok(ValidationContext {
            root: root.to_path_buf(),
            manifest_path: manifest_path.to_string(),
            manifest,
            identity,
            inputs,
        })
    }

    /// The workspace root.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The manifest path (workspace-relative).
    pub fn manifest_path(&self) -> &str {
        &self.manifest_path
    }

    /// The parsed manifest.
    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }

    /// The bundle identity for the report reference.
    pub fn identity(&self) -> &BundleIdentity {
        &self.identity
    }

    /// The digests of the bytes actually read so far.
    pub fn input_digests(&self) -> &BTreeMap<String, String> {
        &self.inputs
    }

    /// Read a workspace-relative file, recording the digest of the actual
    /// bytes (IT-8D D1.2 analog: hashes are over really-read bytes).
    pub fn read_bytes(&mut self, rel_path: &str) -> Result<Vec<u8>, ValidationError> {
        let joined = resolve_under(&self.root, rel_path)?;
        let bytes = std::fs::read(&joined)
            .map_err(|err| ValidationError::io(&joined, format!("the file cannot be read: {err}")))?;
        let digest = sha256_hex(&bytes);
        self.inputs.insert(rel_path.to_string(), digest);
        Ok(bytes)
    }

    /// Find an artifact entry by its path.
    pub fn artifact_by_path(&self, path: &str) -> Option<&ArtifactEntry> {
        self.manifest.artifacts.iter().find(|a| a.path == path)
    }

    /// Find artifacts by content kind (deterministic order).
    pub fn artifacts_of_kind(&self, content_kind: &str) -> Vec<&ArtifactEntry> {
        let mut found: Vec<&ArtifactEntry> = self
            .manifest
            .artifacts
            .iter()
            .filter(|a| a.content_kind == content_kind)
            .collect();
        found.sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));
        found
    }

    /// Resolve a pinned reference: read the reference document from
    /// `<root>/references/<key>.json`, verify its digest against the pinned
    /// value, and parse the JSON. Returns (value, actual_digest).
    ///
    /// Missing pinned key -> `Err(REFERENCE_MISSING)` (the caller decides
    /// whether that makes the check unavailable or not_required).
    pub fn resolve_reference(
        &mut self,
        profile: &ValidationProfile,
        key: &str,
    ) -> Result<(serde_json::Value, String), ValidationError> {
        let pinned = profile.reference_digest(key).ok_or_else(|| {
            ValidationError::rule(
                crate::error::REFERENCE_MISSING,
                format!("the profile does not pin the reference '{}'", key),
            )
        })?;
        let path = format!("references/{key}.json");
        let bytes = self.read_bytes(&path)?;
        let actual = sha256_hex(&bytes);
        if &actual != pinned {
            return Err(ValidationError::rule(
                crate::error::DEPENDENCY_CHANGED,
                format!(
                    "the pinned reference '{}' changed: expected {}, actual {}",
                    key, pinned, actual
                ),
            ));
        }
        let value = parse_json(&bytes, &format!("the reference '{key}'"))?;
        Ok((value, actual))
    }

    /// Whether the profile kind is supported by this module. Reconstruction
    /// stays with the existing IT-8D evaluator and is refused here.
    pub fn check_kind_supported(profile: &ValidationProfile) -> Result<(), ValidationError> {
        if profile.kind == ProfileKind::Reconstruction {
            return Err(ValidationError::rule(
                crate::error::UNSUPPORTED_PROFILE_KIND,
                "the reconstruction kind belongs to the existing IT-8D evaluator; \
                 this module never re-implements it",
            ));
        }
        Ok(())
    }
}
