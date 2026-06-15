# Blender Worker Contract

## Invocation

```bash
blender --background --python blender/worker.py -- <job.json> <result.json>
```

## Input (`job.json`)

```json
{
  "job_id": "job_...",
  "spec": {
    "operations": [
      {"op": "create_box", "name": "body", "size": [2.0, 1.0, 0.5], "location": [0.0, 0.0, 0.0]},
      {"op": "set_material", "object": "body", "color": [0.2, 0.4, 0.8, 1.0]},
      {"op": "set_camera_set", "views": ["front", "side", "top", "perspective"]},
      {"op": "save_blend", "path": "blender/scenes/smoke.blend"},
      {"op": "export_glb", "path": "blender/exports/smoke.glb"}
    ]
  },
  "workspace_root": "/path/to/project"
}
```

## Output (`result.json`)

```json
{
  "job_id": "job_...",
  "status": "completed",
  "message": "All operations completed",
  "artifacts": [
    {"path": "blender/scenes/smoke.blend", "type": "blend", "size_bytes": 853784}
  ],
  "scene_summary": {"objects": 2, "meshes": 1, "triangles": 12, "vertices": 24}
}
```

On failure:

```json
{
  "job_id": "job_...",
  "status": "failed",
  "message": "Operation failed",
  "error": "RuntimeError: ...",
  "artifacts": []
}
```

## Safe Operations

| Operation | Handler | Parameters |
|---|---|---|
| `create_box` | `op_create_box` | `name`, `size`, `location` |
| `set_material` | `op_set_material` | `object`, `color` |
| `set_camera_set` | `op_set_camera_set` | `views` |
| `render_views` | `op_render_views` | `path` (base output directory) |
| `export_glb` | `op_export_glb` | `path` |
| `save_blend` | `op_save_blend` | `path` |
| `get_scene_summary` | `op_get_scene_summary` | _(none)_ |

## Path Validation

All file paths are validated: absolute paths and parent-traversal (`..`) are rejected.
Paths are resolved relative to the workspace root.

## Logging

Human-readable progress logs go to stderr with `[michelangelo]` prefix.
Stdout is never used by the worker (reserved for protocol JSONL in the Rust host).
