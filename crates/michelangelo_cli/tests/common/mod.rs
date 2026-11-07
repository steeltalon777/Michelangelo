//! The fixtures of the CLI integration tests of the assistant surface.
//!
//! The fixture is a tiny, synthetic workspace: one run with two components
//! (a base and a lid), a minimal reconstruction report, a minimal input
//! package and one candidate. It carries no dataset content.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// The identifier of the package of the fixture.
pub const PACKAGE_ID: &str = "demo-lantern";
/// The identifier of the run of the fixture.
pub const RUN_ID: &str = "run-0001";

/// A box of 10 cm, centred at the origin.
pub const BASE_OBJ: &str = "o base
v -0.05 -0.05 -0.05
v 0.05 -0.05 -0.05
v 0.05 0.05 -0.05
v -0.05 0.05 -0.05
v -0.05 -0.05 0.05
v 0.05 -0.05 0.05
v 0.05 0.05 0.05
v -0.05 0.05 0.05
f 1 4 3 2
f 5 6 7 8
f 1 2 6 5
f 4 8 7 3
f 1 5 8 4
f 2 3 7 6
";

/// The lid of the box (8 x 4 x 8 cm).
pub const LID_OBJ: &str = "o lid
v -0.04 0.05 -0.04
v 0.04 0.05 -0.04
v 0.04 0.09 -0.04
v -0.04 0.09 -0.04
v -0.04 0.05 0.04
v 0.04 0.05 0.04
v 0.04 0.09 0.04
v -0.04 0.09 0.04
f 1 4 3 2
f 5 6 7 8
f 1 2 6 5
f 4 8 7 3
f 1 5 8 4
f 2 3 7 6
";

/// The minimal reconstruction report of the run.
pub const REPORT_JSON: &str = r#"{
  "schema": "michelangelo.reconstruction-report/0",
  "run": {
    "run_id": "recon_0001_0001",
    "run_dir": "recon/r0001/run-0001",
    "created_at": "2026-01-01T00:00:00Z"
  },
  "package": {
    "package_id": "demo-lantern",
    "revision": 1,
    "validation_status": "valid_complete",
    "calibrated_views": ["front"]
  },
  "solver": {
    "name": "voxel-carve",
    "version": 1,
    "resolution": 32,
    "voxel_size": 0.01,
    "grid": [10, 10, 10],
    "obj_up": "y"
  },
  "build": {
    "components": [
      {"name": "base", "obj": "base.obj", "voxels": 1000, "triangles": 12},
      {"name": "lid", "obj": "lid.obj", "voxels": 512, "triangles": 12}
    ],
    "supplements": [],
    "unknowns": [
      {"id": "u_lid_inside", "status": "unresolved", "detail": "the inside of the lid is unknown"}
    ],
    "measurements": []
  },
  "worker": {
    "job_id": "recon_0001_0001",
    "status": "completed",
    "artifacts": [],
    "blend": null,
    "glb": null
  },
  "quality": {
    "source": {"views": [], "mean_iou": 0.94, "min_iou": 0.91},
    "evaluation": null,
    "thresholds": {
      "source_mean_iou": 0.9,
      "source_min_iou": 0.85,
      "evaluation_mean_iou": 0.8,
      "evaluation_min_iou": 0.7,
      "bbox_relative_error_max": 0.05
    },
    "status": "pass"
  },
  "status": "pass",
  "unresolved": [
    {"id": "u_lid_inside", "reason": "the inside of the lid is unknown"}
  ],
  "diagnostics": []
}"#;

/// The minimal manifest of the package of the fixture.
pub const PACKAGE_JSON: &str = r#"{
  "schema": "michelangelo.input-package/0",
  "package_id": "demo-lantern",
  "object": {"name": "demo-lantern"},
  "revision": 1,
  "units": "m",
  "coordinate_system": {
    "handedness": "right",
    "up": "+Z",
    "right": "+X",
    "forward": "-Y"
  },
  "views": [
    {
      "view_id": "front",
      "kind": "calibrated",
      "image": {
        "path": "images/front.png",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000001",
        "width_px": 4,
        "height_px": 4
      },
      "mask": {
        "path": "masks/front.png",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000002",
        "format": "grayscale8",
        "foreground": "white"
      },
      "projection": {
        "type": "orthographic",
        "location_m": [0.0, -4.0, 0.0],
        "look_at_m": [0.0, 0.0, 0.0],
        "up_world": [0.0, 0.0, 1.0],
        "ortho_scale_m": 1.0
      }
    }
  ],
  "unknowns": [
    {
      "id": "u_lid_inside",
      "description": "the inside of the lid is unknown",
      "affected_parts": [],
      "policy": "author_required"
    }
  ],
  "provenance": {"origin": "human", "detail": "fixture"}
}"#;

/// A workspace with one reconstruction run.
pub struct Fixture {
    /// The temporary root of the fixture.
    pub root: TempDir,
    /// The workspace of the fixture.
    pub workspace: PathBuf,
    /// The run directory of the fixture.
    pub run_dir: PathBuf,
}

impl Fixture {
    /// Build the fixture.
    pub fn new() -> std::io::Result<Fixture> {
        let root = tempfile::tempdir()?;
        let workspace = root.path().join("workspace");
        let run_dir = workspace.join("recon").join("r0001").join(RUN_ID);
        std::fs::create_dir_all(run_dir.join("source"))?;
        std::fs::create_dir_all(run_dir.join("renders"))?;
        std::fs::create_dir_all(run_dir.join("package").join("masks"))?;
        std::fs::create_dir_all(run_dir.join("package").join("images"))?;
        std::fs::write(run_dir.join("source").join("base.obj"), BASE_OBJ)?;
        std::fs::write(run_dir.join("source").join("lid.obj"), LID_OBJ)?;
        std::fs::write(run_dir.join("report.json"), REPORT_JSON)?;
        std::fs::write(run_dir.join("quality_report.json"), REPORT_JSON)?;
        std::fs::write(run_dir.join("package").join("package.json"), PACKAGE_JSON)?;
        std::fs::write(
            run_dir.join("package").join("masks").join("front.png"),
            b"mask",
        )?;
        std::fs::write(
            run_dir.join("package").join("images").join("front.png"),
            b"image",
        )?;
        std::fs::write(run_dir.join("renders").join("front.png"), b"render")?;
        Ok(Fixture {
            root,
            workspace,
            run_dir,
        })
    }

    /// Write a candidate directory and return its path.
    pub fn write_candidate(
        &self,
        name: &str,
        candidate_id: &str,
        task_id: &str,
    ) -> std::io::Result<PathBuf> {
        let directory = self.workspace.join(name);
        std::fs::create_dir_all(directory.join("geometry"))?;
        let document = candidate_document(candidate_id, task_id);
        std::fs::write(
            directory.join("candidate.json"),
            serde_json::to_string_pretty(&document)
                .map_err(|error| std::io::Error::other(error.to_string()))?,
        )?;
        std::fs::write(directory.join("geometry").join("lid.obj"), LID_OBJ)?;
        Ok(directory)
    }

    /// Write the policy of the workspace.
    pub fn write_policy(&self, text: &str) -> std::io::Result<()> {
        let directory = self.workspace.join(".michelangelo");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join("assistant_policy.json"), text)?;
        Ok(())
    }
}

/// A candidate document of the fixture.
pub fn candidate_document(candidate_id: &str, task_id: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": "michelangelo.edit-candidate/0",
        "candidate_id": candidate_id,
        "base": {
            "package_id": PACKAGE_ID,
            "package_revision": 1,
            "scene_revision": "s0001",
            "run_id": RUN_ID
        },
        "task": {
            "task_id": task_id,
            "allowed_components": ["lid"],
            "allowed_region": {
                "min": [-0.04, 0.05, -0.04],
                "max": [0.04, 0.09, 0.04]
            },
            "protected": ["base"],
            "allowed_intersections": [],
            "require_author_acceptance": true,
            "max_changed_vertices": 100000
        },
        "changes": [
            {"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}
        ],
        "provenance": {"origin": "human", "detail": "a fixture candidate"},
        "budget": {"max_calls": 1, "max_seconds": 120, "max_cost_usd": 0.0}
    })
}

/// The binary of the CLI.
pub fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_michelangelo")
}

/// The path of a directory relative to the workspace.
pub fn relative(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .map(|rest| rest.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}
