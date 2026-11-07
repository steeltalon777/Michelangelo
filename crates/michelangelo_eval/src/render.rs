//! The renderer of the evaluated geometry.
//!
//! The renderer is injected so that the evaluation can be tested without
//! Blender: the production implementation renders through the existing worker
//! operations (`michelangelo_recon::plan::build_job` and the headless
//! Blender adapter), the tests use a deterministic fake.

use std::path::{Path, PathBuf};

use michelangelo_blender::HeadlessBlenderAdapter;
use michelangelo_input::manifest::{InputPackage, Units};
use michelangelo_recon::plan::{build_job, PlannedMesh};
use serde_json::{json, Value};

use crate::error::EvalError;

/// The digest of an empty file (the placeholders of the synthetic manifest).
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The calibration of one view.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSpec {
    /// The identifier of the view.
    pub view_id: String,
    /// The location of the camera, in package units.
    pub location_m: [f64; 3],
    /// The point the camera looks at, in package units.
    pub look_at_m: [f64; 3],
    /// The world space up vector of the camera.
    pub up_world: [f64; 3],
    /// The visible world width of the image.
    pub ortho_scale_m: f64,
    /// The horizontal lens shift.
    pub shift_x: f64,
    /// The vertical lens shift.
    pub shift_y: f64,
    /// The near clip distance.
    pub clip_start_m: f64,
    /// The far clip distance.
    pub clip_end_m: f64,
    /// The width of the image, in pixels.
    pub width_px: u32,
    /// The height of the image, in pixels.
    pub height_px: u32,
}

/// A set of views that are rendered with the same settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSet {
    /// The unit of the lengths of the views.
    pub units: Units,
    /// The views of the set.
    pub views: Vec<ViewSpec>,
}

/// A staged set of components (the OBJ files of one geometry).
#[derive(Debug, Clone, PartialEq)]
pub struct GeometrySet {
    /// The directory that carries the OBJ files.
    pub dir: PathBuf,
    /// The components of the set, with their digests.
    pub components: Vec<(String, String)>,
}

/// A request to render a geometry with a set of views.
pub struct RenderRequest<'a> {
    /// The geometry to render.
    pub geometry: &'a GeometrySet,
    /// The views to render.
    pub views: &'a ViewSet,
    /// The root the relative paths are resolved against.
    pub workspace_root: &'a Path,
    /// The workspace-relative directory of the geometry.
    pub source_dir_rel: &'a str,
    /// The workspace-relative directory of the renders.
    pub renders_dir_rel: &'a str,
    /// The workspace-relative directory of the work area of the evaluation.
    pub work_dir_rel: &'a str,
    /// The absolute directory of the renders.
    pub renders_dir: &'a Path,
    /// The identifier of the Blender job.
    pub job_id: &'a str,
    /// The directory of the Blender job files.
    pub output_dir: &'a Path,
}

/// The renderer of a geometry.
pub trait Renderer {
    /// Render the geometry of the request with the views of the request.
    fn render(&self, request: &RenderRequest<'_>) -> Result<(), EvalError>;
}

/// The renderer that runs the real Blender worker.
pub struct BlenderRenderer {
    adapter: HeadlessBlenderAdapter,
    timeout_ms: Option<u64>,
    fake_blender: Option<PathBuf>,
}

impl BlenderRenderer {
    /// The renderer that uses the given Blender binary.
    pub fn new(blender: Option<&Path>, timeout_ms: Option<u64>) -> Self {
        let blender_bin = blender
            .map(|path| path.display().to_string())
            .or_else(|| std::env::var("BLENDER_BIN").ok())
            .unwrap_or_else(|| "blender".to_string());
        BlenderRenderer {
            adapter: HeadlessBlenderAdapter::with_config(blender_bin, worker_script_path()),
            timeout_ms,
            fake_blender: None,
        }
    }

    /// The renderer that uses a fake Blender binary (the test hook).
    pub fn with_fake(mut self, fake_blender: Option<PathBuf>) -> Self {
        self.fake_blender = fake_blender;
        self
    }
}

impl Renderer for BlenderRenderer {
    fn render(&self, request: &RenderRequest<'_>) -> Result<(), EvalError> {
        let manifest = manifest_of(request.views)?;
        let meshes: Vec<PlannedMesh> = request
            .geometry
            .components
            .iter()
            .map(|(name, _)| PlannedMesh {
                name: name.clone(),
                obj: format!("{name}.obj"),
                path: String::new(),
            })
            .collect();
        let planned = build_job(
            &manifest,
            &meshes,
            request.work_dir_rel,
            request.source_dir_rel,
            request.renders_dir_rel,
            false,
        )
        .map_err(|err| EvalError::Render {
            message: format!("the Blender job cannot be built: {err}"),
        })?;

        let result = match &self.fake_blender {
            Some(fake) => self.adapter.run_job_with_fake(
                fake,
                request.job_id,
                &planned.spec,
                request.workspace_root,
                request.output_dir,
                self.timeout_ms,
            ),
            None => self.adapter.run_job(
                request.job_id,
                &planned.spec,
                request.workspace_root,
                request.output_dir,
                self.timeout_ms,
            ),
        }
        .map_err(|err| EvalError::Render {
            message: format!("the Blender job failed: {err}"),
        })?;

        if result.status != "completed" {
            return Err(EvalError::Render {
                message: format!(
                    "the Blender job '{}' finished with the status '{}'",
                    request.job_id, result.status
                ),
            });
        }
        for view in &request.views.views {
            let path = request.renders_dir.join(format!("{}.png", view.view_id));
            let metadata = std::fs::metadata(&path).map_err(|err| EvalError::Render {
                message: format!("the render '{}' is missing: {err}", path.display()),
            })?;
            if !metadata.is_file() || metadata.len() == 0 {
                return Err(EvalError::Render {
                    message: format!("the render '{}' is empty", path.display()),
                });
            }
        }
        Ok(())
    }
}

/// The path of the Blender worker script.
fn worker_script_path() -> PathBuf {
    let candidate = michelangelo_recon::solver::repo_root()
        .join("blender")
        .join("worker.py");
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from("blender").join("worker.py")
    }
}

/// Build the synthetic input package of a render request.
///
/// The package carries the calibrated views of the request; the worker only
/// uses the calibration of the views and their resolution.
fn manifest_of(views: &ViewSet) -> Result<InputPackage, EvalError> {
    let units = match views.units {
        Units::M => "m",
        Units::Cm => "cm",
        Units::Mm => "mm",
    };
    let entries: Vec<Value> = views
        .views
        .iter()
        .map(|view| {
            json!({
                "view_id": view.view_id,
                "kind": "calibrated",
                "image": {
                    "path": format!("renders/{}.png", view.view_id),
                    "sha256": EMPTY_SHA256,
                    "width_px": view.width_px,
                    "height_px": view.height_px,
                },
                "mask": {
                    "path": format!("masks/{}.png", view.view_id),
                    "sha256": EMPTY_SHA256,
                    "format": "grayscale8",
                    "foreground": "white",
                },
                "projection": {
                    "type": "orthographic",
                    "location_m": view.location_m,
                    "look_at_m": view.look_at_m,
                    "up_world": view.up_world,
                    "ortho_scale_m": view.ortho_scale_m,
                    "shift_x": view.shift_x,
                    "shift_y": view.shift_y,
                    "clip_start_m": view.clip_start_m,
                    "clip_end_m": view.clip_end_m,
                },
            })
        })
        .collect();
    let value = json!({
        "schema": "michelangelo.input-package/0",
        "package_id": "michelangelo.evaluation",
        "object": {"name": "evaluation"},
        "revision": 1,
        "units": units,
        "coordinate_system": {
            "handedness": "right",
            "up": "+Y",
            "right": "+X",
            "forward": "-Z",
        },
        "views": entries,
        "provenance": {
            "origin": "human",
            "detail": "the manifest of an evaluation is assembled by the Core",
        },
    });
    serde_json::from_value(value).map_err(|err| EvalError::Invalid {
        message: format!("the views of the render cannot be assembled: {err}"),
    })
}
