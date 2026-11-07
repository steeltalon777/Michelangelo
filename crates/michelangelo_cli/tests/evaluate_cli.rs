//! The integration tests of the `michelangelo evaluate` subcommand.
//!
//! The tests run the built binary against a scratch copy of the committed
//! fixture of `examples/edit`. The real Blender is not available in the CI:
//! the tests prove the usage contract and the refusal of an incomplete run,
//! not the render itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives inside the workspace")
        .to_path_buf()
}

fn fixtures() -> PathBuf {
    repo_root().join("examples").join("edit")
}

fn michelangelo(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_michelangelo"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("the michelangelo binary runs")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn code_of(output: &Output) -> i32 {
    output.status.code().expect("the process exits normally")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("the target directory is created");
    for entry in fs::read_dir(source).expect("the source directory is readable") {
        let entry = entry.expect("the directory entry is readable");
        let file_type = entry.file_type().expect("the file type is readable");
        let destination = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), &destination).expect("the file is copied");
        }
    }
}

/// A scratch copy of the fixture run directory.
fn scratch_run() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    copy_tree(&fixtures().join("run-0001"), &run_dir);
    (directory, run_dir)
}

#[test]
fn the_evaluation_needs_a_candidate_or_a_revision() {
    let (_directory, run_dir) = scratch_run();
    let output = michelangelo(&["evaluate", "--run", &run_dir.display().to_string()]);
    assert_eq!(code_of(&output), 2, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("--candidate"));
}

#[test]
fn the_evaluation_needs_exactly_one_target() {
    let (_directory, run_dir) = scratch_run();
    let output = michelangelo(&[
        "evaluate",
        "--run",
        &run_dir.display().to_string(),
        "--candidate",
        "cand-0001",
        "--revision",
        "s0001",
    ]);
    assert_eq!(code_of(&output), 2, "{}", stderr_of(&output));
}

#[test]
fn a_missing_run_directory_is_a_usage_error() {
    let output = michelangelo(&[
        "evaluate",
        "--run",
        "does-not-exist",
        "--candidate",
        "cand-0001",
    ]);
    assert_eq!(code_of(&output), 2, "{}", stderr_of(&output));
}

#[test]
fn a_run_without_a_package_copy_cannot_be_evaluated() {
    let (_directory, run_dir) = scratch_run();
    michelangelo(&["edit", "init", "--run", &run_dir.display().to_string()]);
    let candidate = fixtures().join("candidate-valid");
    let submit = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &run_dir.display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code_of(&submit), 0, "{}", stderr_of(&submit));

    // The fixture carries no copy of the input package: the masks of the
    // views cannot be pinned and the evaluation is insufficient.
    let output = michelangelo(&[
        "evaluate",
        "--run",
        &run_dir.display().to_string(),
        "--candidate",
        "cand-0001",
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("EVALUATION_INPUT_MISSING"));
}

#[test]
fn the_evaluation_help_is_available() {
    let output = michelangelo(&["evaluate", "--help"]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let text = stdout_of(&output);
    for flag in [
        "--run",
        "--candidate",
        "--revision",
        "--evaluation",
        "--json",
    ] {
        assert!(text.contains(flag), "the help misses '{flag}'");
    }
}

#[test]
fn an_acceptance_without_an_evaluation_is_refused() {
    let (_directory, run_dir) = scratch_run();
    michelangelo(&["edit", "init", "--run", &run_dir.display().to_string()]);
    let candidate = fixtures().join("candidate-valid");
    let submit = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &run_dir.display().to_string(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code_of(&submit), 0, "{}", stderr_of(&submit));

    let output = michelangelo(&[
        "edit",
        "accept",
        "--run",
        &run_dir.display().to_string(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("EVALUATION_MISSING"));
    assert!(!run_dir.join("scene/s0002").exists());
}
