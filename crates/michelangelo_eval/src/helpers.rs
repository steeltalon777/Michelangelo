//! The shared helpers of the evaluator.

use std::path::{Path, PathBuf};

use michelangelo_evidence::report::PackageView;
use michelangelo_input::manifest::Units;
use serde_json::Value;

use crate::render::ViewSpec;

/// The unit of the lengths of a package.
pub fn units_of(package: &Value) -> Units {
    match package.get("units").and_then(|value| value.as_str()) {
        Some("cm") => Units::Cm,
        Some("mm") => Units::Mm,
        _ => Units::M,
    }
}

/// The render specification of a view of the package.
pub fn view_spec(view: &PackageView) -> ViewSpec {
    ViewSpec {
        view_id: view.view_id.clone(),
        location_m: view.camera.location_m,
        look_at_m: view.camera.look_at_m,
        up_world: view.camera.up_world,
        ortho_scale_m: view.camera.ortho_scale_m,
        shift_x: view.camera.shift_x,
        shift_y: view.camera.shift_y,
        clip_start_m: view.camera.clip_start_m,
        clip_end_m: view.camera.clip_end_m,
        width_px: view.camera.width_px,
        height_px: view.camera.height_px,
    }
}

/// The paths of the masks of the views of a package.
pub fn mask_paths_of(package_dir: &Path, package: &Value) -> Vec<(String, PathBuf)> {
    let mut paths: Vec<(String, PathBuf)> = Vec::new();
    let entries = match package.get("views").and_then(|value| value.as_array()) {
        Some(entries) => entries,
        None => return paths,
    };
    for entry in entries {
        let kind = entry
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("calibrated");
        if kind != "calibrated" {
            continue;
        }
        let view_id = entry
            .get("view_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let relative = entry
            .get("mask")
            .and_then(|mask| mask.get("path"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if relative.is_empty() {
            continue;
        }
        let separator = std::path::MAIN_SEPARATOR.to_string();
        let path = package_dir.join(relative.replace('/', &separator));
        paths.push((view_id, path));
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths
}

/// The unit direction of the camera of a view.
pub fn direction_of(view: &ViewSpec) -> [f64; 3] {
    let mut direction = [
        view.location_m[0] - view.look_at_m[0],
        view.location_m[1] - view.look_at_m[1],
        view.location_m[2] - view.look_at_m[2],
    ];
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length > 0.0 {
        for value in direction.iter_mut() {
            *value /= length;
        }
    }
    direction
}

/// The scalar product of two directions.
pub fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}
