//! Integration tests: happy paths, negative paths, idempotency and the
//! INPUT_CHANGED contract. Each test uses a disposable temp workspace.

mod common;

use std::fs;
use std::path::Path;

use common::*;
use michelangelo_asset_intake::error::IntakeError;
use michelangelo_asset_intake::export::{convert, export_passthrough};
use michelangelo_asset_intake::license::LicenseStatus;
use michelangelo_asset_intake::license::ProvenanceInfo;
use michelangelo_asset_intake::limits::IntakeLimits;
use michelangelo_asset_intake::report::{DiagnosticCode, ReportStatus};
use michelangelo_asset_intake::snapshot::{
    execute_plan, plan_intake, DependencyKind, IntakeRequest, SnapshotManifest, SNAPSHOT_SCHEMA,
};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "michelangelo-intake-test-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn plan_for(source: &Path) -> michelangelo_asset_intake::IntakePlan {
    plan_intake(
        &IntakeRequest::with_unknown_provenance(source),
        &IntakeLimits::default(),
    )
    .expect("plan should succeed")
}

fn rejection(code: DiagnosticCode, err: &IntakeError) -> bool {
    match err {
        IntakeError::PlanRejected { rejection } => rejection
            .diagnostics
            .iter()
            .any(|d| d.code == code && d.is_error()),
        _ => false,
    }
}

/// Happy plans carry exactly one warning: the unknown-license flag (the
/// fixture uses unknown provenance on purpose). Any other diagnostic is a
/// failure.
fn assert_only_license_warning(plan: &michelangelo_asset_intake::IntakePlan) {
    assert!(
        plan.diagnostics
            .iter()
            .all(|d| d.code == DiagnosticCode::UnknownLicense && !d.is_error()),
        "unexpected diagnostics: {:?}",
        plan.diagnostics
    );
    assert_eq!(plan.status, ReportStatus::WithWarnings);
}

// ---------------------------------------------------------------------------
// Happy paths / smoke
// ---------------------------------------------------------------------------

#[test]
fn smoke_glb_full_pipeline() {
    let dir = temp_dir("glb-smoke");
    let glb = minimal_glb_with_image();
    let source = dir.join("asset.glb");
    fs::write(&source, &glb).unwrap();

    // plan: two dependencies — the BIN chunk and the embedded image view.
    let plan = plan_for(&source);
    assert_eq!(plan.format, michelangelo_asset_intake::SourceFormat::Glb);
    assert_only_license_warning(&plan);
    assert_eq!(plan.dependencies.len(), 2);
    assert!(matches!(
        plan.dependencies[0].kind,
        DependencyKind::GlbBinChunk { .. }
    ));
    assert!(matches!(
        plan.dependencies[1].kind,
        DependencyKind::GlbBufferView { .. }
    ));
    let png = png_bytes();
    let view_dep = &plan.dependencies[1];
    assert_eq!(view_dep.size_bytes, png.len() as u64);
    assert_eq!(
        view_dep.sha256,
        michelangelo_asset_intake::hash::sha256_hex(&png)
    );

    // execute: immutable snapshot in the disposable workspace.
    let workspace = dir.join("workspace");
    let snapshot = execute_plan(&plan, &workspace).expect("execute");
    assert!(!snapshot.reused);
    assert!(snapshot.manifest_path.is_file());
    assert_eq!(snapshot.manifest.schema, SNAPSHOT_SCHEMA);
    assert!(
        snapshot.manifest.provisional,
        "manifest must be marked provisional"
    );

    // Original bytes copied unmodified.
    let copy = snapshot.source_copy();
    assert!(copy.is_file());
    assert_eq!(
        michelangelo_asset_intake::hash::sha256_hex(&fs::read(&copy).unwrap()),
        plan.source.sha256
    );

    // Dependency copies present with pinned hashes.
    for dep in &snapshot.manifest.dependencies {
        if let Some(name) = &dep.stored_name {
            let copy = snapshot.dependency_copy(name);
            assert!(copy.is_file(), "dependency copy {name} missing");
            assert_eq!(
                michelangelo_asset_intake::hash::sha256_hex(&fs::read(&copy).unwrap()),
                dep.sha256
            );
        }
    }

    // Manifest parses back from disk.
    let raw = fs::read(&snapshot.manifest_path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&raw).expect("manifest is valid JSON");
    let parsed: SnapshotManifest = serde_json::from_value(value).expect("manifest round-trip");
    assert_eq!(parsed.snapshot_id, snapshot.manifest.snapshot_id);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn passthrough_export_is_byte_identical_and_collision_safe() {
    let dir = temp_dir("glb-export");
    let source = dir.join("asset.glb");
    fs::write(&source, minimal_glb_with_image()).unwrap();
    let plan = plan_for(&source);
    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();

    let out = dir.join("out");
    let export = export_passthrough(&snapshot, &out).expect("export");
    assert!(export.report.byte_identical);
    assert!(
        export.report.loss_report.conversions.is_empty(),
        "no conversion, no loss entries"
    );
    // original + BIN chunk + embedded image view
    assert_eq!(export.report.files.len(), 3);
    let export_manifest = out
        .join("passthrough")
        .join(&snapshot.manifest.snapshot_id)
        .join("export.json");
    assert!(export_manifest.is_file());

    // No-clobber: a second export to the same root must refuse.
    let second = export_passthrough(&snapshot, &out);
    assert!(matches!(second, Err(IntakeError::OutputExists { .. })));

    // Conversion is explicitly not implemented; the source stays untouched.
    assert!(matches!(
        convert(&snapshot, "obj"),
        Err(IntakeError::ConversionNotImplemented)
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn happy_gltf_with_external_dependencies() {
    let dir = temp_dir("gltf-happy");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    let plan = plan_for(&fixture.source_path);
    assert_only_license_warning(&plan);
    assert_eq!(plan.dependencies.len(), 2);
    let buffer_dep = &plan.dependencies[0];
    assert_eq!(buffer_dep.role, "buffer");
    assert!(matches!(buffer_dep.kind, DependencyKind::ExternalFile));
    assert_eq!(buffer_dep.stored_name.as_deref(), Some("0000__scene.bin"));
    let tex_dep = &plan.dependencies[1];
    assert_eq!(tex_dep.role, "image");
    assert_eq!(
        tex_dep.stored_name.as_deref(),
        Some("0001__textures__albedo.png")
    );

    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();
    assert!(snapshot.dependency_copy("0000__scene.bin").is_file());
    assert!(snapshot
        .dependency_copy("0001__textures__albedo.png")
        .is_file());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn happy_gltf_with_data_uri_buffer() {
    let dir = temp_dir("gltf-datauri");
    let fixture = write_gltf_fixture(&dir, &data_uri_buffer(), None, false);
    let plan = plan_for(&fixture.source_path);
    assert_eq!(plan.dependencies.len(), 1);
    let dep = &plan.dependencies[0];
    assert!(matches!(dep.kind, DependencyKind::EmbeddedDataUri));
    assert_eq!(dep.size_bytes, triangle_bin().len() as u64);
    assert!(dep.source_uri.is_some());

    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();
    let stored = dep.stored_name.as_deref().unwrap();
    let copy = snapshot.dependency_copy(stored);
    let bytes = fs::read(&copy).unwrap();
    assert_eq!(
        bytes,
        triangle_bin(),
        "decoded data URI payload must equal the original bin"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn happy_glb_with_data_uri_buffer_instead_of_bin() {
    let dir = temp_dir("glb-datauri");
    let source = dir.join("asset.glb");
    fs::write(&source, minimal_glb_with_data_uri_buffer()).unwrap();
    let plan = plan_for(&source);
    assert_eq!(plan.dependencies.len(), 1);
    assert!(matches!(
        plan.dependencies[0].kind,
        DependencyKind::EmbeddedDataUri
    ));
    execute_plan(&plan, &dir.join("ws")).expect("execute");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn happy_obj_with_mtl_and_texture() {
    let dir = temp_dir("obj-happy");
    let fixture = write_obj_fixture(&dir);
    let plan = plan_for(&fixture.source_path);
    // OBJ geometry is not parsed: an info note is expected, nothing more.
    assert_eq!(plan.status, ReportStatus::WithWarnings);
    assert!(plan.diagnostics.iter().all(|d| !d.is_error()));
    let roles: Vec<&str> = plan.dependencies.iter().map(|d| d.role.as_str()).collect();
    assert_eq!(roles, vec!["material_library", "texture_map"]);
    assert_eq!(plan.dependencies[0].declared_ref, "materials.mtl");
    assert_eq!(plan.dependencies[1].declared_ref, "textures/albedo.png");

    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();
    assert_eq!(snapshot.manifest.dependencies.len(), 2);
    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Identity, idempotency, collision safety
// ---------------------------------------------------------------------------

#[test]
fn identity_is_content_derived_and_portable() {
    let dir_a = temp_dir("identity-a");
    let dir_b = temp_dir("identity-b");
    let fixture_a = write_gltf_fixture(&dir_a, "scene.bin", Some("textures/albedo.png"), true);
    let fixture_b = write_gltf_fixture(&dir_b, "scene.bin", Some("textures/albedo.png"), true);
    let plan_a = plan_for(&fixture_a.source_path);
    let plan_b = plan_for(&fixture_b.source_path);
    assert_eq!(
        plan_a.identity, plan_b.identity,
        "same content, same identity (path-independent)"
    );

    // Different content => different identity, no overwrite of the old dir.
    let png = png_bytes();
    fs::write(
        dir_b.join("textures").join("albedo.png"),
        &png[..png.len() - 1],
    )
    .unwrap();
    let plan_c = plan_for(&fixture_b.source_path);
    assert_ne!(plan_a.identity, plan_c.identity);
    let ws = dir_a.join("ws");
    let snap_a = execute_plan(&plan_a, &ws).unwrap();
    let snap_c = execute_plan(&plan_c, &ws).unwrap();
    assert_ne!(snap_a.dir, snap_c.dir);
    assert!(
        snap_a.source_copy().is_file(),
        "previous snapshot untouched"
    );
    let _ = fs::remove_dir_all(dir_a);
    let _ = fs::remove_dir_all(dir_b);
}

#[test]
fn execute_is_idempotent_by_identity() {
    let dir = temp_dir("idempotent");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    let plan = plan_for(&fixture.source_path);
    let ws = dir.join("ws");
    let first = execute_plan(&plan, &ws).unwrap();
    assert!(!first.reused);
    let second = execute_plan(&plan, &ws).unwrap();
    assert!(
        second.reused,
        "second execution must reuse the verified snapshot"
    );
    assert_eq!(first.dir, second.dir);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn tampered_existing_snapshot_is_a_conflict_not_a_success() {
    let dir = temp_dir("tamper-dir");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    let plan = plan_for(&fixture.source_path);
    let ws = dir.join("ws");
    let snapshot = execute_plan(&plan, &ws).unwrap();

    // Corrupt the manifest.
    fs::write(&snapshot.manifest_path, b"{ broken").unwrap();
    match execute_plan(&plan, &ws) {
        Err(IntakeError::SnapshotDirConflict { .. }) => {}
        other => panic!("expected SnapshotDirConflict, got {other:?}"),
    }
    let _ = fs::remove_dir_all(dir);

    // Corrupt a dependency copy with an intact manifest.
    let dir2 = temp_dir("tamper-copy");
    let fixture2 = write_gltf_fixture(&dir2, "scene.bin", Some("textures/albedo.png"), true);
    let plan2 = plan_for(&fixture2.source_path);
    let ws2 = dir2.join("ws");
    let snap2 = execute_plan(&plan2, &ws2).unwrap();
    let dep_name = snap2.manifest.dependencies[0].stored_name.clone().unwrap();
    let dep_path = snap2.dependency_copy(&dep_name);
    let bytes = fs::read(&dep_path).unwrap();
    let mut corrupted = bytes.clone();
    corrupted[0] ^= 0xFF;
    fs::write(&dep_path, &corrupted).unwrap();
    match execute_plan(&plan2, &ws2) {
        Err(IntakeError::SnapshotCorrupt { .. }) => {}
        other => panic!("expected SnapshotCorrupt, got {other:?}"),
    }
    // Restore: the reuse path verifies integrity and must succeed again.
    fs::write(&dep_path, &bytes).unwrap();
    let again = execute_plan(&plan2, &ws2).unwrap();
    assert!(again.reused);
    let _ = fs::remove_dir_all(dir2);
}

#[test]
fn dependency_substitution_after_planning_is_input_changed() {
    let dir = temp_dir("input-changed-dep");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    let plan = plan_for(&fixture.source_path);

    // Substitute the dependency AFTER planning.
    fs::write(dir.join("scene.bin"), vec![0u8; 42]).unwrap();
    match execute_plan(&plan, &dir.join("ws")) {
        Err(IntakeError::InputChanged {
            path,
            expected_sha256,
            actual_sha256,
        }) => {
            assert!(path.ends_with("scene.bin"));
            assert_eq!(expected_sha256, plan.dependencies[0].sha256);
            assert_ne!(actual_sha256, expected_sha256);
        }
        other => panic!("expected InputChanged, got {other:?}"),
    }

    // Substituting the SOURCE after planning is also INPUT_CHANGED.
    let dir2 = temp_dir("input-changed-src");
    let fixture2 = write_gltf_fixture(&dir2, "scene.bin", Some("textures/albedo.png"), true);
    let plan2 = plan_for(&fixture2.source_path);
    let mut gltf = fs::read(&fixture2.source_path).unwrap();
    gltf.insert(0, b' ');
    fs::write(&fixture2.source_path, &gltf).unwrap();
    match execute_plan(&plan2, &dir2.join("ws")) {
        Err(IntakeError::InputChanged { .. }) => {}
        other => panic!("expected InputChanged for the source, got {other:?}"),
    }
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
}

// ---------------------------------------------------------------------------
// Negative: unsafe / unsupported references
// ---------------------------------------------------------------------------

#[test]
fn negative_missing_texture() {
    let dir = temp_dir("neg-missing");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), false);
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &IntakeLimits::default(),
    )
    .expect_err("missing texture must reject the plan");
    assert!(rejection(DiagnosticCode::MissingDependency, &err), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_path_traversal() {
    let dir = temp_dir("neg-traversal");
    let fixture = write_gltf_fixture(&dir, "../escape.bin", None, false);
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &IntakeLimits::default(),
    )
    .expect_err("traversal must reject the plan");
    assert!(rejection(DiagnosticCode::PathTraversal, &err), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_absolute_references() {
    for (name, uri) in [
        ("abs-posix", "/etc/passwd"),
        ("abs-win", "C:/Windows/abs.png"),
    ] {
        let dir = temp_dir(&format!("neg-abs-{name}"));
        let fixture = write_gltf_fixture(&dir, uri, None, false);
        let err = plan_intake(
            &IntakeRequest::with_unknown_provenance(&fixture.source_path),
            &IntakeLimits::default(),
        )
        .expect_err("absolute reference must reject the plan");
        assert!(
            rejection(DiagnosticCode::AbsoluteReference, &err),
            "{name}: {err}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn negative_network_references() {
    for (name, uri) in [
        ("https", "https://example.com/tex.png"),
        ("file", "file:///etc/passwd"),
    ] {
        let dir = temp_dir(&format!("neg-net-{name}"));
        let fixture = write_gltf_fixture(&dir, uri, None, false);
        let err = plan_intake(
            &IntakeRequest::with_unknown_provenance(&fixture.source_path),
            &IntakeLimits::default(),
        )
        .expect_err("network reference must reject the plan");
        assert!(
            rejection(DiagnosticCode::NetworkReference, &err),
            "{name}: {err}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn negative_percent_encoded_reference() {
    let dir = temp_dir("neg-percent");
    let fixture = write_gltf_fixture(&dir, "scene%20.bin", None, false);
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &IntakeLimits::default(),
    )
    .expect_err("percent-encoded reference must reject the plan (conservative)");
    assert!(
        rejection(DiagnosticCode::PercentEncodedReference, &err),
        "{err}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn negative_symlink_escape() {
    let dir = temp_dir("neg-symlink");
    // Secret outside the source directory.
    let outside = dir.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.bin"), b"outside").unwrap();
    // Symlink inside the source directory pointing out.
    let source_dir = dir.join("assets");
    fs::create_dir_all(&source_dir).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.join("secret.bin"), source_dir.join("leak.bin")).unwrap();
    let fixture = write_gltf_fixture(&source_dir, "leak.bin", None, false);
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &IntakeLimits::default(),
    )
    .expect_err("symlink escape must reject the plan");
    assert!(rejection(DiagnosticCode::SymlinkEscape, &err), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[cfg(not(unix))]
#[test]
fn negative_symlink_escape() {
    // SKIPPED on this platform: creating junctions requires privileges that
    // the test sandbox may not have. The path-safe logic itself is covered
    // by the resolve_reference unit tests (absolute/traversal) and the
    // canonical containment check; a Windows junction reproduction is
    // listed as SKIPPED in TEST_EVIDENCE.md, NOT as a pass.
    let dir = temp_dir("neg-symlink-skipped");
    let fixture = write_gltf_fixture(&dir, "scene.bin", None, true);
    let plan = plan_for(&fixture.source_path);
    assert_only_license_warning(&plan);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_truncated_glb() {
    let dir = temp_dir("neg-truncated");
    let glb = minimal_glb_with_image();
    let source = dir.join("asset.glb");
    fs::write(&source, &glb[..glb.len() - 10]).unwrap();
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&source),
        &IntakeLimits::default(),
    )
    .expect_err("truncated GLB must reject the plan");
    assert!(rejection(DiagnosticCode::TruncatedGlb, &err), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_glb_declared_length_mismatch() {
    let dir = temp_dir("neg-glb-len");
    let glb = minimal_glb_plain();
    let patched = patch_glb_total_length(&glb, glb.len() as u32 + 1);
    let source = dir.join("asset.glb");
    fs::write(&source, patched).unwrap();
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&source),
        &IntakeLimits::default(),
    )
    .expect_err("declared length must match file size");
    assert!(rejection(DiagnosticCode::TruncatedGlb, &err), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_dependency_count_limit() {
    let dir = temp_dir("neg-limit-count");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    let limits = IntakeLimits {
        max_dependency_count: 1,
        ..IntakeLimits::default()
    };
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &limits,
    )
    .expect_err("dependency count above the limit must reject the plan");
    assert!(
        rejection(DiagnosticCode::TooManyDependencies, &err),
        "{err}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_dependency_size_limit() {
    let dir = temp_dir("neg-limit-size");
    let fixture = write_gltf_fixture(&dir, "scene.bin", None, true);
    let limits = IntakeLimits {
        max_dependency_bytes: 8,
        ..IntakeLimits::default()
    };
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &limits,
    )
    .expect_err("oversized dependency must reject the plan");
    assert!(rejection(DiagnosticCode::DependencyTooLarge, &err), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_unsupported_blend_is_explicit() {
    let dir = temp_dir("neg-blend");
    let source = dir.join("model.blend");
    fs::write(&source, b"BLENDER-v402-exotic-bytes").unwrap();
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&source),
        &IntakeLimits::default(),
    )
    .expect_err(".blend must be rejected explicitly");
    match &err {
        IntakeError::UnsupportedFormat { reason, .. } => {
            assert!(
                reason.contains(".blend"),
                "reason must name .blend: {reason}"
            );
        }
        other => panic!("expected UnsupportedFormat, got {other:?}"),
    }
    // No snapshot directory was created.
    assert!(!dir.join("ws").join("snapshots").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_remote_source_uri_is_never_fetched() {
    let dir = temp_dir("neg-remote");
    let source = dir.join("https://example.invalid/model.glb");
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&source),
        &IntakeLimits::default(),
    )
    .expect_err("a remote URI is not a local file and must fail as io, not be fetched");
    assert!(matches!(err, IntakeError::Io { .. }), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_glb_magic_mismatch() {
    let dir = temp_dir("neg-glb-magic");
    let source = dir.join("asset.glb");
    fs::write(&source, b"NOTGLB-not-a-container-at-all........").unwrap();
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&source),
        &IntakeLimits::default(),
    )
    .expect_err("a .glb without magic must be rejected");
    match &err {
        IntakeError::UnsupportedFormat { reason, .. } => {
            assert!(
                reason.contains("magic"),
                "reason must mention the magic bytes: {reason}"
            );
        }
        other => panic!("expected UnsupportedFormat, got {other:?}"),
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn negative_required_extension_unsupported() {
    let dir = temp_dir("neg-ext");
    let fixture = write_gltf_fixture(&dir, "scene.bin", None, true);
    let mut json = fs::read_to_string(&fixture.source_path).unwrap();
    json = json.replace(
        "\"asset\": {\"version\": \"2.0\"},",
        "\"asset\": {\"version\": \"2.0\"},\n  \"extensionsRequired\": [\"EXT_meshopt_compression\"],",
    );
    fs::write(&fixture.source_path, json).unwrap();
    let err = plan_intake(
        &IntakeRequest::with_unknown_provenance(&fixture.source_path),
        &IntakeLimits::default(),
    )
    .expect_err("unsupported required extension must reject the plan");
    assert!(
        rejection(DiagnosticCode::UnsupportedRequiredExtension, &err),
        "{err}"
    );
    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Diagnostics: collisions; license rule
// ---------------------------------------------------------------------------

#[test]
fn path_collision_between_distinct_references_is_a_warning() {
    let dir = temp_dir("collision");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    // Duplicate the image entry with a different (but equivalent) spelling.
    let mut json = fs::read_to_string(&fixture.source_path).unwrap();
    json = json.replace(
        "\"images\": [{\"uri\": \"textures/albedo.png\"}],",
        "\"images\": [{\"uri\": \"textures/albedo.png\"}, {\"uri\": \"./textures/./albedo.png\"}],",
    );
    fs::write(&fixture.source_path, json).unwrap();
    let plan = plan_for(&fixture.source_path);
    assert_eq!(plan.status, ReportStatus::WithWarnings);
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::PathCollision && d.severity_str() == "warning"),
        "expected a PathCollision warning"
    );
    // Both references are still pinned; the snapshot proceeds.
    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();
    assert_eq!(snapshot.manifest.dependencies.len(), 3);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn unknown_license_is_recorded_and_never_a_permission() {
    let dir = temp_dir("license");
    let fixture = write_gltf_fixture(&dir, "scene.bin", None, true);

    // Unknown provenance: warning diagnostic + decision unknown_no_permission.
    let plan = plan_for(&fixture.source_path);
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::UnknownLicense),
        "unknown license must be flagged"
    );
    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();
    let raw = fs::read(&snapshot.manifest_path).unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains("unknown_no_permission"), "{text}");
    assert!(
        !text.contains("\"permitted\""),
        "no commercial permission may be asserted: {text}"
    );
    assert!(!text.contains("declared_not_evaluated"));

    // Declared license: recorded verbatim, still not evaluated here.
    let dir2 = temp_dir("license-declared");
    let fixture2 = write_gltf_fixture(&dir2, "scene.bin", None, true);
    let req = IntakeRequest {
        source: fixture2.source_path.clone(),
        provenance: ProvenanceInfo {
            supplier: Some("test-supplier".into()),
            external_id: Some("EXT-1".into()),
            license: LicenseStatus::Declared {
                spdx: "CC-BY-4.0".into(),
            },
            notes: None,
        },
    };
    let plan2 = plan_intake(&req, &IntakeLimits::default()).unwrap();
    assert!(
        !plan2
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::UnknownLicense),
        "declared license does not produce the unknown warning"
    );
    let snap2 = execute_plan(&plan2, &dir2.join("ws")).unwrap();
    let raw2 = fs::read(&snap2.manifest_path).unwrap();
    let text2 = String::from_utf8(raw2).unwrap();
    assert!(text2.contains("declared_not_evaluated"), "{text2}");
    assert!(text2.contains("CC-BY-4.0"));
    assert!(!text2.contains("\"permitted\""));
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
}

// ---------------------------------------------------------------------------
// Provisional DTO seam
// ---------------------------------------------------------------------------

#[test]
fn provisional_dto_view_maps_the_manifest() {
    let dir = temp_dir("dto");
    let fixture = write_gltf_fixture(&dir, "scene.bin", Some("textures/albedo.png"), true);
    let plan = plan_for(&fixture.source_path);
    let snapshot = execute_plan(&plan, &dir.join("ws")).unwrap();
    let view = michelangelo_asset_intake::source_snapshot_view(&snapshot.manifest);
    assert_eq!(view.schema, "provisional/intake-source-snapshot/0");
    assert_eq!(view.snapshot_id, snapshot.manifest.snapshot_id);
    assert_eq!(view.payload.sha256, plan.source.sha256);
    assert_eq!(view.dependencies.len(), 2);
    let _ = fs::remove_dir_all(dir);
}
