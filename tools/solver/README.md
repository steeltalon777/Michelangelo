# Stage B solver (Python, standard library only)

Deterministic visual-hull solver and quality comparator for the Michelangelo
Stage B reconstruction PoC (see `docs/tz/TZ_RECON_POC_STAGE_B.md`).

No third party packages, no `bpy`, no network access: only the Python standard
library (`zlib`, `json`, `argparse`, ...).

## Modules

| File | Purpose |
|---|---|
| `pngread.py` | PNG decoder (8-bit gray / gray+alpha / RGB / RGBA, all filters, non-interlaced) and mask helpers |
| `mask.py` | binary masks (bbox, coverage, IoU) and the calibrated orthographic view maths (pixel convention of input package v0) |
| `carve.py` | hull AABB derivation from mask bboxes, uniform voxel grid, visual hull carving |
| `mesh.py` | blocky surface meshing of a voxel set to OBJ (quads, CCW outward) |
| `supplements.py` | voxel space supplement rules (thickness, cavity, closure/connection, curve, symmetry) and unknown stubs |
| `solve.py` | solver CLI, writes `blockout.obj`, `supplement_<id>.obj`, `stub_<id>.obj`, `build_report.json` |
| `compare.py` | comparator CLI, writes `quality_report.json` |

## Usage

```text
python tools/solver/solve.py --package <package_dir> --out <out_dir> [--resolution N] [--json]
python tools/solver/compare.py --package <pkg> --renders <dir> --out <dir>
                               [--evaluation-package <pkg> --evaluation-renders <dir>]
                               [--source-mean-iou F] [--source-min-iou F]
                               [--evaluation-mean-iou F] [--evaluation-min-iou F]
                               [--bbox-relative-error-max F] [--json]
```

Exit codes: `solve.py` 0 = success, 2 = invalid/unusable package (a build report is
written when possible), 3 = solver failure. `compare.py` 0 = pass, 1 = fail,
2 = usage/IO error.

## Conventions

* Pixel convention (fixed by input package v0): image origin top-left, `u` right,
  `v` down, `u_axis = normalize(cross(view_dir, up_world))`, `v_axis = up_world`,
  `u_px = (0.5 + u_cam/ortho_scale) * width_px`, `v_px = (0.5 - v_cam/ortho_scale) * width_px`.
* All manifest lengths are package units (`m`, `cm`, `mm`); the solver converts
  them to metres. Every length in the reports is in metres.
* A view with a non-zero `shift_x`/`shift_y` is rejected (v0 limitation).
* `--resolution N` is the number of voxels along the longest hull axis
  (default 64, capped at 192, at most 2,000,000 voxels).
* Outputs are deterministic: sorted lists, no timestamps, no absolute paths in
  the reports.

## v0 behaviour that needs to be known

* The hull AABB is the intersection of the half spaces implied by the mask
  bboxes of the calibrated views (exact, not a per-axis approximation). One
  calibrated view, or several views with parallel directions, leave the hull
  unbounded and are reported as an unusable package.
* Supplements are applied in manifest order. `thickness`, `cavity` and
  `symmetry` modify the hull; `closure`/`connection` and `curve` add
  geometry that is written to their own `supplement_<id>.obj` (added geometry
  only). `section`, `profile` and `primitive_fill` are reported as
  `unsupported` with a diagnostic - they are never guessed.
* Input package v0 carries no geometry for unknowns. A `stub_allowed` unknown
  therefore gets a deterministic stub box of 10 % of the hull bbox centred on
  the hull centre; `author_required` and unresolved unknowns stay
  `unresolved` and never modify the hull.
* The meshing is blocky (one quad per exposed voxel face).

## Tests

```text
python tools/solver/tests/test_solver.py
python -m unittest discover -s tools/solver/tests -t .
```
