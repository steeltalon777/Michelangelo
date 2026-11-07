//! The Blender job specification of a reconstruction run.
//!
//! The plan is derived from the input package (the calibrated views) and
//! from the solver build report (the produced OBJ meshes):
//!
//! 1. one `import_mesh` per solver component, supplement or stub; every
//!    mesh goes into its own component collection and keeps the file stem as
//!    its object name,
//! 2. one `create_ortho_camera` per calibrated view (name = view id),
//! 3. one `set_render_settings` plus one `render_camera` per calibrated
//!    view (the render resolution is the resolution of the view image),
//! 4. `save_blend` and `export_glb` for the editable blockout.
//!
//! All paths are workspace-relative and use forward slashes.

use michelangelo_input::manifest::{InputPackage, ViewKind};
use michelangelo_protocol::blender::MAX_BLENDER_OPERATIONS;
use michelangelo_protocol::{BlenderJobSpec, BlenderOperation, BlenderSpecBody};

use crate::camera;
use crate::error::ReconError;
use crate::units;

/// The job type of a reconstruction job.
pub const JOB_TYPE: &str = "recon_blockout";

/// One mesh of the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedMesh {
    /// The object name (the file stem of the OBJ).
    pub name: String,
    /// The file name of the OBJ inside the source directory.
    pub obj: String,
    /// The workspace-relative path of the OBJ (filled by the planner).
    pub path: String,
}

/// A finished plan.
#[derive(Debug, Clone)]
pub struct PlannedRun {
    /// The Blender job specification.
    pub spec: BlenderJobSpec,
    /// The imported meshes, sorted by name.
    pub meshes: Vec<PlannedMesh>,
    /// The workspace-relative render paths, one per calibrated view.
    pub render_paths: Vec<String>,
    /// The workspace-relative path of the blend file.
    pub blend_path: String,
    /// The workspace-relative path of the glb file.
    pub glb_path: String,
}

/// Build the Blender job specification of a reconstruction run.
///
/// `include_blockout_artifacts` adds the `save_blend` and `export_glb`
/// operations (the scoring pass renders only and does not write a blockout).
pub fn build_job(
    manifest: &InputPackage,
    meshes: &[PlannedMesh],
    run_dir_rel: &str,
    source_dir_rel: &str,
    renders_dir_rel: &str,
    include_blockout_artifacts: bool,
) -> Result<PlannedRun, ReconError> {
    let units = manifest.units;
    let mut operations: Vec<BlenderOperation> = Vec::new();
    let mut planned_meshes: Vec<PlannedMesh> = Vec::new();

    let mut sorted: Vec<&PlannedMesh> = meshes.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));

    for mesh in sorted {
        let obj = mesh.obj.clone();
        let stem = obj
            .strip_suffix(".obj")
            .ok_or_else(|| ReconError::InvalidSpec {
                message: format!("the solver component '{obj}' is not an OBJ file"),
            })?
            .to_string();
        if stem.is_empty() || stem.contains('/') || stem.contains('\\') {
            return Err(ReconError::InvalidSpec {
                message: format!("the solver component '{obj}' has an unusable file name"),
            });
        }
        let path = format!("{source_dir_rel}/{obj}");
        operations.push(BlenderOperation::ImportMesh {
            path: path.clone(),
            name: Some(stem.clone()),
            component: Some(stem.clone()),
        });
        planned_meshes.push(PlannedMesh {
            name: stem,
            obj,
            path,
        });
    }

    if planned_meshes.is_empty() {
        return Err(ReconError::InvalidSpec {
            message: "the solver produced no component mesh".to_string(),
        });
    }

    let mut render_paths: Vec<String> = Vec::new();
    let mut camera_names: Vec<String> = Vec::new();

    for view in &manifest.views {
        if view.kind != ViewKind::Calibrated {
            continue;
        }
        let projection = view
            .projection
            .as_ref()
            .ok_or_else(|| ReconError::InvalidCamera {
                view_id: view.view_id.clone(),
                message: "the view has no projection".to_string(),
            })?;

        if projection.shift_x.abs() > f64::EPSILON || projection.shift_y.abs() > f64::EPSILON {
            return Err(ReconError::UnsupportedShift {
                view_id: view.view_id.clone(),
            });
        }

        let location = units::point_to_metres(projection.location_m, units)?;
        let look_at = units::point_to_metres(projection.look_at_m, units)?;
        let up_world = units::point_to_metres(projection.up_world, units)?;
        let rotation = camera::rotation_degrees(&location, &look_at, &up_world).map_err(|err| {
            ReconError::InvalidCamera {
                view_id: view.view_id.clone(),
                message: err.to_string(),
            }
        })?;

        let ortho_scale = units::to_metres(projection.ortho_scale_m, units)?;
        let clip_start = units::to_metres(projection.clip_start_m, units)?;
        let clip_end = units::to_metres(projection.clip_end_m, units)?;

        camera_names.push(view.view_id.clone());
        operations.push(BlenderOperation::CreateOrthoCamera {
            name: view.view_id.clone(),
            location: location.to_vec(),
            rotation_deg: Some(rotation.to_vec()),
            look_at: None,
            ortho_scale: Some(ortho_scale),
            shift_x: None,
            shift_y: None,
            clip_start: Some(clip_start),
            clip_end: Some(clip_end),
            component: None,
        });

        operations.push(BlenderOperation::SetRenderSettings {
            resolution: Some(vec![view.image.width_px, view.image.height_px]),
            resolution_percentage: None,
        });

        let render_path = format!("{renders_dir_rel}/{}.png", view.view_id);
        render_paths.push(render_path.clone());
        operations.push(BlenderOperation::RenderCamera {
            camera: view.view_id.clone(),
            path: render_path,
            silhouette: Some(true),
        });
    }

    if camera_names.is_empty() {
        return Err(ReconError::InvalidSpec {
            message: "the package has no calibrated view".to_string(),
        });
    }

    let blend_path = format!("{run_dir_rel}/blockout.blend");
    let glb_path = format!("{run_dir_rel}/blockout.glb");
    if include_blockout_artifacts {
        operations.push(BlenderOperation::SaveBlend {
            path: blend_path.clone(),
        });
        operations.push(BlenderOperation::ExportGlb {
            path: glb_path.clone(),
        });
    }

    check_operation_count(operations.len())?;
    check_unique_names(&planned_meshes, &camera_names)?;
    let mut paths: Vec<&str> = render_paths.iter().map(String::as_str).collect();
    paths.push(blend_path.as_str());
    paths.push(glb_path.as_str());
    paths.extend(planned_meshes.iter().map(|mesh| mesh.path.as_str()));
    for path in paths {
        check_relative_path(path)?;
    }

    let spec = BlenderJobSpec {
        job_type: JOB_TYPE.to_string(),
        wait: true,
        timeout_ms: None,
        spec: BlenderSpecBody { operations },
    };
    spec.validate()
        .map_err(|message| ReconError::InvalidSpec { message })?;

    Ok(PlannedRun {
        spec,
        meshes: planned_meshes,
        render_paths,
        blend_path,
        glb_path,
    })
}

/// The maximum number of operations of one Blender job.
fn check_operation_count(count: usize) -> Result<(), ReconError> {
    if count > MAX_BLENDER_OPERATIONS {
        return Err(ReconError::InvalidSpec {
            message: format!(
                "the job needs {count} operations, but at most {MAX_BLENDER_OPERATIONS} are allowed"
            ),
        });
    }
    Ok(())
}

/// Every created object and camera name must be unique.
fn check_unique_names(meshes: &[PlannedMesh], cameras: &[String]) -> Result<(), ReconError> {
    let mut names: Vec<&str> = Vec::new();
    for mesh in meshes {
        names.push(mesh.name.as_str());
    }
    for camera in cameras {
        names.push(camera.as_str());
    }
    names.sort_unstable();
    for pair in names.windows(2) {
        if pair[0] == pair[1] {
            return Err(ReconError::InvalidSpec {
                message: format!("the name '{}' is used twice in one job", pair[0]),
            });
        }
    }
    Ok(())
}

/// A path must be workspace-relative and free of traversal components.
fn check_relative_path(path: &str) -> Result<(), ReconError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains("..")
    {
        return Err(ReconError::InvalidSpec {
            message: format!("the path '{path}' is not a safe workspace-relative path"),
        });
    }
    if path.split('/').any(|part| part.is_empty()) {
        return Err(ReconError::InvalidSpec {
            message: format!("the path '{path}' is not a safe workspace-relative path"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use michelangelo_input::load_manifest;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("input_packages")
            .join("valid_full")
    }

    fn fixture_manifest() -> InputPackage {
        load_manifest(&fixture_dir()).expect("the committed fixture is valid")
    }

    fn meshes() -> Vec<PlannedMesh> {
        vec![
            PlannedMesh {
                name: "blockout".to_string(),
                obj: "blockout.obj".to_string(),
                path: String::new(),
            },
            PlannedMesh {
                name: "stub_u_interior".to_string(),
                obj: "stub_u_interior.obj".to_string(),
                path: String::new(),
            },
        ]
    }

    fn plan_of(manifest: &InputPackage) -> Result<PlannedRun, ReconError> {
        build_job(
            manifest,
            &meshes(),
            "recon/r0001/run-0001",
            "recon/r0001/run-0001/source",
            "recon/r0001/run-0001/renders",
            true,
        )
    }

    #[test]
    fn the_plan_starts_with_the_imports_and_ends_with_the_blockout() {
        let manifest = fixture_manifest();
        let plan = plan_of(&manifest).expect("the plan is built");
        let operations = &plan.spec.spec.operations;
        assert_eq!(operations.len(), 2 + 3 * 2 + 2);
        assert_eq!(operations[0].op_name(), "import_mesh");
        assert_eq!(operations[1].op_name(), "import_mesh");
        assert_eq!(operations[operations.len() - 2].op_name(), "save_blend");
        assert_eq!(operations[operations.len() - 1].op_name(), "export_glb");
    }

    #[test]
    fn every_mesh_goes_into_its_own_component() {
        let manifest = fixture_manifest();
        let plan = plan_of(&manifest).expect("the plan is built");
        for operation in &plan.spec.spec.operations {
            if let BlenderOperation::ImportMesh {
                path,
                name,
                component,
            } = operation
            {
                assert_eq!(name, component);
                assert_eq!(
                    path.as_str(),
                    format!(
                        "recon/r0001/run-0001/source/{}.obj",
                        name.clone().unwrap_or_default()
                    )
                );
            }
        }
        assert_eq!(plan.meshes.len(), 2);
        assert_eq!(plan.meshes[0].name, "blockout");
    }

    #[test]
    fn every_view_gets_a_camera_a_resolution_and_a_render() {
        let manifest = fixture_manifest();
        let plan = plan_of(&manifest).expect("the plan is built");
        let operations = &plan.spec.spec.operations;
        let camera = operations
            .iter()
            .find(|operation| operation.op_name() == "create_ortho_camera")
            .expect("a camera is created");
        match camera {
            BlenderOperation::CreateOrthoCamera {
                name, ortho_scale, ..
            } => {
                assert_eq!(name, "front");
                assert_eq!(*ortho_scale, Some(1.0));
            }
            other => panic!("unexpected operation: {other:?}"),
        }
        let settings = operations
            .iter()
            .find(|operation| operation.op_name() == "set_render_settings")
            .expect("the render settings are set");
        match settings {
            BlenderOperation::SetRenderSettings { resolution, .. } => {
                assert_eq!(resolution.as_ref(), Some(&vec![8_u32, 8_u32]));
            }
            other => panic!("unexpected operation: {other:?}"),
        }
        assert_eq!(
            plan.render_paths,
            vec![
                "recon/r0001/run-0001/renders/front.png".to_string(),
                "recon/r0001/run-0001/renders/right.png".to_string(),
            ]
        );
    }

    #[test]
    fn the_camera_rotation_matches_the_reference() {
        let manifest = fixture_manifest();
        let plan = plan_of(&manifest).expect("the plan is built");
        let rotations: Vec<(String, Vec<f64>)> = plan
            .spec
            .spec
            .operations
            .iter()
            .filter_map(|operation| match operation {
                BlenderOperation::CreateOrthoCamera {
                    name, rotation_deg, ..
                } => Some((name.clone(), rotation_deg.clone().unwrap_or_default())),
                _ => None,
            })
            .collect();
        assert_eq!(rotations.len(), 2);
        assert!((rotations[0].1[0] - 90.0).abs() < 1e-6);
        assert!((rotations[0].1[1]).abs() < 1e-6);
        assert!((rotations[0].1[2]).abs() < 1e-6);
        assert!((rotations[1].1[0] - 90.0).abs() < 1e-6);
        assert!((rotations[1].1[2] - 90.0).abs() < 1e-6);
    }

    #[test]
    fn a_non_zero_shift_is_rejected() {
        let mut value: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture_dir().join("package.json")).unwrap(),
        )
        .expect("the fixture is valid JSON");
        value["views"][0]["projection"]["shift_x"] = serde_json::json!(0.25);
        let manifest: InputPackage = serde_json::from_value(value).expect("the manifest is valid");
        let error = plan_of(&manifest).expect_err("a shifted view must be rejected");
        assert!(matches!(error, ReconError::UnsupportedShift { .. }));
        assert!(error.to_string().contains("UNSUPPORTED_SHIFT"));
    }

    #[test]
    fn the_operation_count_is_bounded() {
        assert!(check_operation_count(256).is_ok());
        let error = check_operation_count(257).expect_err("257 operations are too many");
        assert!(matches!(error, ReconError::InvalidSpec { .. }));
    }

    #[test]
    fn duplicated_names_are_rejected() {
        let meshes = vec![
            PlannedMesh {
                name: "blockout".to_string(),
                obj: "blockout.obj".to_string(),
                path: String::new(),
            },
            PlannedMesh {
                name: "blockout".to_string(),
                obj: "blockout.obj".to_string(),
                path: String::new(),
            },
        ];
        let manifest = fixture_manifest();
        let error = build_job(
            &manifest,
            &meshes,
            "recon/r0001/run-0001",
            "recon/r0001/run-0001/source",
            "recon/r0001/run-0001/renders",
            true,
        )
        .expect_err("duplicated names must be rejected");
        assert!(matches!(error, ReconError::InvalidSpec { .. }));
    }

    #[test]
    fn the_paths_must_be_workspace_relative() {
        assert!(check_relative_path("recon/r0001/run-0001/renders/front.png").is_ok());
        assert!(check_relative_path("/absolute/front.png").is_err());
        assert!(check_relative_path("recon/../etc/passwd").is_err());
        assert!(check_relative_path("C:/windows/front.png").is_err());
    }

    #[test]
    fn the_plan_passes_the_protocol_validation() {
        let manifest = fixture_manifest();
        let plan = plan_of(&manifest).expect("the plan is built");
        assert!(plan.spec.validate().is_ok());
        assert_eq!(plan.blend_path, "recon/r0001/run-0001/blockout.blend");
        assert_eq!(plan.glb_path, "recon/r0001/run-0001/blockout.glb");
    }

    #[test]
    fn a_package_without_a_calibrated_view_is_rejected() {
        let mut value: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture_dir().join("package.json")).unwrap(),
        )
        .expect("the fixture is valid JSON");
        value["views"][0]["kind"] = serde_json::json!("reference");
        value["views"][1]["kind"] = serde_json::json!("reference");
        let manifest: InputPackage = serde_json::from_value(value).expect("the manifest is valid");
        let error =
            plan_of(&manifest).expect_err("a package without a calibrated view is unusable");
        assert!(matches!(error, ReconError::InvalidSpec { .. }));
    }
}
