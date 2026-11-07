#!/usr/bin/env python3
"""Synthetic Stage B benchmark cases (ADR 0004 §13) generator for Blender 5.2.x.

Builds three deterministic, parametric objects and renders them into a
dataset-shaped directory that mirrors the local PoC datasets, so the existing
bench tooling can consume the result unchanged:

    <out>/<case>/solver_input/cameras.json
    <out>/<case>/solver_input/external_dimensions.json
    <out>/<case>/solver_input/views/<view>.png        (8-bit RGB beauty)
    <out>/<case>/solver_input/views/<view>_mask.png   (8-bit gray 0/255 occupancy)
    <out>/<case>/solver_input/views/<view>.json       (per-view camera copy)
    <out>/<case>/evaluation/cameras.json
    <out>/<case>/evaluation/views/<view>.png
    <out>/<case>/evaluation/views/<view>_mask.png
    <out>/<case>/evaluation/views/<view>.json
    <out>/<case>/ground_truth/<case>.blend
    <out>/<case>/ground_truth/meshes/*.obj
    <out>/summary.json

Cases (ADR §13):
    blind_recess    box body with a blind cylindrical recess (boolean difference,
                    applied) plus a rim lip -- the silhouette-hull limit.
    alien_plant     non-symmetric organic plant: bezier trunk, overlapping
                    branches, thin leaves/buds partly occluded.
    composite_lamp  organic glass bulb on a mechanical base (two styles).

Usage (headless):

    blender --background --factory-startup --python tools/bench/make_synthetic_case.py -- \
        --out target/techlead/IT-5/synthetic [--cases blind_recess alien_plant ...] \
        [--resolution 512]

Deterministic: fixed parameters, fixed material colours, fixed camera transforms,
no randomness, no clocks and no timestamps in the produced JSON.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import struct
import sys
import zlib
from array import array
from pathlib import Path

try:  # only importable inside Blender
    import bpy
    import mathutils
except ImportError:  # pragma: no cover - keeps plain-python syntax checks working
    bpy = None
    mathutils = None

CASE_NAMES = ("blind_recess", "alien_plant", "composite_lamp")
RESOLUTION_PX = 512
CLIP_START_M = 0.01
CLIP_END_M = 100.0
LENS_MM = 50.0
SENSOR_WIDTH_MM = 36.0
FRAME_FILL = 0.78              # the object spans ~78 % of the square frame
BACKGROUND_RGB = (0.055, 0.055, 0.065)

#: axis aligned solver views: the camera sits on `dir`, looking at the origin
ORTHO_SOLVER_VIEWS = {
    "front": {"dir": (0.0, -1.0, 0.0), "up": (0.0, 0.0, 1.0)},
    "right": {"dir": (1.0, 0.0, 0.0), "up": (0.0, 0.0, 1.0)},
    "top": {"dir": (0.0, 0.0, 1.0), "up": (0.0, 1.0, 0.0)},
}
#: held-out evaluation views (orthographic, disjoint from the solver set)
ORTHO_EVALUATION_VIEWS = {
    "control_left": {"dir": (-1.0, 0.0, 0.0), "up": (0.0, 0.0, 1.0)},
    "control_rear": {"dir": (0.0, 1.0, 0.0), "up": (0.0, 0.0, 1.0)},
}
#: perspective reference views (rendered for humans only, never scored)
PERSP_REFERENCE_VIEWS = {
    "oblique_01": {"azimuth_deg": 40.0, "elevation_deg": 22.0},
    "oblique_02": {"azimuth_deg": 215.0, "elevation_deg": 14.0},
}

COORDINATE_SYSTEM = {
    "handedness": "right",
    "up": "+Z",
    "right": "+X",
    "forward": "-Y",
    "forward_meaning": "the asset front faces -Y; the front camera sits on -Y and looks +Y",
    "units": "meters",
}
VIEW_CONVENTIONS = {
    "pixel_origin": "top-left",
    "u": "right",
    "v": "down",
    "camera_local_forward": "-Z",
    "camera_local_right": "+X",
    "camera_local_up": "+Y",
    "ortho_scale_is_view_width_for_square_image": True,
    "mask": (
        "8-bit grayscale PNG, 0 = background, 255 = object; geometric occupancy; "
        "alpha >= 0.5 of the transparent-film silhouette render; no hole filling "
        "(true through holes stay background)"
    ),
    "beauty": (
        "8-bit RGB PNG; Workbench flat shading with MATERIAL colours; uniform dark "
        "grey background; no alpha"
    ),
}


# small helpers
def _r(value, digits=6):
    """Round a float for stable JSON output."""
    return round(float(value), digits)


def _vec(values, digits=6):
    return [_r(v, digits) for v in values]


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(payload, indent=2, ensure_ascii=False, sort_keys=False)
    path.write_text(text + "\n", encoding="utf-8")


def write_gray_png(path: Path, width: int, height: int, rows) -> None:
    """Write an 8-bit grayscale (colour type 0) PNG from top-down byte rows."""
    raw = bytearray()
    for row in rows:
        raw.append(0)                     # filter type: None
        raw.extend(row)
    compressed = zlib.compress(bytes(raw), 9)

    def chunk(tag: bytes, payload: bytes) -> bytes:
        crc = struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
        return struct.pack(">I", len(payload)) + tag + payload + crc

    png = bytearray(b"\x89PNG\r\n\x1a\n")
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0))
    png += chunk(b"IDAT", compressed)
    png += chunk(b"IEND", b"")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(bytes(png))


# scene, material and geometry helpers
def reset_scene():
    """Start from an empty scene with a deterministic world background."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    world = bpy.data.worlds.new("SyntheticWorld")
    world.use_nodes = True
    background = world.node_tree.nodes["Background"]
    background.inputs[0].default_value = BACKGROUND_RGB + (1.0,)
    scene.world = world
    return scene


def make_material(name: str, rgb):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    mat.diffuse_color = tuple(rgb) + (1.0,)
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Base Color"].default_value = tuple(rgb) + (1.0,)
    bsdf.inputs["Roughness"].default_value = 0.45
    return mat


def _activate(obj):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    return obj


def _finish(obj, name, mat):
    obj.name = name
    obj.data.name = name + "_mesh"
    if mat is not None:
        obj.data.materials.append(mat)
    return obj


def add_primitive(name, kind, mat, location=(0.0, 0.0, 0.0), **kw):
    """Add a box / cylinder / cone / uv-sphere and name it deterministically."""
    if kind == "box":
        bpy.ops.mesh.primitive_cube_add(size=1.0, location=location)
        obj = bpy.context.active_object
        obj.scale = kw["size"]                     # full extent per axis
        _activate(obj)
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    elif kind == "cylinder":
        bpy.ops.mesh.primitive_cylinder_add(
            vertices=kw.get("vertices", 64), radius=kw["radius"],
            depth=kw["depth"], location=location)
        obj = bpy.context.active_object
    elif kind == "cone":
        bpy.ops.mesh.primitive_cone_add(
            vertices=kw.get("vertices", 64), radius1=kw["radius1"],
            radius2=kw["radius2"], depth=kw["depth"], location=location)
        obj = bpy.context.active_object
    elif kind == "sphere":
        obj = make_uv_sphere(name, kw.get("segments", 48), kw.get("rings", 24),
                             kw["scale"], location)
        obj.data.materials.append(mat)
        return obj
    else:                                          # pragma: no cover
        raise ValueError("unknown primitive: %s" % kind)
    return _finish(obj, name, mat)


def make_uv_sphere(name, segments, rings, scale, location):
    """Deterministic UV sphere (Blender's own operator randomises face order)."""
    verts = [(0.0, 0.0, scale[2])]
    for ring in range(1, rings):
        theta = math.pi * ring / rings
        radius = math.sin(theta)
        z = math.cos(theta)
        for index in range(segments):
            phi = 2.0 * math.pi * index / segments
            verts.append((radius * math.cos(phi) * scale[0],
                          radius * math.sin(phi) * scale[1], z * scale[2]))
    bottom = len(verts)
    verts.append((0.0, 0.0, -scale[2]))

    faces = []
    for index in range(segments):
        nxt = (index + 1) % segments
        faces.append((0, 1 + index, 1 + nxt))              # north fan
    for ring in range(rings - 2):
        base = 1 + ring * segments
        upper = base + segments
        for index in range(segments):
            nxt = (index + 1) % segments
            faces.append((base + index, base + nxt, upper + nxt, upper + index))
    last = 1 + (rings - 2) * segments
    for index in range(segments):
        nxt = (index + 1) % segments
        faces.append((bottom, last + nxt, last + index))  # south fan

    mesh = bpy.data.meshes.new(name + "_mesh")
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    obj.location = location
    return obj


def add_cylinder_between(name, start, end, radius, mat, vertices=32):
    """Cylinder spanning two points (struts / arms)."""
    start, end = mathutils.Vector(start), mathutils.Vector(end)
    delta = end - start
    obj = add_primitive(name, "cylinder", mat, (start + end) * 0.5,
                        radius=radius, depth=delta.length, vertices=vertices)
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = mathutils.Vector((0.0, 0.0, 1.0)).rotation_difference(
        delta.normalized())
    return obj


def add_curve_tube(name, points, bevel_depth, mat, radii=None, resolution=12):
    """Bezier curve with a round bevel, converted to a mesh for the ground truth."""
    curve = bpy.data.curves.new(name + "_curve", type="CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = resolution
    curve.bevel_depth = bevel_depth
    curve.bevel_resolution = 6
    curve.fill_mode = "FULL"
    curve.use_fill_caps = True
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for index, (point, co) in enumerate(zip(spline.bezier_points, points)):
        point.co = co
        point.handle_left_type = "AUTO"
        point.handle_right_type = "AUTO"
        if radii is not None:
            point.radius = radii[index]
    obj = bpy.data.objects.new(name, curve)
    bpy.context.scene.collection.objects.link(obj)
    obj.data.materials.append(mat)
    _activate(obj)
    bpy.ops.object.convert(target="MESH")
    obj = bpy.context.active_object
    obj.name = name
    obj.data.name = name + "_mesh"
    return obj


def add_rect_ring(name, outer, inner, z0, z1, mat):
    """Closed rectangular ring (a lip) built from explicit vertices."""
    ox, oy = outer[0] * 0.5, outer[1] * 0.5
    ix, iy = inner[0] * 0.5, inner[1] * 0.5
    verts = []
    for z in (z0, z1):
        verts += [(-ox, -oy, z), (ox, -oy, z), (ox, oy, z), (-ox, oy, z)]
    for z in (z0, z1):
        verts += [(-ix, -iy, z), (ix, -iy, z), (ix, iy, z), (-ix, iy, z)]
    faces = [
        (0, 1, 9, 8), (1, 2, 10, 9), (2, 3, 11, 10), (3, 0, 8, 11),      # outer
        (5, 4, 12, 13), (6, 5, 13, 14), (7, 6, 14, 15), (4, 7, 15, 12),  # inner
        (8, 9, 13, 12), (9, 10, 14, 13), (10, 11, 15, 14), (11, 8, 12, 15),
        (0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0),
    ]
    mesh = bpy.data.meshes.new(name + "_mesh")
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    obj.data.materials.append(mat)
    return obj


# case builders
def build_blind_recess():
    """0.40 x 0.30 x 0.20 m box with a blind cylindrical recess + a rim lip."""
    mat_body = make_material("BR_BodyMetal", (0.62, 0.64, 0.68))
    mat_rim = make_material("BR_RimPaint", (0.85, 0.48, 0.14))
    mat_recess = make_material("BR_RecessInner", (0.26, 0.27, 0.31))
    mat_bolt = make_material("BR_BoltSteel", (0.40, 0.42, 0.46))

    body = add_primitive("BR_Body", "box", mat_body, (0.0, 0.0, 0.10),
                          size=(0.40, 0.30, 0.20))

    # pristine copy of the body: it becomes the "hypothetical closed box" proof
    closed_copy = body.copy()
    closed_copy.data = body.data.copy()
    closed_copy.name = "BR_BodyClosedProof"
    closed_copy.data.name = "BR_BodyClosedProof_mesh"
    bpy.context.scene.collection.objects.link(closed_copy)
    closed_copy.hide_render = True
    closed_copy.hide_viewport = True

    # blind recess: radius 0.05 m, depth 0.08 m below the top face (z = 0.20);
    # the cutter reaches above the top face so the cut is clean
    cutter = add_primitive("BR_RecessCutter", "cylinder", mat_recess,
                           (0.09, 0.02, 0.18), radius=0.05, depth=0.12, vertices=96)
    modifier = body.modifiers.new("BR_RecessCut", "BOOLEAN")
    modifier.operation = "DIFFERENCE"
    modifier.solver = "EXACT"
    modifier.object = cutter
    if hasattr(modifier, "material_mode"):
        modifier.material_mode = "TRANSFER"
    _activate(body)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    bpy.data.objects.remove(cutter, do_unlink=True)

    # 0.02 m rim lip around the top face (raised 0.02 m, wall 0.03 m)
    rim = add_rect_ring("BR_RimLip", (0.40, 0.30), (0.34, 0.24), 0.19, 0.22, mat_rim)

    bolts = [
        add_primitive("BR_Bolt_%02d" % index, "cylinder", mat_bolt, (bx, by, 0.226),
                      radius=0.013, depth=0.012, vertices=16)
        for index, (bx, by) in enumerate(
            ((-0.165, -0.115), (0.165, -0.115), (-0.165, 0.115), (0.165, 0.115)))
    ]
    return {
        "objects": [body, rim] + bolts,
        "materials": [mat_body, mat_rim, mat_recess, mat_bolt],
        "proof": {"closed_body": closed_copy, "real_body": body},
        "notes": {
            "body": "box body 0.40 x 0.30 x 0.20 m",
            "recess": ("blind cylindrical recess, radius 0.05 m, depth 0.08 m, cut "
                       "into the top face by an applied boolean difference"),
            "rim_lip": "0.02 m high lip around the top face, 0.03 m wall width",
        },
    }


def build_alien_plant():
    """Non-symmetric alien plant: bezier trunk, overlapping branches, thin leaves."""
    mat_bark = make_material("AP_TrunkBark", (0.30, 0.19, 0.12))
    mat_branch = make_material("AP_BranchWood", (0.46, 0.36, 0.18))
    mat_leaf = make_material("AP_Leaf", (0.16, 0.44, 0.17))
    mat_bud = make_material("AP_Bud", (0.78, 0.24, 0.48))

    trunk = add_curve_tube(
        "AP_Trunk",
        [(0.000, 0.000, 0.000), (0.018, 0.012, 0.180),
         (0.046, -0.016, 0.350), (0.030, 0.022, 0.500)],
        0.048, mat_bark, radii=[1.0, 0.86, 0.62, 0.42])

    branch_specs = [
        ("AP_Branch_00", 0.020,
         [(0.022, 0.006, 0.150), (0.100, -0.030, 0.215),
          (0.165, -0.082, 0.290), (0.222, -0.122, 0.345)], [1.0, 0.80, 0.58, 0.38]),
        ("AP_Branch_01", 0.014,
         [(-0.008, -0.012, 0.235), (-0.075, 0.010, 0.300),
          (-0.140, 0.062, 0.362), (-0.196, 0.104, 0.404)], [1.0, 0.78, 0.56, 0.36]),
        ("AP_Branch_02", 0.010,
         [(0.042, 0.000, 0.330), (0.078, 0.060, 0.410),
          (0.104, 0.120, 0.482), (0.128, 0.176, 0.542)], [1.0, 0.76, 0.54, 0.34]),
        ("AP_Branch_03", 0.007,
         [(0.044, -0.004, 0.395), (0.010, -0.070, 0.450),
          (-0.034, -0.128, 0.500), (-0.068, -0.170, 0.528)], [1.0, 0.74, 0.52, 0.32]),
    ]
    branches = [add_curve_tube(name, pts, depth, mat_branch, radii=radii)
                for name, depth, pts, radii in branch_specs]

    # thin leaves / buds; several sit between branches and are partly occluded
    leaf_specs = [
        ("AP_Leaf_00", (0.238, -0.132, 0.352), (0.078, 0.038, 0.0015), (22, -34, 48)),
        ("AP_Leaf_01", (0.172, -0.086, 0.300), (0.062, 0.030, 0.0010), (-16, 28, 22)),
        ("AP_Leaf_02", (-0.206, 0.110, 0.412), (0.070, 0.034, 0.0020), (30, 18, -40)),
        ("AP_Leaf_03", (-0.104, 0.052, 0.358), (0.054, 0.027, 0.0015), (-24, -20, 66)),
        ("AP_Leaf_04", (0.134, 0.184, 0.550), (0.058, 0.029, 0.0018), (14, 40, -18)),
        ("AP_Leaf_05", (0.062, 0.086, 0.452), (0.048, 0.024, 0.0012), (-12, -26, 12)),
        ("AP_Leaf_06", (0.176, 0.030, 0.418), (0.044, 0.022, 0.0010), (18, 22, 70)),
    ]
    bud_specs = [
        ("AP_Bud_00", (-0.076, -0.176, 0.532), (0.024, 0.024, 0.0018), (10, -14, 30)),
        ("AP_Bud_01", (0.034, 0.028, 0.522), (0.026, 0.026, 0.0020), (-8, 16, -22)),
    ]
    leaves, buds = [], []
    for name, location, scale, rotation in leaf_specs:
        leaf = add_primitive(name, "sphere", mat_leaf, location, scale=scale,
                             segments=32, rings=16)
        leaf.rotation_euler = tuple(math.radians(v) for v in rotation)
        leaves.append(leaf)
    for name, location, scale, rotation in bud_specs:
        bud = add_primitive(name, "sphere", mat_bud, location, scale=scale,
                            segments=32, rings=16)
        bud.rotation_euler = tuple(math.radians(v) for v in rotation)
        buds.append(bud)

    return {
        "objects": [trunk] + branches + leaves + buds,
        "materials": [mat_bark, mat_branch, mat_leaf, mat_bud],
        "proof": None,
        "notes": {
            "trunk": "bezier curve with round bevel, radius taper 1.0 -> 0.42",
            "branches": ("4 overlapping bezier branches, bevel radius "
                         "0.020 / 0.014 / 0.010 / 0.007"),
            "leaves": ("7 thin leaves (thickness 0.002 - 0.004 m), several of them "
                       "partly occluded between the branches"),
            "buds": "2 buds (thickness 0.0036 - 0.0040 m)",
        },
    }


def build_composite_lamp():
    """Organic glass bulb on a mechanical base -- two construction styles."""
    mat_glass = make_material("CL_BulbGlass", (0.52, 0.78, 0.83))
    mat_base = make_material("CL_BaseMetal", (0.21, 0.22, 0.25))
    mat_bolt = make_material("CL_BoltSteel", (0.58, 0.59, 0.63))
    mat_collar = make_material("CL_CollarAccent", (0.74, 0.35, 0.13))

    base = add_primitive("CL_BasePlate", "cylinder", mat_base, (0.0, 0.0, 0.0225),
                          radius=0.110, depth=0.045)
    bolts = []
    for index in range(8):
        angle = math.radians(index * 45.0)
        bolts.append(add_primitive(
            "CL_BaseBolt_%02d" % index, "cylinder", mat_bolt,
            (0.085 * math.cos(angle), 0.085 * math.sin(angle), 0.051),
            radius=0.012, depth=0.012, vertices=16))
    arm = add_cylinder_between("CL_ArmStrut", (0.004, 0.0, 0.044),
                                (0.062, 0.010, 0.196), 0.022, mat_collar)
    neck = add_primitive("CL_BulbNeck", "cone", mat_collar, (0.085, 0.013, 0.235),
                          radius1=0.055, radius2=0.032, depth=0.110)
    bulb = add_primitive("CL_GlassBulb", "sphere", mat_glass, (0.085, 0.013, 0.340),
                          scale=(0.115, 0.115, 0.125), segments=64, rings=32)
    return {
        "objects": [base] + bolts + [arm, neck, bulb],
        "materials": [mat_glass, mat_base, mat_bolt, mat_collar],
        "proof": None,
        "notes": {
            "bulb": "organic glass bulb: uv sphere, radii 0.115 x 0.115 x 0.125 m",
            "neck": "cone neck joining the bulb with the base",
            "base": "mechanical base: cylinder plate + ring of 8 bolts",
            "arm": "diagonal strut between the base and the bulb neck",
        },
    }


CASE_BUILDERS = {
    "blind_recess": build_blind_recess,
    "alien_plant": build_alien_plant,
    "composite_lamp": build_composite_lamp,
}


# cameras
def camera_matrix(location, target, up_hint):
    """Blender world matrix (columns = right / up / back) plus its inverse."""
    forward = (mathutils.Vector(target) - mathutils.Vector(location)).normalized()
    back = -forward
    right = mathutils.Vector(up_hint).cross(back)
    if right.length < 1e-9:                       # pragma: no cover - degenerate hint
        right = mathutils.Vector((1.0, 0.0, 0.0)).cross(back)
    right.normalize()
    up = back.cross(right).normalized()
    loc = mathutils.Vector(location)
    matrix = mathutils.Matrix((
        (right.x, up.x, back.x, loc.x),
        (right.y, up.y, back.y, loc.y),
        (right.z, up.z, back.z, loc.z),
        (0.0, 0.0, 0.0, 1.0),
    ))
    return matrix, matrix.inverted(), right, up, forward


def create_camera(name, location, target, up_hint, projection, ortho_scale=None):
    cam_data = bpy.data.cameras.new(name)
    cam_data.type = projection
    cam_data.sensor_fit = "HORIZONTAL"
    cam_data.sensor_width = SENSOR_WIDTH_MM
    cam_data.clip_start = CLIP_START_M
    cam_data.clip_end = CLIP_END_M
    cam_data.shift_x = 0.0
    cam_data.shift_y = 0.0
    if projection == "ORTHO":
        cam_data.ortho_scale = float(ortho_scale)
    else:
        cam_data.lens = LENS_MM
    obj = bpy.data.objects.new(name, cam_data)
    bpy.context.scene.collection.objects.link(obj)
    obj.matrix_world = camera_matrix(location, target, up_hint)[0]
    return obj


def world_matrix(obj):
    """World matrix built from the object properties (bound_box and matrix_world
    are only refreshed by a depsgraph evaluation, which may not have run yet)."""
    rotation = obj.rotation_quaternion if obj.rotation_mode == "QUATERNION" \
        else obj.rotation_euler
    return mathutils.Matrix.LocRotScale(obj.location, rotation, obj.scale)


def bbox_of(objects):
    """World axis aligned bounding box (min, max) over the given geometry."""
    lo = mathutils.Vector((1e9, 1e9, 1e9))
    hi = mathutils.Vector((-1e9, -1e9, -1e9))
    for obj in objects:
        matrix = world_matrix(obj)
        for vertex in obj.data.vertices:
            point = matrix @ vertex.co
            for axis in range(3):
                lo[axis] = min(lo[axis], point[axis])
                hi[axis] = max(hi[axis], point[axis])
    return lo, hi


def projected_extent(lo, hi, right, up):
    corners = [mathutils.Vector((x, y, z))
               for x in (lo.x, hi.x) for y in (lo.y, hi.y) for z in (lo.z, hi.z)]
    us = [c.dot(right) for c in corners]
    vs = [c.dot(up) for c in corners]
    return max(us) - min(us), max(vs) - min(vs)


def _dir_label(direction):
    names = [("+" if axis > 0 else "-") + name
             for axis, name in zip(direction, ("X", "Y", "Z")) if abs(axis) > 0.5]
    return "".join(names) if names else "?"


# rendering
def configure_render(scene, resolution, silhouette):
    render = scene.render
    render.engine = "BLENDER_WORKBENCH"
    render.resolution_x = resolution
    render.resolution_y = resolution
    render.resolution_percentage = 100
    render.pixel_aspect_x = 1.0
    render.pixel_aspect_y = 1.0
    render.film_transparent = bool(silhouette)
    render.image_settings.file_format = "PNG"
    render.image_settings.color_depth = "8"
    render.image_settings.color_mode = "RGBA" if silhouette else "RGB"
    render.use_overwrite = True
    # keep the material colours faithful (no AgX desaturation)
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.view_settings.exposure = 0.0
    scene.view_settings.gamma = 1.0
    shading = scene.display.shading
    shading.light = "FLAT"
    shading.background_type = "WORLD"
    shading.show_shadows = False
    shading.show_cavity = False
    shading.show_specular_highlight = False
    shading.show_object_outline = False
    shading.show_backface_culling = False
    if silhouette:
        shading.color_type = "SINGLE"
        shading.single_color = (1.0, 1.0, 1.0)
    else:
        shading.color_type = "MATERIAL"


KEEP_PNG_CHUNKS = {b"IHDR", b"sRGB", b"gAMA", b"cHRM", b"pHYs", b"IDAT", b"IEND"}


def strip_png_metadata(path: Path) -> None:
    """Drop ancillary chunks (EXIF/text) so the file is byte reproducible."""
    data = path.read_bytes()
    out = bytearray(b"\x89PNG\r\n\x1a\n")
    pos = 8
    while pos + 8 <= len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        tag = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        if tag in KEEP_PNG_CHUNKS:
            crc = struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
            out += struct.pack(">I", length) + tag + payload + crc
        pos += 12 + length
    path.write_bytes(bytes(out))


def render_to_file(scene, camera, path: Path):
    scene.camera = camera
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    if not path.is_file():
        raise RuntimeError("render did not produce %s" % path)


def alpha_to_mask(rgba_path: Path, mask_path: Path):
    """Convert a transparent-film RGBA render into an 8-bit 0/255 grayscale PNG.

    Returns (width, height, rows, covered_pixels); rows are top-down byte rows.
    """
    image = bpy.data.images.load(str(rgba_path))
    try:
        width, height = image.size
        buffer = array("f", [0.0]) * (width * height * 4)
        try:
            image.pixels.foreach_get(buffer)
        except AttributeError:                   # pragma: no cover - old Blender
            buffer = array("f", image.pixels[:])
    finally:
        bpy.data.images.remove(image)

    alphas = bytes(min(255, max(0, int(value * 255.0 + 0.5)))
                   for value in buffer[3::4])
    table = bytes(255 if value >= 128 else 0 for value in range(256))
    rows, covered = [], 0
    for row in range(height):
        # Blender stores the pixels bottom-up, PNG rows are top-down
        source = height - 1 - row
        line = alphas[source * width:(source + 1) * width].translate(table)
        covered += line.count(255)
        rows.append(line)
    write_gray_png(mask_path, width, height, rows)
    return width, height, rows, covered


def render_view(scene, camera, views_dir: Path, view_id: str, resolution: int):
    """Render the beauty image and the geometric occupancy mask of one view."""
    views_dir.mkdir(parents=True, exist_ok=True)
    beauty_path = views_dir / ("%s.png" % view_id)
    mask_path = views_dir / ("%s_mask.png" % view_id)

    configure_render(scene, resolution, silhouette=False)
    render_to_file(scene, camera, beauty_path)
    strip_png_metadata(beauty_path)

    configure_render(scene, resolution, silhouette=True)
    tmp_path = views_dir / ("_%s_alpha.png" % view_id)
    render_to_file(scene, camera, tmp_path)
    width, height, rows, covered = alpha_to_mask(tmp_path, mask_path)
    os.remove(tmp_path)
    return {
        "beauty": beauty_path,
        "mask": mask_path,
        "width": width,
        "height": height,
        "rows": rows,
        "coverage": covered / float(width * height),
    }


# ground truth export
def export_meshes(objects, meshes_dir: Path):
    """Export every ground truth component as a world-frame OBJ (metres, +Z up)."""
    meshes_dir.mkdir(parents=True, exist_ok=True)
    for obj in objects:
        bpy.ops.object.select_all(action="DESELECT")
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        path = meshes_dir / ("%s.obj" % obj.name)
        bpy.ops.wm.obj_export(
            filepath=str(path),
            export_selected_objects=True,
            apply_modifiers=True,
            export_materials=True,
            forward_axis="Y",
            up_axis="Z",
        )
        header = ("# ground truth component %s\n"
                  "# units: metres, right handed, +Z up, world coordinates\n"
                  % obj.name)
        path.write_text(header + path.read_text(encoding="utf-8"), encoding="utf-8")


def triangle_count(objects):
    total = 0
    for obj in objects:
        if obj.type != "MESH":
            continue
        for polygon in obj.data.polygons:
            total += max(0, len(polygon.vertices) - 2)
    return total


# view payloads
def view_payload(view_id, group, role, projection, resolution, location, target,
                 right, up, forward, matrix, inverse, files, ortho_scale=None,
                 lens_mm=None, extra=None):
    payload = {
        "view_id": view_id,
        "group": group,
        "role": role,
        "projection": projection,
        "resolution_px": {"width": resolution, "height": resolution},
        "pixel_aspect": 1.0,
        "sensor_fit": "HORIZONTAL",
        "sensor_width_mm": SENSOR_WIDTH_MM,
        "shift_x": 0.0,
        "shift_y": 0.0,
        "clip_start_m": CLIP_START_M,
        "clip_end_m": CLIP_END_M,
        "camera_location_m": _vec(location),
        "target_m": _vec(target),
        "camera_axes_world": {"right": _vec(right), "up": _vec(up),
                              "forward": _vec(forward)},
        "camera_to_world": [[_r(v) for v in row] for row in matrix],
        "world_to_camera": [[_r(v) for v in row] for row in inverse],
        "camera_to_world_convention": (
            "row-major 4x4; the columns are the camera right/up/back axes in world "
            "space, the translation is the camera location"),
        "files": {"beauty_png": files["beauty"], "mask_png": files["mask"]},
    }
    if projection == "orthographic":
        payload["ortho_scale_m"] = _r(ortho_scale)
        payload["ortho_pixel_size_m"] = _r(ortho_scale / resolution)
        payload["lens_mm"] = None
    else:
        payload["ortho_scale_m"] = None
        payload["ortho_pixel_size_m"] = None
        payload["lens_mm"] = _r(lens_mm, 3)
    if extra:
        payload.update(extra)
    return payload


def build_ortho_view(scene, case_dir, view_id, spec, group, role, lo, hi, resolution):
    """Calibrated orthographic view; ortho_scale fits ~FRAME_FILL of the frame."""
    centre = (lo + hi) * 0.5
    extent = max(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z)
    distance = 2.0 + 2.0 * extent
    direction = mathutils.Vector(spec["dir"]).normalized()
    location = centre + direction * distance
    target = centre
    up_hint = spec["up"]

    camera = create_camera("CAM_" + view_id, location, target, up_hint, "ORTHO",
                            ortho_scale=1.0)
    matrix, inverse, right, up, forward = camera_matrix(location, target, up_hint)
    width, height = projected_extent(lo, hi, right, up)
    ortho_scale = max(width, height) / FRAME_FILL
    camera.data.ortho_scale = ortho_scale

    sub = "solver_input" if group == "solver_input" else "evaluation"
    rendered = render_view(scene, camera, case_dir / sub / "views", view_id, resolution)
    payload = view_payload(
        view_id, group, role, "orthographic", resolution, location, target,
        right, up, forward, matrix, inverse,
        {"beauty": rendered["beauty"].name, "mask": rendered["mask"].name},
        ortho_scale=ortho_scale,
        extra={
            "object_frame_fill": _r(max(width, height) / ortho_scale, 4),
            "description": "%s orthographic, camera on %s looking %s" % (
                view_id, _dir_label(spec["dir"]),
                _dir_label([-v for v in spec["dir"]])),
        })
    return payload, rendered


def build_perspective_view(scene, case_dir, view_id, spec, lo, hi, resolution):
    """Perspective reference view (50 mm look-at camera), never scored in v0."""
    centre = (lo + hi) * 0.5
    azimuth = math.radians(spec["azimuth_deg"])
    elevation = math.radians(spec["elevation_deg"])
    direction = mathutils.Vector((
        math.cos(elevation) * math.sin(azimuth),
        -math.cos(elevation) * math.cos(azimuth),
        math.sin(elevation)))
    radius = 0.5 * (hi - lo).length
    half_fov = math.atan(0.5 * SENSOR_WIDTH_MM / LENS_MM)
    distance = radius / math.sin(half_fov) * 1.15
    location = centre + direction * distance
    target = centre
    up_hint = (0.0, 0.0, 1.0)

    camera = create_camera("CAM_" + view_id, location, target, up_hint, "PERSP")
    matrix, inverse, right, up, forward = camera_matrix(location, target, up_hint)
    rendered = render_view(scene, camera, case_dir / "evaluation" / "views",
                           view_id, resolution)
    payload = view_payload(
        view_id, "evaluation", "reference_perspective", "perspective", resolution,
        location, target, right, up, forward, matrix, inverse,
        {"beauty": rendered["beauty"].name, "mask": rendered["mask"].name},
        lens_mm=LENS_MM,
        extra={
            "distance_m": _r(distance),
            "azimuth_deg": _r(spec["azimuth_deg"], 3),
            "elevation_deg": _r(spec["elevation_deg"], 3),
            "scored": False,
            "note": "perspective reference view; not a metric constraint in v0",
        })
    return payload, rendered


# proof helper: is the blind recess silhouette observable?
def recess_proof(scene, case, case_dir, top_payload, top_rows, resolution):
    """Compare the real top mask with the top mask of a hypothetical closed box.

    The recess is blind, so the top silhouette of the real object is identical to
    the silhouette of the same body without the recess: a visual hull cannot
    recover it, therefore the solver has to report it as a limit.
    """
    proof = case["proof"]
    closed, real = proof["closed_body"], proof["real_body"]
    closed.hide_render = False
    closed.hide_viewport = False
    real.hide_render = True
    real.hide_viewport = True

    scratch = case_dir / "_proof"
    scratch.mkdir(parents=True, exist_ok=True)
    camera = create_camera("CAM_proof_top", top_payload["camera_location_m"],
                            top_payload["target_m"], (0.0, 1.0, 0.0), "ORTHO",
                            ortho_scale=top_payload["ortho_scale_m"])
    configure_render(scene, resolution, silhouette=True)
    alpha_path = scratch / "closed_top_alpha.png"
    render_to_file(scene, camera, alpha_path)
    _w, _h, closed_rows, closed_px = alpha_to_mask(alpha_path,
                                                    scratch / "closed_top_mask.png")

    closed.hide_render = True
    closed.hide_viewport = True
    real.hide_render = False
    real.hide_viewport = False
    bpy.data.objects.remove(camera, do_unlink=True)
    shutil.rmtree(scratch, ignore_errors=True)

    real_px = sum(row.count(255) for row in top_rows)
    intersection = sum(
        sum(1 for x, y in zip(row_a, row_b) if x == 255 and y == 255)
        for row_a, row_b in zip(top_rows, closed_rows))
    union = real_px + closed_px - intersection
    return {
        "hypothetical_closed_box": ("same body and rim with the blind recess "
                                    "filled (no boolean cut)"),
        "top_mask_object_px": real_px,
        "closed_box_top_mask_object_px": closed_px,
        "intersection_px": intersection,
        "union_px": union,
        "top_mask_iou": _r(intersection / float(union) if union else 0.0, 6),
        "differing_px": real_px + closed_px - 2 * intersection,
    }


# case generation
def generate_case(case_name, out_root: Path, resolution: int):
    case_dir = out_root / case_name
    if case_dir.exists():
        shutil.rmtree(case_dir)
    case_dir.mkdir(parents=True, exist_ok=True)

    scene = reset_scene()
    case = CASE_BUILDERS[case_name]()
    objects = case["objects"]
    lo, hi = bbox_of(objects)
    size = hi - lo

    solver_payloads, evaluation_payloads, coverages = [], [], {}
    mask_rows = {}
    for view_id, spec in ORTHO_SOLVER_VIEWS.items():
        payload, rendered = build_ortho_view(
            scene, case_dir, view_id, spec, "solver_input", "primary_orthographic",
            lo, hi, resolution)
        solver_payloads.append(payload)
        coverages[view_id] = rendered["coverage"]
        mask_rows[view_id] = rendered["rows"]
    for view_id, spec in ORTHO_EVALUATION_VIEWS.items():
        payload, rendered = build_ortho_view(
            scene, case_dir, view_id, spec, "evaluation", "heldout_control",
            lo, hi, resolution)
        evaluation_payloads.append(payload)
        coverages[view_id] = rendered["coverage"]
    for view_id, spec in PERSP_REFERENCE_VIEWS.items():
        payload, rendered = build_perspective_view(
            scene, case_dir, view_id, spec, lo, hi, resolution)
        evaluation_payloads.append(payload)
        coverages[view_id] = rendered["coverage"]

    render_defaults = {
        "engine": "BLENDER_WORKBENCH", "shading": "FLAT",
        "color_type": "MATERIAL", "resolution_x": resolution,
        "resolution_y": resolution, "resolution_percentage": 100,
    }
    solver_dir = case_dir / "solver_input"
    write_json(solver_dir / "cameras.json", {
        "schema_version": "1.0", "asset_id": case_name, "dataset_id": case_name,
        "synthetic": True, "coordinate_system": dict(COORDINATE_SYSTEM),
        "view_conventions": dict(VIEW_CONVENTIONS),
        "render_defaults": dict(render_defaults, film_transparent_beauty=False,
                                 film_transparent_silhouette=True),
        "note": "Calibrated orthographic solver views (front/right/top).",
        "views": solver_payloads,
    })
    for payload in solver_payloads:
        write_json(solver_dir / "views" / ("%s.json" % payload["view_id"]), payload)
    write_json(solver_dir / "external_dimensions.json", {
        "schema_version": "1.0", "asset_id": case_name, "units": "meters",
        "coordinate_system": dict(COORDINATE_SYSTEM),
        "overall_bbox_m": {"x": _r(size.x), "y": _r(size.y), "z": _r(size.z)},
        "note": "allowed externally measurable dimensions only",
    })

    evaluation_dir = case_dir / "evaluation"
    write_json(evaluation_dir / "cameras.json", {
        "schema_version": "1.0", "asset_id": case_name, "dataset_id": case_name,
        "synthetic": True, "hidden_control_set": True,
        "coordinate_system": dict(COORDINATE_SYSTEM),
        "view_conventions": dict(VIEW_CONVENTIONS),
        "render_defaults": render_defaults,
        "note": ("Held-out orthographic controls plus perspective reference views; "
                 "only the orthographic controls are scored in v0."),
        "views": evaluation_payloads,
    })
    for payload in evaluation_payloads:
        write_json(evaluation_dir / "views" / ("%s.json" % payload["view_id"]),
                   payload)

    ground_truth = case_dir / "ground_truth"
    ground_truth.mkdir(parents=True, exist_ok=True)
    export_meshes(objects, ground_truth / "meshes")
    bpy.ops.wm.save_as_mainfile(
        filepath=str(ground_truth / ("%s.blend" % case_name)), check_existing=False)

    summary = {
        "case": case_name,
        "views": {"solver_input": [p["view_id"] for p in solver_payloads],
                  "evaluation": [p["view_id"] for p in evaluation_payloads]},
        "mask_coverage": {key: _r(value, 6) for key, value in coverages.items()},
        "overall_bbox_m": {"x": _r(size.x), "y": _r(size.y), "z": _r(size.z)},
        "bbox_min_m": _vec(lo), "bbox_max_m": _vec(hi),
        "object_count": len(objects),
        "triangle_count": triangle_count(objects),
        "material_count": len(case["materials"]),
        "notes": case["notes"],
    }
    if case.get("proof"):
        top_payload = next(p for p in solver_payloads if p["view_id"] == "top")
        summary["blind_recess_note"] = {
            "claim": ("the blind recess is NOT silhouette observable: the top view "
                      "mask of the real object is identical to the top view mask of "
                      "the same body with the recess filled, so a visual hull can "
                      "not recover it and the solver must report it as a limit"),
            "proof": recess_proof(scene, case, case_dir, top_payload,
                                  mask_rows["top"], resolution),
        }
    return summary


def main(argv):
    parser = argparse.ArgumentParser(
        description="Generate the synthetic Stage B benchmark cases.")
    parser.add_argument("--out", required=True, help="output root directory")
    parser.add_argument("--cases", nargs="*", default=list(CASE_NAMES),
                        help="cases to generate")
    parser.add_argument("--resolution", type=int, default=RESOLUTION_PX)
    args = parser.parse_args(argv)

    if bpy is None:
        raise SystemExit("this script must run inside Blender (bpy)")
    out_root = Path(args.out).expanduser().resolve()
    out_root.mkdir(parents=True, exist_ok=True)

    summaries = {}
    for case_name in args.cases:
        if case_name not in CASE_BUILDERS:
            raise SystemExit("unknown case: %s" % case_name)
        print("[synthetic] generating %s" % case_name)
        summaries[case_name] = generate_case(case_name, out_root, args.resolution)

    write_json(out_root / "summary.json", {
        "schema": "michelangelo.synthetic-summary/0",
        "generator": "tools/bench/make_synthetic_case.py",
        "blender_version": bpy.app.version_string,
        "resolution_px": args.resolution,
        "deterministic": True,
        "cases": summaries,
    })
    print("[synthetic] done: %s" % out_root)
    return 0


if __name__ == "__main__":
    script_args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    sys.exit(main(script_args))
