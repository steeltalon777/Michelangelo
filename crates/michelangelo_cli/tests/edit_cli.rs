//! Integration tests for the `michelangelo edit` subcommands.
//!
//! The tests run the built binary (`CARGO_BIN_EXE_michelangelo`) against the
//! committed fixtures of `examples/edit`. The run directory is copied into a
//! temporary directory first: the tests never write into the repository.

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
struct Scratch {
    _directory: tempfile::TempDir,
    run_dir: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let run_dir = directory.path().join("run-0001");
        copy_tree(&fixtures().join("run-0001"), &run_dir);
        Scratch {
            _directory: directory,
            run_dir,
        }
    }

    fn path(&self) -> String {
        self.run_dir.display().to_string()
    }
}

fn digest(path: &Path) -> String {
    let bytes = fs::read(path).expect("the file is read");
    michelangelo_input::sha256::sha256_hex(&bytes)
}

#[test]
fn the_init_creates_the_first_revision() {
    let scratch = Scratch::new();
    let output = michelangelo(&["edit", "init", "--run", &scratch.path()]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    assert!(scratch.run_dir.join("scene/s0001/scene.json").is_file());
    assert!(scratch
        .run_dir
        .join("scene/s0001/components/base.obj")
        .is_file());
    assert!(scratch
        .run_dir
        .join("scene/s0001/components/lid.obj")
        .is_file());
    assert!(scratch.run_dir.join("current.json").is_file());
    assert!(stdout_of(&output).contains("s0001"));
}

/// A run-level protected component (established at init) cannot be redefined by a
/// candidate: the candidate may not shrink the author's protection set.
#[test]
fn a_run_level_protected_component_cannot_be_changed() {
    let scratch = Scratch::new();
    let output = michelangelo(&[
        "edit",
        "init",
        "--run",
        &scratch.path(),
        "--protect",
        "lid",
        "--json",
    ]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let scene: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("the scene is JSON");
    let lid = scene["components"]
        .as_array()
        .expect("the components")
        .iter()
        .find(|component| component["name"] == "lid")
        .expect("the lid component is listed");
    assert_eq!(lid["protected"], true);

    let candidate = fixtures().join("candidate-valid").display().to_string();
    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate,
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("PROTECTED_MODIFIED"));
}

#[test]
fn the_init_is_idempotent() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let marker = scratch.run_dir.join("scene/s0001/components/base.obj");
    fs::write(&marker, b"# a marker\n").expect("the marker is written");
    let output = michelangelo(&["edit", "init", "--run", &scratch.path()]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(&marker).expect("the marker is readable"),
        "# a marker\n"
    );
}

#[test]
fn the_init_prints_the_scene_as_json() {
    let scratch = Scratch::new();
    let output = michelangelo(&["edit", "init", "--run", &scratch.path(), "--json"]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let value: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("the output is JSON");
    assert_eq!(value["schema"], "michelangelo.scene/0");
    assert_eq!(value["scene_revision"], "s0001");
    assert_eq!(
        value["components"]
            .as_array()
            .expect("the components")
            .len(),
        2
    );
}

#[test]
fn a_valid_candidate_is_submitted_and_accepted() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);

    let candidate = fixtures().join("candidate-valid");
    let submit = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code_of(&submit), 0, "{}", stderr_of(&submit));
    assert!(stdout_of(&submit).contains("decision: proposed"));

    let accept = michelangelo(&[
        "edit",
        "accept",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
    ]);
    assert_eq!(code_of(&accept), 0, "{}", stderr_of(&accept));
    assert!(stdout_of(&accept).contains("decision: accepted"));
    assert!(scratch.run_dir.join("scene/s0002/scene.json").is_file());

    let pointer: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(scratch.run_dir.join("current.json")).expect("the pointer is readable"),
    )
    .expect("the pointer is JSON");
    assert_eq!(pointer["scene_revision"], "s0002");
    assert_eq!(pointer["candidate_id"], "cand-0001");
}

#[test]
fn the_untouched_components_stay_byte_identical() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let before = digest(&scratch.run_dir.join("scene/s0001/components/base.obj"));
    let candidate = fixtures().join("candidate-valid");
    michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    michelangelo(&[
        "edit",
        "accept",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
    ]);
    assert_eq!(
        before,
        digest(&scratch.run_dir.join("scene/s0002/components/base.obj"))
    );
}

#[test]
fn a_stale_candidate_is_rejected() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-valid");
    michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    michelangelo(&[
        "edit",
        "accept",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
    ]);

    // A second candidate that was built on the revision s0001.
    let stale = scratch
        .run_dir
        .parent()
        .expect("the parent exists")
        .join("stale");
    copy_tree(&candidate, &stale);
    let document =
        fs::read_to_string(stale.join("candidate.json")).expect("the document is readable");
    let document = document.replace("\"cand-0001\"", "\"cand-0002\"");
    fs::write(stale.join("candidate.json"), document).expect("the document is written");

    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &stale.display().to_string(),
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("STALE_BASE"));
}

#[test]
fn a_protected_tamper_is_rejected() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-protected");
    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("PROTECTED_MODIFIED"));
    assert!(!scratch.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_region_violation_is_rejected() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-out-of-region");
    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("REGION_VIOLATION"));
    assert!(!scratch.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_rollback_points_back_to_the_previous_revision() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-valid");
    michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    michelangelo(&[
        "edit",
        "accept",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
    ]);
    let output = michelangelo(&[
        "edit",
        "rollback",
        "--run",
        &scratch.path(),
        "--to",
        "s0001",
        "--by",
        "alice",
    ]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let pointer: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(scratch.run_dir.join("current.json")).expect("the pointer is readable"),
    )
    .expect("the pointer is JSON");
    assert_eq!(pointer["scene_revision"], "s0001");
}

#[test]
fn the_show_prints_the_state_of_the_store() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-valid");
    michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    let output = michelangelo(&["edit", "show", "--run", &scratch.path(), "--json"]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let value: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("the output is JSON");
    assert_eq!(value["current"]["scene_revision"], "s0001");
    assert_eq!(
        value["candidates"]
            .as_array()
            .expect("the candidates")
            .len(),
        1
    );
    assert_eq!(value["candidates"][0]["status"], "proposed");
}

#[test]
fn a_candidate_can_be_rejected_by_an_author() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-valid");
    michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    let output = michelangelo(&[
        "edit",
        "reject",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-0001",
        "--by",
        "bob",
        "--reason",
        "the lid is wrong",
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("decision: rejected"));
    assert!(stdout_of(&output).contains("the lid is wrong"));
}

#[test]
fn a_missing_run_directory_is_a_usage_error() {
    let output = michelangelo(&["edit", "init", "--run", "does-not-exist"]);
    assert_eq!(code_of(&output), 2, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("michelangelo"));
}

#[test]
fn a_missing_candidate_is_a_usage_error() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        "does-not-exist",
    ]);
    assert_eq!(code_of(&output), 2, "{}", stderr_of(&output));
}

#[test]
fn the_decision_report_is_printed_as_json() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-valid");
    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
        "--json",
    ]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let value: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("the output is JSON");
    assert_eq!(value["schema"], "michelangelo.edit-decision/0");
    assert_eq!(value["decision"], "proposed");
    assert_eq!(value["candidate_id"], "cand-0001");
    assert!(value["checks"].as_array().expect("the checks").len() >= 10);
}

#[test]
fn the_edit_help_is_available() {
    let output = michelangelo(&["edit", "--help"]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let text = stdout_of(&output);
    for command in ["init", "submit", "accept", "reject", "rollback", "show"] {
        assert!(text.contains(command), "the help misses '{command}'");
    }
}
