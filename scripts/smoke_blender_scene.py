#!/usr/bin/env python3
"""Cross-platform end-to-end acceptance smoke for Michelangelo safe Blender scene ops.

One Python 3 (stdlib only) driver, runnable on Windows and Linux without `just`:

    python scripts/smoke_blender_scene.py [--fake] [--blender-bin PATH]
                                          [--cli PATH] [--keep]
                                          [--timeout-ms N] [--require-blender]

Real mode runs the acceptance JSONL scenario against the configured Blender
(`--blender-bin` > `BLENDER_BIN` > `blender` on `PATH`). `--fake` mode generates
a platform-appropriate fake Blender executable in a temporary directory and sets
`BLENDER_BIN` for the CLI; it validates protocol plumbing and the artifact
contract without rendering real pixels.

What is validated (single CLI process, then a second process for reopen):

* every stdout line is a JSON protocol message; response/event classification;
* one response per request id, no protocol error envelopes;
* `project.create` workspace, `blender.get_capabilities` (binary/bpy/18 ops);
* job lifecycle events: exactly `job.queued` -> `job.started` -> one terminal
  `job.completed`, all for the same job id, with no `job.failed`/`job.cancelled`,
  and each event written before the `blender.run_job` response;
* an invalid-spec request returning `-32602` with no job record/events;
* `blender.run_job` response status, persisted job identity and timestamps;
* artifact metadata (five expected paths, types, sizes) and artifact files:
  `.blend` header (`BLENDER` or zstd), `.glb` magic/version/length, PNG
  dimensions/bit depth/color type;
* the calibrated orthographic front silhouette (real Blender only): exactly 16384
  pixels with alpha > 127 and bbox x 128..383, y 224..287 for the documented
  512x512 fixture;
* the framed scene silhouette (real Blender only): `cam_scene` starts at
  `ortho_scale=100`, so a working `frame_camera` is required for meaningful alpha
  coverage; the driver asserts coverage fraction, bbox span and a border margin;
* scene summary structure (3 meshes, 2 cameras, named components, 1 material);
* `job.list` persistence and `project.get_snapshot` agreement;
* a second CLI process (`project.open` + `job.list` + `project.get_snapshot`)
  showing the completed job with `started_at`/`finished_at` and artifacts.

Exit codes:

* 0 — all checks passed, or an explicit SKIP because Blender is unavailable;
* 1 — validation failure (job failed, bad JSONL, failed pixel assertions, ...);
* 2 — setup/precondition error (missing CLI, `--require-blender` without Blender).

The composite scene itself lives in
`examples/scene_ops/composite_scene_spec.json`; this script consumes that file so
the documented example and the acceptance scenario cannot drift.
"""

import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import traceback
import zlib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CLI = (
    REPO_ROOT / "target" / "debug" / ("michelangelo.exe" if os.name == "nt" else "michelangelo")
)
ACCEPTANCE_SPEC_PATH = REPO_ROOT / "examples" / "scene_ops" / "composite_scene_spec.json"
DEFAULT_JOB_TYPE = "blender_acceptance_scene"
BUILD_HINT = "cargo build -p michelangelo_cli"

# Expected artifacts of the acceptance scene (path -> artifact type).
EXPECTED_ARTIFACTS = {
    "blender/renders/front_silhouette.png": "png",
    "blender/renders/scene.png": "png",
    "blender/renders/scene_silhouette.png": "png",
    "blender/scenes/scene.blend": "blend",
    "blender/exports/scene.glb": "glb",
}

EXPECTED_PNG = (512, 512, 8, 6)  # width, height, bit depth, color type (RGBA)
SILHOUETTE_ALPHA_THRESHOLD = 127
SILHOUETTE_COUNT = 16384
SILHOUETTE_BBOX = (128, 383, 224, 287)  # min_x, max_x, min_y, max_y

# Framed-scene silhouette assertions (real Blender only).
#
# The acceptance scene deliberately creates `cam_scene` with `ortho_scale=100`
# and then calls `frame_camera cam_scene margin=1.1`. A correct frame_camera
# rewrites ortho_scale to roughly the scene extent times 1.1, so the scene
# silhouette fills most of the 512x512 frame (~83% x 49% bbox, measured ~10%
# opaque pixels; the scene is three separated objects, so the opaque area is
# smaller than its bounding box). If frame_camera were missing or broken,
# `ortho_scale=100` would make the whole scene a few dozen pixels wide: the
# measured counterfactual (same scene without frame_camera) collapses to ~0.03%
# coverage and a 25x15 px bbox, which these assertions must reject.
FRAMING_COVERAGE_MIN_FRACTION = 0.03  # measured ~0.10 framed, ~0.0003 unframed
FRAMING_COVERAGE_MAX_FRACTION = 0.95
FRAMING_MIN_SPAN_FRACTION = 0.40  # measured ~83%/49% framed, ~5% unframed
FRAMING_EDGE_MARGIN_PX = 8  # content must not touch the frame border

EXPECTED_WORKER_OPS = [
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
]

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"


# ---------------------------------------------------------------------------
# Fake Blender implementation (written into the temp dir by --fake)
# ---------------------------------------------------------------------------

FAKE_IMPL = r'''#!/usr/bin/env python3
"""Deterministic fake Blender used by scripts/smoke_blender_scene.py --fake.

It is NOT a Blender replacement. It understands exactly the CLI adapter's
invocations:

* ``fake_blender --version`` -> one version line on stdout, exit 0;
* ``fake_blender ... -- --capabilities`` -> one MICHELANGELO_CAPABILITIES JSON
  marker line on stdout, exit 0;
* ``fake_blender ... -- <job.json> <result.json>`` -> reads job.json, creates
  the artifacts named by the spec file operations under ``workspace_root``,
  echoes the requested ``job_id`` and writes a ``completed`` result.json.

PNG artifacts are real 512x512 8-bit RGBA PNGs (transparent); no scene is
rendered. Artifact header/size validation and protocol/artifact checks are still
exercised end to end.
"""
import json
import os
import re
import struct
import sys
import zlib
from pathlib import Path

MARKER = "MICHELANGELO_CAPABILITIES:"
OPERATIONS = [
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
]
LEGACY_ALIASES = {"side": "left"}
DEFAULT_VIEWS = ["front", "side", "top", "perspective"]
DEFAULT_RESOLUTION = (512, 512)
JOB_ID_PATTERN = re.compile(r"\A[A-Za-z0-9_-]{1,128}\Z")


def log(message):
    sys.stderr.write("[fake-blender] " + message + "\n")
    sys.stderr.flush()


def _png_chunk(kind, body):
    return (
        struct.pack(">I", len(body))
        + kind
        + body
        + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
    )


def write_png(path, width, height):
    raw = b"".join(b"\x00" + b"\x00\x00\x00\x00" * width for _ in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    data = (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", ihdr)
        + _png_chunk(b"IDAT", zlib.compress(raw))
        + _png_chunk(b"IEND", b"")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def write_glb(path):
    body = b"{}"
    padding = (4 - len(body) % 4) % 4
    json_chunk = struct.pack("<I", len(body) + padding) + b"JSON" + body + b" " * padding
    total = 12 + len(json_chunk)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"glTF" + struct.pack("<II", 2, total) + json_chunk)


def write_blend(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"BLENDER-fake-smoke-blend\n" * 4)


def _relative_path(value):
    if not isinstance(value, str) or not value:
        raise ValueError("path must be a non-empty string")
    if value.startswith("/") or value.startswith("\\") or (len(value) >= 2 and value[1] == ":"):
        raise ValueError("path must be workspace-relative: %r" % value)
    parts = Path(value).parts
    if any(part == ".." for part in parts):
        raise ValueError("path must not contain '..': %r" % value)
    return Path(value)


def _write_result(result_path, payload):
    text = json.dumps(payload, ensure_ascii=False)
    tmp = result_path.with_name(result_path.name + ".tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, result_path)


def run_job(job_path, result_path):
    job = json.loads(job_path.read_text(encoding="utf-8"))
    job_id = job.get("job_id")
    if not isinstance(job_id, str) or not JOB_ID_PATTERN.match(job_id):
        log("invalid job_id")
        return 2
    workspace_root = Path(job.get("workspace_root") or "")
    if not workspace_root.is_dir():
        _write_result(
            result_path,
            {"job_id": job_id, "status": "failed", "error": "workspace_root not found", "artifacts": []},
        )
        return 1

    spec = job.get("spec") or {}
    operations = spec.get("operations") if isinstance(spec, dict) else None
    if not isinstance(operations, list):
        _write_result(
            result_path,
            {"job_id": job_id, "status": "failed", "error": "spec.operations missing", "artifacts": []},
        )
        return 1

    resolution = DEFAULT_RESOLUTION
    objects = []  # every scene object (meshes + cameras)
    meshes = 0
    materials = 0
    cameras = 0
    components = {}

    def ensure_component(name):
        if isinstance(name, str) and name not in components:
            components[name] = []

    def add_material(op):
        nonlocal materials
        if op.get("object") is not None:
            materials += 1

    artifacts = []
    try:
        for index, op in enumerate(operations):
            if not isinstance(op, dict):
                raise ValueError("operation[%d] is not an object" % index)
            name = op.get("op")
            if name not in OPERATIONS:
                raise ValueError("unknown operation %r" % name)

            if name == "create_component":
                ensure_component(op.get("name"))
            elif name in ("create_box", "create_cylinder", "create_sphere"):
                default = {"create_box": "Box", "create_cylinder": "Cylinder", "create_sphere": "Sphere"}[name]
                object_name = op.get("name") or default
                objects.append(object_name)
                meshes += 1
                component = op.get("component")
                ensure_component(component)
                if component is not None:
                    components[component].append(object_name)
            elif name == "import_mesh":
                object_name = op.get("name") or ("imported_%d" % index)
                objects.append(object_name)
                meshes += 1
                component = op.get("component")
                ensure_component(component)
                if component is not None:
                    components[component].append(object_name)
            elif name == "create_ortho_camera":
                objects.append(op.get("name") or ("Camera_%d" % index))
                cameras += 1
            elif name == "set_camera_set":
                requested = op.get("views") or DEFAULT_VIEWS
                canonical = []
                for view in requested:
                    canonical.append(LEGACY_ALIASES.get(view, view))
                cameras += len(set(canonical))
                objects.extend("Cam_" + view for view in sorted(set(canonical)))
            elif name == "set_material":
                add_material(op)
            elif name == "set_render_settings":
                requested_resolution = op.get("resolution")
                if isinstance(requested_resolution, list) and len(requested_resolution) == 2:
                    resolution = (int(requested_resolution[0]), int(requested_resolution[1]))
            elif name == "render_camera":
                relative = _relative_path(op.get("path"))
                target = workspace_root / relative
                write_png(target, resolution[0], resolution[1])
                artifacts.append(
                    {"path": relative.as_posix(), "type": "png", "size_bytes": target.stat().st_size}
                )
            elif name == "render_views":
                if cameras == 0:
                    raise ValueError("render_views: no cameras to render")
                base = op.get("path") or "blender/renders"
                base_relative = _relative_path(base + "/fake_view.png")
                target = workspace_root / base_relative
                write_png(target, resolution[0], resolution[1])
                artifacts.append(
                    {"path": base_relative.as_posix(), "type": "png", "size_bytes": target.stat().st_size}
                )
            elif name == "save_blend":
                relative = _relative_path(op.get("path"))
                target = workspace_root / relative
                write_blend(target)
                artifacts.append(
                    {"path": relative.as_posix(), "type": "blend", "size_bytes": target.stat().st_size}
                )
            elif name == "export_glb":
                relative = _relative_path(op.get("path"))
                target = workspace_root / relative
                write_glb(target)
                artifacts.append(
                    {"path": relative.as_posix(), "type": "glb", "size_bytes": target.stat().st_size}
                )
            # Remaining operations (set_transform, set_camera_pose, ...) do not
            # produce artifacts; a plumbing fake does not emulate their effects.
    except Exception as exc:  # noqa: BLE001 - report as a failed worker result
        log("job failed: %s" % exc)
        _write_result(
            result_path,
            {
                "job_id": job_id,
                "status": "failed",
                "message": "Fake Blender failed",
                "error": str(exc),
                "artifacts": artifacts,
                "scene_summary": None,
            },
        )
        return 1

    summary = {
        "objects": len(objects),
        "meshes": meshes,
        "materials": materials,
        "cameras": cameras,
        "triangles": 0,
        "vertices": 0,
        "components": [
            {"name": "Component_" + component_name, "objects": sorted(members)}
            for component_name, members in sorted(components.items())
        ],
    }
    if objects:
        summary["bounds"] = {"min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0]}

    _write_result(
        result_path,
        {
            "job_id": job_id,
            "status": "completed",
            "message": "Fake Blender completed %d operation(s)" % len(operations),
            "artifacts": artifacts,
            "scene_summary": summary,
        },
    )
    return 0


def main(argv):
    args = list(argv)
    if "--version" in args:
        sys.stdout.write("Blender 5.2.3 (fake smoke)\n")
        sys.stdout.flush()
        return 0
    if "--capabilities" in args:
        payload = {
            "operations": list(OPERATIONS),
            "blender_version": "Blender 5.2.3 (fake smoke)",
            "python_version": "%d.%d.%d" % sys.version_info[:3],
            "gltf_export_available": True,
            "render_engines": ["BLENDER_WORKBENCH"],
        }
        sys.stdout.write(MARKER + json.dumps(payload) + "\n")
        sys.stdout.flush()
        return 0
    if "--" in args:
        payload_args = args[args.index("--") + 1 :]
    else:
        payload_args = args
    if len(payload_args) < 2:
        log("usage: fake_blender ... -- <job.json> <result.json>")
        return 2
    return run_job(Path(payload_args[0]), Path(payload_args[1]))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
'''


# ---------------------------------------------------------------------------
# Check reporting
# ---------------------------------------------------------------------------


class Checker:
    """Collect PASS/FAIL/SKIP lines and exit-code-relevant failures."""

    def __init__(self):
        self.passed = 0
        self.failed = []
        self.skipped = []

    def ok(self, name, detail=""):
        self.passed += 1
        suffix = " — " + detail if detail else ""
        print("PASS: " + name + suffix)

    def fail(self, name, detail=""):
        self.failed.append(name)
        suffix = " — " + detail if detail else ""
        print("FAIL: " + name + suffix)

    def check(self, name, condition, detail=""):
        if condition:
            self.ok(name, detail)
        else:
            self.fail(name, detail)
        return bool(condition)

    def skip(self, name, reason):
        self.skipped.append(name)
        print("SKIP: " + name + " — " + reason)


def _short(value, limit=400):
    text = value if isinstance(value, str) else repr(value)
    if len(text) > limit:
        return text[:limit] + "...<truncated>"
    return text


def _dict(value):
    return value if isinstance(value, dict) else {}


def _list(value):
    return value if isinstance(value, list) else []


# ---------------------------------------------------------------------------
# Process helpers
# ---------------------------------------------------------------------------


def program_command(program):
    """Return the argv prefix for a program path (Windows .cmd/.bat wrapper)."""
    program_str = str(program)
    if os.name == "nt" and program_str.lower().endswith((".cmd", ".bat")):
        return ["cmd", "/C", "call", program_str]
    return [program_str]


def run_cli(cli, requests, env, timeout_s):
    payload = "\n".join(json.dumps(request, ensure_ascii=False) for request in requests) + "\n"
    return subprocess.run(
        [str(cli), "core", "stdio"],
        input=payload.encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        cwd=str(REPO_ROOT),
        timeout=timeout_s,
    )


def parse_protocol(proc, label, expected_ids, checker, expected_error_ids=()):
    """Validate JSONL structure/responses and return (messages, raw lines).

    `expected_error_ids` names requests that are intentionally invalid and must
    receive a JSON-RPC error envelope instead of a result (used for the negative
    invalid-spec check); every other response must be a success envelope.
    """
    raw = proc.stdout.decode("utf-8", errors="replace")
    lines = [line for line in raw.splitlines() if line.strip()]
    messages = []
    parse_ok = True
    for index, line in enumerate(lines, 1):
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as exc:
            checker.fail(f"{label}: stdout line {index} is valid JSON", str(exc))
            parse_ok = False
            continue
        if not isinstance(obj, dict):
            checker.fail(f"{label}: stdout line {index} is a JSON object", _short(line))
            parse_ok = False
            continue
        messages.append(obj)
    if parse_ok:
        checker.ok(f"{label}: all {len(lines)} stdout lines are JSON")

    stderr_text = proc.stderr.decode("utf-8", errors="replace")
    checker.check(
        f"{label}: process exited 0",
        proc.returncode == 0,
        f"exit={proc.returncode}, stderr={_short(stderr_text)}",
    )

    responses = [msg for msg in messages if "id" in msg]
    unrecognized = [
        msg for msg in messages if "event" not in msg and "id" not in msg
    ]
    checker.check(
        f"{label}: every message is an event or a response",
        not unrecognized,
        f"{len(unrecognized)} unrecognized",
    )

    response_ids = [response.get("id") for response in responses]
    checker.check(
        f"{label}: one response per request",
        sorted(response_ids) == sorted(expected_ids),
        f"ids={response_ids}",
    )
    checker.check(
        f"{label}: no duplicate response ids",
        len(response_ids) == len(set(response_ids)),
        str(response_ids),
    )
    expected_error_ids = set(expected_error_ids)
    unexpected_errors = [
        {"id": response.get("id"), "error": response.get("error")}
        for response in responses
        if "error" in response and response.get("id") not in expected_error_ids
    ]
    checker.check(
        f"{label}: no unexpected protocol error responses",
        not unexpected_errors,
        _short(unexpected_errors),
    )
    missing_expected_errors = [
        request_id
        for request_id in expected_error_ids
        if not any(
            response.get("id") == request_id and "error" in response
            for response in responses
        )
    ]
    if expected_error_ids:
        # Only meaningful when a negative request was actually sent; do not emit
        # a vacuous PASS for ordinary request streams (e.g. the reopen process).
        checker.check(
            f"{label}: intentionally invalid requests returned protocol errors",
            not missing_expected_errors,
            f"missing error envelopes: {missing_expected_errors}",
        )
    return messages, lines


def response_envelope(messages, request_id):
    for message in messages:
        if message.get("id") == request_id:
            return message
    return {}


def response_result(messages, request_id):
    return response_envelope(messages, request_id).get("result")


def message_index(messages, predicate):
    for index, message in enumerate(messages):
        if predicate(message):
            return index
    return -1


def artifact_map(artifacts):
    mapping = {}
    for artifact in _list(artifacts):
        if isinstance(artifact, dict) and isinstance(artifact.get("path"), str):
            mapping[artifact["path"]] = artifact
    return mapping


def validate_artifact_metadata(checker, label, artifacts, check_disk, workspace):
    """Check the artifact list against the expected acceptance artifacts."""
    mapping = artifact_map(artifacts)
    checker.check(
        f"{label}: lists exactly the {len(EXPECTED_ARTIFACTS)} expected artifacts",
        set(mapping) == set(EXPECTED_ARTIFACTS),
        f"got {sorted(mapping)}",
    )
    for relative_path, expected_type in EXPECTED_ARTIFACTS.items():
        artifact = mapping.get(relative_path, {})
        size = artifact.get("size_bytes")
        metadata_ok = (
            artifact.get("type") == expected_type
            and isinstance(size, int)
            and size > 0
        )
        checker.check(
            f"{label}: artifact metadata for {relative_path}",
            metadata_ok,
            _short(artifact),
        )
        if check_disk:
            target = workspace / relative_path
            exists = target.is_file()
            actual_size = target.stat().st_size if exists else -1
            checker.check(
                f"{label}: artifact file exists for {relative_path}",
                exists and actual_size > 0,
                f"{actual_size} bytes",
            )
            if isinstance(size, int):
                checker.check(
                    f"{label}: reported size matches disk for {relative_path}",
                    actual_size == size,
                    f"reported {size}, actual {actual_size}",
                )
    return mapping


# ---------------------------------------------------------------------------
# PNG decoding (stdlib only; filters 0-4, non-interlaced, 8-bit RGBA)
# ---------------------------------------------------------------------------


def decode_png(data):
    """Return (width, height, bit_depth, color_type, rows) for an 8-bit RGBA PNG."""
    if data[:8] != PNG_SIGNATURE:
        raise ValueError("bad PNG signature")
    position = 8
    header = None
    idat = bytearray()
    saw_end = False
    while position + 8 <= len(data):
        length = struct.unpack(">I", data[position : position + 4])[0]
        chunk_type = data[position + 4 : position + 8]
        chunk = data[position + 8 : position + 8 + length]
        if len(chunk) != length:
            raise ValueError("truncated PNG chunk")
        position += 12 + length
        if chunk_type == b"IHDR":
            if length != 13:
                raise ValueError("invalid IHDR length")
            header = struct.unpack(">IIBBBBB", chunk)
        elif chunk_type == b"IDAT":
            idat.extend(chunk)
        elif chunk_type == b"IEND":
            saw_end = True
            break
    if header is None:
        raise ValueError("missing IHDR")
    if not saw_end:
        raise ValueError("missing IEND")
    width, height, bit_depth, color_type, _compression, _filter, interlace = header
    if interlace != 0:
        raise ValueError("interlaced PNG is not supported")
    if bit_depth != 8 or color_type != 6:
        raise ValueError(
            f"expected 8-bit RGBA, got bit_depth={bit_depth} color_type={color_type}"
        )
    raw = zlib.decompress(bytes(idat))
    bytes_per_pixel = 4
    stride = width * bytes_per_pixel
    if len(raw) != height * (stride + 1):
        raise ValueError(
            f"unexpected decompressed size {len(raw)} (expected {height * (stride + 1)})"
        )
    rows = []
    previous = bytearray(stride)
    offset = 0
    for _y in range(height):
        filter_type = raw[offset]
        offset += 1
        line = bytearray(raw[offset : offset + stride])
        offset += stride
        if filter_type == 1:  # Sub
            for i in range(bytes_per_pixel, stride):
                line[i] = (line[i] + line[i - bytes_per_pixel]) & 0xFF
        elif filter_type == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_type == 3:  # Average
            for i in range(stride):
                left = line[i - bytes_per_pixel] if i >= bytes_per_pixel else 0
                line[i] = (line[i] + ((left + previous[i]) // 2)) & 0xFF
        elif filter_type == 4:  # Paeth
            for i in range(stride):
                left = line[i - bytes_per_pixel] if i >= bytes_per_pixel else 0
                above = previous[i]
                upper_left = previous[i - bytes_per_pixel] if i >= bytes_per_pixel else 0
                estimate = left + above - upper_left
                pa = abs(estimate - left)
                pb = abs(estimate - above)
                pc = abs(estimate - upper_left)
                predictor = left if (pa <= pb and pa <= pc) else (above if pb <= pc else upper_left)
                line[i] = (line[i] + predictor) & 0xFF
        elif filter_type != 0:
            raise ValueError(f"unsupported PNG filter type {filter_type}")
        rows.append(bytes(line))
        previous = line
    return width, height, bit_depth, color_type, rows


def silhouette_stats(rows, width):
    count = 0
    min_x = min_y = None
    max_x = max_y = None
    for y, row in enumerate(rows):
        for x in range(width):
            if row[x * 4 + 3] > SILHOUETTE_ALPHA_THRESHOLD:
                count += 1
                if min_x is None:
                    min_x = max_x = x
                    min_y = max_y = y
                else:
                    if x < min_x:
                        min_x = x
                    if x > max_x:
                        max_x = x
                    if y < min_y:
                        min_y = y
                    if y > max_y:
                        max_y = y
    bbox = None if min_x is None else (min_x, max_x, min_y, max_y)
    return count, bbox


# ---------------------------------------------------------------------------
# Fake / real Blender setup
# ---------------------------------------------------------------------------


def create_fake_blender(directory):
    """Generate the platform-specific fake Blender + shared fake_impl.py."""
    python_executable = (
        sys.executable or shutil.which("python3") or shutil.which("python")
    )
    if not python_executable:
        raise RuntimeError("no Python executable found for the fake Blender wrapper")
    impl_path = directory / "fake_impl.py"
    with open(impl_path, "w", encoding="utf-8", newline="") as handle:
        handle.write(FAKE_IMPL)
    if os.name == "nt":
        wrapper = directory / "fake_blender.cmd"
        with open(wrapper, "w", encoding="utf-8", newline="") as handle:
            handle.write(
                "@echo off\r\n"
                f'"{python_executable}" "%~dp0fake_impl.py" %*\r\n'
                "exit /b %ERRORLEVEL%\r\n"
            )
        return wrapper
    wrapper = directory / "fake_blender.sh"
    with open(wrapper, "w", encoding="utf-8", newline="") as handle:
        handle.write(
            "#!/bin/sh\n"
            f'exec "{python_executable}" "$(dirname "$0")/fake_impl.py" "$@"\n'
        )
    wrapper.chmod(0o755)
    return wrapper


def resolve_real_blender(explicit):
    if explicit:
        return explicit
    return os.environ.get("BLENDER_BIN") or "blender"


def probe_blender(binary):
    """Return (available, version_line, error) for the configured Blender."""
    try:
        proc = subprocess.run(
            program_command(binary) + ["--version"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=60,
            cwd=str(REPO_ROOT),
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, None, str(exc)
    output = proc.stdout.decode("utf-8", errors="replace")
    version_line = next((line.strip() for line in output.splitlines() if line.strip()), None)
    if proc.returncode != 0 or not version_line:
        stderr_text = proc.stderr.decode("utf-8", errors="replace").strip()
        return False, None, f"exit={proc.returncode}, stderr={_short(stderr_text)}"
    return True, version_line, None


# ---------------------------------------------------------------------------
# Acceptance scenario
# ---------------------------------------------------------------------------


def load_acceptance_spec():
    data = json.loads(ACCEPTANCE_SPEC_PATH.read_text(encoding="utf-8"))
    if isinstance(data, list):
        return DEFAULT_JOB_TYPE, data
    if not isinstance(data, dict):
        raise ValueError("acceptance spec must be an object or a list of operations")
    operations = data.get("operations")
    if not isinstance(operations, list) or not operations:
        raise ValueError("acceptance spec must contain a non-empty 'operations' list")
    job_type = data.get("job_type") or DEFAULT_JOB_TYPE
    return job_type, operations


def validate_job_events(checker, messages, job_id, job_type, response_artifacts, response_id="job"):
    events = [message for message in messages if "event" in message]
    checker.check(
        "job: all lifecycle events carry a job id",
        all(
            isinstance(_dict(message.get("data")).get("job_id"), str)
            for message in events
        ),
        _short(events[:3]),
    )
    names = [message.get("event") for message in events]
    checker.check(
        "job: events belong to the acceptance job",
        all(_dict(message.get("data")).get("job_id") == job_id for message in events),
        f"job_id={job_id}, events={names}",
    )
    checker.check(
        "job: exactly one job.queued event", names.count("job.queued") == 1, str(names)
    )
    checker.check(
        "job: exactly one job.started event", names.count("job.started") == 1, str(names)
    )
    terminals = [name for name in names if name in ("job.completed", "job.failed", "job.cancelled")]
    checker.check(
        "job: exactly one terminal event", len(terminals) == 1, str(names)
    )
    checker.check(
        "job: terminal event is job.completed",
        terminals == ["job.completed"],
        str(terminals),
    )
    checker.check("job: no job.failed event", "job.failed" not in names, str(names))
    checker.check("job: no job.cancelled event", "job.cancelled" not in names, str(names))
    if all(name in ("job.queued", "job.started", "job.completed") for name in names):
        checker.check(
            "job: event order queued -> started -> completed",
            names == ["job.queued", "job.started", "job.completed"],
            str(names),
        )
    else:
        checker.fail("job: event order queued -> started -> completed", str(names))
    for message in events:
        data = _dict(message.get("data"))
        if message.get("event") in ("job.queued", "job.started", "job.completed"):
            checker.check(
                f"job: {message.get('event')} carries job_type",
                data.get("job_type") == job_type,
                f"job_type={data.get('job_type')!r}",
            )

    # Terminal events must be emitted before the blender.run_job response and the
    # events must precede the response of the request that produced them.
    queued_index = message_index(
        messages,
        lambda message: message.get("event") == "job.queued"
        and _dict(message.get("data")).get("job_id") == job_id,
    )
    started_index = message_index(
        messages,
        lambda message: message.get("event") == "job.started"
        and _dict(message.get("data")).get("job_id") == job_id,
    )
    terminal_index = message_index(
        messages,
        lambda message: message.get("event") in ("job.completed", "job.failed", "job.cancelled")
        and _dict(message.get("data")).get("job_id") == job_id,
    )
    response_index = message_index(messages, lambda message: message.get("id") == response_id)
    ordering_ok = (
        queued_index >= 0
        and started_index > queued_index
        and terminal_index > started_index
        and response_index > terminal_index
    )
    checker.check(
        "job: events are ordered before the blender.run_job response",
        ordering_ok,
        f"queued={queued_index}, started={started_index}, "
        f"terminal={terminal_index}, response={response_index}",
    )

    completed_events = [m for m in events if m.get("event") == "job.completed"]
    if completed_events:
        event_artifacts = artifact_map(_dict(completed_events[0].get("data")).get("artifacts"))
        checker.check(
            "job: job.completed event lists the expected artifacts",
            set(event_artifacts) == set(EXPECTED_ARTIFACTS),
            f"got {sorted(event_artifacts)}",
        )
        event_metadata = {
            path: (artifact.get("type"), artifact.get("size_bytes"))
            for path, artifact in event_artifacts.items()
        }
        response_metadata = {
            path: (artifact.get("type"), artifact.get("size_bytes"))
            for path, artifact in artifact_map(response_artifacts).items()
        }
        checker.check(
            "job: terminal event artifact metadata matches the response output_json",
            event_metadata == response_metadata,
            f"event={_short(event_metadata)}, response={_short(response_metadata)}",
        )


def validate_summary(checker, label, summary, fake_mode):
    summary = _dict(summary)
    checker.check(
        f"{label}: scene summary objects=5", summary.get("objects") == 5, _short(summary)
    )
    checker.check(
        f"{label}: scene summary meshes=3", summary.get("meshes") == 3, _short(summary)
    )
    checker.check(
        f"{label}: scene summary cameras=2", summary.get("cameras") == 2, _short(summary)
    )
    checker.check(
        f"{label}: scene summary materials=1", summary.get("materials") == 1, _short(summary)
    )
    components = _list(summary.get("components"))
    component_names = {
        _dict(component).get("name") for component in components if isinstance(component, dict)
    }
    checker.check(
        f"{label}: scene summary lists Component_body and Component_extra",
        component_names >= {"Component_body", "Component_extra"},
        _short(sorted(str(name) for name in component_names)),
    )
    if not fake_mode:
        checker.check(
            f"{label}: scene summary has bounds",
            isinstance(summary.get("bounds"), dict),
            _short(summary.get("bounds")),
        )


def check_artifact_files(checker, workspace, fake_mode):
    blend_path = workspace / "blender/scenes/scene.blend"
    glb_path = workspace / "blender/exports/scene.glb"
    silhouette_path = workspace / "blender/renders/front_silhouette.png"
    scene_path = workspace / "blender/renders/scene.png"
    scene_silhouette_path = workspace / "blender/renders/scene_silhouette.png"

    blend = blend_path.read_bytes() if blend_path.is_file() else b""
    checker.check(
        ".blend header is BLENDER or zstd",
        blend.startswith(b"BLENDER") or blend[:4] == ZSTD_MAGIC,
        blend[:8].hex(),
    )

    glb = glb_path.read_bytes() if glb_path.is_file() else b""
    glb_ok = len(glb) >= 12 and glb[:4] == b"glTF"
    version = struct.unpack("<I", glb[4:8])[0] if len(glb) >= 12 else -1
    declared_length = struct.unpack("<I", glb[8:12])[0] if len(glb) >= 12 else -1
    checker.check(
        ".glb magic is glTF and version is 2",
        glb_ok and version == 2,
        f"magic={glb[:4]!r}, version={version}",
    )
    checker.check(
        ".glb declared length matches file size",
        declared_length == len(glb),
        f"declared {declared_length}, actual {len(glb)}",
    )

    for label, path in (
        ("front_silhouette.png", silhouette_path),
        ("scene.png", scene_path),
        ("scene_silhouette.png", scene_silhouette_path),
    ):
        data = path.read_bytes() if path.is_file() else b""
        try:
            width, height, bit_depth, color_type, rows = decode_png(data)
        except Exception as exc:  # noqa: BLE001 - report the decoding problem
            checker.fail(f"{label}: decodable 8-bit RGBA PNG", str(exc))
            continue
        checker.check(
            f"{label}: 512x512 8-bit RGBA PNG",
            (width, height, bit_depth, color_type) == EXPECTED_PNG,
            f"{width}x{height} bit={bit_depth} color_type={color_type}",
        )
        if label == "front_silhouette.png":
            if fake_mode:
                checker.skip(
                    "front_silhouette.png: calibrated alpha assertions",
                    "fake mode renders no pixels",
                )
            else:
                count, bbox = silhouette_stats(rows, width)
                checker.check(
                    "front_silhouette.png: exactly 16384 pixels with alpha > 127",
                    count == SILHOUETTE_COUNT,
                    f"count={count}",
                )
                checker.check(
                    "front_silhouette.png: bbox is (128, 383, 224, 287)",
                    bbox == SILHOUETTE_BBOX,
                    f"bbox={bbox}",
                )
        elif label == "scene.png":
            # The normal Workbench render is opaque by definition; only its PNG
            # header is validated above. Framing is proven by scene_silhouette.png.
            pass
        else:
            if fake_mode:
                checker.skip(
                    "scene_silhouette.png: framed-scene alpha assertions",
                    "fake mode renders no pixels",
                )
            else:
                # `cam_scene` starts at ortho_scale=100; these assertions only
                # pass when `frame_camera` actually reframed the camera. A
                # broken frame_camera leaves the scene tiny (near-zero coverage).
                count, bbox = silhouette_stats(rows, width)
                total_pixels = width * height
                fraction = count / total_pixels if total_pixels else 0.0
                checker.check(
                    "scene_silhouette.png: alpha coverage fraction is within "
                    f"{FRAMING_COVERAGE_MIN_FRACTION:.2f}..{FRAMING_COVERAGE_MAX_FRACTION:.2f}",
                    FRAMING_COVERAGE_MIN_FRACTION <= fraction <= FRAMING_COVERAGE_MAX_FRACTION,
                    f"fraction={fraction:.4f} ({count} px)",
                )
                if bbox is None:
                    checker.fail(
                        "scene_silhouette.png: alpha bbox exists (frame_camera did not produce content)"
                    )
                else:
                    min_x, max_x, min_y, max_y = bbox
                    span_x = max_x - min_x + 1
                    span_y = max_y - min_y + 1
                    checker.check(
                        "scene_silhouette.png: alpha bbox spans a meaningful part of the frame",
                        span_x >= FRAMING_MIN_SPAN_FRACTION * width
                        and span_y >= FRAMING_MIN_SPAN_FRACTION * height,
                        f"span={span_x}x{span_y} (min "
                        f"{FRAMING_MIN_SPAN_FRACTION * width:.0f}px)",
                    )
                    checker.check(
                        "scene_silhouette.png: alpha bbox stays inside the frame margin",
                        min_x >= FRAMING_EDGE_MARGIN_PX
                        and min_y >= FRAMING_EDGE_MARGIN_PX
                        and max_x <= width - 1 - FRAMING_EDGE_MARGIN_PX
                        and max_y <= height - 1 - FRAMING_EDGE_MARGIN_PX,
                        f"bbox={bbox}, margin={FRAMING_EDGE_MARGIN_PX}px",
                    )


def validate_list_and_snapshot(checker, label, list_result, snapshot_result, expected_job_id):
    jobs = _list(_dict(list_result).get("jobs"))
    checker.check(f"{label}: job.list returns one persisted job", len(jobs) == 1, str(len(jobs)))
    record = _dict(jobs[0]) if jobs else {}
    checker.check(
        f"{label}: persisted job id matches response",
        record.get("id") == expected_job_id,
        f"{record.get('id')!r} vs {expected_job_id!r}",
    )
    checker.check(
        f"{label}: persisted job status is completed",
        record.get("status") == "completed",
        str(record.get("status")),
    )
    checker.check(
        f"{label}: persisted job has started_at",
        isinstance(record.get("started_at"), str) and bool(record.get("started_at")),
        str(record.get("started_at")),
    )
    checker.check(
        f"{label}: persisted job has finished_at",
        isinstance(record.get("finished_at"), str) and bool(record.get("finished_at")),
        str(record.get("finished_at")),
    )
    persisted_output = record.get("output_json")
    try:
        persisted_artifacts = (
            json.loads(persisted_output).get("artifacts") if isinstance(persisted_output, str) else []
        )
    except json.JSONDecodeError:
        persisted_artifacts = []
    validate_artifact_metadata(
        checker, f"{label}: persisted", persisted_artifacts, False, None
    )

    snapshot = _dict(snapshot_result)
    project = _dict(snapshot.get("project"))
    checker.check(
        f"{label}: snapshot has project name",
        project.get("name") == "acceptance-scene",
        str(project.get("name")),
    )
    snapshot_jobs = _list(snapshot.get("jobs"))
    checker.check(
        f"{label}: snapshot returns one persisted job",
        len(snapshot_jobs) == 1,
        str(len(snapshot_jobs)),
    )
    snapshot_job = _dict(snapshot_jobs[0]) if snapshot_jobs else {}
    checker.check(
        f"{label}: snapshot job id matches",
        snapshot_job.get("id") == expected_job_id,
        f"{snapshot_job.get('id')!r}",
    )
    checker.check(
        f"{label}: snapshot job status is completed",
        snapshot_job.get("status") == "completed",
        str(snapshot_job.get("status")),
    )
    checker.check(
        f"{label}: snapshot job started_at agrees with job.list",
        snapshot_job.get("started_at") == record.get("started_at"),
        f"{snapshot_job.get('started_at')!r} vs {record.get('started_at')!r}",
    )
    checker.check(
        f"{label}: snapshot completed_at agrees with job.list finished_at",
        snapshot_job.get("completed_at") == record.get("finished_at"),
        f"{snapshot_job.get('completed_at')!r} vs {record.get('finished_at')!r}",
    )
    snapshot_artifacts = artifact_map(snapshot_job.get("artifacts"))
    checker.check(
        f"{label}: snapshot artifacts agree with job.list artifacts",
        set(snapshot_artifacts) == set(artifact_map(persisted_artifacts)),
        f"{sorted(snapshot_artifacts)}",
    )
    return record


def validate_job_directory(checker, workspace, job_id):
    job_dir = workspace / "jobs" / job_id
    job_json = job_dir / "job.json"
    result_json = job_dir / "result.json"
    checker.check("job directory: job.json persisted", job_json.is_file(), str(job_json))
    checker.check("job directory: result.json persisted", result_json.is_file(), str(result_json))
    if not result_json.is_file():
        return
    try:
        result = json.loads(result_json.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        checker.fail("job directory: result.json parses", str(exc))
        return
    checker.check(
        "job directory: result.json echoes the job id",
        result.get("job_id") == job_id,
        str(result.get("job_id")),
    )
    checker.check(
        "job directory: result.json status is completed",
        result.get("status") == "completed",
        str(result.get("status")),
    )
    checker.check(
        "job directory: stdout/stderr logs exist",
        (job_dir / "stdout.log").is_file() and (job_dir / "stderr.log").is_file(),
        str(job_dir),
    )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def parse_args(argv):
    parser = argparse.ArgumentParser(
        description=(
            "Michelangelo acceptance smoke: composite Blender scene through JSONL "
            "(real Blender or a generated fake)."
        ),
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "--fake",
        action="store_true",
        help="use a generated fake Blender (no real Blender required)",
    )
    parser.add_argument(
        "--blender-bin",
        default=None,
        help="path to the Blender executable (real mode only)",
    )
    parser.add_argument(
        "--cli",
        default=None,
        help="path to the michelangelo CLI binary",
    )
    parser.add_argument(
        "--keep",
        action="store_true",
        help="keep the temporary workspace and fake directory",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        default=300_000,
        help="per-job Blender timeout passed to blender.run_job",
    )
    parser.add_argument(
        "--require-blender",
        action="store_true",
        help="exit 2 instead of 0 when real Blender is unavailable",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    checker = Checker()
    mode = "fake" if args.fake else "real"
    workspace = None
    fake_dir = None
    process_one_lines = []
    exit_code = 1

    try:
        print(f"=== Michelangelo acceptance smoke ({mode} mode) ===")
        if args.fake and args.require_blender:
            print("NOTE: --require-blender is ignored in --fake mode (no real Blender involved).")
        if args.fake and args.blender_bin:
            print("NOTE: --blender-bin is ignored in --fake mode (a fake Blender is generated).")

        cli = Path(args.cli).resolve() if args.cli else DEFAULT_CLI
        print(f"Repo root: {REPO_ROOT}")
        print(f"CLI: {cli}")
        if not cli.is_file():
            print(f"ERROR: CLI not found at '{cli}'.")
            print(f"  Build it first: {BUILD_HINT}")
            return 2

        if not ACCEPTANCE_SPEC_PATH.is_file():
            print(f"ERROR: acceptance spec not found at '{ACCEPTANCE_SPEC_PATH}'.")
            return 2
        try:
            job_type, operations = load_acceptance_spec()
        except (ValueError, json.JSONDecodeError) as exc:
            print(f"ERROR: cannot load acceptance spec: {exc}")
            return 2
        print(f"Acceptance spec: {ACCEPTANCE_SPEC_PATH} ({len(operations)} operations)")

        env = dict(os.environ)
        blender_version = None
        if args.fake:
            fake_dir = Path(tempfile.mkdtemp(prefix="michelangelo-fake-blender-"))
            fake_blender = create_fake_blender(fake_dir)
            env["BLENDER_BIN"] = str(fake_blender)
            print(f"Blender: fake executable at {fake_blender}")
        else:
            blender_binary = resolve_real_blender(args.blender_bin)
            available, blender_version, probe_error = probe_blender(blender_binary)
            if not available:
                print(
                    "SKIP: Blender is not available "
                    f"('{blender_binary}'): {probe_error}"
                )
                print(
                    "SKIP: install Blender 5.x, set BLENDER_BIN or pass --blender-bin; "
                    "use --fake for the deterministic protocol/artifact smoke."
                )
                if args.require_blender:
                    print("ERROR: --require-blender was passed but Blender is unavailable.")
                    return 2
                print("SKIP: acceptance smoke skipped (Blender unavailable).")
                return 0
            if args.blender_bin:
                env["BLENDER_BIN"] = blender_binary
            print(f"Blender: {blender_version}")
            print(f"Blender binary: {blender_binary}")

        workspace = Path(tempfile.mkdtemp(prefix="michelangelo-acceptance-"))
        print(f"Workspace: {workspace}")
        print()

        job_timeout_ms = max(1, int(args.timeout_ms))
        outer_timeout_s = max(120, job_timeout_ms // 1000 + 120)
        requests_one = [
            {
                "id": "create",
                "method": "project.create",
                "params": {"path": str(workspace), "name": "acceptance-scene"},
            },
            {"id": "caps", "method": "blender.get_capabilities", "params": {}},
            # Negative protocol check: an invalid spec must be rejected with
            # -32602 before any job record/event exists (no orphan job).
            {
                "id": "bad",
                "method": "blender.run_job",
                "params": {
                    "job_type": "invalid_acceptance_spec",
                    "wait": True,
                    "spec": {
                        "operations": [
                            {"op": "create_box", "name": "bad_box", "size": [1.0, 2.0]}
                        ]
                    },
                },
            },
            {
                "id": "job",
                "method": "blender.run_job",
                "params": {
                    "job_type": job_type,
                    "wait": True,
                    "timeout_ms": job_timeout_ms,
                    "spec": {"operations": operations},
                },
            },
            {"id": "jobs", "method": "job.list", "params": {}},
            {"id": "snapshot", "method": "project.get_snapshot", "params": {}},
        ]
        try:
            proc_one = run_cli(cli, requests_one, env, outer_timeout_s)
        except subprocess.TimeoutExpired:
            checker.fail("run: CLI process finished within the timeout", f"{outer_timeout_s}s")
            return 1
        except OSError as exc:
            print(f"ERROR: cannot run the CLI: {exc}")
            return 1

        messages, process_one_lines = parse_protocol(
            proc_one,
            "run",
            ["create", "caps", "bad", "job", "jobs", "snapshot"],
            checker,
            expected_error_ids=["bad"],
        )

        # -- project.create ---------------------------------------------------
        create_result = _dict(response_result(messages, "create"))
        checker.check(
            "project.create: returns the acceptance workspace name",
            create_result.get("name") == "acceptance-scene",
            _short(create_result),
        )
        checker.check(
            "project.create: returns a workspace root path",
            isinstance(create_result.get("root_path"), str) and bool(create_result.get("root_path")),
            _short(create_result.get("root_path")),
        )

        # -- capabilities -----------------------------------------------------
        capabilities = _dict(response_result(messages, "caps"))
        checker.check(
            "capabilities: binary_found is true", capabilities.get("binary_found") is True,
            _short(capabilities),
        )
        checker.check(
            "capabilities: bpy_available is true", capabilities.get("bpy_available") is True,
            _short(capabilities),
        )
        capabilities_version = capabilities.get("blender_version")
        if args.fake:
            checker.check(
                "capabilities: Blender version is reported",
                isinstance(capabilities_version, str) and bool(capabilities_version),
                str(capabilities_version),
            )
            checker.skip(
                "capabilities: real Blender version string", "fake mode reports a fake version"
            )
        else:
            checker.check(
                "capabilities: Blender version starts with 'Blender'",
                isinstance(capabilities_version, str)
                and capabilities_version.startswith("Blender"),
                str(capabilities_version),
            )
        supported_operations = _list(capabilities.get("supported_operations"))
        missing_operations = [
            operation for operation in EXPECTED_WORKER_OPS if operation not in supported_operations
        ]
        checker.check(
            "capabilities: all 18 safe worker operations are advertised",
            not missing_operations,
            f"missing={missing_operations}, got {len(supported_operations)}",
        )

        # -- negative protocol request: invalid spec must not create a job -----
        bad_response = response_envelope(messages, "bad")
        bad_error = _dict(bad_response.get("error"))
        bad_message = bad_error.get("message") or ""
        checker.check(
            "invalid spec: returns -32602",
            bad_error.get("code") == -32602,
            _short(bad_response),
        )
        checker.check(
            "invalid spec: response carries no result",
            "result" not in bad_response,
            _short(bad_response),
        )
        checker.check(
            "invalid spec: error names the offending operation",
            "operation[0]" in bad_message and "create_box" in bad_message,
            _short(bad_message),
        )
        first_event_index = message_index(messages, lambda message: "event" in message)
        bad_response_index = message_index(messages, lambda message: message.get("id") == "bad")
        checker.check(
            "invalid spec: no lifecycle event was emitted for the rejected request",
            bad_response_index >= 0
            and (first_event_index == -1 or bad_response_index < first_event_index),
            f"bad_response={bad_response_index}, first_event={first_event_index}",
        )

        # -- blender.run_job response ----------------------------------------
        job_result = _dict(response_result(messages, "job"))
        checker.check(
            "blender.run_job: response status is completed",
            job_result.get("status") == "completed",
            _short(job_result.get("error") or job_result.get("status")),
        )
        job = _dict(job_result.get("job"))
        job_id = job.get("id")
        checker.check(
            "blender.run_job: returns the persisted job",
            isinstance(job_id, str) and bool(job_id),
            _short(job),
        )
        checker.check(
            "blender.run_job: persisted job status is completed",
            job.get("status") == "completed",
            str(job.get("status")),
        )
        checker.check(
            "blender.run_job: persisted job has started_at",
            isinstance(job.get("started_at"), str) and bool(job.get("started_at")),
            str(job.get("started_at")),
        )
        checker.check(
            "blender.run_job: persisted job has finished_at",
            isinstance(job.get("finished_at"), str) and bool(job.get("finished_at")),
            str(job.get("finished_at")),
        )

        output_json = job.get("output_json")
        output = {}
        try:
            if isinstance(output_json, str):
                output = json.loads(output_json)
        except json.JSONDecodeError:
            output = {}
        checker.check(
            "blender.run_job: output_json is valid JSON",
            isinstance(output, dict) and bool(output),
            _short(output_json),
        )
        validate_artifact_metadata(
            checker, "run: output_json", output.get("artifacts"), True, workspace
        )
        validate_summary(checker, "run", output.get("scene_summary"), args.fake)

        if isinstance(job_id, str) and job_id:
            validate_job_events(
                checker,
                messages,
                job_id,
                job_type,
                output.get("artifacts"),
            )
        else:
            checker.fail("job: lifecycle events can be related to the job id", "no job id")

        # -- job.list / snapshot (same process) ------------------------------
        validate_list_and_snapshot(
            checker,
            "run",
            response_result(messages, "jobs"),
            response_result(messages, "snapshot"),
            job_id,
        )

        # -- artifacts: headers and calibrated silhouette --------------------
        check_artifact_files(checker, workspace, args.fake)

        # -- process 2: reopen across the process boundary -------------------
        print()
        requests_two = [
            {"id": "open", "method": "project.open", "params": {"path": str(workspace)}},
            {"id": "jobs", "method": "job.list", "params": {}},
            {"id": "snapshot", "method": "project.get_snapshot", "params": {}},
        ]
        try:
            proc_two = run_cli(cli, requests_two, env, outer_timeout_s)
        except subprocess.TimeoutExpired:
            checker.fail("reopen: CLI process finished within the timeout", f"{outer_timeout_s}s")
            exit_code = 1
        else:
            messages_two, _lines_two = parse_protocol(
                proc_two, "reopen", ["open", "jobs", "snapshot"], checker
            )
            open_result = _dict(response_result(messages_two, "open"))
            checker.check(
                "reopen: project.open returns the acceptance workspace name",
                open_result.get("name") == "acceptance-scene",
                _short(open_result),
            )
            validate_list_and_snapshot(
                checker,
                "reopen",
                response_result(messages_two, "jobs"),
                response_result(messages_two, "snapshot"),
                job_id,
            )
            if isinstance(job_id, str) and job_id:
                validate_job_directory(checker, workspace, job_id)

        print()
        if checker.failed:
            print(
                f"FAILED {len(checker.failed)} check(s): "
                + ", ".join(checker.failed)
            )
            print("--- raw JSONL tail (first CLI process) ---")
            for line in process_one_lines[-20:]:
                print(_short(line, 2000))
            exit_code = 1
        else:
            suffix = f" ({blender_version})" if blender_version and not args.fake else ""
            print(
                f"ACCEPTANCE PASSED: {checker.passed} checks, "
                f"{len(checker.skipped)} skipped{suffix}"
            )
            exit_code = 0
    except Exception:  # noqa: BLE001 - never crash without a clear FAIL line
        traceback.print_exc()
        print("FAIL: unexpected error in the acceptance smoke driver")
        exit_code = 1
    finally:
        if workspace is not None:
            if args.keep:
                print(f"KEPT workspace: {workspace}")
            else:
                shutil.rmtree(workspace, ignore_errors=True)
        if fake_dir is not None:
            if args.keep:
                print(f"KEPT fake Blender dir: {fake_dir}")
            else:
                shutil.rmtree(fake_dir, ignore_errors=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
