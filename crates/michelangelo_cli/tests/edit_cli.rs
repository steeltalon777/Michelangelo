//! Integration tests for the `michelangelo edit` subcommands.
//!
//! The tests run the built binary (`CARGO_BIN_EXE_michelangelo`) against the
//! committed fixtures of `examples/edit`. The run directory is copied into a
//! temporary directory first: the tests never write into the repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

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

/// Register a synthetic evaluation for the candidate of a scratch run.
///
/// The acceptance of IT-8B needs a registered evaluation: the helper builds
/// one with the evidence crate (pinned to the snapshot, to the package copy
/// and to the base revision of the candidate) and registers it in the
/// registry of the Core.
fn register_evaluation(run_dir: &Path) -> String {
    let registry =
        michelangelo_evidence::registry::load_registry(run_dir).expect("the registry is read");
    let evaluation_id = registry.next_evaluation_id();
    let report = michelangelo_evidence::testing::synthetic_evaluation(
        run_dir,
        "cand-0001",
        &evaluation_id,
        "2026-01-03T04:05:06Z",
    )
    .expect("the synthetic evaluation is built");
    let entry = michelangelo_evidence::registry::write_and_register(run_dir, &report)
        .expect("the evaluation is registered");
    run_dir.join(entry.report_path).display().to_string()
}

/// A child helper that is always killed and reaped when the test ends, also on
/// panic: a leaked lock holder would keep the store busy for later tests.
struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
    // The bundle of the revision references its immutable unknown state.
    let unknowns = &value["verification"]["unknowns"];
    assert_eq!(unknowns["revision"], "s0001");
    assert_eq!(
        unknowns["path"],
        "verification/unknowns/revisions/s0001.json"
    );
    let referenced = scratch
        .run_dir
        .join(unknowns["path"].as_str().unwrap_or_default());
    assert!(referenced.is_file(), "the referenced document exists");
    assert_eq!(unknowns["sha256"], digest(&referenced));
}

#[test]
fn the_init_seeds_the_unknown_state() {
    let scratch = Scratch::new();
    let output = michelangelo(&["edit", "init", "--run", &scratch.path()]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    let path = scratch
        .run_dir
        .join("verification/unknowns/revisions/s0001.json");
    assert!(path.is_file(), "the unknown state is seeded");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("the state is readable"))
            .expect("the state is JSON");
    assert_eq!(value["schema"], "michelangelo.unknown-state/0");
    assert_eq!(value["revision"], "s0001");
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
        "--evaluation",
        &register_evaluation(&scratch.run_dir),
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
fn the_submission_registers_a_snapshot_of_the_full_result() {
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
    let snapshot = scratch.run_dir.join("verification/snapshots/cand-0001");
    assert!(snapshot.join("manifest.json").is_file());
    assert!(snapshot.join("geometry/base.obj").is_file());
    assert!(snapshot.join("geometry/lid.obj").is_file());
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(snapshot.join("manifest.json")).expect("readable"),
    )
    .expect("the manifest is JSON");
    assert_eq!(manifest["schema"], "michelangelo.geometry-snapshot/0");
    assert_eq!(manifest["snapshot_id"], "cand-0001");
    assert_eq!(manifest["base"]["scene_revision"], "s0001");
    assert_eq!(manifest["files"].as_array().expect("the files").len(), 2);
    // The unchanged component is copied byte-identically from the base.
    assert_eq!(
        digest(&snapshot.join("geometry/base.obj")),
        digest(&scratch.run_dir.join("scene/s0001/components/base.obj"))
    );
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
        "--evaluation",
        &register_evaluation(&scratch.run_dir),
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
        "--evaluation",
        &register_evaluation(&scratch.run_dir),
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
        "--evaluation",
        &register_evaluation(&scratch.run_dir),
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
    // The authoritative bundle is the one of the target revision.
    let scene: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(scratch.run_dir.join("scene/s0001/scene.json"))
            .expect("the scene is readable"),
    )
    .expect("the scene is JSON");
    assert_eq!(scene["verification"]["unknowns"]["revision"], "s0001");
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
    assert_eq!(value["snapshot_id"], "cand-0001");
    assert!(!value["aggregate_sha256"]
        .as_str()
        .unwrap_or_default()
        .is_empty());
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

// ---------------------------------------------------------------------------
// The acceptance integrity of IT-8A
// ---------------------------------------------------------------------------

/// Submit the valid fixture into a scratch run.
fn submitted(scratch: &Scratch) {
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    let candidate = fixtures().join("candidate-valid");
    let output = michelangelo(&[
        "edit",
        "submit",
        "--run",
        &scratch.path(),
        "--candidate",
        &candidate.display().to_string(),
    ]);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
}

fn accept(scratch: &Scratch) -> Output {
    michelangelo(&[
        "edit",
        "accept",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-0001",
        "--by",
        "alice",
        "--evaluation",
        &register_evaluation(&scratch.run_dir),
    ])
}

#[test]
fn a_repeated_acceptance_is_idempotent() {
    let scratch = Scratch::new();
    submitted(&scratch);
    let first = accept(&scratch);
    assert_eq!(code_of(&first), 0, "{}", stderr_of(&first));
    assert!(stdout_of(&first).contains("decision: accepted"));
    assert!(stdout_of(&first).contains("resulting_scene_revision: s0002"));

    let accepted = fs::read(
        scratch
            .run_dir
            .join("candidates/cand-0001/decisions/0002-accept.json"),
    )
    .expect("the accepted decision is readable");

    let second = accept(&scratch);
    assert_eq!(code_of(&second), 0, "{}", stderr_of(&second));
    assert!(stdout_of(&second).contains("decision: accepted"));
    assert!(stdout_of(&second).contains("resulting_scene_revision: s0002"));

    assert_eq!(
        accepted,
        fs::read(
            scratch
                .run_dir
                .join("candidates/cand-0001/decisions/0002-accept.json")
        )
        .expect("the accepted decision is readable"),
        "the accepted history is never rewritten"
    );
    assert!(scratch
        .run_dir
        .join("candidates/cand-0001/decisions/0003-accept-idempotent.json")
        .is_file());
    let index: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(scratch.run_dir.join("candidates/index.json"))
            .expect("the index is readable"),
    )
    .expect("the index is JSON");
    assert_eq!(index["candidates"][0]["status"], "accepted");
    assert_eq!(index["candidates"][0]["scene_revision"], "s0002");
}

#[test]
fn a_rejection_of_an_accepted_candidate_is_refused() {
    let scratch = Scratch::new();
    submitted(&scratch);
    let accepted = accept(&scratch);
    assert_eq!(code_of(&accepted), 0, "{}", stderr_of(&accepted));
    let history = fs::read(
        scratch
            .run_dir
            .join("candidates/cand-0001/decisions/0002-accept.json"),
    )
    .expect("the accepted decision is readable");

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
        "too late",
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("CANDIDATE_ALREADY_ACCEPTED"));
    assert_eq!(
        history,
        fs::read(
            scratch
                .run_dir
                .join("candidates/cand-0001/decisions/0002-accept.json")
        )
        .expect("the accepted decision is readable"),
        "the accepted history is never rewritten"
    );
    let index: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(scratch.run_dir.join("candidates/index.json"))
            .expect("the index is readable"),
    )
    .expect("the index is JSON");
    assert_eq!(index["candidates"][0]["status"], "accepted");
}

#[test]
fn a_repeated_rejection_is_idempotent() {
    let scratch = Scratch::new();
    submitted(&scratch);
    let reject = || {
        michelangelo(&[
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
        ])
    };
    let first = reject();
    assert_eq!(code_of(&first), 1, "{}", stderr_of(&first));
    let history = fs::read(
        scratch
            .run_dir
            .join("candidates/cand-0001/decisions/0002-reject.json"),
    )
    .expect("the rejected decision is readable");
    let second = reject();
    assert_eq!(code_of(&second), 1, "{}", stderr_of(&second));
    assert_eq!(
        history,
        fs::read(
            scratch
                .run_dir
                .join("candidates/cand-0001/decisions/0002-reject.json")
        )
        .expect("the rejected decision is readable"),
        "the rejected history is never rewritten"
    );
    assert!(scratch
        .run_dir
        .join("candidates/cand-0001/decisions/0003-reject-idempotent.json")
        .is_file());
}

#[test]
fn a_tampered_candidate_geometry_is_refused() {
    let scratch = Scratch::new();
    submitted(&scratch);
    let geometry = scratch
        .run_dir
        .join("candidates/cand-0001/geometry/lid.obj");
    fs::write(&geometry, "# a tampered lid\n").expect("the geometry is written");
    let output = accept(&scratch);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("EVIDENCE_MISMATCH"));
    assert!(!scratch.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_tampered_snapshot_is_refused() {
    let scratch = Scratch::new();
    submitted(&scratch);
    let geometry = scratch
        .run_dir
        .join("verification/snapshots/cand-0001/geometry/lid.obj");
    fs::write(&geometry, "# a tampered lid\n").expect("the geometry is written");
    let output = accept(&scratch);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("EVIDENCE_MISMATCH"));
    assert!(!scratch.run_dir.join("scene/s0002").exists());
}

#[test]
fn a_candidate_without_a_snapshot_cannot_be_accepted() {
    let scratch = Scratch::new();
    michelangelo(&["edit", "init", "--run", &scratch.path()]);
    // A legacy candidate: it is in the store, but it carries no snapshot.
    let legacy = scratch.run_dir.join("candidates/cand-legacy");
    fs::create_dir_all(legacy.join("geometry")).expect("the directory is created");
    let document = fs::read_to_string(fixtures().join("candidate-valid/candidate.json"))
        .expect("the fixture is readable");
    let document = document.replace("\"cand-0001\"", "\"cand-legacy\"");
    fs::write(legacy.join("candidate.json"), document).expect("the document is written");
    copy_tree(
        &fixtures().join("candidate-valid/geometry"),
        &legacy.join("geometry"),
    );
    let output = michelangelo(&[
        "edit",
        "accept",
        "--run",
        &scratch.path(),
        "--candidate",
        "cand-legacy",
        "--by",
        "alice",
    ]);
    assert_eq!(code_of(&output), 1, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("SNAPSHOT_MISSING"));
    assert!(!scratch.run_dir.join("scene/s0002").exists());
}

/// The helper of the two-process test of the lock.
///
/// The test does nothing unless the environment asks for it: the parent
/// process starts a copy of this binary that holds the lock of the store.
#[test]
fn the_lock_holder_waits_for_the_lock() {
    let run = match std::env::var("MICHELANGELO_LOCK_RUN") {
        Ok(run) => run,
        Err(_) => return,
    };
    let ready = std::env::var("MICHELANGELO_LOCK_READY").unwrap_or_default();
    let store = michelangelo_edit::scene::SceneStore::new(Path::new(&run));
    let _lock = michelangelo_edit::verification::StoreLock::acquire(&store, "holder")
        .expect("the lock is taken");
    fs::write(&ready, "locked").expect("the ready file is written");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_busy_store_is_refused_and_recovers_when_the_holder_dies() {
    let scratch = Scratch::new();
    submitted(&scratch);
    let ready_path = scratch.run_dir.join("verification/holder.ready");

    let child_log = scratch.run_dir.join("holder.log");
    let child_error = scratch.run_dir.join("holder.err");
    let child = ChildGuard(
        Command::new(std::env::current_exe().expect("the test binary"))
            .args([
                "--exact",
                "the_lock_holder_waits_for_the_lock",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("MICHELANGELO_LOCK_RUN", &scratch.run_dir)
            .env("MICHELANGELO_LOCK_READY", &ready_path)
            .stdout(fs::File::create(&child_log).expect("the log is created"))
            .stderr(fs::File::create(&child_error).expect("the log is created"))
            .spawn()
            .expect("the helper starts"),
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        ready_path.exists(),
        "the helper holds the lock: {}",
        fs::read_to_string(&child_error).unwrap_or_default()
    );

    let output = accept(&scratch);
    assert_eq!(code_of(&output), 2, "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("STORE_BUSY"));
    assert!(!scratch.run_dir.join("scene/s0002").exists());

    drop(child);
    std::thread::sleep(Duration::from_millis(200));

    let output = accept(&scratch);
    assert_eq!(code_of(&output), 0, "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("decision: accepted"));
}
