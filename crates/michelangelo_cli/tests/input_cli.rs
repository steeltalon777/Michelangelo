//! Integration tests for the `michelangelo input` subcommands.
//!
//! The tests run the built binary (`CARGO_BIN_EXE_michelangelo`) against the
//! committed fixtures in `examples/input_packages` and against temporary
//! copies of them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    // <repo>/crates/michelangelo_cli -> <repo>
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives inside the workspace")
        .to_path_buf()
}

fn fixture(name: &str) -> PathBuf {
    repo_root()
        .join("examples")
        .join("input_packages")
        .join(name)
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

/// Copy a directory tree (files only, the fixtures have no sub-directories
/// below the package root beyond images/ and masks/).
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

#[test]
fn validate_valid_full_exits_zero_with_a_complete_report() {
    let package = fixture("valid_full");
    let output = michelangelo(&["input", "validate", "--package", package.to_str().unwrap()]);

    assert_eq!(code_of(&output), 0, "stderr: {}", stderr_of(&output));

    let report: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("stdout is JSON");
    assert_eq!(report["status"], "valid_complete");
    assert_eq!(report["package_id"], "stage-a-fixture-full");
    assert_eq!(report["revision"], 1);
    assert_eq!(report["schema"], "michelangelo.input-package-report/0");

    for key in [
        "schema",
        "package_id",
        "revision",
        "status",
        "levels",
        "completeness",
        "diagnostics",
        "resources",
    ] {
        assert!(
            report.get(key).is_some(),
            "the report has the top-level key '{key}'"
        );
    }

    assert_eq!(report["levels"]["format_safety"]["ok"], true);
    assert_eq!(report["levels"]["sufficiency"]["ok"], true);
    assert_eq!(report["completeness"]["status"], "complete");
    assert_eq!(report["completeness"]["calibrated_views"], 2);
    assert_eq!(report["completeness"]["distinct_directions"], 2);
    assert_eq!(report["resources"].as_array().map(|r| r.len()), Some(4));
    for resource in report["resources"].as_array().expect("resources") {
        assert_eq!(resource["verified"], true);
    }
}

#[test]
fn validate_valid_partial_exits_zero_and_is_partial() {
    let package = fixture("valid_partial");
    let output = michelangelo(&["input", "validate", "--package", package.to_str().unwrap()]);

    assert_eq!(code_of(&output), 0, "stderr: {}", stderr_of(&output));

    let report: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("stdout is JSON");
    assert_eq!(report["status"], "valid_partial");
    let missing = report["completeness"]["missing"]
        .as_array()
        .expect("completeness.missing is an array");
    assert!(
        missing
            .iter()
            .any(|code| code == "INSUFFICIENT_VIEW_DIRECTIONS"),
        "the report lists INSUFFICIENT_VIEW_DIRECTIONS: {missing:?}"
    );
}

#[test]
fn validate_valid_partial_with_require_complete_exits_one() {
    let package = fixture("valid_partial");
    let output = michelangelo(&[
        "input",
        "validate",
        "--package",
        package.to_str().unwrap(),
        "--require-complete",
    ]);

    assert_eq!(code_of(&output), 1, "stderr: {}", stderr_of(&output));
}

#[test]
fn validate_missing_package_exits_two() {
    let package = repo_root()
        .join("examples")
        .join("input_packages")
        .join("does_not_exist");
    let output = michelangelo(&["input", "validate", "--package", package.to_str().unwrap()]);

    assert_eq!(code_of(&output), 2, "stderr: {}", stderr_of(&output));
    assert!(stdout_of(&output).is_empty(), "no report is printed");
    assert!(
        stderr_of(&output).contains("does not exist"),
        "the reason is on stderr: {}",
        stderr_of(&output)
    );
}

#[test]
fn validate_tampered_mask_exits_one_with_hash_mismatch() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let package = temp.path().join("r0001");
    copy_tree(&fixture("valid_full"), &package);

    let mask = package.join("masks").join("front.png");
    let mut bytes = fs::read(&mask).expect("the mask is readable");
    assert!(bytes.len() > 50, "the mask is long enough to patch");
    // Byte 45 sits inside the IDAT payload: the PNG container stays valid, only
    // the content hash changes.
    bytes[45] ^= 0xff;
    fs::write(&mask, &bytes).expect("the mask is written");

    let output = michelangelo(&["input", "validate", "--package", package.to_str().unwrap()]);

    assert_eq!(code_of(&output), 1, "stderr: {}", stderr_of(&output));

    let report: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("stdout is JSON");
    assert_eq!(report["status"], "invalid");
    let diagnostics = report["diagnostics"].as_array().expect("diagnostics");
    let mismatch = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "HASH_MISMATCH")
        .expect("the report contains a HASH_MISMATCH diagnostic");
    assert_eq!(mismatch["severity"], "error");
    assert_eq!(mismatch["level"], 1);
    assert_eq!(mismatch["address"], "views[0].mask.sha256");
}

#[test]
fn validate_report_option_writes_the_report_file() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let report_path = temp.path().join("report.json");
    let package = fixture("valid_full");

    let output = michelangelo(&[
        "input",
        "validate",
        "--package",
        package.to_str().unwrap(),
        "--report",
        report_path.to_str().unwrap(),
    ]);

    assert_eq!(code_of(&output), 0, "stderr: {}", stderr_of(&output));
    assert!(report_path.is_file(), "the report file is written");

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_path).expect("the report is readable"))
            .expect("the report file is JSON");
    let printed: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("stdout is JSON");
    assert_eq!(
        written, printed,
        "the file and stdout carry the same report"
    );
}

#[test]
fn validate_refuses_a_report_path_inside_the_package() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let package = temp.path().join("r0001");
    copy_tree(&fixture("valid_full"), &package);
    let report_path = package.join("report.json");

    let output = michelangelo(&[
        "input",
        "validate",
        "--package",
        package.to_str().unwrap(),
        "--report",
        report_path.to_str().unwrap(),
    ]);

    assert_eq!(code_of(&output), 2, "stderr: {}", stderr_of(&output));
    assert!(!report_path.exists(), "nothing is written into the package");
    assert!(
        stderr_of(&output).contains("inside the package"),
        "the reason is on stderr: {}",
        stderr_of(&output)
    );
}

#[test]
fn validate_quiet_keeps_stdout_empty() {
    let package = fixture("valid_full");
    let output = michelangelo(&[
        "input",
        "validate",
        "--package",
        package.to_str().unwrap(),
        "--quiet",
    ]);

    assert_eq!(code_of(&output), 0, "stderr: {}", stderr_of(&output));
    assert!(
        stdout_of(&output).is_empty(),
        "stdout stays empty: {}",
        stdout_of(&output)
    );
    let stderr = stderr_of(&output);
    assert_eq!(
        stderr.lines().filter(|line| !line.is_empty()).count(),
        1,
        "stderr holds exactly one summary line: {stderr}"
    );
}

#[test]
fn describe_prints_a_summary_and_exits_zero() {
    let package = fixture("valid_full");
    let output = michelangelo(&["input", "describe", "--package", package.to_str().unwrap()]);

    assert_eq!(code_of(&output), 0, "stderr: {}", stderr_of(&output));
    let stdout = stdout_of(&output);
    assert!(stdout.contains("package_id: stage-a-fixture-full"));
    assert!(stdout.contains("revision: 1"));
    assert!(stdout.contains("front: calibrated"));
    assert!(stdout.contains("right: calibrated"));
    assert!(stdout.contains("parts: 1"));
    assert!(stdout.contains("supplements: 0"));
    assert!(stdout.contains("unknowns: 1"));
    assert!(stdout.contains("completeness: complete"));
}

#[test]
fn describe_fails_for_a_broken_package() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let package = temp.path().join("r0001");
    copy_tree(&fixture("valid_full"), &package);

    let mask = package.join("masks").join("front.png");
    let mut bytes = fs::read(&mask).expect("the mask is readable");
    bytes[45] ^= 0xff;
    fs::write(&mask, &bytes).expect("the mask is written");

    let output = michelangelo(&["input", "describe", "--package", package.to_str().unwrap()]);

    assert_eq!(code_of(&output), 1, "stderr: {}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("HASH_MISMATCH"),
        "the diagnostic is on stderr: {}",
        stderr_of(&output)
    );
}
