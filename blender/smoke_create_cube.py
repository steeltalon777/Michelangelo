#!/usr/bin/env python3
"""
Michelangelo Blender Smoke Test — minimal standalone script.

Creates a simple cube, saves a .blend file, and optionally exports GLB.
Used for quick Blender availability verification.

Usage:
    blender --background --python smoke_create_cube.py -- <output_dir>
"""

import json
import os
import sys
from pathlib import Path


def main():
    import bpy

    # Parse output directory from args
    try:
        dash_idx = sys.argv.index("--")
        output_dir = Path(sys.argv[dash_idx + 1])
    except (ValueError, IndexError):
        output_dir = Path(".")

    output_dir.mkdir(parents=True, exist_ok=True)
    print(f"[smoke] Output directory: {output_dir}", file=sys.stderr)

    # Remove default cube and re-create cleanly
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)

    # Create a cube
    bpy.ops.mesh.primitive_cube_add(size=2.0, location=(0, 0, 0))
    cube = bpy.context.active_object
    cube.name = "SmokeCube"
    print(f"[smoke] Created cube '{cube.name}'", file=sys.stderr)

    # Add a simple material
    mat = bpy.data.materials.new(name="SmokeMaterial")
    mat.use_nodes = False
    mat.diffuse_color = (0.2, 0.4, 0.8, 1.0)
    cube.data.materials.append(mat)
    print(f"[smoke] Applied material", file=sys.stderr)

    # Save .blend
    blend_path = output_dir / "smoke_cube.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
    blend_size = blend_path.stat().st_size
    print(f"[smoke] Saved .blend: {blend_path} ({blend_size} bytes)", file=sys.stderr)

    # Export GLB (fail gracefully if not supported)
    try:
        glb_path = output_dir / "smoke_cube.glb"
        bpy.ops.export_scene.gltf(
            filepath=str(glb_path),
            export_format='GLB',
            use_selection=False,
        )
        glb_size = glb_path.stat().st_size
        print(f"[smoke] Exported GLB: {glb_path} ({glb_size} bytes)", file=sys.stderr)
    except Exception as e:
        print(f"[smoke] GLB export skipped: {e}", file=sys.stderr)

    # Print summary as JSON to stderr
    summary = {
        "blend": str(blend_path),
        "blend_size_bytes": blend_size,
        "objects": len(bpy.data.objects),
        "materials": len(bpy.data.materials),
    }
    print(f"[smoke] Summary: {json.dumps(summary)}", file=sys.stderr)


if __name__ == "__main__":
    main()
