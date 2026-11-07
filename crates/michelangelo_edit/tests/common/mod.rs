#![allow(dead_code)]

//! Shared helpers of the edit boundary tests.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Write a text file and create the parent directories.
pub fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent directory is created");
    }
    std::fs::write(path, content).expect("the file is written");
}

/// A box OBJ of the given size, centred at the origin.
pub fn box_obj(size: f64) -> String {
    let half = size / 2.0;
    let corners: [(f64, f64, f64); 8] = [
        (-1.0, -1.0, -1.0),
        (1.0, -1.0, -1.0),
        (1.0, 1.0, -1.0),
        (-1.0, 1.0, -1.0),
        (-1.0, -1.0, 1.0),
        (1.0, -1.0, 1.0),
        (1.0, 1.0, 1.0),
        (-1.0, 1.0, 1.0),
    ];
    let mut text = String::from(
        "# a test box
",
    );
    for (x, y, z) in corners {
        text.push_str(&format!(
            "v {} {} {}
",
            x * half,
            y * half,
            z * half
        ));
    }
    let faces: [[usize; 4]; 6] = [
        [1, 4, 3, 2],
        [5, 6, 7, 8],
        [1, 2, 6, 5],
        [4, 8, 7, 3],
        [1, 5, 8, 4],
        [2, 3, 7, 6],
    ];
    for face in faces {
        text.push_str(&format!(
            "f {} {} {} {}
",
            face[0], face[1], face[2], face[3]
        ));
    }
    text
}

/// A lid box that sits on top of a 0.1 m base (the base spans y in
/// [-0.05, 0.05]).
pub fn lid_obj(height: f64) -> String {
    let mut text = String::from(
        "# a test lid
",
    );
    let x = 0.04;
    let z = 0.04;
    let bottom = 0.05;
    let top = bottom + height;
    let corners: [(f64, f64, f64); 8] = [
        (-x, bottom, -z),
        (x, bottom, -z),
        (x, top, -z),
        (-x, top, -z),
        (-x, bottom, z),
        (x, bottom, z),
        (x, top, z),
        (-x, top, z),
    ];
    for (px, py, pz) in corners {
        text.push_str(&format!(
            "v {px} {py} {pz}
"
        ));
    }
    let faces: [[usize; 4]; 6] = [
        [1, 4, 3, 2],
        [5, 6, 7, 8],
        [1, 2, 6, 5],
        [4, 8, 7, 3],
        [1, 5, 8, 4],
        [2, 3, 7, 6],
    ];
    for face in faces {
        text.push_str(&format!(
            "f {} {} {} {}
",
            face[0], face[1], face[2], face[3]
        ));
    }
    text
}

/// A lid box that leaves the region of the task (it is far away).
pub fn far_lid_obj() -> String {
    let mut text = String::from(
        "# a lid far away
",
    );
    let corners: [(f64, f64, f64); 8] = [
        (0.9, 0.9, 0.9),
        (1.0, 0.9, 0.9),
        (1.0, 1.0, 0.9),
        (0.9, 1.0, 0.9),
        (0.9, 0.9, 1.0),
        (1.0, 0.9, 1.0),
        (1.0, 1.0, 1.0),
        (0.9, 1.0, 1.0),
    ];
    for (x, y, z) in corners {
        text.push_str(&format!(
            "v {x} {y} {z}
"
        ));
    }
    let faces: [[usize; 4]; 6] = [
        [1, 4, 3, 2],
        [5, 6, 7, 8],
        [1, 2, 6, 5],
        [4, 8, 7, 3],
        [1, 5, 8, 4],
        [2, 3, 7, 6],
    ];
    for face in faces {
        text.push_str(&format!(
            "f {} {} {} {}
",
            face[0], face[1], face[2], face[3]
        ));
    }
    text
}

/// A minimal reconstruction report.
pub fn report_json(package_id: &str, revision: u32) -> String {
    json!({
        "schema": "michelangelo.reconstruction-report/0",
        "run": {
            "run_id": "recon_0001_0001",
            "run_dir": "recon/r0001/run-0001",
            "created_at": "2026-01-01T00:00:00Z"
        },
        "package": {
            "package_id": package_id,
            "revision": revision,
            "validation_status": "valid_complete",
            "calibrated_views": ["front"]
        }
    })
    .to_string()
}

/// Create a run directory with two components and a minimal report.
pub fn make_run(run_dir: &Path) {
    write_file(&run_dir.join("source/base.obj"), &box_obj(0.1));
    write_file(&run_dir.join("source/lid.obj"), &lid_obj(0.02));
    write_file(
        &run_dir.join("report.json"),
        &report_json("demo-package", 1),
    );
}

/// Merge an override into a value (objects are merged recursively).
pub fn merge(base: &mut Value, override_value: &Value) {
    match (base, override_value) {
        (Value::Object(base_map), Value::Object(override_map)) => {
            for (key, value) in override_map {
                match base_map.get_mut(key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, override_value) => *base = override_value.clone(),
    }
}

/// The default candidate of the tests.
pub fn candidate(overrides: &Value) -> Value {
    let mut value = json!({
        "schema": "michelangelo.edit-candidate/0",
        "candidate_id": "cand-0001",
        "base": {
            "package_id": "demo-package",
            "package_revision": 1,
            "scene_revision": "s0001",
            "run_id": "run-0001"
        },
        "task": {
            "task_id": "fix-lid",
            "allowed_components": ["lid"],
            "allowed_region": {"min": [-0.2, -0.2, -0.2], "max": [0.2, 0.2, 0.2]},
            "protected": ["base"],
            "allowed_intersections": [],
            "require_author_acceptance": true,
            "max_changed_vertices": 100000
        },
        "changes": [{"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}],
        "provenance": {"origin": "human", "detail": "a manual repair of the lid"},
        "budget": {"max_calls": 1, "max_seconds": 120, "max_cost_usd": 0.0}
    });
    merge(&mut value, overrides);
    value
}

/// Write a candidate directory (the document and its geometry).
pub fn write_candidate(dir: &Path, value: &Value, geometry: &[(&str, String)]) {
    write_file(&dir.join("candidate.json"), &value.to_string());
    for (name, content) in geometry {
        write_file(&dir.join("geometry").join(name), content);
    }
}

/// The path of a candidate document.
pub fn candidate_file(dir: &Path) -> PathBuf {
    dir.join("candidate.json")
}

/// The digest of a file.
pub fn digest(path: &Path) -> String {
    michelangelo_input::sha256::sha256_hex(&std::fs::read(path).expect("the file is read"))
}
