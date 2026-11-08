//! Deterministic fixture builders for tests and PoC harnesses (never a
//! production path). The builders write a disposable validation workspace:
//! artifact files, dependencies, the provisional scene_layout / sockets /
//! rig metadata artifacts, the manifest, and the pinned profile references.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::canonical::sha256_hex;
use crate::check::{CheckId, DocRef};
use crate::context::{
    ArtifactEntry, BundleManifest, ConversionDecl, DependencyEntry, LossDecl, OutputDecls,
};
use crate::error::ValidationError;
use crate::profile::{
    AppliesTo, ProfileKind, ProfileReferences, TargetKind, Threshold, ValidationProfile,
    PROFILE_SCHEMA,
};

/// The fixed fixture timestamp (reports are deterministic in tests).
pub const TIMESTAMP: &str = "2026-11-08T12:00:00Z";

/// A disposable workspace under construction.
pub struct WorkspaceBuilder {
    root: PathBuf,
    candidate_id: String,
    artifacts: Vec<ArtifactEntry>,
    dependencies: Vec<DependencyEntry>,
    conversions: Vec<ConversionDecl>,
    losses: Vec<LossDecl>,
    outputs: OutputDecls,
}

impl WorkspaceBuilder {
    /// Start a workspace under `root` (created on demand).
    pub fn new(root: &Path, candidate_id: &str) -> WorkspaceBuilder {
        WorkspaceBuilder {
            root: root.to_path_buf(),
            candidate_id: candidate_id.to_string(),
            artifacts: Vec::new(),
            dependencies: Vec::new(),
            conversions: Vec::new(),
            losses: Vec::new(),
            outputs: OutputDecls::default(),
        }
    }

    /// The workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write raw bytes into the workspace; returns the digest.
    pub fn write_bytes(&self, rel_path: &str, bytes: &[u8]) -> String {
        let joined = self.root.join(rel_path);
        let parent = joined.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).expect("fixture directory");
        std::fs::write(&joined, bytes).expect("fixture file");
        sha256_hex(bytes)
    }

    /// Write a text file; returns the digest.
    pub fn write_text(&self, rel_path: &str, text: &str) -> String {
        self.write_bytes(rel_path, text.as_bytes())
    }

    /// Add an artifact from already-written bytes.
    pub fn add_artifact(
        &mut self,
        artifact_id: &str,
        rel_path: &str,
        content_kind: &str,
        bytes: &[u8],
        roles: &[&str],
        protected: bool,
    ) -> String {
        let digest = self.write_bytes(rel_path, bytes);
        self.artifacts.push(ArtifactEntry {
            artifact_id: artifact_id.to_string(),
            path: rel_path.to_string(),
            sha256: digest.clone(),
            size_bytes: bytes.len() as u64,
            content_kind: content_kind.to_string(),
            roles: roles.iter().map(|role| role.to_string()).collect(),
            protected,
        });
        digest
    }

    /// Add an OBJ geometry artifact.
    pub fn add_obj(
        &mut self,
        artifact_id: &str,
        rel_path: &str,
        content: &str,
        roles: &[&str],
        protected: bool,
    ) -> String {
        self.add_artifact(artifact_id, rel_path, "geometry", content.as_bytes(), roles, protected)
    }

    /// Add a dependency from bytes.
    pub fn add_dependency(
        &mut self,
        dep_id: &str,
        rel_path: &str,
        bytes: &[u8],
        revision: u64,
    ) -> String {
        let digest = self.write_bytes(rel_path, bytes);
        self.dependencies.push(DependencyEntry {
            dep_id: dep_id.to_string(),
            path: rel_path.to_string(),
            sha256: digest.clone(),
            revision,
        });
        digest
    }

    /// Declare a format conversion.
    pub fn declare_conversion(&mut self, from: &str, to: &str) {
        self.conversions.push(ConversionDecl {
            from: from.to_string(),
            to: to.to_string(),
        });
    }

    /// Declare a conversion loss.
    pub fn declare_loss(&mut self, kind: &str, detail: &str) {
        self.losses.push(LossDecl {
            kind: kind.to_string(),
            detail: detail.to_string(),
        });
    }

    /// Declare outputs.
    pub fn declare_outputs(&mut self, export_glb: bool, editable_blend: bool) {
        self.outputs = OutputDecls {
            export_glb,
            editable_blend,
        };
    }

    /// Write the scene_layout metadata artifact (a hashed artifact).
    pub fn set_layout(
        &mut self,
        artifact_id: &str,
        components: Vec<crate::checks::parsing::LayoutComponent>,
    ) -> String {
        let document = crate::checks::parsing::LayoutDoc { components };
        let bytes = serde_json::to_vec_pretty(&document).expect("layout json");
        self.add_artifact(artifact_id, "metadata/scene_layout.json", "scene_layout", &bytes, &[], false)
    }

    /// Write the sockets metadata artifact.
    pub fn set_sockets(
        &mut self,
        artifact_id: &str,
        sockets: Vec<crate::checks::parsing::SocketDef>,
    ) -> String {
        let document = crate::checks::parsing::SocketsDoc { sockets };
        let bytes = serde_json::to_vec_pretty(&document).expect("sockets json");
        self.add_artifact(artifact_id, "metadata/sockets.json", "metadata", &bytes, &[], false)
    }

    /// Write the rig_definition artifact.
    pub fn set_rig(
        &mut self,
        artifact_id: &str,
        rig: &crate::checks::parsing::RigDoc,
        rel_path: &str,
    ) -> String {
        let bytes = serde_json::to_vec_pretty(rig).expect("rig json");
        self.add_artifact(artifact_id, rel_path, "rig_definition", &bytes, &[], false)
    }

    /// Finalize: recompute the aggregates, write the manifest, return the
    /// workspace handle.
    pub fn finalize(self) -> FinalizedWorkspace {
        let Self {
            root,
            candidate_id,
            artifacts,
            dependencies,
            conversions,
            losses,
            outputs,
        } = self;
        let asset_digest = {
            let entries: Vec<(String, String, String, String)> = artifacts
                .iter()
                .map(|a| {
                    (
                        a.artifact_id.clone(),
                        a.path.clone(),
                        a.content_kind.clone(),
                        a.sha256.clone(),
                    )
                })
                .collect();
            crate::canonical::asset_digest(&entries)
        };
        let dependency_digest = {
            let entries: Vec<(String, String, String)> = dependencies
                .iter()
                .map(|d| (d.dep_id.clone(), d.revision.to_string(), d.sha256.clone()))
                .collect();
            crate::canonical::dependency_digest(&entries)
        };
        let manifest = BundleManifest {
            schema: crate::context::MANIFEST_SCHEMA.to_string(),
            candidate_id,
            artifacts,
            dependencies,
            asset_digest: Some(asset_digest.clone()),
            dependency_digest: Some(dependency_digest.clone()),
            conversions,
            declared_losses: losses,
            outputs,
        };
        let bytes = serde_json::to_vec_pretty(&manifest).expect("manifest json");
        let manifest_sha256 = sha256_hex(&bytes);
        let joined = root.join("manifest.json");
        std::fs::write(&joined, &bytes).expect("manifest file");
        FinalizedWorkspace {
            root,
            manifest_path: "manifest.json".to_string(),
            manifest,
            manifest_sha256,
            asset_digest,
            dependency_digest,
        }
    }
}

/// A finalized fixture workspace.
pub struct FinalizedWorkspace {
    pub root: PathBuf,
    pub manifest_path: String,
    pub manifest: BundleManifest,
    pub manifest_sha256: String,
    pub asset_digest: String,
    pub dependency_digest: String,
}

impl FinalizedWorkspace {
    /// The workspace root path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Write a pinned profile reference document under `<root>/references/`.
pub fn write_reference<T: Serialize>(root: &Path, key: &str, document: &T) -> String {
    let path = root.join("references").join(format!("{key}.json"));
    std::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))
        .expect("references directory");
    let bytes = serde_json::to_vec_pretty(document).expect("reference json");
    std::fs::write(&path, &bytes).expect("reference file");
    sha256_hex(&bytes)
}

/// Assemble a validation profile. The `references` argument may be empty;
/// pinned digests are attached afterwards via `with_reference` / `with_clips`.
#[allow(clippy::too_many_arguments)]
pub fn make_profile(
    profile_id: &str,
    kind: ProfileKind,
    revision: u32,
    required: Vec<CheckId>,
    optional: Vec<CheckId>,
    thresholds: Vec<Threshold>,
    conditions: Vec<String>,
    references: ProfileReferences,
) -> ValidationProfile {
    ValidationProfile {
        schema: PROFILE_SCHEMA.to_string(),
        profile_id: profile_id.to_string(),
        revision,
        created_at: TIMESTAMP.to_string(),
        kind,
        applies_to: AppliesTo {
            targets: vec![TargetKind::CandidateAssetBundle],
            export_targets: Vec::new(),
            conditions,
        },
        required_checks: required,
        optional_checks: optional,
        thresholds,
        references,
        evaluator_versions: {
            let mut versions = BTreeMap::new();
            versions.insert("test-evaluator".to_string(), "0".to_string());
            versions
        },
        coverage_notes: None,
    }
}

/// Attach a pinned reference digest to the profile.
pub fn with_reference(mut profile: ValidationProfile, key: &str, digest: &str) -> ValidationProfile {
    profile
        .references
        .reference_digests
        .insert(key.to_string(), digest.to_string());
    profile
}

/// Attach pinned animation clips to the profile.
pub fn with_clips(mut profile: ValidationProfile, clips: Vec<DocRef>) -> ValidationProfile {
    profile.references.animation_clips = clips;
    profile
}

/// A minimal OBJ cube with UVs (deterministic text).
pub fn cube_obj() -> String {
    [
        "v 0 0 0", "v 1 0 0", "v 1 1 0", "v 0 1 0", "v 0 0 1", "v 1 0 1", "v 1 1 1", "v 0 1 1",
        "vt 0 0", "vt 1 0", "vt 1 1", "vt 0 1",
        "usemtl wall",
        "f 1/1 2/2 3/3", "f 1/1 3/3 4/4",
    ]
    .join("\n")
}

/// An OBJ block spanning `size` units along each axis.
pub fn block_obj(size: f64, uvs: usize) -> String {
    let s = size;
    let mut lines = vec![
        format!("v 0 0 0"),
        format!("v {s} 0 0"),
        format!("v {s} {s} 0"),
        format!("v 0 {s} 0"),
        format!("v 0 0 {s}"),
        format!("v {s} 0 {s}"),
        format!("v {s} {s} {s}"),
        format!("v 0 {s} {s}"),
    ];
    for index in 0..uvs {
        lines.push(format!("vt 0.{index:02} 0.5"));
    }
    lines.push("usemtl wall".to_string());
    lines.push("f 1/1 2/2 3/3".to_string());
    lines.push("f 1/1 3/3 4/4".to_string());
    lines.join("\n")
}

/// A minimal MTL library with one texture map.
pub fn wall_mtl(texture_path: &str) -> String {
    format!("newmtl wall\nKd 0.8 0.8 0.8\nmap_Kd {texture_path}\n")
}

/// Evaluate a fixture workspace with the reserved report id.
pub fn evaluate_fixture(
    workspace: &FinalizedWorkspace,
    profile: &ValidationProfile,
    report_id: &str,
) -> Result<(crate::report::ValidationReport, String), ValidationError> {
    crate::evaluate::evaluate(crate::evaluate::EvaluationRequest {
        report_id: report_id.to_string(),
        created_at: TIMESTAMP.to_string(),
        workspace_root: &workspace.root,
        manifest_path: workspace.manifest_path.clone(),
        profile: profile.clone(),
        roundtrip_backend: None,
        playback_backend: None,
    })
}

// ------------------------------------------------- shared fixture: family

/// One bone of a fixture skeleton (16-number identity rest transform).
pub fn bone(bone_id: &str, parent: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "bone_id": bone_id,
        "parent": parent,
        "rest": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    })
}

/// One fixture component binding.
pub fn binding(component_id: &str, bone_id: &str) -> serde_json::Value {
    serde_json::json!({"component_id": component_id, "bone_id": bone_id})
}

/// Parse a fixture rig document.
pub fn rig_doc(
    skeleton_ref: &str,
    bones: Vec<serde_json::Value>,
    bindings: Vec<serde_json::Value>,
) -> crate::checks::parsing::RigDoc {
    serde_json::from_value(serde_json::json!({
        "skeleton_ref": skeleton_ref,
        "bones": bones,
        "bindings": bindings,
    }))
    .expect("rig doc")
}

/// A threshold entry for fixture profiles.
pub fn threshold(
    check: CheckId,
    metric: &str,
    comparison: crate::profile::Comparison,
    value: f64,
) -> Threshold {
    Threshold {
        check_id: check,
        metric: metric.to_string(),
        comparison,
        value,
    }
}

/// The shared family-static workspace: body + roof + protected anchor +
/// entrance doorway + sockets, with every pinned reference attached to the
/// returned profile.
pub fn build_family_workspace(
    root: &Path,
    candidate_id: &str,
) -> (FinalizedWorkspace, ValidationProfile) {
    use crate::checks::parsing::{LayoutComponent, SocketDef};
    use crate::profile::{Comparison, ProfileKind, ProfileReferences};

    let mut ws = WorkspaceBuilder::new(root, candidate_id);
    let mtl = wall_mtl("textures/wall.png");
    ws.add_dependency("wall-mtl", "deps/wall.mtl", mtl.as_bytes(), 1);
    ws.add_dependency("wall-tex", "textures/wall.png", &[9u8, 8, 7], 1);

    let _body = ws.add_obj("body-mesh", "artifacts/body.obj", &block_obj(1.0, 2), &["wall"], false);
    ws.add_obj("roof-mesh", "artifacts/roof.obj", &block_obj(1.0, 2), &["roof"], false);
    let anchor = ws.add_obj(
        "anchor-mesh",
        "artifacts/anchor.obj",
        &block_obj(0.1, 1),
        &["entry-anchor"],
        true,
    );
    ws.add_obj("door-mesh", "artifacts/door.obj", &block_obj(1.0, 2), &["entrance"], false);

    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let roof_transform = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 1.0, // translate z = 1
        0.0, 0.0, 0.0, 1.0,
    ];
    let door_transform = [
        1.0, 0.0, 0.0, 5.0, // translate x = 5
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    ws.set_layout(
        "layout",
        vec![
            LayoutComponent {
                component_id: "body".to_string(),
                artifact: "body-mesh".to_string(),
                parent: None,
                transform: identity,
                role: "wall".to_string(),
                protected: false,
            },
            LayoutComponent {
                component_id: "roof".to_string(),
                artifact: "roof-mesh".to_string(),
                parent: Some("body".to_string()),
                transform: roof_transform,
                role: "roof".to_string(),
                protected: false,
            },
            LayoutComponent {
                component_id: "anchor".to_string(),
                artifact: "anchor-mesh".to_string(),
                parent: Some("body".to_string()),
                transform: identity,
                role: "entry-anchor".to_string(),
                protected: true,
            },
            LayoutComponent {
                component_id: "doorway".to_string(),
                artifact: "door-mesh".to_string(),
                parent: None,
                transform: door_transform,
                role: "entrance".to_string(),
                protected: false,
            },
        ],
    );
    ws.set_sockets(
        "sockets",
        vec![
            SocketDef {
                socket_id: "wall-socket-1".to_string(),
                component: "body".to_string(),
                kind: "wall-roof".to_string(),
                position: [0.5, 0.0, 1.0],
            },
            SocketDef {
                socket_id: "roof-socket-1".to_string(),
                component: "roof".to_string(),
                kind: "roof-wall".to_string(),
                position: [0.5, 0.0, 0.0],
            },
        ],
    );
    ws.declare_outputs(false, true);
    let ws = ws.finalize();

    let expected = serde_json::json!({ "asset_digest": ws.asset_digest });
    let expected_dep = serde_json::json!({ "dependency_digest": ws.dependency_digest });
    let protected = serde_json::json!({ "components": [
        {"component_id": "anchor", "artifact_sha256": anchor, "world_transform": identity},
    ]});
    let connectors = serde_json::json!({ "pairs": [
        {"a": "wall-socket-1", "b": "roof-socket-1", "a_kind": "wall-roof", "b_kind": "roof-wall"},
    ]});
    let uv_counts = serde_json::json!({ "counts": [
        {"path": "artifacts/body.obj", "vt": 2},
        {"path": "artifacts/roof.obj", "vt": 2},
        {"path": "artifacts/anchor.obj", "vt": 1},
        {"path": "artifacts/door.obj", "vt": 2},
    ]});
    let digest_pin = write_reference(ws.root(), "expected_asset_digest", &expected);
    let dep_pin = write_reference(ws.root(), "expected_dependency_digest", &expected_dep);
    let protected_pin = write_reference(ws.root(), "protected_scene_layout", &protected);
    let connector_pin = write_reference(ws.root(), "connector_profile", &connectors);
    let uv_pin = write_reference(ws.root(), "expected_uv_counts", &uv_counts);

    let profile = make_profile(
        "family-static-v0",
        ProfileKind::FamilyStatic,
        1,
        vec![
            CheckId::PayloadIntegrity,
            CheckId::DependencyClosure,
            CheckId::PathContainment,
            CheckId::AssetDigestFull,
            CheckId::BudgetGeometry,
            CheckId::ProtectedRoles,
            CheckId::ConnectorCompatibility,
            CheckId::MinimumClearance,
            CheckId::MaximumDimension,
            CheckId::MaterialPresence,
            CheckId::TextureIntegrity,
            CheckId::UvPreservation,
        ],
        vec![CheckId::FormatLossDeclared],
        vec![
            threshold(CheckId::BudgetGeometry, "max_evaluated_triangles", Comparison::AtMost, 100_000.0),
            threshold(CheckId::MaximumDimension, "max_extent_x_m", Comparison::AtMost, 10.0),
            threshold(CheckId::MaximumDimension, "max_extent_z_m", Comparison::AtMost, 20.0),
            threshold(CheckId::MinimumClearance, "min_clearance_m", Comparison::AtLeast, 1.2),
            threshold(CheckId::ConnectorCompatibility, "connector_tolerance_m", Comparison::AtMost, 0.1),
        ],
        vec![
            "constraints:connector".to_string(),
            "constraints:clearance".to_string(),
            "constraints:dimension".to_string(),
            "textures_declared".to_string(),
            "uv_preserved".to_string(),
        ],
        ProfileReferences::default(),
    );
    let profile = with_reference(profile, "expected_asset_digest", &digest_pin);
    let profile = with_reference(profile, "expected_dependency_digest", &dep_pin);
    let profile = with_reference(profile, "protected_scene_layout", &protected_pin);
    let profile = with_reference(profile, "connector_profile", &connector_pin);
    let profile = with_reference(profile, "expected_uv_counts", &uv_pin);
    (ws, profile)
}
