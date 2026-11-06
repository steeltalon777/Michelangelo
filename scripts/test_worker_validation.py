#!/usr/bin/env python3
"""
Unit checks for `blender/worker.py` that run WITHOUT Blender/bpy.

The worker module intentionally has no top-level `bpy` import, so its
validation rules, contract constants and camera-math helpers can be tested from
a plain CPython interpreter (the same standard library set Blender bundles).

Run:
    python scripts/test_worker_validation.py

Exit code 0 on success, non-zero on the first failing test set.
"""

import importlib.util
import json
import math
import os
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
WORKER_PATH = REPO_ROOT / "blender" / "worker.py"

# Hard-coded expected whitelist (mirror of
# `michelangelo_protocol::SAFE_WORKER_OPS` and the IT-2 brief). Keeping this
# independent of the worker module lets the test catch registry drift.
EXPECTED_OPS = (
    "create_box",
    "set_material",
    "set_camera_set",
    "render_views",
    "export_glb",
    "save_blend",
    "get_scene_summary",
    "create_cylinder",
    "create_sphere",
    "import_mesh",
    "set_transform",
    "create_ortho_camera",
    "set_camera_pose",
    "set_camera_settings",
    "frame_camera",
    "set_render_settings",
    "render_camera",
    "create_component",
)


def load_worker():
    spec = importlib.util.spec_from_file_location("michelangelo_worker", WORKER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load worker module from {WORKER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


worker = load_worker()

TESTS = []


def test(function):
    TESTS.append(function)
    return function


def expect_validation_error(operations, needle, workspace_root=None):
    """Assert that `validate_spec` rejects the operations with `needle`."""
    spec = {"operations": operations}
    try:
        worker.validate_spec(spec, workspace_root=workspace_root)
    except worker.ValidationError as error:
        message = str(error)
        assert needle in message, f"expected {needle!r} in error, got: {message}"
        return message
    raise AssertionError(
        f"expected ValidationError containing {needle!r}, spec was accepted: {spec}"
    )


def expect_valid(operations, workspace_root=None):
    spec = {"operations": operations}
    return worker.validate_spec(spec, workspace_root=workspace_root)


def valid_composite_operations():
    return [
        {"op": "create_component", "name": "body"},
        {
            "op": "create_box",
            "name": "box",
            "size": [2.0, 1.0, 0.5],
            "location": [0.0, 0.0, 0.0],
            "rotation_deg": [0.0, 0.0, 45.0],
            "component": "body",
        },
        {
            "op": "create_cylinder",
            "name": "cyl",
            "radius": 0.3,
            "depth": 1.0,
            "location": [3.0, 0.0, 1.0],
            "vertices": 32,
        },
        {
            "op": "create_sphere",
            "name": "sphere",
            "radius": 0.5,
            "location": [-3.0, 0.0, 0.5],
            "segments": 16,
            "ring_count": 8,
        },
        {
            "op": "import_mesh",
            "path": "assets/model.glb",
            "name": "imported",
            "component": "body",
        },
        {
            "op": "set_transform",
            "object": "box",
            "location": [1.0, 0.0, 0.0],
            "rotation_deg": [0.0, 0.0, 90.0],
            "scale": [1.0, 2.0, 1.0],
            "apply_scale": True,
        },
        {"op": "set_material", "object": "box", "color": [0.2, 0.4, 0.8, 1.0]},
        {"op": "set_camera_set", "views": ["front", "left"]},
        {
            "op": "create_ortho_camera",
            "name": "Cam_ortho",
            "location": [0.0, -10.0, 0.0],
            "look_at": [0.0, 0.0, 0.0],
            "ortho_scale": 4.0,
            "shift_x": 0.1,
            "shift_y": -0.1,
            "clip_start": 0.1,
            "clip_end": 100.0,
        },
        {
            "op": "set_camera_pose",
            "camera": "Cam_ortho",
            "location": [1.0, -10.0, 0.0],
            "look_at": [0.0, 0.0, 0.0],
        },
        {
            "op": "set_camera_settings",
            "camera": "Cam_ortho",
            "ortho_scale": 2.0,
            "clip_start": 0.2,
            "clip_end": 50.0,
        },
        {
            "op": "frame_camera",
            "camera": "Cam_ortho",
            "objects": ["box"],
            "margin": 1.2,
        },
        {
            "op": "set_render_settings",
            "resolution": [512, 512],
            "resolution_percentage": 100,
        },
        {
            "op": "render_camera",
            "camera": "Cam_ortho",
            "path": "blender/renders/silhouette.png",
            "silhouette": True,
        },
        {"op": "render_views", "path": "blender/renders"},
        {"op": "save_blend", "path": "blender/scenes/composite.blend"},
        {"op": "export_glb", "path": "blender/exports/composite.glb"},
        {"op": "get_scene_summary"},
    ]


# ---------------------------------------------------------------------------
# Registry / contract
# ---------------------------------------------------------------------------

@test
def test_operation_registry_matches_expected_whitelist():
    assert worker.SAFE_OPS_ORDER == EXPECTED_OPS, (
        "worker SAFE_OPS_ORDER drifted from the Rust whitelist:\n"
        f"  worker:   {worker.SAFE_OPS_ORDER}\n"
        f"  expected: {EXPECTED_OPS}"
    )
    assert tuple(worker.OPERATIONS.keys()) == EXPECTED_OPS
    assert worker.SAFE_OPS == frozenset(EXPECTED_OPS)
    assert len(EXPECTED_OPS) == 18


@test
def test_only_whitelisted_operations_are_registered():
    for name in ("execute_arbitrary_python", "explode", "run_script", ""):
        assert name not in worker.SAFE_OPS


@test
def test_contract_constants_match_protocol():
    assert worker.MAX_OPERATIONS == 256
    assert worker.MAX_NAME_CHARS == 64
    assert worker.MAX_SPEC_PATH_CHARS == 1024
    assert worker.MAX_VIEWS == 64
    assert worker.MAX_RESOLUTION == 16384
    assert worker.WORKBENCH_ENGINE == "BLENDER_WORKBENCH"
    assert worker.IMPORT_MESH_EXTENSIONS == ("glb", "gltf", "obj", "stl", "ply")
    assert worker.CAPABILITIES_MARKER == "MICHELANGELO_CAPABILITIES:"
    # Legacy view contract: `side` is a documented Phase 2 alias of `left`;
    # the default view literal is the documented four (front/side/top/perspective).
    assert worker.LEGACY_VIEW_ALIASES == {"side": "left"}
    assert "side" in worker.LEGACY_VIEW_NAMES
    assert worker.LEGACY_DEFAULT_VIEWS == ("front", "side", "top", "perspective")


@test
def test_payload_args_parsing():
    assert worker.payload_args(
        ["blender", "--background", "--python", "worker.py", "--", "job.json", "res.json"]
    ) == ["job.json", "res.json"]
    assert worker.payload_args(["blender", "--python", "worker.py", "--", "--capabilities"]) == [
        "--capabilities"
    ]
    # The capabilities marker works even when no `--` separator is present.
    assert worker.payload_args(["worker.py", "--capabilities"]) == ["--capabilities"]
    assert worker.payload_args(["worker.py", "job.json", "res.json"]) == [
        "job.json",
        "res.json",
    ]


# ---------------------------------------------------------------------------
# Job envelope validation
# ---------------------------------------------------------------------------

@test
def test_validate_job_accepts_valid_envelope():
    with tempfile.TemporaryDirectory() as temp_dir:
        job = {
            "job_id": "job_001-abc",
            "workspace_root": str(Path(temp_dir).resolve()),
            "spec": {"operations": [{"op": "get_scene_summary"}]},
        }
        job_id, root, spec = worker.validate_job(job)
        assert job_id == "job_001-abc"
        assert root == Path(temp_dir).resolve()
        assert spec == {"operations": [{"op": "get_scene_summary"}]}


@test
def test_validate_job_rejects_bad_job_id():
    with tempfile.TemporaryDirectory() as temp_dir:
        base = {
            "workspace_root": str(Path(temp_dir).resolve()),
            "spec": {"operations": []},
        }
        for bad_id in ("", "../evil", "with space", "äöü", "a" * 129, "job/../id"):
            job = dict(base, job_id=bad_id)
            try:
                worker.validate_job(job)
            except worker.ValidationError as error:
                assert "job_id" in str(error), f"unexpected message: {error}"
            else:
                raise AssertionError(f"job_id {bad_id!r} must be rejected")


@test
def test_validate_job_rejects_bad_workspace_root():
    for bad_root in ("", "relative/dir", None, 42):
        job = {"job_id": "job_1", "workspace_root": bad_root, "spec": {"operations": []}}
        try:
            worker.validate_job(job)
        except worker.ValidationError:
            pass
        else:
            raise AssertionError(f"workspace_root {bad_root!r} must be rejected")


@test
def test_windows_verbatim_workspace_path_normalization():
    if os.name == "nt":
        assert (
            worker._normalize_workspace_path(r"\\?\I:\prog\michelangelo")
            == r"I:\prog\michelangelo"
        )
        assert (
            worker._normalize_workspace_path(r"\\?\UNC\server\share\workspace")
            == r"\\server\share\workspace"
        )
        assert (
            worker._normalize_workspace_path(r"I:\already\normal")
            == r"I:\already\normal"
        )

        # A verbatim workspace_root (as stored by the Rust core on Windows) is
        # accepted and normalized, so add-ons that reject verbatim paths work.
        with tempfile.TemporaryDirectory() as temp_dir:
            resolved = str(Path(temp_dir).resolve())
            verbatim = "\\\\?\\" + resolved
            job = {
                "job_id": "job_verbatim",
                "workspace_root": verbatim,
                "spec": {"operations": [{"op": "get_scene_summary"}]},
            }
            job_id, root, _spec = worker.validate_job(job)
            assert job_id == "job_verbatim"
            assert not str(root).startswith("\\\\?\\"), (
                "worker must normalize verbatim workspace roots, got "
                f"{root}"
            )
    else:
        # The prefix is a Windows-only filesystem concept.
        assert worker._normalize_workspace_path("/home/user/ws") == "/home/user/ws"


# ---------------------------------------------------------------------------
# Spec validation
# ---------------------------------------------------------------------------

@test
def test_validate_spec_accepts_full_composite():
    operations = valid_composite_operations()
    assert len(operations) == 18
    assert expect_valid(operations) == operations


@test
def test_json_text_spec_pre_validation():
    text = json.dumps({"operations": [{"op": "create_box", "name": "box"}]})
    assert expect_valid(json.loads(text)["operations"])

    bad_text = json.dumps({"operations": [{"op": "run_python", "code": "print(1)"}]})
    expect_validation_error(json.loads(bad_text)["operations"], "unsupported operation")

    # Non-object / missing operations containers are rejected defensively.
    for bad_spec in (None, {}, {"operations": "nope"}, {"operations": [42]}):
        try:
            worker.validate_spec(bad_spec)
        except worker.ValidationError:
            pass
        else:
            raise AssertionError(f"spec {bad_spec!r} must be rejected")


@test
def test_validate_spec_rejects_unknown_and_missing_fields():
    expect_validation_error([{"op": "explode"}], "unsupported operation")
    expect_validation_error([{"op": "save_blend"}], "missing required field 'path'")
    expect_validation_error([{"op": "render_camera", "path": "a.png"}], "camera")
    expect_validation_error([{"op": "create_ortho_camera"}], "name")


@test
def test_operations_limit():
    operations = [{"op": "get_scene_summary"}] * (worker.MAX_OPERATIONS + 1)
    expect_validation_error(operations, "too many operations")
    expect_valid([{"op": "get_scene_summary"}] * worker.MAX_OPERATIONS)


@test
def test_name_rules():
    cases = [
        ("", "must not be empty"),
        ("   ", "must not be empty"),
        ("a" * (worker.MAX_NAME_CHARS + 1), "too long"),
        ("bad\nname", "control"),
        ("bad/name", "'/' or '\\'"),
        ("bad\\name", "'/' or '\\'"),
        (".", "'.' or '..'"),
        ("..", "'.' or '..'"),
    ]
    for name, needle in cases:
        expect_validation_error([{"op": "create_component", "name": name}], needle)
    expect_valid([{"op": "create_component", "name": "body_part-01"}])


@test
def test_duplicate_created_names():
    expect_validation_error(
        [
            {"op": "create_box", "name": "body"},
            {"op": "create_box", "name": "body"},
        ],
        "duplicate created object/camera name 'body'",
    )
    # Default names are tracked too.
    expect_validation_error(
        [{"op": "create_box"}, {"op": "create_box"}],
        "duplicate created object/camera name 'Box'",
    )
    expect_validation_error(
        [
            {"op": "create_cylinder", "name": "shared"},
            {
                "op": "create_ortho_camera",
                "name": "shared",
                "location": [0.0, -10.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
            },
        ],
        "duplicate created object/camera name 'shared'",
    )
    # Legacy camera-set default names collide with a later ortho camera.
    expect_validation_error(
        [
            {"op": "set_camera_set"},
            {
                "op": "create_ortho_camera",
                "name": "Cam_front",
                "location": [0.0, -10.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
            },
        ],
        "duplicate created object/camera name 'Cam_front'",
    )


@test
def test_path_rules():
    long_path = "a" * (worker.MAX_SPEC_PATH_CHARS + 1) + ".blend"
    absolute = "C:/absolute/path.blend" if os.name == "nt" else "/absolute/path.blend"
    for path, needle in [
        ("", "must not be empty"),
        (absolute, "workspace-relative"),
        ("../evil.blend", "parent-traversal"),
        ("dir/../../evil.blend", "parent-traversal"),
        ("a\x00b.blend", "NUL"),
        (long_path, "too long"),
        # Portable hardening: drive-relative and UNC forms are always rejected.
        ("C:drive-relative.blend", "workspace-relative"),
        ("\\\\server\\share\\evil.blend", "workspace-relative"),
    ]:
        expect_validation_error([{"op": "save_blend", "path": path}], needle)

    # `..` inside a file name is not traversal.
    expect_valid([{"op": "save_blend", "path": "blender/scenes/model.v1..2.blend"}])


@test
def test_extension_rules():
    expect_valid([{"op": "save_blend", "path": "a.blend"}])
    expect_valid([{"op": "save_blend", "path": "a.BLEND"}])
    expect_validation_error([{"op": "save_blend", "path": "a.glb"}], ".blend")
    expect_valid([{"op": "export_glb", "path": "a.glb"}])
    expect_validation_error([{"op": "export_glb", "path": "a.blend"}], ".glb")
    expect_valid(
        [{"op": "render_camera", "camera": "c", "path": "renders/sil.png"}]
    )
    expect_validation_error(
        [{"op": "render_camera", "camera": "c", "path": "renders/sil.jpg"}], ".png"
    )
    # Legacy render_views base directory has no extension requirement.
    expect_valid([{"op": "render_views"}])
    expect_valid([{"op": "render_views", "path": "blender/renders"}])
    for extension in worker.IMPORT_MESH_EXTENSIONS:
        expect_valid([{"op": "import_mesh", "path": f"assets/model.{extension}"}])
    expect_valid([{"op": "import_mesh", "path": "assets/model.PLY"}])
    expect_validation_error(
        [{"op": "import_mesh", "path": "assets/model.fbx"}], ".glb"
    )
    expect_validation_error([{"op": "import_mesh", "path": "assets/model"}], "extensions")


@test
def test_views_rules():
    expect_valid([{"op": "set_camera_set"}])
    for view in worker.LEGACY_VIEW_NAMES:
        expect_valid([{"op": "set_camera_set", "views": [view]}])
    # `side` is a documented Phase 2 alias and must be accepted explicitly.
    expect_valid(
        [{"op": "set_camera_set", "views": ["front", "side", "top", "perspective"]}]
    )
    # `side` + `left` are the same camera: accepted and deduplicated, not an error.
    expect_valid([{"op": "set_camera_set", "views": ["side", "left"]}])
    expect_validation_error([{"op": "set_camera_set", "views": []}], "1 and 64")
    # Only truly unknown view names are rejected.
    expect_validation_error(
        [{"op": "set_camera_set", "views": ["sideways"]}], "unsupported view"
    )
    expect_validation_error(
        [{"op": "set_camera_set", "views": [42]}], "unsupported view"
    )
    expect_validation_error(
        [{"op": "set_camera_set", "views": ["front", "front"]}], "duplicate view"
    )
    expect_validation_error(
        [{"op": "set_camera_set", "views": ["front"] * (worker.MAX_VIEWS + 1)}],
        "1 and 64",
    )


@test
def test_side_view_alias_resolution_and_dedupe():
    assert worker.canonical_view_name("side") == "left"
    assert worker.canonical_view_name("left") == "left"
    assert worker.canonical_view_name("front") == "front"
    assert worker.canonical_view_name("sideways") is None

    # The documented Phase 2 default literal resolves through the alias.
    assert worker.resolve_legacy_views(None) == (
        "front",
        "left",
        "top",
        "perspective",
    )
    assert worker.resolve_legacy_views(["front", "side", "top", "perspective"]) == (
        "front",
        "left",
        "top",
        "perspective",
    )
    # Alias duplicates collapse to one camera, keeping first-seen order.
    assert worker.resolve_legacy_views(["side", "left"]) == ("left",)
    assert worker.resolve_legacy_views(["left", "side"]) == ("left",)
    assert worker.resolve_legacy_views(["side", "front", "side"]) == (
        "left",
        "front",
    )
    # The canonical camera name collision is visible to duplicate-name
    # validation: a later `Cam_left` conflicts with the default `side` camera.
    expect_validation_error(
        [
            {"op": "set_camera_set"},
            {
                "op": "create_ortho_camera",
                "name": "Cam_left",
                "location": [0.0, -10.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
            },
        ],
        "duplicate created object/camera name 'Cam_left'",
    )
    expect_validation_error(
        [
            {"op": "set_camera_set", "views": ["side"]},
            {
                "op": "create_ortho_camera",
                "name": "Cam_left",
                "location": [0.0, -10.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
            },
        ],
        "duplicate created object/camera name 'Cam_left'",
    )


@test
def test_vector_and_numeric_ranges():
    cases = [
        ({"op": "create_box", "size": [2.0, 1.0]}, "exactly 3 components"),
        ({"op": "create_box", "size": [2.0, 0.0, 1.0]}, "> 0"),
        ({"op": "create_box", "location": [0.0, 0.0, 2_000_000.0]}, "1000000"),
        ({"op": "create_box", "rotation_deg": [0.0, 0.0]}, "exactly 3 components"),
        ({"op": "set_material", "object": "x", "color": [0.0, 0.0]}, "3 or 4"),
        ({"op": "set_material", "object": "x", "color": [1.5, 0.0, 0.0]}, "0..=1"),
        ({"op": "set_transform", "object": "x", "scale": [1.0, 0.0, 1.0]}, "non-zero"),
        ({"op": "set_transform", "object": "x"}, "at least one"),
        ({"op": "create_cylinder", "radius": 0.0}, "> 0"),
        ({"op": "create_cylinder", "depth": -1.0}, "> 0"),
        ({"op": "create_cylinder", "vertices": 2}, "3..=1024"),
        ({"op": "create_cylinder", "vertices": 1025}, "3..=1024"),
        ({"op": "create_sphere", "segments": 2}, "3..=1024"),
        ({"op": "create_sphere", "ring_count": 1}, "2..=1024"),
        (
            {
                "op": "create_ortho_camera",
                "name": "c",
                "location": [0.0, 0.0, 0.0],
                "ortho_scale": 0.0,
            },
            "> 0",
        ),
        (
            {
                "op": "create_ortho_camera",
                "name": "c",
                "location": [0.0, 0.0, 0.0],
                "shift_x": 11.0,
            },
            "-10..=10",
        ),
        (
            {
                "op": "create_ortho_camera",
                "name": "c",
                "location": [0.0, 0.0, 0.0],
                "clip_start": 10.0,
                "clip_end": 5.0,
            },
            "clip_end",
        ),
        ({"op": "set_render_settings", "resolution": [0, 512]}, "1..=16384"),
        ({"op": "set_render_settings", "resolution": [512]}, "exactly 2 components"),
        ({"op": "set_render_settings", "resolution_percentage": 0}, "1..=100"),
        ({"op": "set_render_settings", "resolution_percentage": 101}, "1..=100"),
        ({"op": "frame_camera", "camera": "c", "margin": 0.05}, "0.1..=10"),
        ({"op": "frame_camera", "camera": "c", "margin": 11.0}, "0.1..=10"),
    ]
    for operations, needle in cases:
        expect_validation_error([operations], needle)

    # Non-finite values can only be built programmatically.
    for operation, needle in [
        (
            {
                "op": "create_box",
                "size": [float("nan"), 1.0, 1.0],
            },
            "non-finite",
        ),
        (
            {
                "op": "create_box",
                "location": [float("inf"), 0.0, 0.0],
            },
            "non-finite",
        ),
    ]:
        expect_validation_error([operation], needle)


@test
def test_camera_pose_and_settings_rules():
    expect_validation_error(
        [
            {
                "op": "create_ortho_camera",
                "name": "c",
                "location": [0.0, -10.0, 0.0],
                "rotation_deg": [90.0, 0.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
            }
        ],
        "mutually exclusive",
    )
    # create_ortho_camera may omit both pose fields (default orientation).
    expect_valid(
        [
            {
                "op": "create_ortho_camera",
                "name": "c",
                "location": [0.0, -10.0, 0.0],
            }
        ]
    )
    expect_validation_error(
        [{"op": "set_camera_pose", "camera": "c"}], "at least one"
    )
    expect_valid(
        [{"op": "set_camera_pose", "camera": "c", "location": [1.0, 2.0, 3.0]}]
    )
    expect_valid(
        [{"op": "set_camera_pose", "camera": "c", "rotation_deg": [90.0, 0.0, 0.0]}]
    )
    expect_validation_error(
        [
            {
                "op": "set_camera_pose",
                "camera": "c",
                "rotation_deg": [90.0, 0.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
            }
        ],
        "mutually exclusive",
    )
    expect_validation_error(
        [{"op": "set_camera_settings", "camera": "c"}], "at least one camera setting"
    )
    expect_valid(
        [{"op": "set_camera_settings", "camera": "c", "ortho_scale": 2.0}]
    )
    expect_validation_error(
        [{"op": "frame_camera", "camera": "c", "objects": []}], "must not be empty"
    )
    expect_validation_error(
        [
            {
                "op": "frame_camera",
                "camera": "c",
                "objects": ["box"],
                "component": "body",
            }
        ],
        "at most one",
    )
    expect_validation_error(
        [{"op": "frame_camera", "camera": "c", "objects": ["bad/name"]}],
        "'/' or '\\'",
    )


@test
def test_validation_error_names_operation_index():
    message = expect_validation_error(
        [
            {"op": "create_box", "name": "box"},
            {"op": "save_blend", "path": "out.glb"},
        ],
        ".blend",
    )
    assert message.startswith("operation[1] ('save_blend')"), message


@test
def test_legacy_phase2_spec_with_side_view_accepted():
    legacy = [
        {"op": "create_box", "name": "body", "size": [2.0, 1.0, 0.5], "location": [0.0, 0.0, 0.0]},
        {"op": "set_material", "object": "body", "color": [0.2, 0.4, 0.8, 1.0]},
        {"op": "set_camera_set", "views": ["front", "side", "top", "perspective"]},
        {"op": "save_blend", "path": "blender/scenes/smoke.blend"},
        {"op": "export_glb", "path": "blender/exports/smoke.glb"},
    ]
    expect_valid(legacy)
    # The same documented example parsed from JSON text (wire compatibility).
    assert expect_valid(json.loads(json.dumps(legacy)))


@test
def test_import_mesh_workspace_checks():
    with tempfile.TemporaryDirectory() as temp_dir:
        workspace = Path(temp_dir).resolve()
        missing = [{"op": "import_mesh", "path": "assets/missing.glb"}]
        expect_validation_error(missing, "does not exist", workspace_root=workspace)

        empty = workspace / "assets" / "empty.glb"
        empty.parent.mkdir(parents=True, exist_ok=True)
        empty.write_bytes(b"")
        expect_validation_error(
            [{"op": "import_mesh", "path": "assets/empty.glb"}],
            "is empty",
            workspace_root=workspace,
        )

        directory = workspace / "assets" / "dir.glb"
        directory.mkdir()
        expect_validation_error(
            [{"op": "import_mesh", "path": "assets/dir.glb"}],
            "not a regular file",
            workspace_root=workspace,
        )

        valid = workspace / "assets" / "model.glb"
        valid.write_bytes(b"glTF" + b"\x02\x00\x00\x00" + b"\x00" * 8)
        assert (
            expect_valid(
                [{"op": "import_mesh", "path": "assets/model.glb"}],
                workspace_root=workspace,
            )
            is not None
        )


# ---------------------------------------------------------------------------
# Camera math helpers (used by frame_camera; IT-3 will assert on pixels)
# ---------------------------------------------------------------------------

@test
def test_ortho_scale_for_frame():
    assert worker.ortho_scale_for_frame(2.0, 1.0, 1.0, 1.0) == 2.0
    assert worker.ortho_scale_for_frame(2.0, 2.0, 1.0, 1.1) == 2.2
    # height * aspect dominates when the frame is wider than tall.
    assert worker.ortho_scale_for_frame(1.0, 2.0, 2.0, 1.1) == 4.4
    # Calibrated IT-2 fixture: 2x1x0.5 box, 512x512, margin 1.1 -> 2.2 (width).
    assert math.isclose(
        worker.ortho_scale_for_frame(2.0, 0.5, 1.0, 1.1), 2.2, rel_tol=0.0, abs_tol=1e-12
    )
    for bad_args in ((float("nan"), 1.0, 1.0, 1.0), (1.0, 1.0, 0.0, 1.0), (1.0, 1.0, 1.0, 0.0)):
        try:
            worker.ortho_scale_for_frame(*bad_args)
        except worker.OperationError:
            pass
        else:
            raise AssertionError(f"bad frame args {bad_args} must be rejected")


@test
def test_camera_local_xy_offset():
    identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    assert worker.camera_local_xy_offset(identity, 1.0, 2.0) == (1.0, 2.0, 0.0)
    rotate_z_90 = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
    assert worker.camera_local_xy_offset(rotate_z_90, 1.0, 0.0) == (0.0, 1.0, 0.0)
    assert worker.camera_local_xy_offset(rotate_z_90, 0.0, 1.0) == (-1.0, 0.0, 0.0)

    # Camera looks along +Y (front view, rotation 90 deg around X) with +Z up;
    # a camera-space XY offset must stay in the plane perpendicular to the view.
    rotate_x_90 = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]]
    dx, dy, dz = worker.camera_local_xy_offset(rotate_x_90, 0.0, 0.5)
    assert (dx, dy, dz) == (0.0, 0.0, 0.5)
    try:
        worker.camera_local_xy_offset([[1.0, 0.0], [0.0, 1.0]], 0.0, 0.0)
    except worker.OperationError:
        pass
    else:
        raise AssertionError("non-3x3 rotation matrix must be rejected")


@test
def test_world_bbox_and_corners():
    points = [(-1.0, -0.5, -0.25), (1.0, 0.5, 0.25), (0.0, 0.0, 0.0)]
    bbox_min, bbox_max = worker.world_bbox(points)
    assert bbox_min == (-1.0, -0.5, -0.25)
    assert bbox_max == (1.0, 0.5, 0.25)
    corners = worker.bbox_corners(bbox_min, bbox_max)
    assert len(corners) == 8
    assert len(set(corners)) == 8
    assert (-1.0, -0.5, -0.25) in corners
    assert (1.0, 0.5, 0.25) in corners
    try:
        worker.world_bbox([])
    except worker.OperationError:
        pass
    else:
        raise AssertionError("empty bbox must be rejected")


@test
def test_camera_space_bounds():
    points = [(0.0, 0.0, 0.0), (2.0, 4.0, -1.0), (-1.0, 1.0, 0.0)]
    min_x, max_x, min_y, max_y = worker.camera_space_bounds(points, tuple)
    assert (min_x, max_x, min_y, max_y) == (-1.0, 2.0, 0.0, 4.0)


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

def main():
    failures = 0
    for function in TESTS:
        name = function.__name__
        try:
            function()
        except Exception as error:  # noqa: BLE001 - report every failure
            failures += 1
            print(f"FAIL {name}: {type(error).__name__}: {error}", file=sys.stderr)
        else:
            print(f"PASS {name}")
    print()
    print(f"{len(TESTS) - failures}/{len(TESTS)} worker validation checks passed")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
