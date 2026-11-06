# Blender Worker Contract

Contract for `blender/worker.py` — the bounded batch Blender executor used by
`michelangelo_blender::HeadlessBlenderAdapter`. The worker is a thin `bpy` executor; it makes no
business decisions, never talks to the LLM/Asset Graph and only executes the typed operation
whitelist below.

Status: implemented and verified on **Blender 5.2.3 LTS** (Windows, headless) with bundled
Python 3.13. API surface is plausible on Blender 4.2 LTS, but only 5.2.3 is verified in this
iteration.

## Invocation

Real job:

```bash
blender --background --factory-startup --python blender/worker.py -- <job.json> <result.json>
```

Capabilities probe (lightweight; does not read a job):

```bash
blender --background --factory-startup --python blender/worker.py -- --capabilities
```

- `--factory-startup` keeps user add-ons/startup files out of the deterministic batch run.
- Job mode reads `<job.json>`, executes the operations and writes `<result.json>`.
- Capabilities mode imports `bpy`, prints exactly one machine-readable line
  `MICHELANGELO_CAPABILITIES:<json>` to stdout and exits 0:

  ```json
  {
    "operations": ["create_box", "set_material", "...18 ops in stable order..."],
    "blender_version": "Blender 5.2.3",
    "python_version": "3.13.13",
    "gltf_export_available": true,
    "render_engines": ["BLENDER_WORKBENCH", "BLENDER_EEVEE", "CYCLES"]
  }
  ```

The adapter launches the real binary or a fake executable through the same code path. On Windows,
`.cmd`/`.bat` programs (fake/test doubles) are launched through `cmd /C call`; real Blender is an
`.exe` and is launched directly. The adapter bounds the `blender --version` probe (10 s) and the
capabilities probe (30 s), and redirects job stdout/stderr to per-job log files — never pipes —
so a chatty Blender run cannot deadlock the core.

### Job input (`job.json`)

Written by the adapter into `<workspace>/jobs/<job_id>/job.json`:

```json
{
  "job_id": "job_1790000000000000000",
  "spec": {
    "operations": [
      {"op": "create_component", "name": "body"},
      {"op": "create_box", "name": "body", "size": [2.0, 1.0, 0.5], "component": "body"},
      {"op": "get_scene_summary"},
      {"op": "save_blend", "path": "blender/scenes/smoke.blend"}
    ]
  },
  "workspace_root": "C:\\work\\project"
}
```

- `job_id`: `[A-Za-z0-9_-]{1,128}` (also a safe directory name).
- `workspace_root`: absolute, existing workspace root. All operation paths are workspace-relative.
- `spec.operations`: at most **256** operations.

### Result (`result.json`)

Atomic write (`result.json.tmp` + replace). `completed` -> exit code 0; `failed` -> exit code 1
with partial artifacts preserved.

```json
{
  "job_id": "job_1790000000000000000",
  "status": "completed",
  "message": "Executed 4 operation(s)",
  "artifacts": [
    {"path": "blender/scenes/smoke.blend", "type": "blend", "size_bytes": 113054}
  ],
  "scene_summary": {
    "objects": 1, "meshes": 1, "materials": 0, "cameras": 0,
    "triangles": 12, "vertices": 8,
    "components": [{"name": "Component_body", "objects": ["body"]}],
    "bounds": {"min": [-1.0, -0.5, -0.25], "max": [1.0, 0.5, 0.25]}
  }
}
```

On failure:

```json
{
  "job_id": "job_1790000000000000000",
  "status": "failed",
  "message": "Error during create_box",
  "error": "Operation 'create_box' failed: ...",
  "artifacts": [],
  "scene_summary": null
}
```

`scene_summary` is `null` until `get_scene_summary` runs (or the failing operation provided one).
Every listed artifact is verified by the adapter before a `completed` result is accepted.

## Deterministic scene initialization

Every job starts from a clean scene, independent of Blender startup content:

- all objects, non-master collections, meshes, cameras, lights, materials and images are removed
  (orphans purged); `scene.camera = None`;
- render engine set to `BLENDER_WORKBENCH`, resolution `512 x 512`, `resolution_percentage=100`,
  `image_settings.file_format="PNG"`, `color_mode="RGBA"`, `color_depth="8"`,
  `film_transparent=False`.

There is no default Cube/Camera/Light: a saved/exported scene contains exactly what the spec
created.

## Safe operations (18)

Global rules (validated by the Rust protocol layer **before** a job record is created, and
re-validated defensively by the worker):

- Unknown `op` values, missing required fields and out-of-range params are rejected with `-32602`
  on the protocol side; the worker reports `status:"failed"` with a structured error.
- Max 256 operations per job; duplicate created object/camera names within one job are rejected
  (including default names `Box`/`Cylinder`/`Sphere` and legacy `Cam_<view>` names).
- Names: non-empty/non-blank, ≤ 64 chars, no control characters, no `/` or `\`, not `.`/`..`.
- Paths: non-empty, ≤ 1024 chars, no NUL, workspace-relative, no parent components (`..`), no
  absolute paths, no Windows drive/UNC forms (`C:...`, `\\server\...`, `//...`) on any platform;
  extensions are case-insensitive.
- Numbers must be finite; `|location|, |rotation_deg|, |look_at| <= 1e6`; sizes/radii/depths > 0;
  scale components != 0; colors in `0..=1`.

| # | op | Params (default / range) | Semantics |
|---|---|---|---|
| 1 | `create_box` | `name` (`Box`), `size` (`[2,2,2]`, 3 positive values), `location` (`[0,0,0]`), `rotation_deg` (optional), `component` (optional) | Cube with exact `size` dimensions (scale applied), named object; joins `Component_<component>` or the scene collection. |
| 2 | `set_material` | `object` (required), `color` (`[0.5,0.5,0.5,1.0]`, 3 or 4 components in `0..=1`) | Creates `Mat_<object>`, sets Principled BSDF Base Color (when available) and viewport color; replaces slot 0. |
| 3 | `set_camera_set` | `views` (default `[front, side, top, perspective]`, 1..64 entries) | Legacy perspective cameras `Cam_<canonical>`; accepted names: `front/back/left/right/top/bottom/perspective` plus back-compat `side` → `left`; exact repeats rejected, alias pairs deduplicated. |
| 4 | `render_views` | `path` (base directory, default `blender/renders`) | Renders **every** camera (sorted by name), one PNG per camera. Unsafe/duplicate derived file names are rejected before any write; fails if no cameras exist. |
| 5 | `export_glb` | `path` (required, `.glb`) | Whole-scene glTF binary export; clean error when the bundled glTF exporter is unavailable. |
| 6 | `save_blend` | `path` (required, `.blend`) | Saves the current scene; missing/empty output is an error. |
| 7 | `get_scene_summary` | none | Deterministic summary (`objects`, `meshes`, `materials`, `cameras`, `triangles` via `calc_loop_triangles()`, `vertices`, `components`, `bounds`); also stored in the job result. |
| 8 | `create_cylinder` | `name` (`Cylinder`), `radius` (`0.5`, > 0), `depth` (`1.0`, > 0), `vertices` (`32`, 3..1024), `location` (`[0,0,0]`), `rotation_deg`?, `component`? | Cylinder with exact radius/depth. |
| 9 | `create_sphere` | `name` (`Sphere`), `radius` (`0.5`, > 0), `segments` (`32`, 3..1024), `ring_count` (`16`, 2..1024), `location` (`[0,0,0]`), `rotation_deg`?, `component`? | UV sphere with exact radius. |
| 10 | `import_mesh` | `path` (required; `.glb/.gltf/.obj/.stl/.ply`), `name`? (only when exactly one object is imported), `component`? | Imports one workspace artifact that must exist, be a regular non-empty file and resolve inside the workspace (checked by the adapter before launch). Imported objects can join a component. |
| 11 | `set_transform` | `object` (required), `location`?, `rotation_deg`?, `scale`? (at least one), `apply_scale` (`false`) | Exact transform; `apply_scale=true` bakes scale into mesh data. Missing object is an error. |
| 12 | `create_ortho_camera` | `name` (required), `location` (required), `rotation_deg` XOR `look_at` (both optional), `ortho_scale` (`4.0`, > 0), `shift_x`/`shift_y` (`0`, −10..10), `clip_start` (`0.1`, > 0), `clip_end` (`1000`, > clip_start), `component`? | Orthographic camera, `sensor_fit="HORIZONTAL"`; `look_at` uses `(target-location).to_track_quat('-Z','Y')`. |
| 13 | `set_camera_pose` | `camera` (required), `location`?, `rotation_deg` XOR `look_at`? (at least one field) | Updates an existing camera; look-at is computed from the (possibly new) location. |
| 14 | `set_camera_settings` | `camera` (required), `ortho_scale`?, `shift_x`?, `shift_y`?, `clip_start`?, `clip_end`? (at least one) | `ortho_scale` only applies to orthographic cameras; clip planes validated in final order. |
| 15 | `frame_camera` | `camera` (required), `objects`? or `component`? (at most one), `margin` (`1.1`, 0.1..10) | Orthographic only: frames the world bbox of the given objects/component/all meshes; recenters the camera on the bbox center in its local XY plane and sets `ortho_scale = max(width, height*aspect) * margin`. |
| 16 | `set_render_settings` | `resolution` (`[512,512]`, each 1..16384), `resolution_percentage` (`100`, 1..100) | Updates resolution/percentage; the render engine is always `BLENDER_WORKBENCH` and output is PNG. |
| 17 | `render_camera` | `camera` (required), `path` (required, `.png`), `silhouette` (`false`) | Renders **only** the selected camera to a workspace-relative PNG. `silhouette=true` -> RGBA with transparent film and a flat white silhouette (alpha channel is the mask); otherwise Workbench flat/material render. Render settings are snapshotted and restored afterwards. |
| 18 | `create_component` | `name` (required) | Creates the Blender collection `Component_<name>` (idempotent). Primitives/imports join it via their `component` param. |

### Import format notes

- Accepted: `.glb`, `.gltf`, `.obj`, `.stl`, `.ply` (case-insensitive).
- The worker uses Blender's bundled importers with their **default axis conventions** — no axis
  override parameters are part of this contract. The glTF importer converts glTF Y-up to Blender
  Z-up; OBJ uses the importer's default forward/up axes; STL/PLY use their importer defaults.
  Imported objects keep the transforms stored in the asset.
- `.obj` uses `bpy.ops.wm.obj_import` when available (Blender ≥ 4), with the legacy
  `bpy.ops.import_scene.obj` fallback.
- `name` is accepted only when the import creates exactly one object; requesting the imported
  object's own name is idempotent. A name that already belonged to another object before the
  import is rejected.

## Camera projection contract

Orthographic camera with `sensor_fit = HORIZONTAL` and no lens shift by default:

- Camera looks along local `-Z`, local `+Y` is up. `look_at` computes
  `(target - location).to_track_quat('-Z','Y')`.
- `ortho_scale` is the visible world width (horizontal extent) in Blender units.
- Visible height = `ortho_scale * resolution_y / resolution_x`.
- Visible x range at target depth = `[center_x - ortho_scale/2, center_x + ortho_scale/2]`.
- Pixel mapping for an object whose projected camera-space center is `(cx, cy)` and size
  `(w, h)` (bottom-left origin):
  - `width_px = w * resolution_x / ortho_scale`
  - `height_px = h * resolution_x / ortho_scale` (height in world units times the same
    pixels-per-unit factor; the vertical visible extent follows from the aspect ratio)
  - `x_px = (0.5 + cx / ortho_scale) * resolution_x`
  - `y_px = (0.5 + cy / (ortho_scale * resolution_y / resolution_x)) * resolution_y`

### Calibrated fixture (verified on Blender 5.2.3)

Fixture used by `examples/scene_ops/composite_scene_spec.json` and the acceptance smoke:

- box `2 x 1 x 0.5` at the origin;
- front orthographic camera at `(0, -10, 0)` looking at the origin, `ortho_scale=4`, 512x512,
  `silhouette: true`;
- result: exactly **16,384** pixels with alpha > 127, bbox `x 128..383`, `y 224..287`
  (256 x 64 px); other scene objects are outside the front view;
- `frame_camera` on the scene camera with `margin=1.1` must frame the whole scene. The acceptance
  scene deliberately creates `cam_scene` with `ortho_scale=100`, calls `frame_camera`, then
  renders `blender/renders/scene_silhouette.png` (`silhouette: true`). The smoke driver asserts
  that this framed silhouette has an opaque alpha coverage fraction in `0.03..0.95`, a bbox span
  ≥ 205 px (40% of 512) on each axis, and an ≥ 8 px border margin. Measured on Blender 5.2.3:
  framed coverage ≈ 0.0997 (bbox 424x253, margins ≥ 41 px); without `frame_camera` the scene
  collapses to ≈ 0.00033 coverage and a 25x15 bbox — coverage and span fail (the small bbox still
  sits inside the border margin, so the margin check alone would not catch that regression).

`render_camera` normal (non-silhouette) renders are opaque Workbench renders; only the
silhouette path enables a transparent film/alpha mask.

## Render engine restriction

**Workbench only.** `set_render_settings`, scene reset and `render_camera`/`render_views` force
`BLENDER_WORKBENCH`. EEVEE/Cycles rendering determinism/reliability in headless batch mode is out
of scope for this iteration: the capabilities probe may report other engines available in the
installed Blender, but the worker contract guarantees only Workbench output. Expected output:
512x512 (or the requested resolution), 8-bit RGBA PNG, color type 6.

## Artifact contract

Each artifact entry is `{"path": <workspace-relative string>, "type": "blend"|"glb"|"png",
"size_bytes": <int>}`; the file extension must match the type (`.blend`, `.glb`, `.png`,
case-insensitive).

- `completed` job: every listed artifact must exist, be a regular non-empty file, resolve inside
  `<workspace>` (symlinks resolved), and `size_bytes` (when present) must equal the file size.
  The adapter rejects a completed result otherwise.
- `failed`/`cancelled` job: partial artifacts are reported but must still use safe paths; the
  adapter sanitizes/drops unsafe entries and never lets them mask the original error.
- Output paths are containment-checked **before** any filesystem mutation (parent directory
  creation, stale-file unlink, render); every user- or asset-derived output name must stay inside
  the resolved workspace.

## Validation, logging and exit codes

- The Rust protocol layer validates the whole spec before creating a job record (no orphan queued
  jobs on invalid params) and returns `-32602` naming `operation[i] ('<op>')`. The worker validates
  independently before touching the scene.
- Logging: the worker writes nothing to stdout except the single capabilities marker. Progress and
  diagnostics go to stderr with a `[michelangelo]` prefix and are captured in
  `<workspace>/jobs/<job_id>/stderr.log`; adapter diagnostics are capped (stderr tail ≤ 4 KiB,
  stdout tail ≤ 2 KiB).
- Exit codes: `0` = `completed` result written; `1` = `failed` result written (exception or
  validation failure); usage errors also return non-zero without a result file.
- Execution timeouts (adapter, not worker): explicit `timeout_ms` > `MICHELANGELO_BLENDER_TIMEOUT_MS`
  > built-in `300_000` ms. On timeout the process tree is killed (Windows `taskkill /T /F`, Unix
  process group `SIGKILL`), waited for, and reported as a job failure with the timeout value and
  stderr tail.

## Examples

Capabilities:

```bash
blender --background --factory-startup --python blender/worker.py -- --capabilities
```

Calibrated silhouette job (JSONL request against the core; `wait=true`):

```json
{"id":"j1","method":"blender.run_job","params":{
  "job_type":"blender_acceptance_scene",
  "wait":true,
  "timeout_ms":300000,
  "spec":{"operations":[
    {"op":"create_component","name":"body"},
    {"op":"create_box","name":"body_box","size":[2.0,1.0,0.5],"component":"body"},
    {"op":"create_ortho_camera","name":"cam_front","location":[0.0,-10.0,0.0],"look_at":[0.0,0.0,0.0],"ortho_scale":4.0},
    {"op":"set_render_settings","resolution":[512,512]},
    {"op":"render_camera","camera":"cam_front","path":"blender/renders/front_silhouette.png","silhouette":true},
    {"op":"save_blend","path":"blender/scenes/scene.blend"}
  ]}
}}
```

Run the full composite acceptance scenario with
`python scripts/smoke_blender_scene.py` (real Blender) or `--fake` (deterministic, no Blender).
