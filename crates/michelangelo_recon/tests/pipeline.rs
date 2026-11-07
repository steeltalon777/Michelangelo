//! Integration tests of the reconstruction pipeline.
//!
//! The tests use the committed fixture package
//! (`examples/input_packages/valid_full`) and a deterministic fake Blender
//! executable, so no Blender installation and no dataset are required.

use std::path::{Path, PathBuf};

use michelangelo_recon::pipeline::{run, score, ReconOptions, ScoreOptions};
use michelangelo_recon::report::RunStatus;
use michelangelo_recon::testing::{clear_fake_blender_env, write_fake_blender};
use michelangelo_recon::ReconError;

/// The committed fixture package.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("input_packages")
        .join("valid_full")
}

/// The committed partial fixture package.
fn partial_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("input_packages")
        .join("valid_partial")
}

/// The Python interpreter used by the solver and by the fake Blender.
fn python() -> PathBuf {
    PathBuf::from("python")
}

struct Fake {
    _directory: tempfile::TempDir,
    executable: PathBuf,
    env: michelangelo_recon::testing::FakeBlenderEnv,
}

fn fake_blender(directory: &tempfile::TempDir, masks: &Path) -> Fake {
    let (executable, env) =
        write_fake_blender(directory.path(), &python(), Some(masks)).expect("the fake is written");
    Fake {
        _directory: tempfile::tempdir().expect("a temporary directory"),
        executable,
        env,
    }
}

#[test]
fn a_run_writes_the_reports_and_the_artifacts() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let fake_directory = tempfile::tempdir().expect("a temporary directory");
    let fake = fake_blender(&fake_directory, &fixture().join("masks"));

    let options = ReconOptions {
        resolution: Some(8),
        fake_blender: Some(fake.executable.clone()),
        ..ReconOptions::default()
    };
    let report = run(&fixture(), workspace.path(), &options).expect("the run completes");
    clear_fake_blender_env(&fake.env);

    let run_dir = workspace
        .path()
        .join("recon")
        .join("r0001")
        .join("run-0001");
    assert!(run_dir.join("report.json").is_file());
    assert!(run_dir.join("quality_report.json").is_file());
    assert!(run_dir.join("build_report.json").is_file());
    assert!(run_dir.join("blockout.blend").is_file());
    assert!(run_dir.join("blockout.glb").is_file());
    assert!(run_dir.join("source").join("blockout.obj").is_file());
    assert!(run_dir.join("renders").join("front.png").is_file());
    assert!(run_dir.join("renders").join("right.png").is_file());

    assert_eq!(report.run.run_dir, "recon/r0001/run-0001");
    assert_eq!(report.package.package_id, "stage-a-fixture-full");
    assert_eq!(report.package.revision, 1);
    assert_eq!(report.package.calibrated_views.len(), 2);
    assert_eq!(report.solver.name, "voxel-carve");
    assert_eq!(report.solver.resolution, 8);
    assert_eq!(report.solver.obj_up, "y");
    assert_eq!(report.worker.status, "completed");
    assert!(!report.build.components.is_empty());

    // The fake renders the package masks, so the silhouettes match exactly.
    assert!((report.quality.source.mean_iou - 1.0).abs() < 1e-9);
    // The fixture declares a height of 0.25 m while its mask spans 0.75 m,
    // so the measurement threshold fails and the run is a quality failure.
    assert_eq!(report.status, RunStatus::Fail);
    assert_eq!(report.quality.status, "fail");
    assert_eq!(report.status.exit_code(), 1);
}

#[test]
fn the_report_never_contains_an_absolute_path() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let fake_directory = tempfile::tempdir().expect("a temporary directory");
    let fake = fake_blender(&fake_directory, &fixture().join("masks"));
    let options = ReconOptions {
        resolution: Some(8),
        fake_blender: Some(fake.executable.clone()),
        ..ReconOptions::default()
    };
    let report = run(&fixture(), workspace.path(), &options).expect("the run completes");
    clear_fake_blender_env(&fake.env);

    let json = report.to_json().expect("the report is serializable");
    assert!(!json.contains(":\\"));
    assert!(!json.contains("Temp"));
    let stored = std::fs::read_to_string(
        workspace
            .path()
            .join("recon")
            .join("r0001")
            .join("run-0001")
            .join("report.json"),
    )
    .expect("the report is stored");
    assert!(!stored.contains(":\\"));
}

#[test]
fn a_second_run_uses_the_next_run_directory() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let fake_directory = tempfile::tempdir().expect("a temporary directory");
    let fake = fake_blender(&fake_directory, &fixture().join("masks"));
    let options = ReconOptions {
        resolution: Some(8),
        fake_blender: Some(fake.executable.clone()),
        ..ReconOptions::default()
    };
    let first = run(&fixture(), workspace.path(), &options).expect("the first run completes");
    let second = run(&fixture(), workspace.path(), &options).expect("the second run completes");
    clear_fake_blender_env(&fake.env);

    assert_eq!(first.run.run_dir, "recon/r0001/run-0001");
    assert_eq!(second.run.run_dir, "recon/r0001/run-0002");
    assert!(workspace
        .path()
        .join("recon")
        .join("r0001")
        .join("run-0002")
        .join("report.json")
        .is_file());
}

#[test]
fn an_invalid_package_is_rejected() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let package = tempfile::tempdir().expect("a temporary package");
    std::fs::write(package.path().join("package.json"), "{ not json")
        .expect("the broken manifest is written");
    let error = run(package.path(), workspace.path(), &ReconOptions::default())
        .expect_err("an invalid package must be rejected");
    assert!(matches!(error, ReconError::InvalidPackage { .. }));
    assert_eq!(error.exit_code(), 2);
    assert!(!workspace.path().join("recon").exists());
}

#[test]
fn a_partial_package_is_rejected_by_the_solver() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let error = run(
        &partial_fixture(),
        workspace.path(),
        &ReconOptions {
            resolution: Some(8),
            ..ReconOptions::default()
        },
    )
    .expect_err("a package without two calibrated views must be rejected");
    assert!(matches!(error, ReconError::SolverRejected { .. }));
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn a_score_pass_updates_the_report() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let fake_directory = tempfile::tempdir().expect("a temporary directory");
    let fake = fake_blender(&fake_directory, &fixture().join("masks"));
    let options = ReconOptions {
        resolution: Some(8),
        fake_blender: Some(fake.executable.clone()),
        ..ReconOptions::default()
    };
    let report = run(&fixture(), workspace.path(), &options).expect("the run completes");

    let run_dir = workspace
        .path()
        .join("recon")
        .join("r0001")
        .join("run-0001");
    let score_options = ScoreOptions {
        fake_blender: Some(fake.executable.clone()),
        ..ScoreOptions::default()
    };
    let scored = score(&run_dir, &fixture(), &score_options).expect("the scoring completes");
    clear_fake_blender_env(&fake.env);

    assert!(run_dir.join("evaluation_quality_report.json").is_file());
    assert!(run_dir.join("report.before_score.json").is_file());
    assert!(run_dir
        .join("evaluation_renders")
        .join("front.png")
        .is_file());
    assert!(scored.quality.evaluation.is_some());
    let evaluation = scored
        .quality
        .evaluation
        .expect("the evaluation section exists");
    assert!((evaluation.mean_iou - 1.0).abs() < 1e-9);
    assert_eq!(evaluation.views.len(), 2);

    let stored =
        std::fs::read_to_string(run_dir.join("report.json")).expect("the report is stored");
    assert!(stored.contains("evaluation"));
    assert_eq!(report.status, RunStatus::Fail);
}

#[test]
fn a_failing_solver_writes_an_error_report_with_the_output_tails() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let tools = tempfile::tempdir().expect("a temporary directory");
    let solver = tools.path().join("fake_solver.py");
    std::fs::write(
        &solver,
        "import sys
sys.stderr.write('the solver exploded')
sys.exit(3)
",
    )
    .expect("the fake solver is written");
    std::fs::write(
        tools.path().join("compare.py"),
        "# unused
",
    )
    .expect("the fake comparator is written");

    let error = run(
        &fixture(),
        workspace.path(),
        &ReconOptions {
            resolution: Some(8),
            solver: Some(solver),
            ..ReconOptions::default()
        },
    )
    .expect_err("the solver failure must be an error");
    assert!(matches!(error, ReconError::SolverFailed { .. }));

    let report_path = workspace
        .path()
        .join("recon")
        .join("r0001")
        .join("run-0001")
        .join("report.json");
    assert!(report_path.is_file());
    let stored = std::fs::read_to_string(&report_path).expect("the report is stored");
    let report: michelangelo_recon::report::ReconstructionReport =
        serde_json::from_str(&stored).expect("the report is valid");
    assert_eq!(report.status, RunStatus::Error);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SOLVER_STDERR"
            && diagnostic.message.contains("the solver exploded")));
}

#[test]
fn a_score_pass_without_a_report_is_rejected() {
    let workspace = tempfile::tempdir().expect("a temporary workspace");
    let error = score(
        &workspace.path().join("missing"),
        &fixture(),
        &ScoreOptions::default(),
    )
    .expect_err("a missing run must be rejected");
    assert!(matches!(error, ReconError::MissingPath { .. }));
}
