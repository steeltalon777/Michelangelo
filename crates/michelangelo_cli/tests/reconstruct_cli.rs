//! Integration tests of the `michelangelo reconstruct` subcommands.
//!
//! The tests use the committed fixture package and a deterministic fake
//! Blender executable (the `BLENDER_BIN` environment variable points to it),
//! so no Blender installation is required.

use std::path::{Path, PathBuf};
use std::process::Command;

use michelangelo_recon::testing::{clear_fake_blender_env, write_fake_blender};

/// The committed fixture package.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("input_packages")
        .join("valid_full")
}

fn michelangelo() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_michelangelo"));
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

#[test]
fn reconstruct_without_a_subcommand_is_a_usage_error() {
    let output = michelangelo()
        .arg("reconstruct")
        .output()
        .expect("the CLI runs");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn reconstruct_run_without_a_package_is_a_usage_error() {
    let output = michelangelo()
        .args(["reconstruct", "run", "--workspace", "."])
        .output()
        .expect("the CLI runs");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn show_without_a_report_is_an_error() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let output = michelangelo()
        .args(["reconstruct", "show", "--run"])
        .arg(directory.path())
        .output()
        .expect("the CLI runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(!output.stderr.is_empty());
}

#[test]
fn show_prints_the_stored_report() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let report = directory.path().join("report.json");
    std::fs::write(
        &report,
        r#"{
  "schema": "michelangelo.reconstruction-report/0",
  "run": {"run_id": "recon_0001_0001", "run_dir": "recon/r0001/run-0001", "created_at": "2026-01-01T00:00:00Z"},
  "package": {"package_id": "demo", "revision": 1, "validation_status": "valid_complete", "calibrated_views": ["front"]},
  "solver": {"name": "voxel-carve", "version": 1, "resolution": 8, "voxel_size": 0.1, "grid": [8, 8, 8], "obj_up": "y"},
  "build": {"components": [], "supplements": [], "unknowns": [], "measurements": []},
  "worker": {"job_id": "recon_0001_0001", "status": "completed", "artifacts": [], "blend": null, "glb": null},
  "quality": {"source": {"views": [], "mean_iou": 0.0, "min_iou": 0.0}, "evaluation": null, "thresholds": {"source_mean_iou": 0.9, "source_min_iou": 0.85, "evaluation_mean_iou": 0.8, "evaluation_min_iou": 0.7, "bbox_relative_error_max": 0.05}, "status": "pass"},
  "status": "pass",
  "unresolved": [],
  "diagnostics": []
}
"#,
    )
    .expect("the report is written");

    let output = michelangelo()
        .args(["reconstruct", "show", "--run"])
        .arg(directory.path())
        .arg("--json")
        .output()
        .expect("the CLI runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("michelangelo.reconstruction-report/0"));
    assert!(stdout.contains("recon/r0001/run-0001"));
}

#[test]
fn a_run_with_the_fake_blender_writes_the_report() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let fake_directory = tempfile::tempdir().expect("a temporary directory");
    let (executable, env) = write_fake_blender(
        fake_directory.path(),
        Path::new("python"),
        Some(&fixture().join("masks")),
    )
    .expect("the fake is written");

    let output = michelangelo()
        .args(["reconstruct", "run", "--package"])
        .arg(fixture())
        .arg("--workspace")
        .arg(workspace.path())
        .args(["--resolution", "8", "--json"])
        .env("BLENDER_BIN", &executable)
        .output()
        .expect("the CLI runs");
    clear_fake_blender_env(&env);

    // The fixture declares a height of 0.25 m while its mask spans 0.75 m:
    // the measurement threshold fails, so the documented exit code is 1.
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("michelangelo.reconstruction-report/0"));
    assert!(stdout.contains("\"status\": \"fail\""));

    let run_dir = workspace
        .path()
        .join("recon")
        .join("r0001")
        .join("run-0001");
    assert!(run_dir.join("report.json").is_file());
    assert!(run_dir.join("quality_report.json").is_file());
    assert!(run_dir.join("renders").join("front.png").is_file());
}

#[test]
fn a_run_with_an_invalid_package_is_a_setup_error() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let package = tempfile::tempdir().expect("a temporary package");
    std::fs::write(package.path().join("package.json"), "{ not json")
        .expect("the broken manifest is written");
    let output = michelangelo()
        .args(["reconstruct", "run", "--package"])
        .arg(package.path())
        .arg("--workspace")
        .arg(workspace.path())
        .output()
        .expect("the CLI runs");
    assert_eq!(output.status.code(), Some(2));
}
