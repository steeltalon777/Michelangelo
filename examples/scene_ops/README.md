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
| Scene camera `cam_scene` | orthographic, `(7, -7, 5)` looking at the origin, framed with margin `1.1` |
| Render | 512x512, Workbench, PNG RGBA |
| Outputs | silhouette + scene PNG, `.blend`, `.glb` |

## Calibrated silhouette

The first render is a calibrated silhouette:

- orthographic `cam_front` at `(0, -10, 0)` looking at the origin, `sensor_fit=HORIZONTAL`,
  `ortho_scale=4`, 512x512;
- the `2 x 1 x 0.5` box projects to a 256x64 pixel rectangle (16,384 opaque pixels);
- alpha bbox is `x 128..383`, `y 224..287` (bottom-left origin);
- the cylinder (`x=3`) and sphere (`x=-3`) are outside the front view because the visible
  horizontal range is `[-2, 2]`.

The smoke driver asserts these values with a pure-Python PNG decoder (`zlib` + `struct`) on
real Blender only; `--fake` mode checks protocol/artifact plumbing but skips pixel assertions.

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
