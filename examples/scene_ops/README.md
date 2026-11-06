# Composite Scene Acceptance Example

`composite_scene_spec.json` is the exact `BlenderJobSpec` operations array used by
the IT-3 acceptance smoke:

- `python scripts/smoke_blender_scene.py` — real Blender (auto-SKIP when Blender is absent);
- `python scripts/smoke_blender_scene.py --fake` — deterministic fake Blender;
- `scripts/smoke_blender_scene.py` loads this file, so the documented example and the
  acceptance scenario cannot drift apart.

## What the scene contains

| Element | Value |
|---|---|
| Component `body` | collection `Component_body` |
| Box `body_box` | `[2, 1, 0.5]` at the origin, in `Component_body` |
| Cylinder `post` | radius `0.25`, depth `2`, at `[3, 0, 1]`, in `Component_extra` |
| Sphere `ball` | radius `0.5`, at `[-3, 0, 0.5]`, in `Component_extra` |
| Material | `body_box` base color `[0.2, 0.4, 0.8, 1.0]` |
| Front camera `cam_front` | orthographic, `(0, -10, 0)` looking at the origin, `ortho_scale=4` |
| Scene camera `cam_scene` | orthographic, `(7, -7, 5)` looking at the origin, `ortho_scale=100` |
| Framing | `frame_camera cam_scene margin=1.1` must reframe the deliberately far/wide camera |
| Renders | 512x512, Workbench, PNG RGBA: front silhouette, framed scene, framed scene silhouette |
| Outputs | 5 artifacts: 3 PNGs + `.blend` + `.glb` |

## Calibrated front silhouette

The first render is a calibrated silhouette:

- orthographic `cam_front` at `(0, -10, 0)` looking at the origin, `sensor_fit=HORIZONTAL`,
  `ortho_scale=4`, 512x512;
- the `2 x 1 x 0.5` box projects to a 256x64 pixel rectangle (16,384 opaque pixels);
- alpha bbox is `x 128..383`, `y 224..287` (bottom-left origin);
- the cylinder (`x=3`) and sphere (`x=-3`) are outside the front view because the visible
  horizontal range is `[-2, 2]`.

The smoke driver asserts these values with a pure-Python PNG decoder (`zlib` + `struct`) on
real Blender only; `--fake` mode checks protocol/artifact plumbing but skips pixel assertions.

## Framed-scene silhouette (assertable framing check)

`cam_scene` is created with `ortho_scale=100` on purpose. A correct
`frame_camera cam_scene margin=1.1` rewrites `ortho_scale` to the scene extent times the margin;
the scene silhouette then fills most of the 512x512 frame with a visible border. The driver asserts
(real Blender only):

| Assertion | Threshold (real mode) | Framed (Blender 5.2.3) | Missing/broken `frame_camera` |
|---|---|---|---|
| opaque alpha coverage fraction | `0.03 ≤ f ≤ 0.95` | ≈ 0.0997 (26,142 px) | ≈ 0.00033 (86 px) → **FAIL** |
| alpha bbox span (each axis) | ≥ 205 px (40% of 512) | 424 x 253 px | 25 x 15 px → **FAIL** |
| border margin | ≥ 8 px on each side | ≥ 41 px | bbox `(243, 267, 246, 260)` → passes |
| overall | all three must pass | PASS | **FAIL** (coverage + span only) |

The 0.95 coverage upper bound rejects a degenerate fully-opaque frame. Coverage `0.03` is ~91x
above the measured unframed value (0.03 / 0.00033 ≈ 91) and ~3.3x below the framed value; the span
threshold 205 px is far above the unframed 25/15 px and below the framed 424/253 px. The
missing-`frame_camera` bbox happens to sit inside the border area, so the border-margin check alone
would not catch that regression — coverage and span do.

This makes the framing check meaningful: if `frame_camera` is missing or broken, the acceptance
smoke fails instead of merely observing a non-empty PNG.

## Run it

```bash
# deterministic plumbing check (no Blender installation required)
cargo build -p michelangelo_cli
python scripts/smoke_blender_scene.py --fake

# real Blender 5.x acceptance (auto-SKIP when Blender is absent)
python scripts/smoke_blender_scene.py

# keep the generated workspace and fake Blender directory for inspection
python scripts/smoke_blender_scene.py --fake --keep
```

Direct `blender.run_job` JSONL form (one request, `wait=true`):

```json
{"id":"j1","method":"blender.run_job","params":{"job_type":"blender_acceptance_scene","wait":true,"timeout_ms":300000,"spec":{"operations":[...]}}}
```

The operation list is the `operations` value from `composite_scene_spec.json`. Paths are always
workspace-relative; artifacts land under `blender/renders/`, `blender/scenes/` and
`blender/exports/`.
