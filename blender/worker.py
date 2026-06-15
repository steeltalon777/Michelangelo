#!/usr/bin/env python3
"""
Michelangelo Blender Worker — safe operation executor.

Invocation:
    blender --background --python worker.py -- <job.json> <result.json>

Reads a strict JSON job spec, executes safe operations via bpy,
and writes a JSON result file. All progress goes to stderr.
"""

import json
import os
import sys
import traceback
from pathlib import Path

# ---------------------------------------------------------------------------
# Safe operation handlers
# ---------------------------------------------------------------------------

def op_create_box(ctx, params):
    """Create a named box/cube mesh with given size and location."""
    import bpy
    name = params.get("name", "Box")
    size = params.get("size", [2.0, 2.0, 2.0])
    location = params.get("location", [0.0, 0.0, 0.0])

    bpy.ops.mesh.primitive_cube_add(size=2.0, location=location)
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = (size[0] / 2.0, size[1] / 2.0, size[2] / 2.0)
    bpy.ops.object.transform_apply(scale=True)
    _log(f"Created box '{name}' at {location} size {size}")
    return {"object": name, "type": "MESH"}


def op_set_material(ctx, params):
    """Assign a diffuse color material to an object."""
    import bpy
    object_name = params.get("object", "")
    color = params.get("color", [0.5, 0.5, 0.5, 1.0])

    if object_name not in bpy.data.objects:
        return {"error": f"Object '{object_name}' not found"}

    obj = bpy.data.objects[object_name]
    mat = bpy.data.materials.new(name=f"Mat_{object_name}")
    mat.use_nodes = False
    mat.diffuse_color = color
    if obj.data.materials:
        obj.data.materials[0] = mat
    else:
        obj.data.materials.append(mat)
    _log(f"Applied material to '{object_name}' color={color}")
    return {"object": object_name, "material": mat.name}


def op_set_camera_set(ctx, params):
    """Create cameras for named views."""
    import bpy
    from math import radians
    views = params.get("views", ["front", "side", "top", "perspective"])

    view_defs = {
        "front":    {"location": (0, -10, 0), "rotation": (radians(90), 0, 0)},
        "back":     {"location": (0, 10, 0),  "rotation": (radians(90), 0, radians(180))},
        "left":     {"location": (-10, 0, 0), "rotation": (radians(90), 0, radians(90))},
        "right":    {"location": (10, 0, 0),  "rotation": (radians(90), 0, radians(-90))},
        "top":      {"location": (0, 0, 10),  "rotation": (0, 0, 0)},
        "bottom":   {"location": (0, 0, -10), "rotation": (radians(180), 0, 0)},
        "perspective": {"location": (7, -7, 5), "rotation": (radians(60), 0, radians(45))},
    }

    created = []
    for view in views:
        if view in view_defs:
            vd = view_defs[view]
            cam_data = bpy.data.cameras.new(name=f"Cam_{view}")
            cam_obj = bpy.data.objects.new(name=f"Cam_{view}", object_data=cam_data)
            bpy.context.collection.objects.link(cam_obj)
            cam_obj.location = vd["location"]
            cam_obj.rotation_euler = vd["rotation"]
            created.append(view)
            _log(f"Created camera '{view}'")
        else:
            _log(f"Unknown view '{view}', skipping")
    return {"cameras_created": created}


def op_render_views(ctx, params):
    """Render each camera to a PNG file."""
    import bpy
    base_path = params.get("path", "blender/renders")
    workspace_root = Path(ctx.get("workspace_root", "."))
    output_dir = workspace_root / base_path
    output_dir.mkdir(parents=True, exist_ok=True)

    scene = bpy.context.scene
    old_camera = scene.camera
    rendered = []

    cam_objects = [o for o in bpy.data.objects if o.type == 'CAMERA']
    if not cam_objects:
        return {"error": "No cameras to render", "rendered": []}

    scene.render.image_settings.file_format = 'PNG'

    for cam_obj in cam_objects:
        scene.camera = cam_obj
        view_name = cam_obj.name.replace("Cam_", "")
        filepath = str(output_dir / f"{view_name}.png")
        scene.render.filepath = filepath
        bpy.ops.render.render(write_still=True)
        size = Path(filepath).stat().st_size if Path(filepath).exists() else 0
        rendered.append({"view": view_name, "path": f"{base_path}/{view_name}.png", "size_bytes": size})
        _log(f"Rendered view '{view_name}' to {filepath}")

    scene.camera = old_camera
    return {"rendered": rendered}


def op_export_glb(ctx, params):
    """Export scene to glTF/GLB."""
    import bpy
    export_path = params.get("path", "")
    if not export_path:
        return {"error": "export_glb requires a 'path' parameter"}

    workspace_root = Path(ctx.get("workspace_root", "."))
    abs_path = workspace_root / export_path
    abs_path.parent.mkdir(parents=True, exist_ok=True)

    bpy.ops.export_scene.gltf(
        filepath=str(abs_path),
        export_format='GLB',
        use_selection=False,
        export_draco_mesh_compression_enable=False,
    )
    size = abs_path.stat().st_size if abs_path.exists() else 0
    _log(f"Exported GLB to {export_path} ({size} bytes)")
    return {"path": export_path, "size_bytes": size, "type": "glb"}


def op_save_blend(ctx, params):
    """Save the current .blend file."""
    import bpy
    save_path = params.get("path", "")
    if not save_path:
        return {"error": "save_blend requires a 'path' parameter"}

    workspace_root = Path(ctx.get("workspace_root", "."))
    abs_path = workspace_root / save_path
    abs_path.parent.mkdir(parents=True, exist_ok=True)

    bpy.ops.wm.save_as_mainfile(filepath=str(abs_path))
    size = abs_path.stat().st_size if abs_path.exists() else 0
    _log(f"Saved .blend to {save_path} ({size} bytes)")
    return {"path": save_path, "size_bytes": size, "type": "blend"}


def op_get_scene_summary(ctx, params):
    """Return an overview of the current scene."""
    import bpy
    scene = bpy.context.scene
    objects = list(scene.objects)
    meshes = [o for o in objects if o.type == 'MESH']
    total_triangles = sum(len(m.data.loop_triangles) for m in meshes if m.data)
    total_vertices = sum(len(m.data.vertices) for m in meshes if m.data)

    summary = {
        "objects": len(objects),
        "meshes": len(meshes),
        "materials": len(bpy.data.materials),
        "cameras": len([o for o in objects if o.type == 'CAMERA']),
        "triangles": total_triangles,
        "vertices": total_vertices,
    }
    _log(f"Scene summary: {summary}")
    return summary


# ---------------------------------------------------------------------------
# Operation registry
# ---------------------------------------------------------------------------

OPERATIONS = {
    "create_box": op_create_box,
    "set_material": op_set_material,
    "set_camera_set": op_set_camera_set,
    "render_views": op_render_views,
    "export_glb": op_export_glb,
    "save_blend": op_save_blend,
    "get_scene_summary": op_get_scene_summary,
}

SAFE_OPS = set(OPERATIONS.keys())


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _log(msg):
    """Print a log message to stderr."""
    print(f"[michelangelo] {msg}", file=sys.stderr, flush=True)


def _write_result(path, data):
    """Atomically write the result JSON file."""
    path_str = str(path)
    tmp = path_str + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f, indent=2)
    os.replace(tmp, path_str)


def _validate_path(path_str, workspace_root):
    """Validate that a path is relative, under workspace, and has no parent traversal.

    Raises ValueError if the path is unsafe.
    """
    p = Path(path_str)
    if p.is_absolute():
        raise ValueError(f"Absolute path rejected: '{path_str}'")
    if ".." in path_str.split("/") or ".." in path_str.split("\\"):
        raise ValueError(f"Parent-traversal path rejected: '{path_str}'")
    # Resolve and verify it stays under workspace
    resolved = (workspace_root / p).resolve()
    ws_resolved = workspace_root.resolve()
    if not str(resolved).startswith(str(ws_resolved)):
        raise ValueError(f"Path '{path_str}' resolves outside workspace")


def _ensure_workspace_dirs(workspace_root):
    """Ensure required subdirectories exist under workspace root."""
    dirs = [
        "blender/scenes",
        "blender/renders",
        "blender/exports",
    ]
    for d in dirs:
        (workspace_root / d).mkdir(parents=True, exist_ok=True)


# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

def main():
    # Parse args: everything after -- is our payload
    try:
        dash_idx = sys.argv.index("--")
        args = sys.argv[dash_idx + 1:]
    except ValueError:
        # No -- separator; fall back to last two args
        args = sys.argv[1:]

    if len(args) < 2:
        _log("Usage: blender --background --python worker.py -- <job.json> <result.json>")
        sys.exit(1)

    job_path = Path(args[0])
    result_path = Path(args[1])

    # Read job spec
    try:
        with open(job_path) as f:
            job = json.load(f)
    except Exception as e:
        _write_result(result_path, {
            "job_id": "unknown",
            "status": "failed",
            "error": f"Cannot read job file: {e}",
            "message": "Worker initialization failed",
        })
        sys.exit(1)

    job_id = job.get("job_id", "unknown")
    spec = job.get("spec", {})
    operations = spec.get("operations", [])
    workspace_root = Path(job.get("workspace_root", "."))

    ctx = {"workspace_root": str(workspace_root.resolve())}
    _ensure_workspace_dirs(workspace_root)

    # Pre-validate all operation paths before any file write
    for op_def in operations:
        op_name = op_def.get("op", "")
        if op_name in ("save_blend", "export_glb"):
            p = op_def.get("path", "")
            if p:
                _validate_path(p, workspace_root)
        elif op_name == "render_views":
            bp = op_def.get("path", "blender/renders")
            _validate_path(bp, workspace_root)

    artifacts = []
    scene_summary = None
    _log(f"Starting job {job_id} with {len(operations)} operations")

    for op_def in operations:
        op_name = op_def.get("op", "")
        if op_name not in SAFE_OPS:
            err_msg = f"Unknown or unsafe operation: '{op_name}'"
            _log(err_msg)
            _write_result(result_path, {
                "job_id": job_id,
                "status": "failed",
                "error": err_msg,
                "message": f"Operation '{op_name}' rejected",
                "artifacts": artifacts,
                "scene_summary": scene_summary,
            })
            sys.exit(1)

        try:
            handler = OPERATIONS[op_name]
            result = handler(ctx, op_def)

            # Collect artifacts from known operations
            if op_name in ("save_blend", "export_glb") and "path" in result:
                artifacts.append({
                    "path": result["path"],
                    "type": result.get("type", op_name.replace("save_", "").replace("export_", "")),
                    "size_bytes": result.get("size_bytes"),
                })
            elif op_name == "render_views":
                for r in result.get("rendered", []):
                    artifacts.append({
                        "path": r["path"],
                        "type": "png",
                        "size_bytes": r.get("size_bytes"),
                    })
            elif op_name == "get_scene_summary":
                scene_summary = result

            _log(f"  ✓ {op_name}: {result.get('object', result.get('path', 'ok'))}")

        except Exception as e:
            tb = traceback.format_exc()
            _log(f"  ✗ {op_name} failed: {e}")
            _log(tb)
            _write_result(result_path, {
                "job_id": job_id,
                "status": "failed",
                "error": f"Operation '{op_name}' failed: {e}",
                "message": f"Error during {op_name}",
                "artifacts": artifacts,
                "scene_summary": scene_summary,
            })
            sys.exit(1)

    _log(f"Job {job_id} completed successfully")
    _write_result(result_path, {
        "job_id": job_id,
        "status": "completed",
        "message": f"Executed {len(operations)} operation(s)",
        "artifacts": artifacts,
        "scene_summary": scene_summary,
    })


if __name__ == "__main__":
    main()
