#!/usr/bin/env python3
"""
Michelangelo Blender Worker — safe operation executor.

Invocation:
    blender --background --factory-startup --python worker.py -- <job.json> <result.json>
    blender --background --factory-startup --python worker.py -- --capabilities

Reads a strict JSON job spec, executes the whitelisted safe operations via
`bpy`, and writes a JSON result file. All progress goes to stderr; stdout is
reserved for the single capabilities marker line.

`--capabilities` is a lightweight mode used by the Rust adapter to probe what
the installed worker/Blender actually supports; it prints exactly one
`MICHELANGELO_CAPABILITIES:<json>` line to stdout and exits 0.

Design rules (mirrored by `crates/michelangelo_protocol/src/blender.rs`):
- every job starts from a deterministic empty scene;
- the whole spec is validated before the scene is touched or any file is
  written;
- only the operations in `SAFE_OPS_ORDER` can ever run (no arbitrary Python);
- all artifact paths are workspace-relative and must resolve inside the
  workspace;
- legacy camera views keep the Phase 2 back-compat alias `side` -> `left`
  (created as `Cam_left`); the default view set is
  `front, side, top, perspective`, resolving to the effective cameras
  `Cam_front, Cam_left, Cam_top, Cam_perspective`.

The module has no top-level `bpy` import so its validation and camera-math
helpers stay importable/testable outside Blender
(`python scripts/test_worker_validation.py`).
"""

import json
import math
import os
import platform
import re
import sys
import traceback
import unicodedata
from pathlib import Path

# ---------------------------------------------------------------------------
# Contract constants (must stay in sync with michelangelo_protocol)
# ---------------------------------------------------------------------------

MAX_JOB_ID_CHARS = 128
MAX_OPERATIONS = 256
MAX_NAME_CHARS = 64
MAX_SPEC_PATH_CHARS = 1024
MAX_VIEWS = 64
MAX_COORDINATE_ABS = 1_000_000.0
MAX_RESOLUTION = 16_384
MAX_SHIFT_ABS = 10.0
MIN_FRAME_MARGIN = 0.1
MAX_FRAME_MARGIN = 10.0

DEFAULT_BOX_SIZE = (2.0, 2.0, 2.0)
DEFAULT_RADIUS = 0.5
DEFAULT_DEPTH = 1.0
DEFAULT_CYLINDER_VERTICES = 32
DEFAULT_SPHERE_SEGMENTS = 32
DEFAULT_SPHERE_RINGS = 16
DEFAULT_ORTHO_SCALE = 4.0
DEFAULT_CLIP_START = 0.1
DEFAULT_CLIP_END = 1000.0
DEFAULT_FRAME_MARGIN = 1.1
DEFAULT_RENDER_RESOLUTION = (512, 512)
DEFAULT_MATERIAL_COLOR = (0.5, 0.5, 0.5, 1.0)
DEFAULT_RENDER_BASE = "blender/renders"

WORKBENCH_ENGINE = "BLENDER_WORKBENCH"

JOB_ID_PATTERN = re.compile(r"\A[A-Za-z0-9_-]{1,128}\Z")

LEGACY_VIEWS = (
    "front",
    "back",
    "left",
    "right",
    "top",
    "bottom",
    "perspective",
)

# Back-compat wire aliases accepted by `set_camera_set` (Phase 2 documented and
# shipped `side` although its camera table only defined `left`). Aliases map to
# the canonical camera definition and camera object name (`Cam_left`).
LEGACY_VIEW_ALIASES = {"side": "left"}

# All accepted wire view names (canonical names + aliases).
LEGACY_VIEW_NAMES = LEGACY_VIEWS + ("side",)

# Phase 2 documented default view literal. Through the alias the effective
# deterministic cameras are front/left/top/perspective.
LEGACY_DEFAULT_VIEWS = ("front", "side", "top", "perspective")

# Location/rotation in degrees; converted to radians by the handler.
LEGACY_VIEW_DEFS = {
    "front": {"location": (0.0, -10.0, 0.0), "rotation": (90.0, 0.0, 0.0)},
    "back": {"location": (0.0, 10.0, 0.0), "rotation": (90.0, 0.0, 180.0)},
    "left": {"location": (-10.0, 0.0, 0.0), "rotation": (90.0, 0.0, 90.0)},
    "right": {"location": (10.0, 0.0, 0.0), "rotation": (90.0, 0.0, -90.0)},
    "top": {"location": (0.0, 0.0, 10.0), "rotation": (0.0, 0.0, 0.0)},
    "bottom": {"location": (0.0, 0.0, -10.0), "rotation": (180.0, 0.0, 0.0)},
    "perspective": {"location": (7.0, -7.0, 5.0), "rotation": (60.0, 0.0, 45.0)},
}

IMPORT_MESH_EXTENSIONS = ("glb", "gltf", "obj", "stl", "ply")

# Ordered exactly like `michelangelo_protocol::SAFE_WORKER_OPS`; also used as
# the capabilities operation list so probe output and the Rust static fallback
# agree.
SAFE_OPS_ORDER = (
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

SAFE_OPS = frozenset(SAFE_OPS_ORDER)

# Marker line used by the Rust adapter capabilities probe.
CAPABILITIES_MARKER = "MICHELANGELO_CAPABILITIES:"

# Candidate render engines to probe, in reporting order. Only engines that the
# installed Blender actually accepts are reported; BLENDER_WORKBENCH is the
# supported engine for this project's TZ.
RENDER_ENGINE_CANDIDATES = (
    "BLENDER_WORKBENCH",
    "BLENDER_EEVEE",
    "BLENDER_EEVEE_NEXT",
    "CYCLES",
)


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

class ValidationError(ValueError):
    """Invalid job/workspace/spec input. Rejected before any scene mutation."""


class OperationError(RuntimeError):
    """An operation could not be executed."""


# ---------------------------------------------------------------------------
# Validation helpers (pure Python, no bpy)
# ---------------------------------------------------------------------------

def _is_control_char(char):
    return unicodedata.category(char) == "Cc"


def _is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _is_finite_number(value):
    return _is_number(value) and math.isfinite(float(value))


def _has_control_chars(value):
    return any(_is_control_char(char) for char in value)


def _validate_name_value(value, field, label):
    if not isinstance(value, str):
        raise ValidationError(f"{label}: '{field}' must be a string")
    if not value.strip():
        raise ValidationError(f"{label}: '{field}' must not be empty")
    if len(value) > MAX_NAME_CHARS:
        raise ValidationError(
            f"{label}: '{field}' is too long (max {MAX_NAME_CHARS} chars)"
        )
    if _has_control_chars(value):
        raise ValidationError(
            f"{label}: '{field}' must not contain control characters"
        )
    if "/" in value or "\\" in value:
        raise ValidationError(
            f"{label}: '{field}' must not contain '/' or '\\'"
        )
    if value in (".", ".."):
        raise ValidationError(f"{label}: '{field}' must not be '.' or '..'")
    return value


def _optional_name(params, field, label):
    value = params.get(field)
    if value is None:
        return None
    return _validate_name_value(value, field, label)


def _require_name(params, field, label):
    value = params.get(field)
    if value is None:
        raise ValidationError(f"{label}: missing required field '{field}'")
    return _validate_name_value(value, field, label)


def _optional_string(params, field, label):
    value = params.get(field)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValidationError(f"{label}: '{field}' must be a string")
    return value


def _require_string(params, field, label):
    value = params.get(field)
    if value is None:
        raise ValidationError(f"{label}: missing required field '{field}'")
    if not isinstance(value, str):
        raise ValidationError(f"{label}: '{field}' must be a string")
    return value


def _optional_bool(params, field, label):
    value = params.get(field)
    if value is None:
        return None
    if not isinstance(value, bool):
        raise ValidationError(f"{label}: '{field}' must be a boolean")
    return value


def _optional_uint(params, field, minimum, maximum, label):
    value = params.get(field)
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValidationError(f"{label}: '{field}' must be an integer")
    if value < minimum or value > maximum:
        raise ValidationError(
            f"{label}: '{field}' must be in the {minimum}..={maximum} range"
        )
    return value


def _optional_bounded_number(params, field, minimum, maximum, label):
    value = params.get(field)
    if value is None:
        return None
    if not _is_finite_number(value):
        raise ValidationError(f"{label}: '{field}' must be a finite number")
    if float(value) < minimum or float(value) > maximum:
        raise ValidationError(
            f"{label}: '{field}' must be in the {minimum:g}..={maximum:g} range"
        )
    return float(value)


def _optional_positive_number(params, field, label):
    value = params.get(field)
    if value is None:
        return None
    if not _is_finite_number(value) or float(value) <= 0.0:
        raise ValidationError(f"{label}: '{field}' must be a finite number > 0")
    return float(value)


def _is_absolute_or_prefix_path(value):
    if os.path.isabs(value):
        return True
    # Windows drive-relative ("C:foo") and UNC ("\\\\server\\share") forms are
    # rejected on every platform for defense in depth.
    if re.match(r"\A[A-Za-z]:", value):
        return True
    if value.startswith("\\\\") or value.startswith("//"):
        return True
    return False


def _validate_relative_path(value, field, label):
    if not isinstance(value, str):
        raise ValidationError(f"{label}: '{field}' must be a string")
    if not value.strip():
        raise ValidationError(f"{label}: '{field}' must not be empty")
    if len(value) > MAX_SPEC_PATH_CHARS:
        raise ValidationError(
            f"{label}: '{field}' is too long (max {MAX_SPEC_PATH_CHARS} chars)"
        )
    if "\x00" in value:
        raise ValidationError(f"{label}: '{field}' must not contain NUL")
    if _is_absolute_or_prefix_path(value):
        raise ValidationError(
            f"{label}: '{field}' must be workspace-relative "
            f"(absolute paths are rejected): '{value}'"
        )
    for part in re.split(r"[\\/]", value):
        if part == "..":
            raise ValidationError(
                f"{label}: '{field}' must not contain parent-traversal "
                f"components: '{value}'"
            )
    return value


def _extension(value):
    return Path(value).suffix.lower().lstrip(".")


def _validate_extension(value, allowed, field, label):
    if _extension(value) not in allowed:
        formatted = ", ".join(f".{extension}" for extension in allowed)
        raise ValidationError(
            f"{label}: '{field}' must have one of the extensions: {formatted} "
            f"(case-insensitive)"
        )
    return value


def _require_vector(params, field, label):
    value = params.get(field)
    if value is None:
        raise ValidationError(f"{label}: missing required field '{field}'")
    return value


def _validate_coordinate_vector(value, field, label):
    if not isinstance(value, (list, tuple)) or len(value) != 3:
        raise ValidationError(
            f"{label}: '{field}' must have exactly 3 components"
        )
    for component in value:
        if not _is_finite_number(component):
            raise ValidationError(f"{label}: '{field}' contains non-finite values")
        if abs(float(component)) > MAX_COORDINATE_ABS:
            raise ValidationError(
                f"{label}: '{field}' component exceeds the "
                f"±{MAX_COORDINATE_ABS} limit"
            )
    return value


def _optional_coordinate_vector(params, field, label):
    value = params.get(field)
    if value is None:
        return None
    return _validate_coordinate_vector(value, field, label)


def _optional_positive_vector(params, field, label):
    value = params.get(field)
    if value is None:
        return None
    if not isinstance(value, (list, tuple)) or len(value) != 3:
        raise ValidationError(
            f"{label}: '{field}' must have exactly 3 components"
        )
    for component in value:
        if not _is_finite_number(component):
            raise ValidationError(f"{label}: '{field}' contains non-finite values")
        if float(component) <= 0.0:
            raise ValidationError(
                f"{label}: '{field}' components must be finite numbers > 0"
            )
    return value


def _validate_nonzero_vector(value, field, label):
    if not isinstance(value, (list, tuple)) or len(value) != 3:
        raise ValidationError(
            f"{label}: '{field}' must have exactly 3 components"
        )
    for component in value:
        if not _is_finite_number(component):
            raise ValidationError(f"{label}: '{field}' contains non-finite values")
        if float(component) == 0.0:
            raise ValidationError(
                f"{label}: '{field}' components must be non-zero"
            )
    return value


def _validate_color(value, field, label):
    if not isinstance(value, (list, tuple)) or len(value) not in (3, 4):
        raise ValidationError(f"{label}: '{field}' must have 3 or 4 components")
    for component in value:
        if not _is_finite_number(component):
            raise ValidationError(f"{label}: '{field}' contains non-finite values")
        if not 0.0 <= float(component) <= 1.0:
            raise ValidationError(
                f"{label}: '{field}' components must be in the 0..=1 range"
            )
    return value


def _validate_clip_planes(start, end, label):
    if start is not None and end is not None and float(end) <= float(start):
        raise ValidationError(
            f"{label}: 'clip_end' must be greater than 'clip_start'"
        )


def canonical_view_name(view):
    """Canonical camera view for an accepted legacy wire name, or `None`.

    `side` is the Phase 2 back-compat alias of `left`; both create the same
    camera (`Cam_left` with the left-view location/rotation).
    """
    canonical = LEGACY_VIEW_ALIASES.get(view)
    if canonical is not None:
        return canonical
    if view in LEGACY_VIEWS:
        return view
    return None


def resolve_legacy_views(views):
    """Canonical, deduplicated camera views for one `set_camera_set`.

    Missing `views` expands to `LEGACY_DEFAULT_VIEWS`. Aliases map to their
    canonical camera name and alias duplicates (`side` + `left`) collapse to a
    single camera, preserving first-seen order.
    """
    requested = LEGACY_DEFAULT_VIEWS if views is None else tuple(views)
    resolved = []
    for view in requested:
        canonical = canonical_view_name(view) if isinstance(view, str) else None
        if canonical is not None and canonical not in resolved:
            resolved.append(canonical)
    return tuple(resolved)


def _created_names(params, op_name):
    """Names of objects/cameras created by one operation (mirrors Rust)."""
    if op_name == "create_box":
        return [params.get("name") or "Box"]
    if op_name == "create_cylinder":
        return [params.get("name") or "Cylinder"]
    if op_name == "create_sphere":
        return [params.get("name") or "Sphere"]
    if op_name == "create_ortho_camera":
        return [params["name"]]
    if op_name == "set_camera_set":
        return [
            f"Cam_{view}" for view in resolve_legacy_views(params.get("views"))
        ]
    if op_name == "import_mesh" and params.get("name"):
        return [params["name"]]
    return []


def _validate_operation(params, op_name, label):
    """Validate one whitelisted operation (mirrors Rust `validate_at`)."""
    if op_name == "create_box":
        _optional_name(params, "name", label)
        _optional_name(params, "component", label)
        _optional_positive_vector(params, "size", label)
        _optional_coordinate_vector(params, "location", label)
        _optional_coordinate_vector(params, "rotation_deg", label)

    elif op_name == "create_cylinder":
        _optional_name(params, "name", label)
        _optional_name(params, "component", label)
        _optional_positive_number(params, "radius", label)
        _optional_positive_number(params, "depth", label)
        _optional_coordinate_vector(params, "location", label)
        _optional_coordinate_vector(params, "rotation_deg", label)
        _optional_uint(params, "vertices", 3, 1024, label)

    elif op_name == "create_sphere":
        _optional_name(params, "name", label)
        _optional_name(params, "component", label)
        _optional_positive_number(params, "radius", label)
        _optional_coordinate_vector(params, "location", label)
        _optional_coordinate_vector(params, "rotation_deg", label)
        _optional_uint(params, "segments", 3, 1024, label)
        _optional_uint(params, "ring_count", 2, 1024, label)

    elif op_name == "import_mesh":
        path = _require_string(params, "path", label)
        _validate_relative_path(path, "path", label)
        _validate_extension(path, IMPORT_MESH_EXTENSIONS, "path", label)
        _optional_name(params, "name", label)
        _optional_name(params, "component", label)

    elif op_name == "set_transform":
        _require_name(params, "object", label)
        if not any(
            params.get(field) is not None
            for field in ("location", "rotation_deg", "scale")
        ):
            raise ValidationError(
                f"{label}: at least one of 'location', 'rotation_deg' or "
                f"'scale' is required"
            )
        _optional_coordinate_vector(params, "location", label)
        _optional_coordinate_vector(params, "rotation_deg", label)
        scale = params.get("scale")
        if scale is not None:
            _validate_nonzero_vector(scale, "scale", label)
        _optional_bool(params, "apply_scale", label)

    elif op_name == "set_material":
        _require_name(params, "object", label)
        color = params.get("color")
        if color is not None:
            _validate_color(color, "color", label)

    elif op_name == "set_camera_set":
        views = params.get("views")
        if views is not None:
            if not isinstance(views, (list, tuple)):
                raise ValidationError(f"{label}: 'views' must be a list")
            if len(views) < 1 or len(views) > MAX_VIEWS:
                raise ValidationError(
                    f"{label}: 'views' must contain between 1 and {MAX_VIEWS} entries"
                )
            seen = set()
            for view in views:
                canonical = (
                    canonical_view_name(view) if isinstance(view, str) else None
                )
                if canonical is None:
                    raise ValidationError(
                        f"{label}: unsupported view '{view}' "
                        f"(expected one of: {', '.join(LEGACY_VIEW_NAMES)})"
                    )
                # Exact repeated wire names are invalid; alias collisions
                # (`side` + `left`) collapse to one canonical camera instead.
                if view in seen:
                    raise ValidationError(f"{label}: duplicate view '{view}'")
                seen.add(view)

    elif op_name == "create_ortho_camera":
        _require_name(params, "name", label)
        _optional_name(params, "component", label)
        location = _require_vector(params, "location", label)
        _validate_coordinate_vector(location, "location", label)
        rotation = _optional_coordinate_vector(params, "rotation_deg", label)
        look_at = _optional_coordinate_vector(params, "look_at", label)
        if rotation is not None and look_at is not None:
            raise ValidationError(
                f"{label}: 'rotation_deg' and 'look_at' are mutually exclusive"
            )
        _optional_positive_number(params, "ortho_scale", label)
        _optional_bounded_number(params, "shift_x", -MAX_SHIFT_ABS, MAX_SHIFT_ABS, label)
        _optional_bounded_number(params, "shift_y", -MAX_SHIFT_ABS, MAX_SHIFT_ABS, label)
        clip_start = _optional_positive_number(params, "clip_start", label)
        clip_end = _optional_positive_number(params, "clip_end", label)
        _validate_clip_planes(clip_start, clip_end, label)

    elif op_name == "set_camera_pose":
        _require_name(params, "camera", label)
        if not any(
            params.get(field) is not None
            for field in ("location", "rotation_deg", "look_at")
        ):
            raise ValidationError(
                f"{label}: at least one of 'location', 'rotation_deg' or "
                f"'look_at' is required"
            )
        _optional_coordinate_vector(params, "location", label)
        rotation = _optional_coordinate_vector(params, "rotation_deg", label)
        look_at = _optional_coordinate_vector(params, "look_at", label)
        if rotation is not None and look_at is not None:
            raise ValidationError(
                f"{label}: 'rotation_deg' and 'look_at' are mutually exclusive"
            )

    elif op_name == "set_camera_settings":
        _require_name(params, "camera", label)
        if not any(
            params.get(field) is not None
            for field in ("ortho_scale", "shift_x", "shift_y", "clip_start", "clip_end")
        ):
            raise ValidationError(
                f"{label}: at least one camera setting is required "
                f"(ortho_scale, shift_x, shift_y, clip_start, clip_end)"
            )
        _optional_positive_number(params, "ortho_scale", label)
        _optional_bounded_number(params, "shift_x", -MAX_SHIFT_ABS, MAX_SHIFT_ABS, label)
        _optional_bounded_number(params, "shift_y", -MAX_SHIFT_ABS, MAX_SHIFT_ABS, label)
        clip_start = _optional_positive_number(params, "clip_start", label)
        clip_end = _optional_positive_number(params, "clip_end", label)
        _validate_clip_planes(clip_start, clip_end, label)

    elif op_name == "frame_camera":
        _require_name(params, "camera", label)
        component = _optional_name(params, "component", label)
        objects = params.get("objects")
        if objects is not None and component is not None:
            raise ValidationError(
                f"{label}: at most one of 'objects' and 'component' may be set"
            )
        if objects is not None:
            if not isinstance(objects, (list, tuple)):
                raise ValidationError(f"{label}: 'objects' must be a list")
            if len(objects) == 0:
                raise ValidationError(
                    f"{label}: 'objects' must not be empty "
                    f"(omit it to frame all meshes)"
                )
            for object_name in objects:
                _validate_name_value(object_name, "objects", label)
        _optional_bounded_number(
            params, "margin", MIN_FRAME_MARGIN, MAX_FRAME_MARGIN, label
        )

    elif op_name == "set_render_settings":
        resolution = params.get("resolution")
        if resolution is not None:
            if not isinstance(resolution, (list, tuple)) or len(resolution) != 2:
                raise ValidationError(
                    f"{label}: 'resolution' must have exactly 2 components "
                    f"[width, height]"
                )
            for component in resolution:
                if not isinstance(component, int) or isinstance(component, bool):
                    raise ValidationError(
                        f"{label}: 'resolution' values must be integers"
                    )
                if component < 1 or component > MAX_RESOLUTION:
                    raise ValidationError(
                        f"{label}: 'resolution' must be in the "
                        f"1..={MAX_RESOLUTION} range"
                    )
        _optional_uint(params, "resolution_percentage", 1, 100, label)

    elif op_name == "render_camera":
        _require_name(params, "camera", label)
        path = _require_string(params, "path", label)
        _validate_relative_path(path, "path", label)
        _validate_extension(path, ("png",), "path", label)
        _optional_bool(params, "silhouette", label)

    elif op_name == "render_views":
        path = params.get("path")
        if path is not None:
            _validate_relative_path(path, "path", label)

    elif op_name == "create_component":
        _require_name(params, "name", label)

    elif op_name == "save_blend":
        path = _require_string(params, "path", label)
        _validate_relative_path(path, "path", label)
        _validate_extension(path, ("blend",), "path", label)

    elif op_name == "export_glb":
        path = _require_string(params, "path", label)
        _validate_relative_path(path, "path", label)
        _validate_extension(path, ("glb",), "path", label)

    elif op_name == "get_scene_summary":
        pass

    else:  # pragma: no cover - unreachable: registry and validation agree
        raise ValidationError(
            f"operation: unsupported operation '{op_name}'"
        )


def _is_inside(path, root):
    try:
        Path(path).resolve().relative_to(Path(root).resolve())
        return True
    except (ValueError, OSError):
        return False


def _validate_import_file(relative_path, workspace_root, label):
    root = Path(workspace_root)
    candidate = root / relative_path
    try:
        resolved = candidate.resolve(strict=True)
    except (OSError, RuntimeError) as exc:
        raise ValidationError(
            f"{label}: import file does not exist: '{relative_path}' ({exc})"
        ) from exc
    if not _is_inside(resolved, root):
        raise ValidationError(
            f"{label}: import path resolves outside the workspace: '{relative_path}'"
        )
    if not resolved.is_file():
        raise ValidationError(
            f"{label}: import path is not a regular file: '{relative_path}'"
        )
    try:
        size = resolved.stat().st_size
    except OSError as exc:
        raise ValidationError(
            f"{label}: cannot stat import file '{relative_path}': {exc}"
        ) from exc
    if size <= 0:
        raise ValidationError(f"{label}: import file is empty: '{relative_path}'")


def validate_spec(spec, workspace_root=None):
    """Validate a full spec body before any scene mutation or file write.

    Raises :class:`ValidationError` with an actionable message naming the
    offending operation index and wire name. When `workspace_root` is given,
    `import_mesh` artifacts are also checked for existence/non-emptiness and
    workspace containment.
    """
    if not isinstance(spec, dict):
        raise ValidationError("spec must be a JSON object")
    operations = spec.get("operations")
    if not isinstance(operations, (list, tuple)):
        raise ValidationError("spec.operations must be a list")
    if len(operations) > MAX_OPERATIONS:
        raise ValidationError(
            f"too many operations: {len(operations)} (max {MAX_OPERATIONS})"
        )

    created = set()
    for index, operation in enumerate(operations):
        if not isinstance(operation, dict):
            raise ValidationError(
                f"operation[{index}]: operation must be a JSON object"
            )
        op_name = operation.get("op")
        label = f"operation[{index}] ('{op_name}')"
        if not isinstance(op_name, str) or op_name not in SAFE_OPS:
            raise ValidationError(
                f"operation[{index}]: unsupported operation '{op_name}'"
            )
        _validate_operation(operation, op_name, label)
        for name in _created_names(operation, op_name):
            if name in created:
                raise ValidationError(
                    f"{label}: duplicate created object/camera name '{name}' "
                    f"(already created by an earlier operation)"
                )
            created.add(name)
        if workspace_root is not None and op_name == "import_mesh":
            _validate_import_file(operation["path"], workspace_root, label)

    return operations


def _normalize_workspace_path(path_str):
    """Strip the Windows verbatim prefix (``\\\\?\\`` / ``\\\\?\\UNC\\``).

    The Rust core stores project roots as canonical verbatim paths on Windows.
    Blender itself accepts them, but add-ons — notably the glTF exporter, which
    calls ``os.makedirs`` on its own output path — fail with ``WinError 123`` on
    verbatim paths. Normalizing here keeps every artifact add-on-compatible.
    (The adapter's fake Blender performs the same normalization for PowerShell.)
    """
    if os.name != "nt":
        return path_str
    if path_str.startswith("\\\\?\\UNC\\"):
        return "\\\\" + path_str[len("\\\\?\\UNC\\") :]
    if path_str.startswith("\\\\?\\"):
        return path_str[len("\\\\?\\") :]
    return path_str


def validate_job(job):
    """Validate the worker input envelope.

    Returns `(job_id, resolved_workspace_root, spec)` or raises
    :class:`ValidationError`.
    """
    if not isinstance(job, dict):
        raise ValidationError("job.json must be a JSON object")
    job_id = job.get("job_id")
    if not isinstance(job_id, str) or not JOB_ID_PATTERN.match(job_id):
        raise ValidationError(
            f"invalid job_id: {job_id!r} "
            f"(expected [A-Za-z0-9_-]{{1,{MAX_JOB_ID_CHARS}}})"
        )
    workspace = job.get("workspace_root")
    if not isinstance(workspace, str) or not workspace.strip():
        raise ValidationError("workspace_root must be a non-empty string")
    root = Path(_normalize_workspace_path(workspace))
    if not root.is_absolute():
        raise ValidationError("workspace_root must be an absolute path")
    if not root.is_dir():
        raise ValidationError(
            f"workspace_root is not an existing directory: '{workspace}'"
        )
    spec = job.get("spec")
    if not isinstance(spec, dict):
        raise ValidationError("spec must be a JSON object")
    return job_id, root.resolve(), spec


# ---------------------------------------------------------------------------
# Camera math helpers (pure Python, no bpy; unit-tested outside Blender)
# ---------------------------------------------------------------------------

def world_bbox(points):
    """Axis-aligned world bounding box of `(x, y, z)` points.

    Returns `((min_x, min_y, min_z), (max_x, max_y, max_z))`.
    """
    if not points:
        raise OperationError("cannot compute a bounding box of an empty point set")
    xs = [float(point[0]) for point in points]
    ys = [float(point[1]) for point in points]
    zs = [float(point[2]) for point in points]
    return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))


def bbox_corners(bbox_min, bbox_max):
    """Eight corners of an axis-aligned bounding box."""
    (min_x, min_y, min_z) = bbox_min
    (max_x, max_y, max_z) = bbox_max
    return [
        (x, y, z)
        for x in (min_x, max_x)
        for y in (min_y, max_y)
        for z in (min_z, max_z)
    ]


def ortho_scale_for_frame(width, height, aspect, margin):
    """`ortho_scale = max(width, height * aspect) * margin` (IT-2 contract)."""
    if not _is_finite_number(width) or not _is_finite_number(height):
        raise OperationError("frame bounds must be finite")
    if not _is_finite_number(aspect) or aspect <= 0.0:
        raise OperationError("render aspect ratio must be positive")
    if not _is_finite_number(margin) or margin <= 0.0:
        raise OperationError("frame margin must be positive")
    return max(float(width), float(height) * float(aspect)) * float(margin)


def camera_local_xy_offset(rotation3, center_x, center_y):
    """`R @ (cx, cy, 0)` in camera-local space, using a 3x3 rotation matrix."""
    if len(rotation3) != 3 or any(len(row) != 3 for row in rotation3):
        raise OperationError("rotation matrix must be 3x3")
    return (
        rotation3[0][0] * center_x + rotation3[0][1] * center_y,
        rotation3[1][0] * center_x + rotation3[1][1] * center_y,
        rotation3[2][0] * center_x + rotation3[2][1] * center_y,
    )


def camera_space_bounds(points, world_to_camera):
    """Project world points into camera space; return `(min_x, max_x, min_y, max_y)`."""
    projected = [world_to_camera(point) for point in points]
    xs = [float(point[0]) for point in projected]
    ys = [float(point[1]) for point in projected]
    return min(xs), max(xs), min(ys), max(ys)


# ---------------------------------------------------------------------------
# bpy helpers
# ---------------------------------------------------------------------------

def _bpy():
    import bpy

    return bpy


def _log(message):
    print(f"[michelangelo] {message}", file=sys.stderr, flush=True)


def _write_result(path, data):
    """Atomically write the result JSON file."""
    path_str = str(path)
    tmp = path_str + ".tmp"
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(data, handle, indent=2)
    os.replace(tmp, path_str)


def _remove_stale(path):
    try:
        Path(path).unlink()
    except FileNotFoundError:
        pass


def _workspace_root(ctx):
    return Path(ctx["workspace_root"])


def _prepare_output_path(ctx, relative_path):
    """Create parent dirs and return the absolute output path.

    The parent must resolve inside the workspace (defense against symlinked
    workspace subdirectories escaping the sandbox).
    """
    root = _workspace_root(ctx)
    target = root / relative_path
    target.parent.mkdir(parents=True, exist_ok=True)
    if not _is_inside(target.parent, root):
        raise OperationError(
            f"output path resolves outside the workspace: '{relative_path}'"
        )
    return target


def _prepare_output_dir(ctx, relative_dir):
    root = _workspace_root(ctx)
    target = root / relative_dir
    target.mkdir(parents=True, exist_ok=True)
    if not _is_inside(target, root):
        raise OperationError(
            f"output directory resolves outside the workspace: '{relative_dir}'"
        )
    return target


def _join_artifact_path(base, name):
    return f"{base.replace(chr(92), '/').rstrip('/')}/{name}"


def _require_object(bpy, name):
    obj = bpy.data.objects.get(name)
    if obj is None:
        raise OperationError(f"Object '{name}' not found")
    return obj


def _require_camera(bpy, name):
    obj = _require_object(bpy, name)
    if obj.type != "CAMERA":
        raise OperationError(f"Object '{name}' is not a camera (type {obj.type})")
    return obj


def _look_at_rotation(location, target):
    from mathutils import Vector

    direction = Vector(target) - Vector(location)
    if direction.length == 0.0:
        raise OperationError("look_at target coincides with the camera location")
    return direction.to_track_quat("-Z", "Y").to_euler()


def _euler_degrees(rotation_deg):
    return tuple(math.radians(float(angle)) for angle in rotation_deg)


def _require_free_object_name(bpy, name):
    if name in bpy.data.objects:
        raise OperationError(f"Object name '{name}' already exists")


def _ensure_component(bpy, component):
    collection_name = f"Component_{component}"
    collection = bpy.data.collections.get(collection_name)
    if collection is None:
        collection = bpy.data.collections.new(collection_name)
        bpy.context.scene.collection.children.link(collection)
    return collection


def _link_object(bpy, obj, component=None):
    if component is None:
        bpy.context.scene.collection.objects.link(obj)
    else:
        _ensure_component(bpy, component).objects.link(obj)


def _move_to_component(bpy, obj, component):
    if component is None:
        return
    collection = _ensure_component(bpy, component)
    for user_collection in list(obj.users_collection):
        user_collection.objects.unlink(obj)
    collection.objects.link(obj)


def _select_only(bpy, obj):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj


def _snapshot_render_settings(scene):
    render = scene.render
    shading = scene.display.shading
    return {
        "engine": render.engine,
        "film_transparent": render.film_transparent,
        "resolution_x": render.resolution_x,
        "resolution_y": render.resolution_y,
        "resolution_percentage": render.resolution_percentage,
        "file_format": render.image_settings.file_format,
        "color_mode": render.image_settings.color_mode,
        "color_depth": render.image_settings.color_depth,
        "filepath": render.filepath,
        "shading_light": shading.light,
        "shading_color_type": shading.color_type,
        "shading_single_color": tuple(shading.single_color),
        "camera": scene.camera,
    }


def _restore_render_settings(scene, state):
    render = scene.render
    shading = scene.display.shading
    try:
        render.engine = state["engine"]
        render.film_transparent = state["film_transparent"]
        render.resolution_x = state["resolution_x"]
        render.resolution_y = state["resolution_y"]
        render.resolution_percentage = state["resolution_percentage"]
        render.image_settings.file_format = state["file_format"]
        render.image_settings.color_mode = state["color_mode"]
        render.image_settings.color_depth = state["color_depth"]
        render.filepath = state["filepath"]
        shading.light = state["shading_light"]
        shading.color_type = state["shading_color_type"]
        shading.single_color = state["shading_single_color"]
        scene.camera = state["camera"]
    except Exception as exc:  # noqa: BLE001 - restore must never mask errors
        _log(f"warning: could not fully restore render settings: {exc}")


def _configure_workbench_render(scene, silhouette):
    """Configure Workbench: FLAT shading; MATERIAL color or flat white silhouette."""
    render = scene.render
    render.engine = WORKBENCH_ENGINE
    render.image_settings.file_format = "PNG"
    render.image_settings.color_mode = "RGBA"
    render.image_settings.color_depth = "8"
    shading = scene.display.shading
    shading.light = "FLAT"
    if silhouette:
        shading.color_type = "SINGLE"
        shading.single_color = (1.0, 1.0, 1.0)
        render.film_transparent = True
    else:
        shading.color_type = "MATERIAL"
        render.film_transparent = False


def _render_to_png(scene, camera, output_path, silhouette):
    """Render one camera to a PNG with Workbench; returns the file size."""
    state = _snapshot_render_settings(scene)
    try:
        scene.camera = camera
        _configure_workbench_render(scene, silhouette)
        scene.render.filepath = str(output_path)
        _bpy().ops.render.render(write_still=True)
    finally:
        _restore_render_settings(scene, state)

    if not output_path.is_file():
        raise OperationError(f"render did not produce an output file: '{output_path}'")
    size = output_path.stat().st_size
    if size <= 0:
        raise OperationError(f"render output is empty: '{output_path}'")
    return size


def _reset_scene():
    """Start every job from a deterministic empty scene."""
    bpy = _bpy()
    scene = bpy.context.scene

    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)
    for datablocks in (
        bpy.data.meshes,
        bpy.data.cameras,
        bpy.data.lights,
        bpy.data.materials,
    ):
        for block in list(datablocks):
            try:
                datablocks.remove(block)
            except Exception:  # noqa: BLE001 - best-effort orphan purge
                pass
    for image in list(bpy.data.images):
        try:
            bpy.data.images.remove(image)
        except Exception:  # noqa: BLE001 - e.g. "Render Result" image
            pass

    scene.camera = None
    render = scene.render
    render.engine = WORKBENCH_ENGINE
    render.resolution_x, render.resolution_y = DEFAULT_RENDER_RESOLUTION
    render.resolution_percentage = 100
    render.image_settings.file_format = "PNG"
    render.image_settings.color_mode = "RGBA"
    render.image_settings.color_depth = "8"
    render.film_transparent = False
    _log(
        "Scene reset: deterministic empty scene, engine="
        f"{render.engine}, resolution={render.resolution_x}x{render.resolution_y}"
    )


# ---------------------------------------------------------------------------
# Operation handlers
# ---------------------------------------------------------------------------

def op_create_box(ctx, params):
    bpy = _bpy()
    name = params.get("name") or "Box"
    size = tuple(float(value) for value in params.get("size", DEFAULT_BOX_SIZE))
    location = tuple(float(value) for value in params.get("location", (0.0, 0.0, 0.0)))
    rotation = params.get("rotation_deg")

    _require_free_object_name(bpy, name)
    bpy.ops.mesh.primitive_cube_add(size=2.0, location=location)
    obj = bpy.context.active_object
    obj.name = name
    obj.data.name = f"{name}_mesh"
    obj.scale = (size[0] / 2.0, size[1] / 2.0, size[2] / 2.0)
    _select_only(bpy, obj)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.location = location
    if rotation is not None:
        obj.rotation_euler = _euler_degrees(rotation)
    _move_to_component(bpy, obj, params.get("component"))
    bpy.context.view_layer.update()
    _log(f"Created box '{name}' size={size} location={location}")
    return {
        "object": name,
        "type": "MESH",
        "dimensions": [round(value, 6) for value in obj.dimensions],
    }


def op_create_cylinder(ctx, params):
    bpy = _bpy()
    name = params.get("name") or "Cylinder"
    radius = float(params.get("radius", DEFAULT_RADIUS))
    depth = float(params.get("depth", DEFAULT_DEPTH))
    vertices = int(params.get("vertices", DEFAULT_CYLINDER_VERTICES))
    location = tuple(float(value) for value in params.get("location", (0.0, 0.0, 0.0)))
    rotation = params.get("rotation_deg")

    _require_free_object_name(bpy, name)
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices, radius=radius, depth=depth, location=location
    )
    obj = bpy.context.active_object
    obj.name = name
    obj.data.name = f"{name}_mesh"
    if rotation is not None:
        obj.rotation_euler = _euler_degrees(rotation)
    _move_to_component(bpy, obj, params.get("component"))
    bpy.context.view_layer.update()
    _log(f"Created cylinder '{name}' radius={radius} depth={depth}")
    return {"object": name, "type": "MESH"}


def op_create_sphere(ctx, params):
    bpy = _bpy()
    name = params.get("name") or "Sphere"
    radius = float(params.get("radius", DEFAULT_RADIUS))
    segments = int(params.get("segments", DEFAULT_SPHERE_SEGMENTS))
    ring_count = int(params.get("ring_count", DEFAULT_SPHERE_RINGS))
    location = tuple(float(value) for value in params.get("location", (0.0, 0.0, 0.0)))
    rotation = params.get("rotation_deg")

    _require_free_object_name(bpy, name)
    bpy.ops.mesh.primitive_uv_sphere_add(
        segments=segments,
        ring_count=ring_count,
        radius=radius,
        location=location,
    )
    obj = bpy.context.active_object
    obj.name = name
    obj.data.name = f"{name}_mesh"
    if rotation is not None:
        obj.rotation_euler = _euler_degrees(rotation)
    _move_to_component(bpy, obj, params.get("component"))
    bpy.context.view_layer.update()
    _log(f"Created sphere '{name}' radius={radius}")
    return {"object": name, "type": "MESH"}


def _import_mesh_file(bpy, absolute_path, extension):
    if extension in ("glb", "gltf"):
        bpy.ops.import_scene.gltf(filepath=str(absolute_path))
    elif extension == "obj":
        importer = getattr(bpy.ops.wm, "obj_import", None)
        if importer is not None:
            importer(filepath=str(absolute_path))
        else:  # pragma: no cover - legacy Blender fallback
            bpy.ops.import_scene.obj(filepath=str(absolute_path))
    elif extension == "stl":
        bpy.ops.wm.stl_import(filepath=str(absolute_path))
    elif extension == "ply":
        bpy.ops.wm.ply_import(filepath=str(absolute_path))
    else:  # pragma: no cover - validation rejects other extensions
        raise OperationError(f"Unsupported import extension: '.{extension}'")


def op_import_mesh(ctx, params):
    bpy = _bpy()
    relative_path = params["path"]
    absolute_path = _workspace_root(ctx) / relative_path
    if not absolute_path.is_file():
        raise OperationError(f"import_mesh: file not found: '{relative_path}'")
    extension = _extension(relative_path)

    before = set(bpy.data.objects.keys())
    _import_mesh_file(bpy, absolute_path, extension)
    imported_names = sorted(set(bpy.data.objects.keys()) - before)
    if not imported_names:
        raise OperationError(
            f"import_mesh: no objects imported from '{relative_path}'"
        )
    imported = [bpy.data.objects[name] for name in imported_names]

    if params.get("name") is not None:
        if len(imported) != 1:
            raise OperationError(
                f"import_mesh: 'name' can only be set when exactly one object "
                f"is imported (got {len(imported)})"
            )
        new_name = params["name"]
        _require_free_object_name(bpy, new_name)
        imported[0].name = new_name

    component = params.get("component")
    for obj in imported:
        _move_to_component(bpy, obj, component)
    bpy.context.view_layer.update()
    imported_names = [obj.name for obj in imported]
    _log(f"Imported {len(imported_names)} object(s) from '{relative_path}': {imported_names}")
    return {"path": relative_path, "objects": imported_names}


def op_set_transform(ctx, params):
    bpy = _bpy()
    name = params["object"]
    obj = _require_object(bpy, name)
    if params.get("location") is not None:
        obj.location = tuple(float(value) for value in params["location"])
    if params.get("rotation_deg") is not None:
        obj.rotation_euler = _euler_degrees(params["rotation_deg"])
    if params.get("scale") is not None:
        obj.scale = tuple(float(value) for value in params["scale"])
    applied_scale = bool(params.get("apply_scale", False))
    if applied_scale:
        _select_only(bpy, obj)
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    bpy.context.view_layer.update()
    _log(f"Set transform on '{name}' (apply_scale={applied_scale})")
    return {"object": name, "apply_scale": applied_scale}


def op_set_material(ctx, params):
    bpy = _bpy()
    name = params["object"]
    obj = _require_object(bpy, name)
    raw_color = params.get("color", DEFAULT_MATERIAL_COLOR)
    if len(raw_color) == 3:
        raw_color = tuple(raw_color) + (1.0,)
    color = tuple(float(value) for value in raw_color)

    material = bpy.data.materials.new(name=f"Mat_{name}")
    # `Material.use_nodes` is deprecated and removed in Blender 6.0; only set it
    # on older versions. Blender >= 5 always has a node tree.
    if bpy.app.version < (5, 0, 0):
        material.use_nodes = True
    material.diffuse_color = color
    node_tree = getattr(material, "node_tree", None)
    if node_tree is not None:
        principled = next(
            (node for node in node_tree.nodes if node.type == "BSDF_PRINCIPLED"),
            None,
        )
        if principled is not None:
            socket = principled.inputs.get("Base Color")
            if socket is not None:
                socket.default_value = color

    if obj.data.materials:
        obj.data.materials[0] = material
    else:
        obj.data.materials.append(material)
    _log(f"Applied material '{material.name}' to '{name}' color={color}")
    return {"object": name, "material": material.name}


def op_set_camera_set(ctx, params):
    bpy = _bpy()
    requested_views = params.get("views")
    views = resolve_legacy_views(requested_views)
    created = []
    for view in views:
        name = f"Cam_{view}"
        _require_free_object_name(bpy, name)
        view_def = LEGACY_VIEW_DEFS[view]
        camera_data = bpy.data.cameras.new(name)
        camera_obj = bpy.data.objects.new(name, camera_data)
        bpy.context.scene.collection.objects.link(camera_obj)
        camera_obj.location = view_def["location"]
        camera_obj.rotation_euler = _euler_degrees(view_def["rotation"])
        created.append(view)
        _log(f"Created legacy camera '{name}'")
    bpy.context.view_layer.update()
    _log(
        f"Legacy camera set requested={requested_views!r} "
        f"-> created={created!r}"
    )
    return {"cameras_created": created}


def op_create_ortho_camera(ctx, params):
    bpy = _bpy()
    name = params["name"]
    location = tuple(float(value) for value in params["location"])
    _require_free_object_name(bpy, name)

    camera_data = bpy.data.cameras.new(name)
    camera_data.type = "ORTHO"
    camera_data.sensor_fit = "HORIZONTAL"
    camera_data.ortho_scale = float(params.get("ortho_scale", DEFAULT_ORTHO_SCALE))
    camera_data.shift_x = float(params.get("shift_x", 0.0))
    camera_data.shift_y = float(params.get("shift_y", 0.0))
    camera_data.clip_start = float(params.get("clip_start", DEFAULT_CLIP_START))
    camera_data.clip_end = float(params.get("clip_end", DEFAULT_CLIP_END))

    camera_obj = bpy.data.objects.new(name, camera_data)
    _link_object(bpy, camera_obj, params.get("component"))
    camera_obj.location = location
    if params.get("look_at") is not None:
        camera_obj.rotation_euler = _look_at_rotation(location, params["look_at"])
    elif params.get("rotation_deg") is not None:
        camera_obj.rotation_euler = _euler_degrees(params["rotation_deg"])
    bpy.context.view_layer.update()
    _log(
        f"Created ortho camera '{name}' loc={location} "
        f"ortho_scale={camera_data.ortho_scale}"
    )
    return {
        "camera": name,
        "type": "ORTHO",
        "ortho_scale": camera_data.ortho_scale,
    }


def op_set_camera_pose(ctx, params):
    bpy = _bpy()
    camera_obj = _require_camera(bpy, params["camera"])
    if params.get("location") is not None:
        camera_obj.location = tuple(float(value) for value in params["location"])
    if params.get("look_at") is not None:
        camera_obj.rotation_euler = _look_at_rotation(
            camera_obj.location, params["look_at"]
        )
    elif params.get("rotation_deg") is not None:
        camera_obj.rotation_euler = _euler_degrees(params["rotation_deg"])
    bpy.context.view_layer.update()
    _log(f"Updated camera pose for '{camera_obj.name}'")
    return {"camera": camera_obj.name}


def op_set_camera_settings(ctx, params):
    bpy = _bpy()
    camera_obj = _require_camera(bpy, params["camera"])
    camera_data = camera_obj.data
    if params.get("ortho_scale") is not None:
        if camera_data.type != "ORTHO":
            raise OperationError(
                f"set_camera_settings: 'ortho_scale' requires an orthographic "
                f"camera ('{camera_obj.name}' is {camera_data.type})"
            )
        camera_data.ortho_scale = float(params["ortho_scale"])
    if params.get("shift_x") is not None:
        camera_data.shift_x = float(params["shift_x"])
    if params.get("shift_y") is not None:
        camera_data.shift_y = float(params["shift_y"])
    clip_start = (
        float(params["clip_start"])
        if params.get("clip_start") is not None
        else float(camera_data.clip_start)
    )
    clip_end = (
        float(params["clip_end"])
        if params.get("clip_end") is not None
        else float(camera_data.clip_end)
    )
    if clip_end <= clip_start:
        raise OperationError(
            f"set_camera_settings: 'clip_end' must be greater than 'clip_start' "
            f"(got {clip_start}..{clip_end})"
        )
    camera_data.clip_start = clip_start
    camera_data.clip_end = clip_end
    bpy.context.view_layer.update()
    _log(f"Updated camera settings for '{camera_obj.name}'")
    return {"camera": camera_obj.name, "ortho_scale": camera_data.ortho_scale}


def op_frame_camera(ctx, params):
    from mathutils import Vector

    bpy = _bpy()
    camera_obj = _require_camera(bpy, params["camera"])
    if camera_obj.data.type != "ORTHO":
        raise OperationError(
            f"frame_camera requires an orthographic camera "
            f"('{camera_obj.name}' is {camera_obj.data.type})"
        )

    if params.get("objects") is not None:
        objects = [_require_object(bpy, name) for name in params["objects"]]
        description = f"{len(objects)} object(s)"
    elif params.get("component") is not None:
        collection = bpy.data.collections.get(f"Component_{params['component']}")
        if collection is None:
            raise OperationError(
                f"frame_camera: component '{params['component']}' does not exist"
            )
        objects = list(collection.objects)
        description = f"component '{params['component']}'"
    else:
        objects = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
        description = "all meshes"
    if not objects:
        raise OperationError(f"frame_camera: no objects to frame ({description})")

    world_points = []
    for obj in objects:
        for corner in obj.bound_box:
            world_points.append(tuple(obj.matrix_world @ Vector(corner)))
    bbox_min, bbox_max = world_bbox(world_points)

    bpy.context.view_layer.update()
    inverse_matrix = camera_obj.matrix_world.inverted()
    camera_points = [inverse_matrix @ Vector(point) for point in bbox_corners(bbox_min, bbox_max)]
    min_x, max_x, min_y, max_y = camera_space_bounds(
        camera_points, lambda point: tuple(point)
    )
    center_x = (min_x + max_x) / 2.0
    center_y = (min_y + max_y) / 2.0
    width = max_x - min_x
    height = max_y - min_y

    render = bpy.context.scene.render
    aspect = render.resolution_x / render.resolution_y
    margin = float(params.get("margin", DEFAULT_FRAME_MARGIN))
    camera_obj.data.ortho_scale = ortho_scale_for_frame(
        width, height, aspect, margin
    )

    rotation_euler = camera_obj.rotation_euler
    rotation_matrix = rotation_euler.to_matrix()
    rotation3 = [[float(rotation_matrix[row][column]) for column in range(3)] for row in range(3)]
    offset = camera_local_xy_offset(rotation3, center_x, center_y)
    camera_obj.location = Vector(camera_obj.location) + Vector(offset)
    bpy.context.view_layer.update()

    _log(
        f"Framed camera '{camera_obj.name}' on {description}: "
        f"ortho_scale={camera_obj.data.ortho_scale:.6f} "
        f"center=({center_x:.6f}, {center_y:.6f})"
    )
    return {
        "camera": camera_obj.name,
        "ortho_scale": camera_obj.data.ortho_scale,
        "objects": [obj.name for obj in objects],
        "center": [center_x, center_y],
    }


def op_set_render_settings(ctx, params):
    bpy = _bpy()
    render = bpy.context.scene.render
    if params.get("resolution") is not None:
        width, height = (int(value) for value in params["resolution"])
        render.resolution_x = width
        render.resolution_y = height
    if params.get("resolution_percentage") is not None:
        render.resolution_percentage = int(params["resolution_percentage"])
    render.engine = WORKBENCH_ENGINE
    render.image_settings.file_format = "PNG"
    _log(
        f"Render settings: {render.resolution_x}x{render.resolution_y} "
        f"({render.resolution_percentage}%), engine={render.engine}"
    )
    return {
        "resolution": [render.resolution_x, render.resolution_y],
        "resolution_percentage": render.resolution_percentage,
        "engine": render.engine,
    }


def op_render_camera(ctx, params):
    bpy = _bpy()
    scene = bpy.context.scene
    camera_obj = _require_camera(bpy, params["camera"])
    relative_path = params["path"]
    silhouette = bool(params.get("silhouette", False))

    output_path = _prepare_output_path(ctx, relative_path)
    _remove_stale(output_path)
    size = _render_to_png(scene, camera_obj, output_path, silhouette)
    artifact = {
        "path": relative_path,
        "type": "png",
        "size_bytes": size,
    }
    _log(
        f"Rendered camera '{camera_obj.name}' to '{relative_path}' "
        f"(silhouette={silhouette}, {size} bytes)"
    )
    return {
        "camera": camera_obj.name,
        "path": relative_path,
        "silhouette": silhouette,
        "artifact": artifact,
    }


def op_render_views(ctx, params):
    bpy = _bpy()
    scene = bpy.context.scene
    base_path = params.get("path", DEFAULT_RENDER_BASE)
    output_dir = _prepare_output_dir(ctx, base_path)

    cameras = sorted(
        (obj for obj in scene.objects if obj.type == "CAMERA"),
        key=lambda obj: obj.name,
    )
    if not cameras:
        raise OperationError("render_views: no cameras to render")

    artifacts = []
    rendered = []
    state = _snapshot_render_settings(scene)
    try:
        for camera_obj in cameras:
            view_name = camera_obj.name
            if view_name.startswith("Cam_"):
                view_name = view_name[len("Cam_") :]
            if not view_name:
                view_name = camera_obj.name
            output_path = output_dir / f"{view_name}.png"
            _remove_stale(output_path)
            size = _render_to_png(scene, camera_obj, output_path, silhouette=False)
            relative_path = _join_artifact_path(base_path, f"{view_name}.png")
            artifact = {
                "path": relative_path,
                "type": "png",
                "size_bytes": size,
            }
            artifacts.append(artifact)
            rendered.append(
                {
                    "view": view_name,
                    "path": relative_path,
                    "size_bytes": size,
                }
            )
            _log(f"Rendered view '{view_name}' to '{relative_path}'")
    finally:
        _restore_render_settings(scene, state)
    return {"rendered": rendered, "artifacts": artifacts}


def op_create_component(ctx, params):
    bpy = _bpy()
    collection = _ensure_component(bpy, params["name"])
    _log(f"Ensured component collection '{collection.name}'")
    return {"component": params["name"], "collection": collection.name}


def op_save_blend(ctx, params):
    bpy = _bpy()
    relative_path = params["path"]
    output_path = _prepare_output_path(ctx, relative_path)
    _remove_stale(output_path)
    try:
        bpy.ops.wm.save_as_mainfile(filepath=str(output_path))
    except Exception as exc:  # noqa: BLE001 - surface a worker-level failure
        raise OperationError(f"save_blend failed: {exc}") from exc
    if not output_path.is_file():
        raise OperationError(f"save_blend did not produce '{relative_path}'")
    size = output_path.stat().st_size
    if size <= 0:
        raise OperationError(f"save_blend produced an empty file: '{relative_path}'")
    artifact = {"path": relative_path, "type": "blend", "size_bytes": size}
    _log(f"Saved .blend to '{relative_path}' ({size} bytes)")
    return {"path": relative_path, "size_bytes": size, "artifact": artifact}


def op_export_glb(ctx, params):
    bpy = _bpy()
    relative_path = params["path"]
    if not hasattr(bpy.ops.export_scene, "gltf"):
        raise OperationError(
            "export_glb: bpy.ops.export_scene.gltf is not available in this "
            "Blender installation"
        )
    output_path = _prepare_output_path(ctx, relative_path)
    _remove_stale(output_path)
    try:
        bpy.ops.export_scene.gltf(filepath=str(output_path), export_format="GLB")
    except Exception as exc:  # noqa: BLE001 - surface a worker-level failure
        raise OperationError(f"export_glb failed: {exc}") from exc
    if not output_path.is_file():
        raise OperationError(f"export_glb did not produce '{relative_path}'")
    size = output_path.stat().st_size
    if size <= 0:
        raise OperationError(f"export_glb produced an empty file: '{relative_path}'")
    artifact = {"path": relative_path, "type": "glb", "size_bytes": size}
    _log(f"Exported GLB to '{relative_path}' ({size} bytes)")
    return {"path": relative_path, "size_bytes": size, "artifact": artifact}


def op_get_scene_summary(ctx, params):
    from mathutils import Vector

    bpy = _bpy()
    scene = bpy.context.scene
    objects = sorted(scene.objects, key=lambda obj: obj.name)
    meshes = [obj for obj in objects if obj.type == "MESH"]

    total_triangles = 0
    total_vertices = 0
    world_points = []
    for mesh_obj in meshes:
        mesh = mesh_obj.data
        mesh.calc_loop_triangles()
        total_triangles += len(mesh.loop_triangles)
        total_vertices += len(mesh.vertices)
        for corner in mesh_obj.bound_box:
            world_points.append(tuple(mesh_obj.matrix_world @ Vector(corner)))

    components = []
    for collection in sorted(bpy.data.collections, key=lambda item: item.name):
        components.append(
            {
                "name": collection.name,
                "objects": sorted(obj.name for obj in collection.objects),
            }
        )

    summary = {
        "objects": len(objects),
        "meshes": len(meshes),
        "materials": len(bpy.data.materials),
        "cameras": len([obj for obj in objects if obj.type == "CAMERA"]),
        "triangles": total_triangles,
        "vertices": total_vertices,
        "components": components,
    }
    if world_points:
        bbox_min, bbox_max = world_bbox(world_points)
        summary["bounds"] = {
            "min": [round(value, 6) for value in bbox_min],
            "max": [round(value, 6) for value in bbox_max],
        }
    _log(f"Scene summary: {summary}")
    return summary


OPERATIONS = {
    "create_box": op_create_box,
    "set_material": op_set_material,
    "set_camera_set": op_set_camera_set,
    "render_views": op_render_views,
    "export_glb": op_export_glb,
    "save_blend": op_save_blend,
    "get_scene_summary": op_get_scene_summary,
    "create_cylinder": op_create_cylinder,
    "create_sphere": op_create_sphere,
    "import_mesh": op_import_mesh,
    "set_transform": op_set_transform,
    "create_ortho_camera": op_create_ortho_camera,
    "set_camera_pose": op_set_camera_pose,
    "set_camera_settings": op_set_camera_settings,
    "frame_camera": op_frame_camera,
    "set_render_settings": op_set_render_settings,
    "render_camera": op_render_camera,
    "create_component": op_create_component,
}

if tuple(OPERATIONS.keys()) != SAFE_OPS_ORDER:  # pragma: no cover - parity guard
    raise RuntimeError(
        "worker operation registry is out of sync with SAFE_OPS_ORDER: "
        f"{tuple(OPERATIONS.keys())!r}"
    )


# ---------------------------------------------------------------------------
# Capabilities probe mode
# ---------------------------------------------------------------------------

def _detect_render_engines(bpy):
    """Return the render engines accepted by the current Blender build.

    Assigning `scene.render.engine` raises for engines that are not
    registered; assigning is the only reliable cross-version probe.
    """
    scene = bpy.context.scene
    previous = scene.render.engine
    available = []
    for engine in RENDER_ENGINE_CANDIDATES:
        try:
            scene.render.engine = engine
        except Exception:  # noqa: BLE001 - invalid enum / missing engine
            continue
        if scene.render.engine == engine:
            available.append(engine)
    try:
        scene.render.engine = previous
    except Exception:  # noqa: BLE001 - best effort restore
        pass
    return available


def _capabilities_payload(bpy):
    return {
        "operations": list(SAFE_OPS_ORDER),
        "blender_version": f"Blender {bpy.app.version_string}",
        "python_version": platform.python_version(),
        "gltf_export_available": hasattr(bpy.ops.export_scene, "gltf"),
        "render_engines": _detect_render_engines(bpy),
    }


def _emit_capabilities():
    """Print one machine-readable capabilities line to stdout and exit 0."""
    bpy = _bpy()
    payload = _capabilities_payload(bpy)
    sys.stdout.write(CAPABILITIES_MARKER + json.dumps(payload) + "\n")
    sys.stdout.flush()


# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

def payload_args(argv):
    """Return the worker payload arguments after the `--` separator."""
    try:
        separator_index = argv.index("--")
    except ValueError:
        return list(argv[1:])
    return list(argv[separator_index + 1 :])


def _read_job_json(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def _collect_artifacts(result, artifacts):
    if isinstance(result, dict):
        artifact = result.get("artifact")
        if isinstance(artifact, dict):
            artifacts.append(artifact)
        extra = result.get("artifacts")
        if isinstance(extra, list):
            artifacts.extend(extra)


def run_job(args):
    """Execute one job file pair; returns the process exit code."""
    if len(args) < 2:
        _log(
            "usage: blender --background --factory-startup --python "
            "worker.py -- <job.json> <result.json>"
        )
        return 1

    job_path = Path(args[0])
    result_path = Path(args[1])

    job = None
    try:
        job = _read_job_json(job_path)
        job_id, workspace_root, spec = validate_job(job)
        operations = validate_spec(spec, workspace_root=workspace_root)
    except Exception as exc:  # noqa: BLE001 - report any initialization failure
        reported_id = "unknown"
        if isinstance(job, dict) and isinstance(job.get("job_id"), str):
            reported_id = job["job_id"]
        error = f"Invalid job: {exc}"
        _log(error)
        _write_result(
            result_path,
            {
                "job_id": reported_id,
                "status": "failed",
                "error": error,
                "message": "Worker initialization failed",
                "artifacts": [],
                "scene_summary": None,
            },
        )
        return 1

    artifacts = []
    scene_summary = None
    current_op = None
    _log(f"Starting job {job_id} with {len(operations)} operation(s)")

    try:
        _reset_scene()
        for operation in operations:
            current_op = operation["op"]
            handler = OPERATIONS[current_op]
            result = handler({"workspace_root": str(workspace_root)}, operation)
            _collect_artifacts(result, artifacts)
            if current_op == "get_scene_summary":
                scene_summary = result
            _log(f"  ✓ {current_op}")
    except Exception as exc:  # noqa: BLE001 - any handler failure fails the job
        traceback.print_exc(file=sys.stderr)
        error = (
            f"Operation '{current_op}' failed: {exc}"
            if current_op is not None
            else f"Worker failed: {exc}"
        )
        _log(f"  ✗ {error}")
        _write_result(
            result_path,
            {
                "job_id": job_id,
                "status": "failed",
                "error": error,
                "message": f"Error during {current_op or 'worker initialization'}",
                "artifacts": artifacts,
                "scene_summary": scene_summary,
            },
        )
        return 1

    _log(f"Job {job_id} completed successfully")
    _write_result(
        result_path,
        {
            "job_id": job_id,
            "status": "completed",
            "message": f"Executed {len(operations)} operation(s)",
            "artifacts": artifacts,
            "scene_summary": scene_summary,
        },
    )
    return 0


def main(argv=None):
    argv = list(sys.argv if argv is None else argv)
    args = payload_args(argv)
    # Capabilities probe mode must run before any job/result file handling.
    if "--capabilities" in args:
        _emit_capabilities()
        return 0
    return run_job(args)


if __name__ == "__main__":
    sys.exit(main())
