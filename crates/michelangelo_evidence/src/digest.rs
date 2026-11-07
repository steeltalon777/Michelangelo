//! The digests of the verification contract (contract §3).
//!
//! The digests are SHA-256 over canonical bytes. They are always computed by
//! the Core, never taken from the document that declares them when the
//! document is itself evidence:
//!
//! * **file** — SHA-256 of the file bytes,
//! * **geometry aggregate** — SHA-256 over the lines
//!   `"<component>\u{0}<sha256>\n"` sorted by the component name,
//! * **camera digest** — SHA-256 of the canonical JSON of the calibration
//!   fields of a view,
//! * **inputs digest** — SHA-256 of the canonical JSON of the pinned inputs
//!   of an evaluation.

use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::EvidenceError;
use crate::report::{CameraDigest, EvaluationInputs, ViewPin};

/// The SHA-256 of the bytes, as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    michelangelo_input::sha256::sha256_hex(bytes)
}

/// The SHA-256 of the bytes of a file.
pub fn file_sha256(path: &Path) -> Result<String, EvidenceError> {
    let bytes = std::fs::read(path)
        .map_err(|err| EvidenceError::io(path, format!("the file cannot be read: {err}")))?;
    Ok(sha256_hex(&bytes))
}

/// The aggregate digest of a component set.
///
/// The digest is the SHA-256 over the lines `"<component>\u{0}<sha256>\n"`,
/// sorted by the name of the component (contract §3).
pub fn geometry_aggregate(components: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = components.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let mut buffer: Vec<u8> = Vec::new();
    for (component, sha256) in sorted {
        buffer.extend_from_slice(component.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(sha256.as_bytes());
        buffer.push(b'\n');
    }
    sha256_hex(&buffer)
}

/// The canonical JSON of a value: the object keys are sorted, the text is
/// compact and the numbers keep the representation of the value.
pub fn canonical_json(value: &Value) -> String {
    let mut text = String::new();
    write_canonical(value, &mut text);
    text
}

/// The canonical JSON of a serializable value.
pub fn canonical_json_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(value) => canonical_json(&value),
        Err(_) => String::from("null"),
    }
}

/// Append the canonical JSON of a value to a buffer.
fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (position, key) in keys.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(key));
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_else(|_| "null".to_string())),
    }
}

/// The JSON representation of a string.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"".to_string())
}

/// The digest of the calibration of a view (contract §3).
///
/// The digest is the SHA-256 of the canonical JSON of the view identifier,
/// the camera parameters and the resolution of the image.
pub fn camera_digest(camera: &CameraDigest) -> String {
    let value = json!({
        "view_id": camera.view_id,
        "location_m": camera.location_m,
        "look_at_m": camera.look_at_m,
        "up_world": camera.up_world,
        "ortho_scale_m": camera.ortho_scale_m,
        "shift_x": camera.shift_x,
        "shift_y": camera.shift_y,
        "clip_start_m": camera.clip_start_m,
        "clip_end_m": camera.clip_end_m,
        "width_px": camera.width_px,
        "height_px": camera.height_px,
    });
    sha256_hex(canonical_json(&value).as_bytes())
}

/// The canonical JSON of the pinned inputs of an evaluation.
pub fn inputs_canonical_json(inputs: &EvaluationInputs) -> String {
    let mut views: Vec<&ViewPin> = inputs.views.iter().collect();
    views.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    let pinned: Vec<Value> = views
        .iter()
        .map(|view| {
            json!({
                "view_id": view.view_id,
                "camera_digest": view.camera_digest,
                "mask_sha256": view.mask_sha256,
                "resolution": view.resolution,
                "held_out": view.held_out,
            })
        })
        .collect();
    let value = json!({
        "package_sha256": inputs.package_sha256,
        "views": pinned,
        "evaluation_package_sha256": inputs.evaluation_package_sha256,
    });
    canonical_json(&value)
}

/// The digest of the pinned inputs of an evaluation (contract §3).
pub fn inputs_digest(inputs: &EvaluationInputs) -> String {
    sha256_hex(inputs_canonical_json(inputs).as_bytes())
}
