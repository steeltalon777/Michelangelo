//! Tests of the scene revision store.

mod common;

use common::{box_obj, digest, make_run, write_file};
use michelangelo_edit::scene::{
    init_scene, init_scene_with_protection, list_revisions, load_current, load_index, load_scene,
    next_revision, publish_revision, revision_name, Scene, SceneComponent, SceneSource,
};
use michelangelo_edit::{Clock, FixedClock, SystemClock};
use std::path::PathBuf;

fn clock() -> FixedClock {
    FixedClock::new("2026-01-02T03:04:05Z")
}

fn scene_of(store_components: Vec<SceneComponent>, revision: &str) -> Scene {
    Scene {
        schema: michelangelo_edit::SCENE_SCHEMA.to_string(),
        run_id: "run-0001".to_string(),
        scene_revision: revision.to_string(),
        base_scene_revision: None,
        parent_candidate_id: None,
        created_at: "2026-01-02T03:04:05Z".to_string(),
        components: store_components,
        source: SceneSource {
            package_id: "demo-package".to_string(),
            package_revision: 1,
        },
        notes: Vec::new(),
    }
}

#[test]
fn the_first_revision_is_named_s0001() {
    assert_eq!(revision_name(1), "s0001");
    assert_eq!(revision_name(42), "s0042");
}

#[test]
fn the_initialization_creates_the_first_revision() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let scene = init_scene(&run_dir, &clock()).expect("the scene is created");
    assert_eq!(scene.scene_revision, "s0001");
    assert_eq!(scene.components.len(), 2);
    assert!(run_dir.join("scene/s0001/scene.json").is_file());
    assert!(run_dir.join("scene/s0001/components/base.obj").is_file());
    assert!(run_dir.join("scene/s0001/components/lid.obj").is_file());
}

#[test]
fn the_initialization_can_protect_components() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let scene =
        init_scene_with_protection(&run_dir, &["lid".to_string()], &clock()).expect("the scene");
    let lid = scene.component("lid").expect("the lid is listed");
    let base = scene.component("base").expect("the base is listed");
    assert!(lid.protected);
    assert!(!base.protected);
    let loaded = load_scene(
        &michelangelo_edit::scene::SceneStore::new(&run_dir),
        "s0001",
    )
    .expect("the scene is readable");
    assert!(loaded.component("lid").expect("lid").protected);
}

#[test]
fn the_initialization_rejects_an_unknown_protected_component() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let error = init_scene_with_protection(&run_dir, &["missing".to_string()], &clock())
        .expect_err("an unknown protected component is rejected");
    assert!(error.to_string().contains("missing"));
}

#[test]
fn the_initialization_records_the_digest_and_the_bounding_box() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let scene = init_scene(&run_dir, &clock()).expect("the scene is created");
    let base = scene.component("base").expect("the component is listed");
    assert_eq!(base.sha256, digest(&run_dir.join("source/base.obj")));
    assert_eq!(base.bbox_min, [-0.05, -0.05, -0.05]);
    assert_eq!(base.bbox_max, [0.05, 0.05, 0.05]);
    assert_eq!(base.obj, "components/base.obj");
    assert_eq!(base.part_id, None);
    assert!(!base.protected);
}

#[test]
fn the_components_are_sorted_by_name() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let scene = init_scene(&run_dir, &clock()).expect("the scene is created");
    let names = scene.component_names();
    assert_eq!(names, vec!["base".to_string(), "lid".to_string()]);
}

#[test]
fn the_initialization_is_idempotent() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let first = init_scene(&run_dir, &clock()).expect("the scene is created");
    let second = init_scene(&run_dir, &clock()).expect("the scene is created again");
    assert_eq!(first, second);
    assert_eq!(
        list_revisions(&michelangelo_edit::scene::SceneStore::new(&run_dir))
            .expect("the revisions are listed")
            .len(),
        1
    );
}

#[test]
fn the_initialization_never_overwrites_an_existing_revision() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    let marker = run_dir.join("scene/s0001/components/base.obj");
    write_file(&marker, "# a marker\n");
    init_scene(&run_dir, &clock()).expect("the scene is created again");
    assert_eq!(
        std::fs::read_to_string(&marker).expect("the marker is readable"),
        "# a marker\n"
    );
}

#[test]
fn the_initialization_creates_the_pointer_and_the_index() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    let store = michelangelo_edit::scene::SceneStore::new(&run_dir);
    let pointer = load_current(&store)
        .expect("the pointer is read")
        .expect("the pointer exists");
    assert_eq!(pointer.scene_revision, "s0001");
    assert_eq!(pointer.candidate_id, None);
    let index = load_index(&store).expect("the index is read");
    assert!(index.candidates.is_empty());
}

#[test]
fn the_initialization_needs_a_source_directory() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    std::fs::create_dir_all(&run_dir).expect("the run directory is created");
    let error = init_scene(&run_dir, &clock()).expect_err("the initialization fails");
    assert!(error.to_string().contains("source"));
}

#[test]
fn the_initialization_refuses_a_broken_component() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    write_file(&run_dir.join("source/broken.obj"), "v 0 0 0\nl 1 2\n");
    let error = init_scene(&run_dir, &clock()).expect_err("the initialization fails");
    assert!(error.to_string().contains("broken"));
}

#[test]
fn the_initialization_never_writes_into_the_source_directory() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let before = digest(&run_dir.join("source/base.obj"));
    init_scene(&run_dir, &clock()).expect("the scene is created");
    assert_eq!(before, digest(&run_dir.join("source/base.obj")));
}

#[test]
fn the_next_revision_follows_the_highest_one() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let store = michelangelo_edit::scene::SceneStore::new(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    assert_eq!(
        next_revision(&store).expect("the revision is allocated"),
        "s0002"
    );
}

#[test]
fn a_published_revision_is_never_overwritten() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let store = michelangelo_edit::scene::SceneStore::new(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    let scene = load_scene(&store, "s0001").expect("the scene is read");
    let error = publish_revision(&store, &scene, &[]).expect_err("the publication fails");
    assert!(error.to_string().contains("never overwritten"));
}

#[test]
fn a_published_revision_is_written_atomically() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let store = michelangelo_edit::scene::SceneStore::new(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    let base = load_scene(&store, "s0001").expect("the scene is read");
    let mut scene = scene_of(base.components.clone(), "s0002");
    scene.base_scene_revision = Some("s0001".to_string());
    let payloads = vec![("base".to_string(), box_obj(0.1).into_bytes())];
    publish_revision(&store, &scene, &payloads).expect("the revision is published");
    assert!(run_dir.join("scene/s0002/scene.json").is_file());
    assert!(!run_dir.join("scene/s0002.tmp").exists());
}

#[test]
fn a_failing_publication_leaves_no_temporary_directory() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    let store = michelangelo_edit::scene::SceneStore::new(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    let base = load_scene(&store, "s0001").expect("the scene is read");
    let scene = scene_of(base.components.clone(), "s0002");
    // The temporary directory already exists: the publication must fail.
    std::fs::create_dir_all(run_dir.join("scene/s0002.tmp")).expect("the directory is created");
    let error = publish_revision(&store, &scene, &[]).expect_err("the publication fails");
    assert!(error.to_string().contains("temporary"));
    assert!(run_dir.join("scene/s0002.tmp").is_dir());
    assert!(!run_dir.join("scene/s0002").exists());
}

#[test]
fn the_scene_document_carries_no_absolute_path() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    let text = std::fs::read_to_string(run_dir.join("scene/s0001/scene.json"))
        .expect("the scene is readable");
    assert!(!text.contains(&directory.path().display().to_string()));
    assert!(!text.contains("C:\\"));
}

#[test]
fn the_pointer_is_written_atomically() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir = directory.path().join("run-0001");
    make_run(&run_dir);
    init_scene(&run_dir, &clock()).expect("the scene is created");
    assert!(!run_dir.join("current.json.tmp").exists());
    assert!(!run_dir.join("candidates/index.json.tmp").exists());
}

#[test]
fn the_system_clock_produces_an_rfc3339_timestamp() {
    let stamp = SystemClock.now_rfc3339_utc();
    assert!(stamp.ends_with('Z'));
    assert_eq!(stamp.len(), 20);
}

#[test]
fn the_run_id_is_the_name_of_the_run_directory() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let run_dir: PathBuf = directory.path().join("run-0007");
    make_run(&run_dir);
    let scene = init_scene(&run_dir, &clock()).expect("the scene is created");
    assert_eq!(scene.run_id, "run-0007");
}
