//! Deterministic parsers for the fixture-level formats the evaluators can
//! measure honestly: OBJ geometry, MTL materials, the provisional
//! scene_layout / sockets / rig_definition metadata documents, and 4x4
//! transform math.
//!
//! Honest limitation: real `.gltf` skins, `.blend` node groups and skin
//! weights are NOT parsed by this module. Checks that would need them return
//! `unavailable` (never a fake pass); the verifying backend integration is
//! documented in INTEGRATION_NOTES.md.

use serde::{Deserialize, Serialize};

use crate::error::ValidationError;

/// Parsed OBJ geometry (measured from actual bytes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjData {
    /// Vertex coordinates.
    pub vertices: Vec<[f64; 3]>,
    /// Number of `vt` texture-coordinate lines.
    pub uvs: usize,
    /// Triangle count (n-gon faces fan-triangulated).
    pub triangles: usize,
    /// Material names used by the geometry (`usemtl`).
    pub used_materials: Vec<String>,
    /// Material libraries referenced (`mtllib`).
    pub material_libraries: Vec<String>,
}

/// Parse an OBJ file from bytes.
pub fn parse_obj(bytes: &[u8]) -> Result<ObjData, ValidationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            "the OBJ artifact is not valid UTF-8",
        )
    })?;
    let mut data = ObjData::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("v") => {
                let mut coords = [0.0_f64; 3];
                for (index, token) in tokens.take(3).enumerate() {
                    coords[index] = token.parse::<f64>().map_err(|_| {
                        ValidationError::rule(
                            crate::error::DOCUMENT_INVALID,
                            format!("the OBJ vertex '{}' is not numeric", token),
                        )
                    })?;
                }
                data.vertices.push(coords);
            }
            Some("vt") => data.uvs += 1,
            Some("f") => {
                let count = tokens.count();
                if count >= 3 {
                    data.triangles += count - 2;
                }
            }
            Some("usemtl") => {
                if let Some(name) = tokens.next() {
                    data.used_materials.push(name.to_string());
                }
            }
            Some("mtllib") => {
                if let Some(name) = tokens.next() {
                    data.material_libraries.push(name.to_string());
                }
            }
            _ => {}
        }
    }
    Ok(data)
}

/// Parsed MTL material library (measured from actual bytes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MtlData {
    /// Defined material names (`newmtl`).
    pub materials: Vec<String>,
    /// Texture paths referenced by `map_*` lines.
    pub textures: Vec<String>,
}

/// Parse an MTL file from bytes.
pub fn parse_mtl(bytes: &[u8]) -> Result<MtlData, ValidationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            "the MTL dependency is not valid UTF-8",
        )
    })?;
    let mut data = MtlData::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("newmtl") => {
                if let Some(name) = tokens.next() {
                    data.materials.push(name.to_string());
                }
            }
            Some(prefix) if prefix.starts_with("map_") => {
                let rest = line[prefix.len()..].trim();
                if !rest.is_empty() {
                    data.textures.push(rest.to_string());
                }
            }
            _ => {}
        }
    }
    Ok(data)
}

/// A 4x4 row-major affine transform.
pub type Transform = [f64; 16];

/// The identity transform.
pub const IDENTITY: Transform = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// Multiply two row-major 4x4 matrices: `product = left * right`
/// (column-vector convention: applying `right` first).
pub fn mul(left: &Transform, right: &Transform) -> Transform {
    let mut out = [0.0_f64; 16];
    for row in 0..4 {
        for column in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += left[row * 4 + k] * right[k * 4 + column];
            }
            out[row * 4 + column] = sum;
        }
    }
    out
}

/// Apply a transform to a point (translation included).
pub fn apply(transform: &Transform, point: [f64; 3]) -> [f64; 3] {
    [
        transform[0] * point[0] + transform[1] * point[1] + transform[2] * point[2] + transform[3],
        transform[4] * point[0] + transform[5] * point[1] + transform[6] * point[2] + transform[7],
        transform[8] * point[0]
            + transform[9] * point[1]
            + transform[10] * point[2]
            + transform[11],
    ]
}

/// Distance between two points.
pub fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The tolerance for comparing composed world transforms with pinned values.
pub const TRANSFORM_EPSILON: f64 = 1e-9;

/// Compare two transforms element-wise within [`TRANSFORM_EPSILON`].
pub fn transforms_equal(a: &Transform, b: &Transform) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| (x - y).abs() <= TRANSFORM_EPSILON)
}

/// An axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl BBox {
    /// The bbox of transformed vertices (exact for affine transforms via the
    /// 8 corners).
    pub fn of_transformed(transform: &Transform, vertices: &[[f64; 3]]) -> Option<BBox> {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        let corners = if vertices.is_empty() {
            return None;
        } else {
            // Corner extrema of the local bbox transform exactly like the
            // full vertex set under an affine map.
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for v in vertices {
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(v[axis]);
                    hi[axis] = hi[axis].max(v[axis]);
                }
            }
            let mut corners = Vec::with_capacity(8);
            for x in [lo[0], hi[0]] {
                for y in [lo[1], hi[1]] {
                    for z in [lo[2], hi[2]] {
                        corners.push([x, y, z]);
                    }
                }
            }
            corners
        };
        for corner in corners {
            let world = apply(transform, corner);
            for axis in 0..3 {
                min[axis] = min[axis].min(world[axis]);
                max[axis] = max[axis].max(world[axis]);
            }
        }
        Some(BBox { min, max })
    }

    /// The extent along an axis.
    pub fn extent(&self, axis: usize) -> f64 {
        self.max[axis] - self.min[axis]
    }
}

/// The separation between two bboxes: the largest positive gap along any
/// axis (0.0 when they overlap on every axis). A provisional bbox-based
/// clearance metric; voxel-level clearance needs a verifying backend.
pub fn bbox_separation(a: &BBox, b: &BBox) -> f64 {
    let mut best = 0.0_f64;
    for axis in 0..3 {
        let gap = (a.min[axis] - b.max[axis]).max(b.min[axis] - a.max[axis]);
        best = best.max(gap);
    }
    best
}

// ---------------------------------------------------------------- layout

/// One component of the provisional scene_layout artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutComponent {
    pub component_id: String,
    /// The artifact_id of the geometry.
    pub artifact: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default = "identity_transform")]
    pub transform: Transform,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub protected: bool,
}

fn identity_transform() -> Transform {
    IDENTITY
}

/// The parsed scene_layout document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutDoc {
    pub components: Vec<LayoutComponent>,
}

/// Parse the scene_layout artifact from bytes.
pub fn parse_layout(bytes: &[u8]) -> Result<LayoutDoc, ValidationError> {
    serde_json::from_slice(bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the scene_layout artifact is not usable: {err}"),
        )
    })
}

/// Compute the world transform of each component (parent chain composed,
/// cycle-checked). A layout with a parent cycle or unknown parent is an
/// invalid document.
pub fn world_transforms(
    layout: &LayoutDoc,
) -> Result<std::collections::BTreeMap<String, Transform>, ValidationError> {
    use std::collections::BTreeMap;
    let by_id: BTreeMap<&str, &LayoutComponent> = layout
        .components
        .iter()
        .map(|component| (component.component_id.as_str(), component))
        .collect();
    let mut world: BTreeMap<String, Transform> = BTreeMap::new();
    for component in &layout.components {
        let mut chain: Vec<&LayoutComponent> = Vec::new();
        let mut current = Some(component);
        let mut guard = 0_usize;
        while let Some(link) = current {
            if chain
                .iter()
                .any(|seen| seen.component_id == link.component_id)
            {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    format!(
                        "the scene_layout has a parent cycle at component '{}'",
                        link.component_id
                    ),
                ));
            }
            chain.push(link);
            current = link
                .parent
                .as_deref()
                .and_then(|parent| by_id.get(parent).copied());
            guard += 1;
            if guard > 4096 {
                return Err(ValidationError::rule(
                    crate::error::DOCUMENT_INVALID,
                    "the scene_layout parent chain is unreasonably deep",
                ));
            }
        }
        let mut transform = IDENTITY;
        // Compose from the root down: world = parent_world * local.
        for link in chain.iter().rev() {
            transform = mul(&transform, &link.transform);
        }
        world.insert(component.component_id.clone(), transform);
    }
    Ok(world)
}

// ---------------------------------------------------------------- sockets

/// One socket of the provisional sockets artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketDef {
    pub socket_id: String,
    /// The component the socket belongs to.
    pub component: String,
    pub kind: String,
    /// Local position within the component.
    pub position: [f64; 3],
}

/// The parsed sockets document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketsDoc {
    pub sockets: Vec<SocketDef>,
}

/// Parse the sockets artifact from bytes.
pub fn parse_sockets(bytes: &[u8]) -> Result<SocketsDoc, ValidationError> {
    serde_json::from_slice(bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the sockets artifact is not usable: {err}"),
        )
    })
}

/// One pinned connector pair of the connector profile reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorPair {
    pub a: String,
    pub b: String,
    pub a_kind: String,
    pub b_kind: String,
}

/// The pinned connector profile reference document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorProfile {
    pub pairs: Vec<ConnectorPair>,
}

/// Parse the pinned connector profile from bytes.
pub fn parse_connector_profile(bytes: &[u8]) -> Result<ConnectorProfile, ValidationError> {
    serde_json::from_slice(bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the connector profile reference is not usable: {err}"),
        )
    })
}

// -------------------------------------------------------------------- rig

/// One bone of a rig definition or skeleton profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneDef {
    pub bone_id: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default = "identity_transform")]
    pub rest: Transform,
}

/// A component binding of the provisional rig_definition artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingDef {
    pub component_id: String,
    pub bone_id: String,
}

/// The provisional rig_definition artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigDoc {
    #[serde(default)]
    pub skeleton_ref: Option<String>,
    pub bones: Vec<BoneDef>,
    #[serde(default)]
    pub bindings: Vec<BindingDef>,
}

/// Parse the rig_definition artifact from bytes.
pub fn parse_rig(bytes: &[u8]) -> Result<RigDoc, ValidationError> {
    serde_json::from_slice(bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the rig_definition artifact is not usable: {err}"),
        )
    })
}

/// The pinned skeleton profile reference document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkeletonProfile {
    pub skeleton_id: String,
    pub bones: Vec<BoneDef>,
}

/// Parse the pinned skeleton profile from bytes.
pub fn parse_skeleton_profile(bytes: &[u8]) -> Result<SkeletonProfile, ValidationError> {
    serde_json::from_slice(bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the skeleton profile reference is not usable: {err}"),
        )
    })
}
