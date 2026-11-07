# Reconstruction Pipeline (v0)

Normative contract of the Michelangelo **reconstruction pipeline** (Stage B): a validated input
package is turned into a deterministic voxel blockout, the blockout is rendered from the
calibrated cameras of the package, the renders are compared with the package masks and the
result is written as a machine-readable report.

Status: implemented in Stage B (crate `michelangelo_recon`, CLI `michelangelo reconstruct`,
solver `tools/solver`). Report schema: `michelangelo.reconstruction-report/0`. The solver
build report (`michelangelo.build-report/0`) and the quality report
(`michelangelo.quality-report/0`) are produced by the Python tools and are described in
`tools/solver/README.md`.

## Purpose and ADR reference

ADR 0004 (`docs/adr`, §13.2 *Stage B*) requires a vertical proof of concept **without any
external API**: a prepared package, a limited deterministic build, renders from the source
cameras, a quality report and an editable blockout with explicit unknowns.

The pipeline is Core-side (Rust library + CLI). It performs no network access, mutates no input
package, adds no Blender operation and changes no wire contract.

## Pipeline stages

```text
package dir ──▶ validate ──▶ allocate run ──▶ solve.py ──▶ build spec ──▶ Blender job
                │                │              │             │              │
                │                │              │             │              ├─▶ blockout.blend
                │                │              │             │              └─▶ blockout.glb
                │                │              │             └─▶ renders/<view_id>.png
                │                │              └─▶ source/*.obj + build_report.json
                │                └─▶ recon/rNNNN/run-NNNN/
                └─▶ invalid package ⇒ exit code 2
```

1. **Validate** — the package is validated with `michelangelo_input::validate_package` (levels
   1 and 2, hashes verified). A level-1 error rejects the run (exit code 2). A partial package is
   accepted by the validator but rejected by the solver (exit code 2, `SolverRejected`).
2. **Allocate** — the next free run directory of the store is created (never overwritten).
3. **Solve** — `tools/solver/solve.py` runs into `source/` and writes the OBJ meshes and
   `build_report.json`.
4. **Plan** — a `BlenderJobSpec` is built from the package and the build report (see below).
5. **Render** — the job runs through `HeadlessBlenderAdapter` (Blender binary from
   `--blender` > `BLENDER_BIN` > `blender`). The worker produces the per-view
   silhouette renders, `blockout.blend` and `blockout.glb`.
6. **Compare** — `tools/solver/compare.py` compares the renders with the package masks and
   writes `quality_report.json` into the run directory.
7. **Report** — `report.json` (`michelangelo.reconstruction-report/0`) is written
   atomically. `status` is `pass` only when the worker completed **and** every quality
   threshold was met.

## Solver and comparator contract

The Rust side never re-implements the solver; it only discovers, bounds and parses it.

| Item | Value |
|---|---|
| Interpreter | `--python` > env `MICHELANGELO_PYTHON` > `python` |
| Solver script | `--solver` > env `MICHELANGELO_SOLVER` > `<repo>/tools/solver/solve.py` |
| Comparator | the `compare.py` sibling of the solver script |
| Timeout | `--timeout-ms` (solver) > env `MICHELANGELO_SOLVER_TIMEOUT_MS` > 600 000 ms |
| Output capture | the last 8 KiB of stdout and of stderr |

The repository root is discovered **at build time** from `CARGO_MANIFEST_DIR/../..` (the crate
lives in `crates/michelangelo_recon`), so the default solver path is correct for a build of
this repository and for the test suite.

Solver exit codes are mapped to typed errors:

| Exit code | Meaning | Rust error |
|---|---|---|
| 0 | success | — (the build report is parsed) |
| 2 | the package is invalid or unusable | `SolverRejected` |
| 3 | the solver itself failed | `SolverFailed` |
| other / killed | failure | `SolverFailed` / `SolverTimeout` |

The comparator exit code 1 (thresholds not met) is **not** an error: the quality report is
returned with the status `fail` and the run status becomes `fail` (CLI exit code 1).
Exit code 2 of the comparator (usage or I/O error) is an error.

## Camera and Euler convention

A calibrated view provides `location_m`, `look_at_m`, `up_world` and
`ortho_scale_m` (the full visible world width). The pipeline converts the view into a
Blender camera pose:

```text
direction = normalize(look_at - location)
right     = normalize(cross(direction, up_world))
camera_up = normalize(cross(right, direction))
R         = columns (right, camera_up, -direction)      # camera-local -> world
y = asin(-R[2][0]);  x = atan2(R[2][1], R[2][2]);  z = atan2(R[1][0], R[0][0])
```

The matrix has the **columns** `(right, camera_up, -direction)`; the transposed (row)
convention produces empty renders and is rejected by the unit tests.

Verified reference poses (Blender 5.2.3, checked by `crates/michelangelo_recon/src/camera.rs`):

| View | location | look_at | up_world | `rotation_deg` |
|---|---|---|---|---|
| front | `(0, -4, 0)` | origin | `+Z` | `(90, 0, 0)` |
| right | `(4, 0, 0)` | origin | `+Z` | `(90, 0, 90)` |
| top | `(0, 0, 4)` | origin | `+Y` | `(0, 0, 0)` |

Every length is converted from the package unit (`m`, `cm`, `mm`) to metres before it
reaches Blender; non-finite values are rejected.

## OBJ up-axis convention

The solver writes **Y-up** OBJ files (the standard OBJ convention) in metres; Blender's OBJ
importer converts them to Z-up on import. The pipeline therefore applies **no** compensation
transform: the imported blockout lives in the same Z-up world as the package cameras.

## Blender job shape

One job per run, at most 256 operations, all paths workspace-relative with forward slashes:

```text
import_mesh  <run>/source/blockout.obj        name=blockout      component=blockout
import_mesh  <run>/source/supplement_<id>.obj name=supplement_<id> component=supplement_<id>
import_mesh  <run>/source/stub_<id>.obj       name=stub_<id>     component=stub_<id>
create_ortho_camera <view_id> location=<m> rotation_deg=<XYZ> ortho_scale=<m> clip_start=<m> clip_end=<m>
set_render_settings resolution=[<width_px>, <height_px>]
render_camera <view_id> <run>/renders/<view_id>.png silhouette=true
save_blend    <run>/blockout.blend
export_glb    <run>/blockout.glb
```

- Every mesh goes into **its own component collection** and keeps the file stem as its object
  name.
- The camera name is the `view_id`; the render resolution is the resolution of the view image
  (the mask has the same size).
- The job is validated with `BlenderJobSpec::validate` (protocol rules) and with local checks
  (operation count, unique names, safe relative paths).
- A view with a non-zero `shift_x` or `shift_y` is rejected with the error code
  `UNSUPPORTED_SHIFT`.

## Run store layout

```text
<workspace>/recon/rNNNN/run-NNNN/
├── package/                     # copy of package.json + masks (makes a run self-scoring)
├── source/                      # solver artifacts
│   ├── blockout.obj
│   ├── supplement_<id>.obj
│   ├── stub_<id>.obj
│   └── build_report.json
├── renders/<view_id>.png        # silhouette renders of the source views
├── evaluation_renders/<view_id>.png
├── jobs/<job_id>/               # job.json, result.json, stdout.log, stderr.log
├── blockout.blend
├── blockout.glb
├── build_report.json
├── quality_report.json
├── evaluation_quality_report.json
├── report.before_score.json
└── report.json
```

- `run-NNNN` is the next free four-digit sequence: a rerun never overwrites an existing run.
- The store is created lazily and is **not** part of the workspace layout constants (no
  migration, no layout change).
- Every path inside a report is workspace-relative; no absolute path ever reaches the report.

## Report JSON (`michelangelo.reconstruction-report/0`)

The top-level keys are exactly:

```json
{
  "schema": "michelangelo.reconstruction-report/0",
  "run": {
    "run_id": "recon_0001_0001",
    "run_dir": "recon/r0001/run-0001",
    "created_at": "2026-11-07T12:00:00Z"
  },
  "package": {
    "package_id": "harbor_lantern_v01",
    "revision": 1,
    "validation_status": "valid_complete",
    "calibrated_views": ["front", "right", "top"]
  },
  "solver": {
    "name": "voxel-carve",
    "version": 1,
    "resolution": 96,
    "voxel_size": 0.010416666,
    "grid": [96, 96, 96],
    "obj_up": "y"
  },
  "build": {
    "components": [{"name": "blockout", "obj": "blockout.obj", "voxels": 456, "triangles": 736}],
    "supplements": [{"supplement_id": "lid_thickness", "status": "applied", "rule": "thickness", "detail": "..."}],
    "unknowns": [{"id": "u_interior", "status": "stub", "detail": "..."}],
    "measurements": [{"id": "height", "declared": 0.645986, "measured": 0.646, "absolute_error": 0.000014, "relative_error": 0.0000217, "within_tolerance": true}]
  },
  "worker": {
    "job_id": "recon_0001_0001",
    "status": "completed",
    "artifacts": ["recon/r0001/run-0001/blockout.blend", "recon/r0001/run-0001/blockout.glb"],
    "blend": "recon/r0001/run-0001/blockout.blend",
    "glb": "recon/r0001/run-0001/blockout.glb"
  },
  "quality": {
    "source": {
      "views": [{"view_id": "front", "iou": 0.938, "mask_px": 1048576, "render_px": 1048576, "intersection_px": 983552, "union_px": 1048576, "mask_coverage": 1.0, "render_coverage": 1.0}],
      "mean_iou": 0.938,
      "min_iou": 0.918
    },
    "evaluation": null,
    "thresholds": {
      "source_mean_iou": 0.9,
      "source_min_iou": 0.85,
      "evaluation_mean_iou": 0.8,
      "evaluation_min_iou": 0.7,
      "bbox_relative_error_max": 0.05
    },
    "status": "pass"
  },
  "status": "pass",
  "unresolved": [{"id": "u_recess", "reason": "the recess is not observable from the silhouettes"}],
  "diagnostics": [{"code": "MEASUREMENT_OUT_OF_TOLERANCE", "severity": "warning", "message": "...", "address": "measurements[height].value_m"}]
}
```

- `status` is `pass`, `fail` or `error`. `fail` means the pipeline completed but a
  threshold was not met; `error` means the run did not complete (the error report is written
  into the run directory).
- `unresolved` lists the unknowns that stayed unresolved (`status = "unresolved"` in the
  build report), sorted by identifier.
- `diagnostics` merges the solver diagnostics and the comparator diagnostics, sorted by
  `(code, address, message)`.
- The report is deterministic: the same inputs produce the same bytes.

## CLI and exit codes

```text
michelangelo reconstruct run   --package <DIR> --workspace <WS> [--resolution N] [--solver PATH]
                               [--python PATH] [--blender PATH] [--timeout-ms N] [--json]
michelangelo reconstruct score --run <run_dir> --evaluation <pkg> [--blender PATH] [--json]
michelangelo reconstruct show  --run <run_dir> [--json]
```

| Exit code | Meaning |
|---|---|
| `0` | `run`/`score` passed every threshold; `show` read the stored report |
| `1` | the run completed but a quality threshold was not met |
| `2` | invalid input, broken setup or an I/O error |

Without `--json` the commands print a human summary, with `--json` they print the report
JSON. Human diagnostics always go to stderr. `show` is a read-only inspection: it exits `0`
when the stored report can be read, whatever the status of the stored run is.

`reconstruct score` re-imports the meshes of `source/`, creates the cameras of the
evaluation package, renders them into `evaluation_renders/`, runs the comparator with both
view sets, writes `evaluation_quality_report.json`, keeps the previous report as
`report.before_score.json` and recomputes the status.

## Thresholds and their measured basis

The comparator defaults are the acceptance thresholds of Stage B:

| Threshold | Value | Basis |
|---|---|---|
| source mean IoU | `0.90` | the feasibility probe measured 0.912–0.951 on the local datasets (96³ hull) |
| source min IoU | `0.85` | the probe measured 0.918 on the worst source view |
| evaluation mean IoU | `0.80` | the held-out orthographic control measured 0.914–0.949 |
| evaluation min IoU | `0.70` | margin below the observed control level |
| declared measurement relative error | `0.05` | the declared dimensions of the packages are accurate to about 5 mm |

The thresholds are set at the observed level with margin, not at the theoretical maximum: a
visual hull over-approximates concave regions by construction (ADR §9.1).

## v0 limits

- **Orthographic only.** Perspective views of a package are reference views and are not used as
  metric constraints; the pipeline creates orthographic cameras only.
- **Blocky voxel hull.** The meshing is one quad per exposed voxel face. There is no smoothing,
  remeshing or boolean stage.
- **Perspective controls are not scored.** Only orthographic held-out views are compared.
- **No lens shift.** A view with a non-zero `shift_x` or `shift_y` is rejected
  (`UNSUPPORTED_SHIFT`).
- **No hidden-void recovery.** Blind recesses, undercuts and internal voids are not observable
  from silhouettes; the hull over-approximates them and the corresponding unknown stays in the
  `unresolved` list.
- **One voxel grid.** Supplements are applied in voxel space; there is no per-vertex editing.
- **No network, no external API.** The solver is local Python (standard library only), the
  renderer is the local Blender worker.
