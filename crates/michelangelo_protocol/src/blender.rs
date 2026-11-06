//! Blender job specification, result, capabilities, and operation DTOs.
//!
//! These DTOs define the strict contract between the Rust core and the
//! Blender Python worker. They intentionally exclude arbitrary Python
//! execution or unsafe operations.
//!
//! [`BlenderOperation`] is an internally tagged enum: every operation is
//! serialized as `{"op": "<name>", ...params}` on the wire, which keeps the
//! Phase 2 JSON contract intact, while the Rust side gets a typed whitelist
//! whose unknown variants are rejected at deserialization time.
//!
//! Semantic validation (ranges, names, workspace-relative paths, camera pose
//! rules, duplicate created names) lives in [`BlenderJobSpec::validate`] and
//! is mirrored defensively by `blender/worker.py`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Component, Path};

/// Maximum number of operations accepted in one Blender job spec.
pub const MAX_BLENDER_OPERATIONS: usize = 256;
/// Maximum length of `job_type`, in characters.
pub const MAX_JOB_TYPE_CHARS: usize = 128;
/// Maximum length of an object/camera/component name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Maximum length of a workspace-relative path, in characters.
pub const MAX_SPEC_PATH_CHARS: usize = 1024;
/// Maximum number of legacy camera views in one `set_camera_set` operation.
pub const MAX_VIEWS: usize = 64;
/// Absolute bound for a single `location` / `rotation_deg` / `look_at` component.
pub const MAX_COORDINATE_ABS: f64 = 1_000_000.0;
/// Maximum render resolution (width or height) in pixels.
pub const MAX_RESOLUTION: u32 = 16_384;
/// Absolute bound for a single camera `shift_x` / `shift_y` component.
pub const MAX_SHIFT_ABS: f64 = 10.0;
/// Minimum `frame_camera` margin.
pub const MIN_FRAME_MARGIN: f64 = 0.1;
/// Maximum `frame_camera` margin.
pub const MAX_FRAME_MARGIN: f64 = 10.0;
/// Default `ortho_scale` used by `create_ortho_camera`.
pub const DEFAULT_ORTHO_SCALE: f64 = 4.0;
/// Default `clip_start` used by `create_ortho_camera`.
pub const DEFAULT_CLIP_START: f64 = 0.1;
/// Default `clip_end` used by `create_ortho_camera`.
pub const DEFAULT_CLIP_END: f64 = 1000.0;
/// Default `frame_camera` margin.
pub const DEFAULT_FRAME_MARGIN: f64 = 1.1;

/// Canonical legacy named camera views created by `set_camera_set`.
pub const LEGACY_VIEWS: &[&str] = &[
    "front",
    "back",
    "left",
    "right",
    "top",
    "bottom",
    "perspective",
];

/// Back-compat wire aliases accepted by `set_camera_set`.
///
/// Phase 2 documented and shipped `"side"` as a legacy view name even though the
/// old handler's camera table only had `"left"` and silently skipped `"side"`.
/// `"side"` is therefore accepted as an alias and maps to the same camera
/// definition as `"left"` (including the `Cam_left` object name).
pub const LEGACY_VIEW_ALIASES: &[(&str, &str)] = &[("side", "left")];

/// All wire view names accepted by `set_camera_set` (canonical names + aliases).
pub const LEGACY_VIEW_NAMES: &[&str] = &[
    "front",
    "back",
    "left",
    "right",
    "top",
    "bottom",
    "perspective",
    "side",
];

/// Legacy `set_camera_set` default view set when `views` is omitted.
///
/// This is the Phase 2 documented default literal, including the `"side"`
/// alias. Through the alias the effective deterministic cameras are
/// `Cam_front`, `Cam_left`, `Cam_top`, `Cam_perspective`.
pub const LEGACY_DEFAULT_VIEWS: &[&str] = &["front", "side", "top", "perspective"];

/// File extensions accepted by `import_mesh` (case-insensitive).
pub const IMPORT_MESH_EXTENSIONS: &[&str] = &["glb", "gltf", "obj", "stl", "ply"];

/// Specification for a Blender job to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlenderJobSpec {
    /// Human-readable / machine-friendly job type (e.g. `"blender_smoke_scene"`).
    pub job_type: String,
    /// Whether to block until the job completes.
    #[serde(default)]
    pub wait: bool,
    /// Maximum wall-clock time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Ordered list of Blender operations to execute.
    pub spec: BlenderSpecBody,
}

impl BlenderJobSpec {
    /// Validate the complete job spec before any job record or file is created.
    ///
    /// Error messages are actionable and name the offending operation index and
    /// wire name, e.g. `operation[2] ('save_blend'): 'path' ...`. The Python
    /// worker mirrors these rules defensively.
    pub fn validate(&self) -> Result<(), String> {
        validate_job_type(&self.job_type)?;

        if self.spec.operations.len() > MAX_BLENDER_OPERATIONS {
            return Err(format!(
                "too many operations: {} (max {MAX_BLENDER_OPERATIONS})",
                self.spec.operations.len()
            ));
        }

        let mut created_names: HashSet<String> = HashSet::new();
        for (index, operation) in self.spec.operations.iter().enumerate() {
            let label = format!("operation[{index}] ('{}')", operation.op_name());
            operation.validate_at(&label)?;
            for name in operation.created_names() {
                if !created_names.insert(name.clone()) {
                    return Err(format!(
                        "{label}: duplicate created object/camera name '{name}' \
                         (already created by an earlier operation)"
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Inner body of a Blender job spec containing operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlenderSpecBody {
    pub operations: Vec<BlenderOperation>,
}

/// A single safe Blender operation within a job spec.
///
/// Serialized as `{"op": "<wire name>", ...params}` (internally tagged). The
/// `op` tag is the same string used by Phase 2, so existing JSON keeps parsing.
/// Optional fields are omitted from serialization when absent; unknown `op`
/// values fail deserialization and are mapped by the router to `-32602`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BlenderOperation {
    /// Create a box/cube with exact dimensions.
    CreateBox {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
    },
    /// Create a cylinder with exact radius/depth.
    CreateCylinder {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vertices: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
    },
    /// Create a UV sphere with exact radius.
    CreateSphere {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        segments: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ring_count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
    },
    /// Import one workspace mesh artifact (.glb/.gltf/.obj/.stl/.ply).
    ImportMesh {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
    },
    /// Apply an exact transform to an existing object.
    SetTransform {
        object: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        apply_scale: Option<bool>,
    },
    /// Assign a material color to an existing object.
    SetMaterial {
        object: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Vec<f64>>,
    },
    /// Create the legacy named perspective camera set.
    SetCameraSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        views: Option<Vec<String>>,
    },
    /// Create an orthographic camera at an explicit pose.
    CreateOrthoCamera {
        name: String,
        location: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        look_at: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ortho_scale: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shift_x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shift_y: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip_start: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip_end: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
    },
    /// Update an existing camera location and/or orientation.
    SetCameraPose {
        camera: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_deg: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        look_at: Option<Vec<f64>>,
    },
    /// Update orthographic scale, lens shift and clipping of a camera.
    SetCameraSettings {
        camera: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ortho_scale: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shift_x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shift_y: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip_start: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip_end: Option<f64>,
    },
    /// Frame an orthographic camera on objects/component/all meshes.
    FrameCamera {
        camera: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        objects: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        margin: Option<f64>,
    },
    /// Update render resolution/percentage (Workbench engine is fixed).
    SetRenderSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<Vec<u32>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution_percentage: Option<u32>,
    },
    /// Render only the selected camera to a PNG (optionally a silhouette).
    RenderCamera {
        camera: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        silhouette: Option<bool>,
    },
    /// Legacy: render every camera in the scene to PNG files.
    RenderViews {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// Create a named component collection (`Component_<name>`), idempotent.
    CreateComponent { name: String },
    /// Save the current scene to a `.blend` file.
    SaveBlend { path: String },
    /// Export the current scene to a `.glb` file.
    ExportGlb { path: String },
    /// Return a deterministic scene summary (also stored in the result).
    GetSceneSummary {},
}

impl BlenderOperation {
    /// Wire name of the operation (`op` tag value).
    pub fn op_name(&self) -> &'static str {
        match self {
            Self::CreateBox { .. } => "create_box",
            Self::CreateCylinder { .. } => "create_cylinder",
            Self::CreateSphere { .. } => "create_sphere",
            Self::ImportMesh { .. } => "import_mesh",
            Self::SetTransform { .. } => "set_transform",
            Self::SetMaterial { .. } => "set_material",
            Self::SetCameraSet { .. } => "set_camera_set",
            Self::CreateOrthoCamera { .. } => "create_ortho_camera",
            Self::SetCameraPose { .. } => "set_camera_pose",
            Self::SetCameraSettings { .. } => "set_camera_settings",
            Self::FrameCamera { .. } => "frame_camera",
            Self::SetRenderSettings { .. } => "set_render_settings",
            Self::RenderCamera { .. } => "render_camera",
            Self::RenderViews { .. } => "render_views",
            Self::CreateComponent { .. } => "create_component",
            Self::SaveBlend { .. } => "save_blend",
            Self::ExportGlb { .. } => "export_glb",
            Self::GetSceneSummary {} => "get_scene_summary",
        }
    }

    /// Validate this operation in isolation (without duplicate-name context).
    pub fn validate(&self) -> Result<(), String> {
        self.validate_at(&format!("operation ('{}')", self.op_name()))
    }

    /// Names of objects/cameras this operation creates, in deterministic order.
    ///
    /// Used for cross-operation duplicate detection. Default names are included
    /// so that two unnamed `create_box` operations are rejected even though
    /// Blender would silently create `Box` and `Box.001`.
    fn created_names(&self) -> Vec<String> {
        match self {
            Self::CreateBox { name, .. } => vec![name.clone().unwrap_or_else(|| "Box".into())],
            Self::CreateCylinder { name, .. } => {
                vec![name.clone().unwrap_or_else(|| "Cylinder".into())]
            }
            Self::CreateSphere { name, .. } => {
                vec![name.clone().unwrap_or_else(|| "Sphere".into())]
            }
            Self::CreateOrthoCamera { name, .. } => vec![name.clone()],
            Self::SetCameraSet { views } => canonical_legacy_views(views.as_deref())
                .into_iter()
                .map(|view| format!("Cam_{view}"))
                .collect(),
            Self::ImportMesh { name, .. } => name.clone().into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn validate_at(&self, label: &str) -> Result<(), String> {
        match self {
            Self::CreateBox {
                name,
                size,
                location,
                rotation_deg,
                component,
            } => {
                validate_optional_name(name.as_deref(), "name", label)?;
                validate_optional_name(component.as_deref(), "component", label)?;
                if let Some(size) = size {
                    validate_positive_vector3(size, "size", label)?;
                }
                if let Some(location) = location {
                    validate_coordinate_vector3(location, "location", label)?;
                }
                if let Some(rotation) = rotation_deg {
                    validate_coordinate_vector3(rotation, "rotation_deg", label)?;
                }
                Ok(())
            }
            Self::CreateCylinder {
                name,
                radius,
                depth,
                location,
                rotation_deg,
                vertices,
                component,
            } => {
                validate_optional_name(name.as_deref(), "name", label)?;
                validate_optional_name(component.as_deref(), "component", label)?;
                if let Some(radius) = radius {
                    validate_positive_finite(*radius, "radius", label)?;
                }
                if let Some(depth) = depth {
                    validate_positive_finite(*depth, "depth", label)?;
                }
                if let Some(location) = location {
                    validate_coordinate_vector3(location, "location", label)?;
                }
                if let Some(rotation) = rotation_deg {
                    validate_coordinate_vector3(rotation, "rotation_deg", label)?;
                }
                if let Some(vertices) = vertices {
                    validate_u32_range(*vertices, 3, 1024, "vertices", label)?;
                }
                Ok(())
            }
            Self::CreateSphere {
                name,
                radius,
                location,
                rotation_deg,
                segments,
                ring_count,
                component,
            } => {
                validate_optional_name(name.as_deref(), "name", label)?;
                validate_optional_name(component.as_deref(), "component", label)?;
                if let Some(radius) = radius {
                    validate_positive_finite(*radius, "radius", label)?;
                }
                if let Some(location) = location {
                    validate_coordinate_vector3(location, "location", label)?;
                }
                if let Some(rotation) = rotation_deg {
                    validate_coordinate_vector3(rotation, "rotation_deg", label)?;
                }
                if let Some(segments) = segments {
                    validate_u32_range(*segments, 3, 1024, "segments", label)?;
                }
                if let Some(ring_count) = ring_count {
                    validate_u32_range(*ring_count, 2, 1024, "ring_count", label)?;
                }
                Ok(())
            }
            Self::ImportMesh {
                path,
                name,
                component,
            } => {
                validate_relative_path(path, "path", label)?;
                validate_extension(path, IMPORT_MESH_EXTENSIONS, "path", label)?;
                validate_optional_name(name.as_deref(), "name", label)?;
                validate_optional_name(component.as_deref(), "component", label)?;
                Ok(())
            }
            Self::SetTransform {
                object,
                location,
                rotation_deg,
                scale,
                ..
            } => {
                validate_required_name(object, "object", label)?;
                if location.is_none() && rotation_deg.is_none() && scale.is_none() {
                    return Err(format!(
                        "{label}: at least one of 'location', 'rotation_deg' or 'scale' is required"
                    ));
                }
                if let Some(location) = location {
                    validate_coordinate_vector3(location, "location", label)?;
                }
                if let Some(rotation) = rotation_deg {
                    validate_coordinate_vector3(rotation, "rotation_deg", label)?;
                }
                if let Some(scale) = scale {
                    validate_nonzero_vector3(scale, "scale", label)?;
                }
                Ok(())
            }
            Self::SetMaterial { object, color } => {
                validate_required_name(object, "object", label)?;
                if let Some(color) = color {
                    validate_color(color, "color", label)?;
                }
                Ok(())
            }
            Self::SetCameraSet { views } => {
                if let Some(views) = views {
                    if views.is_empty() || views.len() > MAX_VIEWS {
                        return Err(format!(
                            "{label}: 'views' must contain between 1 and {MAX_VIEWS} entries"
                        ));
                    }
                    let mut seen: HashSet<&str> = HashSet::new();
                    for view in views {
                        if canonical_legacy_view(view).is_none() {
                            return Err(format!(
                                "{label}: unsupported view '{view}' \
                                 (expected one of: {})",
                                LEGACY_VIEW_NAMES.join(", ")
                            ));
                        }
                        // Exact repeated wire names are invalid; alias collisions
                        // (`side` + `left`) are canonicalized to one camera below.
                        if !seen.insert(view.as_str()) {
                            return Err(format!("{label}: duplicate view '{view}'"));
                        }
                    }
                }
                Ok(())
            }
            Self::CreateOrthoCamera {
                name,
                location,
                rotation_deg,
                look_at,
                ortho_scale,
                shift_x,
                shift_y,
                clip_start,
                clip_end,
                component,
            } => {
                validate_required_name(name, "name", label)?;
                validate_optional_name(component.as_deref(), "component", label)?;
                validate_coordinate_vector3(location, "location", label)?;
                validate_pose_fields(rotation_deg, look_at, label)?;
                if let Some(scale) = ortho_scale {
                    validate_positive_finite(*scale, "ortho_scale", label)?;
                }
                validate_optional_shift(*shift_x, "shift_x", label)?;
                validate_optional_shift(*shift_y, "shift_y", label)?;
                let start = validate_optional_positive(*clip_start, "clip_start", label)?;
                let end = validate_optional_positive(*clip_end, "clip_end", label)?;
                validate_clip_order(start, end, label)?;
                Ok(())
            }
            Self::SetCameraPose {
                camera,
                location,
                rotation_deg,
                look_at,
            } => {
                validate_required_name(camera, "camera", label)?;
                if location.is_none() && rotation_deg.is_none() && look_at.is_none() {
                    return Err(format!(
                        "{label}: at least one of 'location', 'rotation_deg' or 'look_at' is required"
                    ));
                }
                validate_pose_fields(rotation_deg, look_at, label)?;
                if let Some(location) = location {
                    validate_coordinate_vector3(location, "location", label)?;
                }
                Ok(())
            }
            Self::SetCameraSettings {
                camera,
                ortho_scale,
                shift_x,
                shift_y,
                clip_start,
                clip_end,
            } => {
                validate_required_name(camera, "camera", label)?;
                if ortho_scale.is_none()
                    && shift_x.is_none()
                    && shift_y.is_none()
                    && clip_start.is_none()
                    && clip_end.is_none()
                {
                    return Err(format!(
                        "{label}: at least one camera setting is required \
                         (ortho_scale, shift_x, shift_y, clip_start, clip_end)"
                    ));
                }
                if let Some(scale) = ortho_scale {
                    validate_positive_finite(*scale, "ortho_scale", label)?;
                }
                validate_optional_shift(*shift_x, "shift_x", label)?;
                validate_optional_shift(*shift_y, "shift_y", label)?;
                let start = validate_optional_positive(*clip_start, "clip_start", label)?;
                let end = validate_optional_positive(*clip_end, "clip_end", label)?;
                validate_clip_order(start, end, label)?;
                Ok(())
            }
            Self::FrameCamera {
                camera,
                objects,
                component,
                margin,
            } => {
                validate_required_name(camera, "camera", label)?;
                validate_optional_name(component.as_deref(), "component", label)?;
                if objects.is_some() && component.is_some() {
                    return Err(format!(
                        "{label}: at most one of 'objects' and 'component' may be set"
                    ));
                }
                if let Some(objects) = objects {
                    if objects.is_empty() {
                        return Err(format!(
                            "{label}: 'objects' must not be empty (omit it to frame all meshes)"
                        ));
                    }
                    for object in objects {
                        validate_required_name(object, "objects", label)?;
                    }
                }
                if let Some(margin) = margin {
                    validate_number_range(
                        *margin,
                        MIN_FRAME_MARGIN,
                        MAX_FRAME_MARGIN,
                        "margin",
                        label,
                    )?;
                }
                Ok(())
            }
            Self::SetRenderSettings {
                resolution,
                resolution_percentage,
            } => {
                if let Some(resolution) = resolution {
                    if resolution.len() != 2 {
                        return Err(format!(
                            "{label}: 'resolution' must have exactly 2 components [width, height]"
                        ));
                    }
                    for value in resolution {
                        validate_u32_range(*value, 1, MAX_RESOLUTION, "resolution", label)?;
                    }
                }
                if let Some(percentage) = resolution_percentage {
                    validate_u32_range(*percentage, 1, 100, "resolution_percentage", label)?;
                }
                Ok(())
            }
            Self::RenderCamera {
                camera,
                path,
                silhouette: _,
            } => {
                validate_required_name(camera, "camera", label)?;
                validate_relative_path(path, "path", label)?;
                validate_extension(path, &["png"], "path", label)?;
                Ok(())
            }
            Self::RenderViews { path } => {
                // Legacy base output directory (defaults to "blender/renders");
                // individual PNG file names are derived by the worker.
                if let Some(path) = path {
                    validate_relative_path(path, "path", label)?;
                }
                Ok(())
            }
            Self::CreateComponent { name } => validate_required_name(name, "name", label),
            Self::SaveBlend { path } => {
                validate_relative_path(path, "path", label)?;
                validate_extension(path, &["blend"], "path", label)
            }
            Self::ExportGlb { path } => {
                validate_relative_path(path, "path", label)?;
                validate_extension(path, &["glb"], "path", label)
            }
            Self::GetSceneSummary {} => Ok(()),
        }
    }
}

/// Result produced by a Blender Python worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlenderJobResult {
    /// The job id that was passed to the worker.
    pub job_id: String,
    /// `"completed"`, `"failed"`, or `"cancelled"`.
    pub status: String,
    /// Human-readable summary message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// List of artifact metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<BlenderArtifactDto>>,
    /// Scene introspection summary (triangles, objects, materials).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_summary: Option<serde_json::Value>,
    /// Structured error information if status is `"failed"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Describes an artifact generated by a Blender job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlenderArtifactDto {
    /// Path relative to the workspace project root.
    pub path: String,
    /// Artifact type (e.g. `"blend"`, `"glb"`, `"png"`).
    #[serde(rename = "type")]
    pub artifact_type: String,
    /// File size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Describes the capabilities of the configured Blender executable.
///
/// Wire compatibility: the Phase 2 fields keep their names and types. New
/// fields are optional and omitted when unknown; `blender_version` is `null`
/// when the binary/probe is unavailable (it used to be an empty string).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlenderCapabilitiesDto {
    /// Blender version string (e.g. `"Blender 5.2.3 LTS"`), `null` when unavailable.
    #[serde(default)]
    pub blender_version: Option<String>,
    /// Whether `bpy` is importable (i.e. headless background mode works).
    pub bpy_available: bool,
    /// List of safe worker operation names supported by the installed worker.
    pub supported_operations: Vec<String>,
    /// Whether the Blender binary was found at the configured path.
    pub binary_found: bool,
    /// Python version bundled with Blender (from the worker capabilities probe).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_version: Option<String>,
    /// Whether `bpy.ops.export_scene.gltf` is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gltf_export_available: Option<bool>,
    /// Render engines actually available in the installed Blender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_engines: Option<Vec<String>>,
    /// Probe error text; only present when probing failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BlenderCapabilitiesDto {
    /// Create an empty/unavailable capabilities descriptor.
    pub fn unavailable() -> Self {
        Self {
            blender_version: None,
            bpy_available: false,
            supported_operations: Vec::new(),
            binary_found: false,
            python_version: None,
            gltf_export_available: None,
            render_engines: None,
            error: None,
        }
    }

    /// Create an unavailable descriptor that falls back to the static whitelist.
    ///
    /// Used when the binary or the capabilities probe is unavailable; the
    /// static list keeps clients functional with a degraded capability set.
    pub fn unavailable_with_static_fallback(binary_found: bool, error: impl Into<String>) -> Self {
        Self {
            blender_version: None,
            bpy_available: false,
            supported_operations: static_operation_list(),
            binary_found,
            python_version: None,
            gltf_export_available: None,
            render_engines: None,
            error: Some(error.into()),
        }
    }
}

/// The set of safe operations accepted by the Blender worker.
///
/// Order is stable (Phase 2 operations first) and mirrored exactly by
/// `blender/worker.py`; the capabilities static fallback uses the same order.
pub const SAFE_WORKER_OPS: &[&str] = &[
    "create_box",
    "set_material",
    "set_camera_set",
    "render_views",
    "export_glb",
    "save_blend",
    "get_scene_summary",
    "create_cylinder",
    "create_sphere",
    "import_mesh",
    "set_transform",
    "create_ortho_camera",
    "set_camera_pose",
    "set_camera_settings",
    "frame_camera",
    "set_render_settings",
    "render_camera",
    "create_component",
];

fn static_operation_list() -> Vec<String> {
    SAFE_WORKER_OPS.iter().map(|op| (*op).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_job_type(job_type: &str) -> Result<(), String> {
    if job_type.trim().is_empty() {
        return Err("spec.job_type must not be empty".to_string());
    }
    if job_type.chars().count() > MAX_JOB_TYPE_CHARS {
        return Err(format!(
            "spec.job_type is too long (max {MAX_JOB_TYPE_CHARS} chars)"
        ));
    }
    if job_type.chars().any(char::is_control) {
        return Err("spec.job_type must not contain control characters".to_string());
    }
    Ok(())
}

fn validate_required_name(name: &str, field: &str, label: &str) -> Result<(), String> {
    validate_optional_name(Some(name), field, label)
}

fn validate_optional_name(name: Option<&str>, field: &str, label: &str) -> Result<(), String> {
    let Some(name) = name else {
        return Ok(());
    };
    if name.trim().is_empty() {
        return Err(format!("{label}: '{field}' must not be empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "{label}: '{field}' is too long (max {MAX_NAME_CHARS} chars)"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(format!(
            "{label}: '{field}' must not contain control characters"
        ));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!("{label}: '{field}' must not contain '/' or '\\'"));
    }
    if name == "." || name == ".." {
        return Err(format!("{label}: '{field}' must not be '.' or '..'"));
    }
    Ok(())
}

/// True for Windows drive/UNC path forms that must be rejected on every host.
///
/// `Path::is_absolute`/component parsing is platform-dependent, so a Linux host
/// would otherwise accept `C:foo.blend` or `\\server\share\x`. The Python
/// worker rejects them portably; Rust validation matches that behavior.
fn has_windows_style_path_form(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive_prefixed = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    drive_prefixed || path.starts_with("\\\\") || path.starts_with("//")
}

fn validate_relative_path(path: &str, field: &str, label: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(format!("{label}: '{field}' must not be empty"));
    }
    if path.chars().count() > MAX_SPEC_PATH_CHARS {
        return Err(format!(
            "{label}: '{field}' is too long (max {MAX_SPEC_PATH_CHARS} chars)"
        ));
    }
    if path.contains('\0') {
        return Err(format!("{label}: '{field}' must not contain NUL"));
    }
    // Windows drive (`C:foo`, `C:\x`), UNC (`\\server\share`) and `//` network
    // forms are rejected on EVERY platform (matching the Python worker), not
    // only when running on Windows.
    if has_windows_style_path_form(path) {
        return Err(format!(
            "{label}: '{field}' must not contain root/prefix components \
             (Windows drive/UNC forms are rejected): '{path}'"
        ));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(format!(
            "{label}: '{field}' must be workspace-relative \
             (absolute paths are rejected): '{path}'"
        ));
    }
    for component in parsed.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "{label}: '{field}' must not contain parent-traversal components: '{path}'"
                ));
            }
            _ => {
                return Err(format!(
                    "{label}: '{field}' must not contain root/prefix components: '{path}'"
                ));
            }
        }
    }
    Ok(())
}

fn validate_extension(
    path: &str,
    allowed: &[&str],
    field: &str,
    label: &str,
) -> Result<(), String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension {
        Some(extension) if allowed.iter().any(|allowed| *allowed == extension) => Ok(()),
        _ => Err(format!(
            "{label}: '{field}' must have one of the extensions: {} (case-insensitive)",
            allowed
                .iter()
                .map(|ext| format!(".{ext}"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn validate_coordinate_vector3(values: &[f64], field: &str, label: &str) -> Result<(), String> {
    if values.len() != 3 {
        return Err(format!("{label}: '{field}' must have exactly 3 components"));
    }
    for value in values {
        if !value.is_finite() {
            return Err(format!("{label}: '{field}' contains non-finite values"));
        }
        if value.abs() > MAX_COORDINATE_ABS {
            return Err(format!(
                "{label}: '{field}' component exceeds the ±{MAX_COORDINATE_ABS} limit"
            ));
        }
    }
    Ok(())
}

fn validate_positive_vector3(values: &[f64], field: &str, label: &str) -> Result<(), String> {
    if values.len() != 3 {
        return Err(format!("{label}: '{field}' must have exactly 3 components"));
    }
    for value in values {
        if !value.is_finite() {
            return Err(format!("{label}: '{field}' contains non-finite values"));
        }
        if *value <= 0.0 {
            return Err(format!(
                "{label}: '{field}' components must be finite numbers > 0"
            ));
        }
    }
    Ok(())
}

fn validate_nonzero_vector3(values: &[f64], field: &str, label: &str) -> Result<(), String> {
    if values.len() != 3 {
        return Err(format!("{label}: '{field}' must have exactly 3 components"));
    }
    for value in values {
        if !value.is_finite() {
            return Err(format!("{label}: '{field}' contains non-finite values"));
        }
        if *value == 0.0 {
            return Err(format!("{label}: '{field}' components must be non-zero"));
        }
    }
    Ok(())
}

fn validate_color(values: &[f64], field: &str, label: &str) -> Result<(), String> {
    if values.len() != 3 && values.len() != 4 {
        return Err(format!("{label}: '{field}' must have 3 or 4 components"));
    }
    for value in values {
        if !value.is_finite() {
            return Err(format!("{label}: '{field}' contains non-finite values"));
        }
        if !(0.0..=1.0).contains(value) {
            return Err(format!(
                "{label}: '{field}' components must be in the 0..=1 range"
            ));
        }
    }
    Ok(())
}

fn validate_positive_finite(value: f64, field: &str, label: &str) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{label}: '{field}' must be a finite number > 0"));
    }
    Ok(())
}

fn validate_optional_positive(
    value: Option<f64>,
    field: &str,
    label: &str,
) -> Result<Option<f64>, String> {
    match value {
        Some(value) => {
            validate_positive_finite(value, field, label)?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

fn validate_optional_shift(value: Option<f64>, field: &str, label: &str) -> Result<(), String> {
    if let Some(value) = value {
        validate_number_range(value, -MAX_SHIFT_ABS, MAX_SHIFT_ABS, field, label)?;
    }
    Ok(())
}

fn validate_clip_order(start: Option<f64>, end: Option<f64>, label: &str) -> Result<(), String> {
    if let (Some(start), Some(end)) = (start, end) {
        if end <= start {
            return Err(format!(
                "{label}: 'clip_end' must be greater than 'clip_start'"
            ));
        }
    }
    Ok(())
}

fn validate_pose_fields(
    rotation_deg: &Option<Vec<f64>>,
    look_at: &Option<Vec<f64>>,
    label: &str,
) -> Result<(), String> {
    if rotation_deg.is_some() && look_at.is_some() {
        return Err(format!(
            "{label}: 'rotation_deg' and 'look_at' are mutually exclusive"
        ));
    }
    if let Some(rotation) = rotation_deg {
        validate_coordinate_vector3(rotation, "rotation_deg", label)?;
    }
    if let Some(look_at) = look_at {
        validate_coordinate_vector3(look_at, "look_at", label)?;
    }
    Ok(())
}

fn validate_u32_range(
    value: u32,
    min: u32,
    max: u32,
    field: &str,
    label: &str,
) -> Result<(), String> {
    if value < min || value > max {
        return Err(format!(
            "{label}: '{field}' must be in the {min}..={max} range"
        ));
    }
    Ok(())
}

fn validate_number_range(
    value: f64,
    min: f64,
    max: f64,
    field: &str,
    label: &str,
) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{label}: '{field}' must be a finite number"));
    }
    if value < min || value > max {
        return Err(format!(
            "{label}: '{field}' must be in the {min}..={max} range"
        ));
    }
    Ok(())
}

/// Canonical camera view name for an accepted legacy wire name.
///
/// Returns `None` for a name that is neither a canonical view nor a documented
/// alias, so validation can produce an "unsupported view" error.
fn canonical_legacy_view(view: &str) -> Option<&'static str> {
    if let Some((_, canonical)) = LEGACY_VIEW_ALIASES.iter().find(|(alias, _)| *alias == view) {
        return Some(canonical);
    }
    LEGACY_VIEWS
        .iter()
        .find(|candidate| **candidate == view)
        .copied()
}

/// Canonical, deduplicated camera views for one `set_camera_set` operation.
///
/// Missing `views` expands to [`LEGACY_DEFAULT_VIEWS`]. Aliases resolve to the
/// canonical camera name (`side` -> `left`) and alias duplicates collapse to a
/// single camera while first-seen order is preserved. Used both for validation
/// and duplicate-created-name tracking, so `["side", "left"]` registers only
/// `Cam_left`.
fn canonical_legacy_views(views: Option<&[String]>) -> Vec<&'static str> {
    let requested: Vec<&str> = match views {
        Some(views) => views.iter().map(String::as_str).collect(),
        None => LEGACY_DEFAULT_VIEWS.to_vec(),
    };
    let mut canonical: Vec<&'static str> = Vec::new();
    for view in requested {
        if let Some(name) = canonical_legacy_view(view) {
            if !canonical.contains(&name) {
                canonical.push(name);
            }
        }
    }
    canonical
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec_with(operations: Vec<BlenderOperation>) -> BlenderJobSpec {
        BlenderJobSpec {
            job_type: "blender_test".into(),
            wait: true,
            timeout_ms: Some(30_000),
            spec: BlenderSpecBody { operations },
        }
    }

    fn parse_spec(value: serde_json::Value) -> BlenderJobSpec {
        serde_json::from_value(value).expect("spec must deserialize")
    }

    fn validate_value(operation: serde_json::Value) -> Result<(), String> {
        let operation: BlenderOperation =
            serde_json::from_value(operation).expect("operation must deserialize");
        spec_with(vec![operation]).validate()
    }

    fn assert_invalid(value: serde_json::Value, needle: &str) {
        let error = validate_value(value).expect_err("spec must be rejected");
        assert!(
            error.contains(needle),
            "expected error containing {needle:?}, got: {error}"
        );
    }

    fn full_valid_spec() -> BlenderJobSpec {
        spec_with(vec![
            BlenderOperation::CreateComponent {
                name: "body".into(),
            },
            BlenderOperation::CreateBox {
                name: Some("box".into()),
                size: Some(vec![2.0, 1.0, 0.5]),
                location: Some(vec![0.0, 0.0, 0.0]),
                rotation_deg: Some(vec![0.0, 0.0, 45.0]),
                component: Some("body".into()),
            },
            BlenderOperation::CreateCylinder {
                name: Some("cyl".into()),
                radius: Some(0.5),
                depth: Some(1.0),
                location: Some(vec![3.0, 0.0, 1.0]),
                rotation_deg: None,
                vertices: Some(32),
                component: None,
            },
            BlenderOperation::CreateSphere {
                name: Some("sphere".into()),
                radius: Some(0.5),
                location: Some(vec![-3.0, 0.0, 0.5]),
                rotation_deg: None,
                segments: Some(32),
                ring_count: Some(16),
                component: None,
            },
            BlenderOperation::ImportMesh {
                path: "blender/exports/imported.glb".into(),
                name: Some("imported".into()),
                component: Some("body".into()),
            },
            BlenderOperation::SetTransform {
                object: "box".into(),
                location: Some(vec![1.0, 0.0, 0.0]),
                rotation_deg: Some(vec![0.0, 0.0, 90.0]),
                scale: Some(vec![1.0, 2.0, 1.0]),
                apply_scale: Some(true),
            },
            BlenderOperation::SetMaterial {
                object: "box".into(),
                color: Some(vec![0.2, 0.4, 0.8, 1.0]),
            },
            BlenderOperation::SetCameraSet {
                views: Some(vec!["front".into(), "left".into()]),
            },
            BlenderOperation::CreateOrthoCamera {
                name: "Cam_ortho".into(),
                location: vec![0.0, -10.0, 0.0],
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
                ortho_scale: Some(4.0),
                shift_x: Some(0.1),
                shift_y: Some(-0.1),
                clip_start: Some(0.1),
                clip_end: Some(100.0),
                component: None,
            },
            BlenderOperation::SetCameraPose {
                camera: "Cam_ortho".into(),
                location: Some(vec![1.0, -10.0, 0.0]),
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
            },
            BlenderOperation::SetCameraSettings {
                camera: "Cam_ortho".into(),
                ortho_scale: Some(2.0),
                shift_x: Some(0.0),
                shift_y: Some(0.0),
                clip_start: Some(0.2),
                clip_end: Some(50.0),
            },
            BlenderOperation::FrameCamera {
                camera: "Cam_ortho".into(),
                objects: Some(vec!["box".into()]),
                component: None,
                margin: Some(1.2),
            },
            BlenderOperation::SetRenderSettings {
                resolution: Some(vec![512, 512]),
                resolution_percentage: Some(100),
            },
            BlenderOperation::RenderCamera {
                camera: "Cam_ortho".into(),
                path: "blender/renders/silhouette.png".into(),
                silhouette: Some(true),
            },
            BlenderOperation::RenderViews {
                path: Some("blender/renders".into()),
            },
            BlenderOperation::SaveBlend {
                path: "blender/scenes/composite.blend".into(),
            },
            BlenderOperation::ExportGlb {
                path: "blender/exports/composite.glb".into(),
            },
            BlenderOperation::GetSceneSummary {},
        ])
    }

    #[test]
    fn test_full_valid_spec_passes_validation() {
        let spec = full_valid_spec();
        assert_eq!(spec.spec.operations.len(), 18);
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_every_operation_roundtrips_through_json() {
        for operation in full_valid_spec().spec.operations {
            let value = serde_json::to_value(&operation).unwrap();
            assert_eq!(value["op"], json!(operation.op_name()));
            let decoded: BlenderOperation = serde_json::from_value(value).unwrap();
            assert_eq!(
                decoded,
                operation,
                "roundtrip failed for {}",
                operation.op_name()
            );
        }
    }

    #[test]
    fn test_op_names_match_safe_worker_ops() {
        // `full_valid_spec` contains exactly one operation of every variant, so
        // its wire names must equal the static whitelist (order is declaration
        // order for the enum and TZ order for the whitelist, hence the sort).
        let from_enum: Vec<&str> = full_valid_spec()
            .spec
            .operations
            .iter()
            .map(BlenderOperation::op_name)
            .collect();
        assert_eq!(from_enum.len(), 18);
        let mut sorted_enum = from_enum.clone();
        sorted_enum.sort_unstable();
        let mut sorted_static = SAFE_WORKER_OPS.to_vec();
        sorted_static.sort_unstable();
        assert_eq!(sorted_enum, sorted_static);

        let mut seen = std::collections::HashSet::new();
        for name in &from_enum {
            assert!(seen.insert(*name), "duplicate operation name {name}");
        }
    }

    #[test]
    fn test_serialization_omits_absent_optional_fields() {
        let save = BlenderOperation::SaveBlend {
            path: "out.blend".into(),
        };
        assert_eq!(
            serde_json::to_value(&save).unwrap(),
            json!({"op": "save_blend", "path": "out.blend"})
        );
        let summary = BlenderOperation::GetSceneSummary {};
        assert_eq!(
            serde_json::to_value(&summary).unwrap(),
            json!({"op": "get_scene_summary"})
        );
        let box_op = BlenderOperation::CreateBox {
            name: Some("b".into()),
            size: None,
            location: None,
            rotation_deg: None,
            component: None,
        };
        assert_eq!(
            serde_json::to_value(&box_op).unwrap(),
            json!({"op": "create_box", "name": "b"})
        );
    }

    #[test]
    fn test_legacy_phase2_json_examples_still_parse() {
        // docs/specs/BLENDER_WORKER.md and TZ Phase 2 example JSON, including
        // the documented `"side"` legacy view name.
        let legacy = json!({
            "job_type": "blender_smoke_scene",
            "wait": true,
            "timeout_ms": 180000,
            "spec": {"operations": [
                {"op": "create_box", "name": "body", "size": [2.0, 1.0, 0.5], "location": [0.0, 0.0, 0.0]},
                {"op": "set_material", "object": "body", "color": [0.2, 0.4, 0.8, 1.0]},
                {"op": "set_camera_set", "views": ["front", "side", "top", "perspective"]},
                {"op": "save_blend", "path": "blender/scenes/smoke.blend"},
                {"op": "export_glb", "path": "blender/exports/smoke.glb"}
            ]}
        });
        let spec = parse_spec(legacy);
        assert_eq!(spec.spec.operations.len(), 5);
        assert_eq!(spec.spec.operations[0].op_name(), "create_box");
        assert_eq!(spec.spec.operations[3].op_name(), "save_blend");
        // The documented Phase 2 scenario must validate end-to-end; `"side"`
        // is a back-compat alias for the canonical `"left"` camera.
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_side_view_alias_and_default_camera_names() {
        // Explicit "side" is accepted.
        assert!(validate_value(json!({"op": "set_camera_set", "views": ["side"]})).is_ok());
        // "side" + "left" collapse to one camera instead of creating two.
        assert!(validate_value(json!({"op": "set_camera_set", "views": ["side", "left"]})).is_ok());
        // The legacy default views (front, side, top, perspective) resolve
        // through the alias to Cam_front/Cam_left/Cam_top/Cam_perspective.
        let defaults = spec_with(vec![
            BlenderOperation::SetCameraSet { views: None },
            BlenderOperation::CreateOrthoCamera {
                name: "Cam_left".into(),
                location: vec![0.0, -10.0, 0.0],
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
                ortho_scale: None,
                shift_x: None,
                shift_y: None,
                clip_start: None,
                clip_end: None,
                component: None,
            },
        ]);
        assert!(
            defaults
                .validate()
                .unwrap_err()
                .contains("duplicate created object/camera name 'Cam_left'"),
            "the default 'side' view must register Cam_left"
        );
        // Explicit "side" registers Cam_left too.
        let explicit = spec_with(vec![
            BlenderOperation::SetCameraSet {
                views: Some(vec!["side".into()]),
            },
            BlenderOperation::CreateOrthoCamera {
                name: "Cam_left".into(),
                location: vec![0.0, -10.0, 0.0],
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
                ortho_scale: None,
                shift_x: None,
                shift_y: None,
                clip_start: None,
                clip_end: None,
                component: None,
            },
        ]);
        assert!(explicit
            .validate()
            .unwrap_err()
            .contains("duplicate created object/camera name 'Cam_left'"));
        // `["side", "left"]` creates one camera, so a later Cam_left also
        // collides (no double registration of Cam_left within the set itself).
        let aliased = spec_with(vec![
            BlenderOperation::SetCameraSet {
                views: Some(vec!["side".into(), "left".into()]),
            },
            BlenderOperation::CreateOrthoCamera {
                name: "Cam_left".into(),
                location: vec![0.0, -10.0, 0.0],
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
                ortho_scale: None,
                shift_x: None,
                shift_y: None,
                clip_start: None,
                clip_end: None,
                component: None,
            },
        ]);
        assert!(aliased
            .validate()
            .unwrap_err()
            .contains("duplicate created object/camera name 'Cam_left'"));
    }

    #[test]
    fn test_legacy_minimal_operations_parse_with_defaults() {
        let box_op: BlenderOperation = serde_json::from_value(json!({"op": "create_box"})).unwrap();
        assert_eq!(
            box_op,
            BlenderOperation::CreateBox {
                name: None,
                size: None,
                location: None,
                rotation_deg: None,
                component: None,
            }
        );
        let summary: BlenderOperation =
            serde_json::from_value(json!({"op": "get_scene_summary"})).unwrap();
        assert_eq!(summary, BlenderOperation::GetSceneSummary {});
        let render_views: BlenderOperation =
            serde_json::from_value(json!({"op": "render_views"})).unwrap();
        assert_eq!(render_views, BlenderOperation::RenderViews { path: None });
    }

    #[test]
    fn test_unknown_operation_rejected() {
        let error = serde_json::from_value::<BlenderJobSpec>(json!({
            "job_type": "x",
            "spec": {"operations": [{"op": "execute_arbitrary_python"}]}
        }))
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("execute_arbitrary_python"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_missing_required_field_rejected() {
        for (op, field) in [
            (json!({"op": "save_blend"}), "path"),
            (json!({"op": "export_glb"}), "path"),
            (json!({"op": "import_mesh"}), "path"),
            (json!({"op": "render_camera"}), "camera"),
            (json!({"op": "set_transform"}), "object"),
            (json!({"op": "set_material"}), "object"),
            (json!({"op": "create_ortho_camera"}), "name"),
            (json!({"op": "set_camera_pose"}), "camera"),
            (json!({"op": "set_camera_settings"}), "camera"),
            (json!({"op": "frame_camera"}), "camera"),
            (json!({"op": "create_component"}), "name"),
        ] {
            let error = serde_json::from_value::<BlenderJobSpec>(json!({
                "job_type": "x",
                "spec": {"operations": [op]}
            }))
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(field),
                "missing {field} must be rejected, got: {error}"
            );
        }
    }

    #[test]
    fn test_job_type_validation() {
        let mut spec = spec_with(vec![BlenderOperation::GetSceneSummary {}]);
        spec.job_type = "   ".into();
        assert!(spec.validate().unwrap_err().contains("must not be empty"));
        spec.job_type = "x".repeat(MAX_JOB_TYPE_CHARS + 1);
        assert!(spec.validate().unwrap_err().contains("too long"));
        spec.job_type = "bad\njob".into();
        assert!(spec.validate().unwrap_err().contains("control"));
        spec.job_type = "ok_job".into();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_operations_limit() {
        let mut spec = spec_with(vec![
            BlenderOperation::GetSceneSummary {};
            MAX_BLENDER_OPERATIONS
        ]);
        assert!(spec.validate().is_ok());
        spec.spec
            .operations
            .push(BlenderOperation::CreateComponent { name: "x".into() });
        assert!(spec.validate().unwrap_err().contains("too many operations"));
    }

    #[test]
    fn test_name_validation() {
        let long_name = "a".repeat(MAX_NAME_CHARS + 1);
        for (name, needle) in [
            ("", "must not be empty"),
            ("   ", "must not be empty"),
            (long_name.as_str(), "too long"),
            ("bad\nname", "control"),
            ("bad/name", "'/' or '\\'"),
            (r"bad\name", "'/' or '\\'"),
            (".", "'.' or '..'"),
            ("..", "'.' or '..'"),
        ] {
            let value = json!({"op": "create_component", "name": name});
            let error = validate_value(value).expect_err("name must be rejected");
            assert!(
                error.contains(needle),
                "expected {needle:?} for name {name:?}, got: {error}"
            );
        }

        // Component and object names use the same rules.
        assert_invalid(
            json!({"op": "create_box", "component": "bad/component"}),
            "'/' or '\\'",
        );
        assert_invalid(
            json!({"op": "set_transform", "object": "bl"}), // missing pose field
            "at least one",
        );
    }

    #[test]
    fn test_duplicate_created_names_rejected() {
        // Explicit duplicate object names.
        let duplicate_boxes = spec_with(vec![
            BlenderOperation::CreateBox {
                name: Some("body".into()),
                size: None,
                location: None,
                rotation_deg: None,
                component: None,
            },
            BlenderOperation::CreateBox {
                name: Some("body".into()),
                size: None,
                location: None,
                rotation_deg: None,
                component: None,
            },
        ]);
        assert!(duplicate_boxes
            .validate()
            .unwrap_err()
            .contains("duplicate created object/camera name 'body'"));

        // Default names are tracked too.
        let default_boxes = spec_with(vec![
            BlenderOperation::CreateBox {
                name: None,
                size: None,
                location: None,
                rotation_deg: None,
                component: None,
            },
            BlenderOperation::CreateBox {
                name: None,
                size: None,
                location: None,
                rotation_deg: None,
                component: None,
            },
        ]);
        assert!(default_boxes.validate().unwrap_err().contains("'Box'"));

        // A camera name colliding with an object name is rejected.
        let mixed = spec_with(vec![
            BlenderOperation::CreateCylinder {
                name: Some("shared".into()),
                radius: None,
                depth: None,
                location: None,
                rotation_deg: None,
                vertices: None,
                component: None,
            },
            BlenderOperation::CreateOrthoCamera {
                name: "shared".into(),
                location: vec![0.0, -10.0, 0.0],
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
                ortho_scale: None,
                shift_x: None,
                shift_y: None,
                clip_start: None,
                clip_end: None,
                component: None,
            },
        ]);
        assert!(mixed.validate().unwrap_err().contains("'shared'"));

        // Legacy camera set names collide with a later ortho camera.
        let camera_set_collision = spec_with(vec![
            BlenderOperation::SetCameraSet { views: None },
            BlenderOperation::CreateOrthoCamera {
                name: "Cam_front".into(),
                location: vec![0.0, -10.0, 0.0],
                rotation_deg: None,
                look_at: Some(vec![0.0, 0.0, 0.0]),
                ortho_scale: None,
                shift_x: None,
                shift_y: None,
                clip_start: None,
                clip_end: None,
                component: None,
            },
        ]);
        assert!(camera_set_collision
            .validate()
            .unwrap_err()
            .contains("'Cam_front'"));
    }

    #[test]
    fn test_path_validation() {
        let absolute = {
            #[cfg(windows)]
            {
                "C:/absolute/path.blend".to_string()
            }
            #[cfg(unix)]
            {
                "/absolute/path.blend".to_string()
            }
        };
        let long = format!("{}.blend", "a".repeat(MAX_SPEC_PATH_CHARS));

        for (op, needle) in [
            (json!({"op": "save_blend", "path": ""}), "must not be empty"),
            (json!({"op": "save_blend", "path": absolute}), "rejected"),
            (
                json!({"op": "save_blend", "path": "../evil.blend"}),
                "parent-traversal",
            ),
            (
                json!({"op": "save_blend", "path": "dir/../evil.blend"}),
                "parent-traversal",
            ),
            (json!({"op": "save_blend", "path": "a\0b.blend"}), "NUL"),
            (json!({"op": "save_blend", "path": long}), "too long"),
        ] {
            assert_invalid(op, needle);
        }
    }

    #[test]
    fn test_windows_style_paths_rejected_on_every_platform() {
        // Drive-relative / drive-absolute / UNC / `//` forms must be rejected
        // identically on Windows and Unix (matching the Python worker).
        let windows_forms = [
            r"C:foo.blend",
            r"C:\tmp\evil.blend",
            r"C:/tmp/evil.blend",
            r"\\server\share\evil.blend",
            r"//server/share/evil.blend",
        ];
        for path in windows_forms {
            let error = validate_value(json!({"op": "save_blend", "path": path}))
                .expect_err("windows path form must be rejected");
            assert!(
                error.contains("root/prefix"),
                "unexpected error for {path:?}: {error}"
            );
            // Import paths use the same validator.
            let error = validate_value(json!({"op": "import_mesh", "path": path}))
                .expect_err("windows import path form must be rejected");
            assert!(
                error.contains("root/prefix"),
                "unexpected import error for {path:?}: {error}"
            );
        }
        // A normal relative path with a drive-letter-like file name component is
        // unaffected (`..`-inside-name rule) — only the leading drive form matters.
        assert!(
            validate_value(json!({"op": "save_blend", "path": "scenes/C_notes.blend"})).is_ok()
        );
    }

    #[test]
    fn test_legacy_flat_json_with_extra_fields_and_null_optionals_parses() {
        // Phase 2 flat DTO tolerated arbitrary extra keys and explicit nulls for
        // all optional fields; the typed enum must keep parsing such payloads.
        let legacy = json!({
            "job_type": "legacy_flat",
            "wait": true,
            "timeout_ms": null,
            "spec": {"operations": [
                {
                    "op": "create_box",
                    "name": "body",
                    "size": [2.0, 1.0, 0.5],
                    "location": [0.0, 0.0, 0.0],
                    "rotation_deg": null,
                    "component": null,
                    // Fields the old flat DTO accepted on every operation.
                    "color": [1.0, 0.0, 0.0, 1.0],
                    "object": "ignored",
                    "views": ["front"],
                    "path": "ignored.blend",
                    "legacy_extra": {"nested": true}
                },
                {
                    "op": "save_blend",
                    "path": "blender/scenes/legacy.blend",
                    "name": null,
                    "size": null,
                    "location": null,
                    "color": null,
                    "object": null,
                    "views": null,
                    "unknown_extra": 42
                },
                {"op": "set_camera_set", "views": null},
                {"op": "render_views", "path": null},
                {"op": "get_scene_summary", "extra": null}
            ]}
        });
        let spec = parse_spec(legacy);
        assert_eq!(spec.spec.operations.len(), 5);
        assert_eq!(spec.spec.operations[0].op_name(), "create_box");
        assert_eq!(spec.spec.operations[1].op_name(), "save_blend");
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_path_extensions() {
        // Valid extensions (case-insensitive).
        assert!(validate_value(json!({"op": "save_blend", "path": "a.blend"})).is_ok());
        assert!(validate_value(json!({"op": "save_blend", "path": "a.BLEND"})).is_ok());
        assert!(validate_value(json!({"op": "export_glb", "path": "a.glb"})).is_ok());
        assert!(validate_value(json!({"op": "export_glb", "path": "a.GLB"})).is_ok());
        assert!(
            validate_value(json!({"op": "render_camera", "camera": "c", "path": "r/sil.png"}))
                .is_ok()
        );
        assert!(validate_value(json!({"op": "render_views", "path": "blender/renders"})).is_ok());
        assert!(validate_value(json!({"op": "render_views"})).is_ok());
        for extension in IMPORT_MESH_EXTENSIONS {
            assert!(validate_value(json!({
                "op": "import_mesh",
                "path": format!("assets/model.{extension}")
            }))
            .is_ok());
        }
        assert!(validate_value(json!({
            "op": "import_mesh",
            "path": "assets/model.PLY"
        }))
        .is_ok());

        // Invalid or mismatched extensions.
        assert_invalid(json!({"op": "save_blend", "path": "a.glb"}), ".blend");
        assert_invalid(json!({"op": "export_glb", "path": "a.blend"}), ".glb");
        assert_invalid(
            json!({"op": "render_camera", "camera": "c", "path": "a.jpg"}),
            ".png",
        );
        assert_invalid(
            json!({"op": "import_mesh", "path": "assets/model.fbx"}),
            ".glb",
        );
        assert_invalid(
            json!({"op": "import_mesh", "path": "assets/model"}),
            "extensions",
        );
        // `..` inside a file name is not traversal.
        assert!(validate_value(json!({
            "op": "save_blend",
            "path": "blender/scenes/model.v1..2.blend"
        }))
        .is_ok());
    }

    #[test]
    fn test_vector_validation() {
        for (value, needle) in [
            (
                json!({"op": "create_box", "size": [2.0, 1.0]}),
                "exactly 3 components",
            ),
            (json!({"op": "create_box", "size": [2.0, 0.0, 1.0]}), "> 0"),
            (json!({"op": "create_box", "size": [2.0, -1.0, 1.0]}), "> 0"),
            (
                json!({"op": "create_box", "location": [0.0, 0.0, 2_000_000.0]}),
                "1000000",
            ),
            (
                json!({"op": "create_box", "rotation_deg": [0.0, 0.0]}),
                "exactly 3 components",
            ),
            (
                json!({"op": "set_material", "object": "x", "color": [0.0, 0.0]}),
                "3 or 4",
            ),
            (
                json!({"op": "set_material", "object": "x", "color": [1.5, 0.0, 0.0, 1.0]}),
                "0..=1",
            ),
            (
                json!({"op": "set_transform", "object": "x", "scale": [1.0, 0.0, 1.0]}),
                "non-zero",
            ),
            (json!({"op": "create_cylinder", "radius": 0.0}), "> 0"),
            (json!({"op": "create_cylinder", "depth": -1.0}), "> 0"),
            (json!({"op": "create_cylinder", "vertices": 2}), "3..=1024"),
            (
                json!({"op": "create_cylinder", "vertices": 1025}),
                "3..=1024",
            ),
            (json!({"op": "create_sphere", "segments": 2}), "3..=1024"),
            (json!({"op": "create_sphere", "ring_count": 1}), "2..=1024"),
            (
                json!({"op": "create_ortho_camera", "name": "c", "location": [0.0, 0.0, 0.0], "ortho_scale": 0.0}),
                "> 0",
            ),
            (
                json!({"op": "create_ortho_camera", "name": "c", "location": [0.0, 0.0, 0.0], "shift_x": 11.0}),
                "-10..=10",
            ),
            (
                json!({"op": "create_ortho_camera", "name": "c", "location": [0.0, 0.0, 0.0], "clip_start": 0.0}),
                "> 0",
            ),
            (
                json!({"op": "create_ortho_camera", "name": "c", "location": [0.0, 0.0, 0.0], "clip_start": 10.0, "clip_end": 5.0}),
                "clip_end",
            ),
            (
                json!({"op": "set_render_settings", "resolution": [0, 512]}),
                "1..=16384",
            ),
            (
                json!({"op": "set_render_settings", "resolution": [512]}),
                "exactly 2 components",
            ),
            (
                json!({"op": "set_render_settings", "resolution_percentage": 0}),
                "1..=100",
            ),
            (
                json!({"op": "set_render_settings", "resolution_percentage": 101}),
                "1..=100",
            ),
            (
                json!({"op": "frame_camera", "camera": "c", "margin": 0.05}),
                "0.1..=10",
            ),
            (
                json!({"op": "frame_camera", "camera": "c", "margin": 11.0}),
                "0.1..=10",
            ),
        ] {
            assert_invalid(value, needle);
        }

        // Non-finite values can only be built programmatically.
        let nan_spec = spec_with(vec![BlenderOperation::CreateBox {
            name: None,
            size: None,
            location: Some(vec![f64::NAN, 0.0, 0.0]),
            rotation_deg: None,
            component: None,
        }]);
        assert!(nan_spec.validate().unwrap_err().contains("non-finite"));
        let inf_spec = spec_with(vec![BlenderOperation::CreateBox {
            name: None,
            size: Some(vec![f64::INFINITY, 1.0, 1.0]),
            location: None,
            rotation_deg: None,
            component: None,
        }]);
        assert!(inf_spec.validate().unwrap_err().contains("non-finite"));
    }

    #[test]
    fn test_views_validation() {
        assert!(validate_value(json!({"op": "set_camera_set"})).is_ok());
        for view in LEGACY_VIEW_NAMES {
            assert!(
                validate_value(json!({"op": "set_camera_set", "views": [view]})).is_ok(),
                "view {view:?} must be accepted"
            );
        }
        assert_invalid(json!({"op": "set_camera_set", "views": []}), "1 and 64");
        // Only truly unknown view names are rejected.
        assert_invalid(
            json!({"op": "set_camera_set", "views": ["sideways"]}),
            "unsupported view",
        );
        assert_invalid(
            json!({"op": "set_camera_set", "views": ["front", "front"]}),
            "duplicate view",
        );
        let too_many: Vec<&str> = std::iter::repeat_n("front", MAX_VIEWS + 1).collect();
        let error = validate_value(json!({"op": "set_camera_set", "views": too_many}))
            .expect_err("too many views must be rejected");
        assert!(error.contains("1 and 64"), "unexpected error: {error}");
    }

    #[test]
    fn test_pose_and_camera_rules() {
        // `create_ortho_camera` accepts no pose fields but rejects both.
        assert!(validate_value(json!({
            "op": "create_ortho_camera",
            "name": "c",
            "location": [0.0, -10.0, 0.0]
        }))
        .is_ok());
        assert_invalid(
            json!({
                "op": "create_ortho_camera",
                "name": "c",
                "location": [0.0, -10.0, 0.0],
                "rotation_deg": [90.0, 0.0, 0.0],
                "look_at": [0.0, 0.0, 0.0]
            }),
            "mutually exclusive",
        );

        // `set_camera_pose` requires at least one pose field.
        assert_invalid(
            json!({"op": "set_camera_pose", "camera": "c"}),
            "at least one",
        );
        assert!(validate_value(json!({
            "op": "set_camera_pose", "camera": "c", "location": [1.0, 2.0, 3.0]
        }))
        .is_ok());
        assert!(validate_value(json!({
            "op": "set_camera_pose", "camera": "c", "rotation_deg": [90.0, 0.0, 0.0]
        }))
        .is_ok());
        assert_invalid(
            json!({
                "op": "set_camera_pose",
                "camera": "c",
                "rotation_deg": [90.0, 0.0, 0.0],
                "look_at": [0.0, 0.0, 0.0]
            }),
            "mutually exclusive",
        );

        // `set_camera_settings` requires at least one setting.
        assert_invalid(
            json!({"op": "set_camera_settings", "camera": "c"}),
            "at least one camera setting",
        );
        assert!(validate_value(json!({
            "op": "set_camera_settings", "camera": "c", "ortho_scale": 2.0
        }))
        .is_ok());

        // `set_transform` requires at least one transform field.
        assert_invalid(
            json!({"op": "set_transform", "object": "o"}),
            "at least one",
        );
    }

    #[test]
    fn test_frame_camera_rules() {
        assert_invalid(
            json!({"op": "frame_camera", "camera": "c", "objects": []}),
            "must not be empty",
        );
        assert_invalid(
            json!({
                "op": "frame_camera",
                "camera": "c",
                "objects": ["box"],
                "component": "body"
            }),
            "at most one",
        );
        assert!(validate_value(json!({
            "op": "frame_camera", "camera": "c", "objects": ["box", "cyl"], "margin": 1.5
        }))
        .is_ok());
        assert!(validate_value(json!({
            "op": "frame_camera", "camera": "c", "component": "body"
        }))
        .is_ok());
        // Bad object names in `objects` are rejected.
        assert_invalid(
            json!({"op": "frame_camera", "camera": "c", "objects": ["bad/name"]}),
            "'/' or '\\'",
        );
    }

    #[test]
    fn test_validation_errors_name_the_operation_index() {
        let spec = spec_with(vec![
            BlenderOperation::GetSceneSummary {},
            BlenderOperation::SaveBlend {
                path: "out.glb".into(),
            },
        ]);
        let error = spec.validate().unwrap_err();
        assert!(error.starts_with("operation[1] ('save_blend')"), "{error}");
        assert!(error.contains(".blend"), "{error}");
    }

    #[test]
    fn test_operation_validate_in_isolation() {
        let operation = BlenderOperation::RenderCamera {
            camera: "c".into(),
            path: "../evil.png".into(),
            silhouette: Some(false),
        };
        let error = operation.validate().unwrap_err();
        assert!(error.contains("render_camera"), "{error}");
        assert!(error.contains("parent-traversal"), "{error}");
    }

    #[test]
    fn test_blender_job_result_serializes() {
        let result = BlenderJobResult {
            job_id: "job_blender_001".into(),
            status: "completed".into(),
            message: Some("Smoke test done".into()),
            artifacts: Some(vec![BlenderArtifactDto {
                path: "blender/scenes/smoke_v001.blend".into(),
                artifact_type: "blend".into(),
                size_bytes: Some(42_000),
            }]),
            scene_summary: Some(serde_json::json!({"objects": 3, "triangles": 24})),
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""job_id":"job_blender_001""#));
        assert!(json.contains(r#""status":"completed""#));
        assert!(json.contains(r#""path":"blender/scenes/smoke_v001.blend""#));
        assert!(json.contains(r#""objects":3"#));
    }

    #[test]
    fn test_blender_job_result_failure() {
        let result = BlenderJobResult {
            job_id: "job_fail_001".into(),
            status: "failed".into(),
            message: Some("Unknown operation: explode".into()),
            artifacts: None,
            scene_summary: None,
            error: Some("Operation 'explode' is not in the safe set".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""status":"failed""#));
        assert!(json.contains(r#""error":"#));
    }

    #[test]
    fn test_blender_capabilities_serializes() {
        let caps = BlenderCapabilitiesDto {
            blender_version: Some("Blender 5.2.3 LTS".into()),
            bpy_available: true,
            supported_operations: SAFE_WORKER_OPS.iter().map(|s| (*s).to_string()).collect(),
            binary_found: true,
            python_version: Some("3.13.13".into()),
            gltf_export_available: Some(true),
            render_engines: Some(vec!["BLENDER_WORKBENCH".into()]),
            error: None,
        };
        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains(r#""blender_version":"Blender 5.2.3 LTS""#));
        assert!(json.contains(r#""bpy_available":true"#));
        assert!(json.contains(r#""python_version":"3.13.13""#));
        assert!(json.contains(r#""gltf_export_available":true"#));
        assert!(json.contains(r#""render_engines":["BLENDER_WORKBENCH"]"#));
        assert!(json.contains(r#""create_box""#));
        assert!(json.contains(r#""import_mesh""#));
        // `error` is omitted on success.
        assert!(!json.contains(r#""error""#));
    }

    #[test]
    fn test_blender_capabilities_unavailable_serializes_null_version() {
        let caps = BlenderCapabilitiesDto::unavailable_with_static_fallback(false, "missing");
        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains(r#""blender_version":null"#));
        assert!(json.contains(r#""bpy_available":false"#));
        assert!(json.contains(r#""binary_found":false"#));
        assert!(json.contains(r#""error":"missing""#));
        assert!(json.contains("create_box"));
        // Static fallback lists exactly the full safe operation set.
        let expected: Vec<String> = SAFE_WORKER_OPS.iter().map(|op| (*op).to_string()).collect();
        assert_eq!(caps.supported_operations, expected);
        // Optional probe-only fields are omitted rather than null.
        assert!(!json.contains("python_version"));
        assert!(!json.contains("gltf_export_available"));
        assert!(!json.contains("render_engines"));
    }

    #[test]
    fn test_blender_capabilities_unavailable() {
        let caps = BlenderCapabilitiesDto::unavailable();
        assert!(!caps.binary_found);
        assert!(!caps.bpy_available);
        assert!(caps.blender_version.is_none());
        assert!(caps.supported_operations.is_empty());
        assert!(caps.error.is_none());
    }

    #[test]
    fn test_blender_capabilities_backward_compatible_deserialization() {
        // Phase 2 payload: no optional fields, string version.
        let json = r#"{
            "blender_version": "Blender 4.0.2",
            "bpy_available": false,
            "supported_operations": ["create_box"],
            "binary_found": true
        }"#;
        let caps: BlenderCapabilitiesDto = serde_json::from_str(json).unwrap();
        assert_eq!(caps.blender_version.as_deref(), Some("Blender 4.0.2"));
        assert!(!caps.bpy_available);
        assert!(caps.binary_found);
        assert!(caps.python_version.is_none());
        assert!(caps.error.is_none());
    }

    #[test]
    fn test_safe_worker_ops_are_known() {
        assert!(SAFE_WORKER_OPS.contains(&"create_box"));
        assert!(SAFE_WORKER_OPS.contains(&"save_blend"));
        assert!(SAFE_WORKER_OPS.contains(&"export_glb"));
        assert!(SAFE_WORKER_OPS.contains(&"render_camera"));
        assert!(SAFE_WORKER_OPS.contains(&"create_ortho_camera"));
        assert_eq!(SAFE_WORKER_OPS.len(), 18);
        assert!(!SAFE_WORKER_OPS.contains(&"execute_arbitrary_python"));
    }

    #[test]
    fn test_blender_artifact_dto_type_alias() {
        let artifact = BlenderArtifactDto {
            path: "output/render.png".into(),
            artifact_type: "png".into(),
            size_bytes: None,
        };
        let json = serde_json::to_string(&artifact).unwrap();
        assert!(json.contains(r#""type":"png""#));
        assert!(!json.contains("artifact_type"));
    }
}
