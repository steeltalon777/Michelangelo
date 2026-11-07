"""Unit tests for the Stage B solver (Python standard library only).

Run with:
    python tools/solver/tests/test_solver.py
    python -m unittest discover -s tools/solver/tests -t .
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

_HERE = os.path.dirname(os.path.abspath(__file__))
_SOLVER = os.path.dirname(_HERE)
if _SOLVER not in sys.path:
    sys.path.insert(0, _SOLVER)

import carve  # noqa: E402
import mask as maskmod  # noqa: E402
import mesh as meshmod  # noqa: E402
import pngread  # noqa: E402
import supplements as supp  # noqa: E402

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SOLVE_PY = os.path.join(_SOLVER, "solve.py")
COMPARE_PY = os.path.join(_SOLVER, "compare.py")


# --- PNG helpers ------------------------------------------------------------
def _chunk(chunk_type: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + chunk_type
        + data
        + struct.pack(">I", zlib.crc32(chunk_type + data) & 0xFFFFFFFF)
    )


def _paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def encode_png(width, height, color_type, pixel_rows, filters=None, bit_depth=8,
               interlace=0):
    """Encode raw rows into a PNG byte string (test helper)."""
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color_type]
    stride = width * channels
    filters = list(filters) if filters is not None else [0] * height
    raw = bytearray()
    prev = bytearray(stride)
    for y in range(height):
        row = bytearray(pixel_rows[y])
        if len(row) != stride:
            raise ValueError("row %d has %d samples, expected %d" % (y, len(row), stride))
        ftype = filters[y]
        raw.append(ftype)
        if ftype == 0:
            line = bytearray(row)
        elif ftype == 1:
            line = bytearray(stride)
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                line[i] = (row[i] - left) & 0xFF
        elif ftype == 2:
            line = bytearray(stride)
            for i in range(stride):
                line[i] = (row[i] - prev[i]) & 0xFF
        elif ftype == 3:
            line = bytearray(stride)
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                line[i] = (row[i] - ((left + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:
            line = bytearray(stride)
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                up_left = prev[i - channels] if i >= channels else 0
                line[i] = (row[i] - _paeth(left, prev[i], up_left)) & 0xFF
        else:
            raise ValueError("unsupported filter %d" % ftype)
        raw += line
        prev = row
    header = struct.pack(
        ">IIBBBBB", width, height, bit_depth, color_type, 0, 0, interlace
    )
    return (
        PNG_SIGNATURE
        + _chunk(b"IHDR", header)
        + _chunk(b"IDAT", zlib.compress(bytes(raw)))
        + _chunk(b"IEND", b"")
    )


def gray_rows(width, height, value=0):
    return [bytes([value]) * width for _ in range(height)]


def rect_rows(width, height, x0, y0, x1, y1, inside=255, outside=0):
    rows = []
    for y in range(height):
        row = bytearray(width)
        for x in range(width):
            row[x] = inside if (x0 <= x <= x1 and y0 <= y <= y1) else outside
        rows.append(bytes(row))
    return rows


def rgba_rows(width, height, x0, y0, x1, y1):
    rows = []
    for y in range(height):
        row = bytearray()
        for x in range(width):
            inside = x0 <= x <= x1 and y0 <= y <= y1
            row += bytes([10, 20, 30, 255 if inside else 0])
        rows.append(bytes(row))
    return rows


def write_png(path, data):
    with open(path, "wb") as handle:
        handle.write(data)


# --- geometry helpers -------------------------------------------------------
def make_view(view_id="front", location=(0.0, -4.0, 0.0), look_at=(0.0, 0.0, 0.0),
              up=(0.0, 0.0, 1.0), ortho=1.0, size=64):
    return maskmod.View(view_id, location, look_at, up, ortho, size, size)


def mask_of_rect(width, height, x0, y0, x1, y1):
    mask = maskmod.Mask(width, height)
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            mask.set(x, y)
    return mask


def mask_of_box(view, box_min, box_max):
    """Silhouette (filled rectangle) of an axis aligned box for an ortho view."""
    us = []
    vs = []
    for x in (box_min[0], box_max[0]):
        for y in (box_min[1], box_max[1]):
            for z in (box_min[2], box_max[2]):
                u, v = view.project_point((x, y, z))
                us.append(u)
                vs.append(v)
    x0 = int(math.floor(min(us)))
    x1 = int(math.ceil(max(us))) - 1
    y0 = int(math.floor(min(vs)))
    y1 = int(math.ceil(max(vs))) - 1
    return mask_of_rect(view.width_px, view.height_px, x0, y0, x1, y1)


def box_voxel_set(nx=1, ny=1, nz=1, size=1.0):
    grid = carve.Grid(min_corner=(0.0, 0.0, 0.0), max_corner=(nx * size, ny * size, nz * size),
                      dims=(nx, ny, nz), voxel_size=size)
    indices = frozenset(range(grid.count()))
    return carve.VoxelSet(grid, indices)


def solid_voxel_set(nx, ny, nz, size=1.0):
    return box_voxel_set(nx, ny, nz, size)


# --- package fixture --------------------------------------------------------
def build_package(root, size=16, units="m", ortho=1.0, rect=(4, 4, 11, 11),
                  measurements=None, supplements=None, unknowns=None,
                  views=None, extra_shadows=False):
    """Create a tiny but valid input package in root; returns the package dir."""
    package_dir = os.path.join(root, "package")
    os.makedirs(os.path.join(package_dir, "images"), exist_ok=True)
    os.makedirs(os.path.join(package_dir, "masks"), exist_ok=True)
    if views is None:
        views = [
            {
                "view_id": "front",
                "location": [0.0, -4.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
                "up": [0.0, 0.0, 1.0],
            },
            {
                "view_id": "right",
                "location": [4.0, 0.0, 0.0],
                "look_at": [0.0, 0.0, 0.0],
                "up": [0.0, 0.0, 1.0],
            },
        ]
    manifest_views = []
    for spec in views:
        view_id = spec["view_id"]
        mask_path = os.path.join(package_dir, "masks", "%s.png" % view_id)
        image_path = os.path.join(package_dir, "images", "%s.png" % view_id)
        write_png(mask_path, encode_png(size, size, 0, rect_rows(size, size, *rect)))
        write_png(image_path, encode_png(size, size, 2, [
            bytes([7, 8, 9]) * size for _ in range(size)
        ]))
        manifest_views.append(
            {
                "view_id": view_id,
                "kind": "calibrated",
                "image": {
                    "path": "images/%s.png" % view_id,
                    "sha256": _sha256(image_path),
                    "width_px": size,
                    "height_px": size,
                },
                "mask": {
                    "path": "masks/%s.png" % view_id,
                    "sha256": _sha256(mask_path),
                    "format": "grayscale8",
                    "foreground": "white",
                },
                "projection": {
                    "type": "orthographic",
                    "location_m": spec["location"],
                    "look_at_m": spec["look_at"],
                    "up_world": spec["up"],
                    "ortho_scale_m": ortho,
                    "shift_x": 0.0,
                    "shift_y": 0.0,
                    "clip_start_m": 0.02,
                    "clip_end_m": 100.0,
                },
                "provenance": {"origin": "ai_author", "detail": "test fixture"},
            }
        )
    manifest = {
        "schema": "michelangelo.input-package/0",
        "package_id": "solver-test-fixture",
        "object": {"name": "Solver test fixture"},
        "revision": 1,
        "units": units,
        "coordinate_system": {
            "handedness": "right",
            "up": "+Z",
            "right": "+X",
            "forward": "-Y",
        },
        "views": manifest_views,
        "parts": [{"part_id": "body", "label": "body"}],
        "supplements": supplements or [],
        "unknowns": unknowns or [],
        "measurements": measurements or [],
        "uncertainty_policy": {
            "allowed_completion_rules": ["thickness", "closure"],
            "stub_policy": "allowed",
            "require_author_acceptance": False,
        },
        "provenance": {"origin": "ai_author", "detail": "test fixture"},
        "acceptance": [],
    }
    with open(os.path.join(package_dir, "package.json"), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
    return package_dir


def _sha256(path):
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def build_renders(root, package_dir, size=16, rect=(4, 4, 11, 11), view_ids=None):
    """Create silhouette renders (RGBA8) matching the package masks."""
    render_dir = os.path.join(root, "renders")
    os.makedirs(render_dir, exist_ok=True)
    with open(os.path.join(package_dir, "package.json"), "r", encoding="utf-8") as handle:
        manifest = json.load(handle)
    if view_ids is None:
        view_ids = [v["view_id"] for v in manifest["views"] if v["kind"] == "calibrated"]
    for view_id in view_ids:
        write_png(
            os.path.join(render_dir, "%s.png" % view_id),
            encode_png(size, size, 6, rgba_rows(size, size, *rect)),
        )
    return render_dir


# --- PNG decoder tests ------------------------------------------------------
class PngDecodeTests(unittest.TestCase):
    def test_decode_gray8(self):
        data = encode_png(3, 2, 0, [bytes([0, 255, 128]), bytes([255, 0, 64])])
        image = pngread.read_png_bytes(data)
        self.assertEqual((image.width, image.height, image.mode), (3, 2, "gray"))
        self.assertEqual(image.channels, 1)
        self.assertEqual(image.sample(0, 0), (0,))
        self.assertEqual(image.sample(1, 0), (255,))
        self.assertEqual(image.sample(2, 1), (64,))

    def test_decode_rgb8(self):
        rows = [bytes([1, 2, 3, 4, 5, 6]), bytes([7, 8, 9, 10, 11, 12])]
        image = pngread.read_png_bytes(encode_png(2, 2, 2, rows))
        self.assertEqual(image.mode, "rgb")
        self.assertEqual(image.channels, 3)
        self.assertEqual(image.sample(0, 0), (1, 2, 3))
        self.assertEqual(image.sample(1, 1), (10, 11, 12))

    def test_decode_rgba8(self):
        rows = [bytes([0, 0, 0, 0, 1, 1, 1, 255]), bytes([2, 2, 2, 128]) * 2]
        image = pngread.read_png_bytes(encode_png(2, 1, 6, rows))
        self.assertEqual(image.mode, "rgba")
        self.assertEqual(image.sample(0, 0), (0, 0, 0, 0))
        self.assertEqual(image.sample(1, 0), (1, 1, 1, 255))

    def test_decode_gray_alpha(self):
        rows = [bytes([5, 255, 6, 0])]
        image = pngread.read_png_bytes(encode_png(2, 1, 4, rows))
        self.assertEqual(image.mode, "gray_alpha")
        self.assertEqual(image.sample(1, 0), (6, 0))

    def test_all_filter_types(self):
        width, height = 4, 5
        base = [
            bytes(((x * 7 + y * 13) % 256) for x in range(width))
            for y in range(height)
        ]
        for ftype in (0, 1, 2, 3, 4):
            data = encode_png(width, height, 0, base, filters=[ftype] * height)
            image = pngread.read_png_bytes(data)
            self.assertEqual(list(image.pixels), [v for row in base for v in row],
                             "filter %d decoded incorrectly" % ftype)

    def test_mixed_filters(self):
        width, height = 3, 4
        base = [
            bytes(((x * 11 + y * 5) % 256) for x in range(width))
            for y in range(height)
        ]
        data = encode_png(width, height, 0, base, filters=[0, 1, 2, 4])
        image = pngread.read_png_bytes(data)
        self.assertEqual(list(image.pixels), [v for row in base for v in row])

    def test_bad_signature_rejected(self):
        with self.assertRaises(pngread.PngError):
            pngread.read_png_bytes(b"NOTAPNG" + b"\x00" * 32)

    def test_truncated_rejected(self):
        data = encode_png(2, 2, 0, [bytes([1, 2]), bytes([3, 4])])
        with self.assertRaises(pngread.PngError):
            pngread.read_png_bytes(data[:len(data) // 2])

    def test_interlaced_rejected(self):
        data = encode_png(2, 2, 0, [bytes([1, 2]), bytes([3, 4])], interlace=1)
        with self.assertRaises(pngread.PngError) as ctx:
            pngread.read_png_bytes(data)
        self.assertIn("interlaced", str(ctx.exception).lower())

    def test_palette_rejected(self):
        data = encode_png(2, 2, 3, [bytes([1, 2]), bytes([3, 4])])
        with self.assertRaises(pngread.PngError) as ctx:
            pngread.read_png_bytes(data)
        self.assertIn("palette", str(ctx.exception).lower())

    def test_sixteen_bit_rejected(self):
        data = encode_png(2, 2, 0, [bytes([1, 2]), bytes([3, 4])], bit_depth=16)
        with self.assertRaises(pngread.PngError) as ctx:
            pngread.read_png_bytes(data)
        self.assertIn("bit depth", str(ctx.exception).lower())

    def test_mask_from_gray_threshold(self):
        rows = [bytes([0, 127, 128, 255])]
        image = pngread.read_png_bytes(encode_png(4, 1, 0, rows))
        mask = pngread.mask_from_gray(image)
        self.assertFalse(mask.at(0, 0))
        self.assertFalse(mask.at(1, 0))
        self.assertTrue(mask.at(2, 0))
        self.assertTrue(mask.at(3,  0))
        self.assertEqual(mask.count(), 2)

    def test_mask_from_alpha_threshold(self):
        rows = [bytes([0, 0, 0, 0, 0, 0, 0, 127, 0, 0, 0, 128])]
        image = pngread.read_png_bytes(encode_png(3, 1, 6, rows))
        mask = pngread.mask_from_alpha(image)
        self.assertFalse(mask.at(0, 0))
        self.assertFalse(mask.at(1, 0))
        self.assertTrue(mask.at(2, 0))
        self.assertEqual(mask.count(), 1)

    def test_mask_from_png_dispatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            gray = os.path.join(tmp, "g.png")
            rgba = os.path.join(tmp, "r.png")
            write_png(gray, encode_png(2, 1, 0, [bytes([0, 255])]))
            write_png(rgba, encode_png(2, 1, 6, [bytes([0, 0, 0, 0, 0, 0, 0, 255])]))
            self.assertEqual(pngread.mask_from_png(gray).count(), 1)
            self.assertEqual(pngread.mask_from_png(rgba).count(), 1)


# --- mask tests -------------------------------------------------------------
class MaskTests(unittest.TestCase):
    def test_bbox(self):
        mask = mask_of_rect(10, 10, 2, 3, 5, 7)
        self.assertEqual(mask.bbox(), (2, 3, 5, 7))
        self.assertEqual(maskmod.Mask(4, 4).bbox(), None)

    def test_area_and_coverage(self):
        mask = mask_of_rect(10, 10, 0, 0, 9, 9)
        self.assertEqual(mask.count(), 100)
        self.assertAlmostEqual(mask.coverage(), 1.0)
        half = mask_of_rect(10, 10, 0, 0, 4, 9)
        self.assertAlmostEqual(half.coverage(), 0.5)

    def test_iou_identical(self):
        mask = mask_of_rect(10, 10, 1, 1, 4, 4)
        self.assertEqual(maskmod.mask_iou(mask, mask), 1.0)

    def test_iou_disjoint(self):
        a = mask_of_rect(10, 10, 0, 0, 2, 2)
        b = mask_of_rect(10, 10, 7, 7, 9, 9)
        self.assertEqual(maskmod.mask_iou(a, b), 0.0)

    def test_iou_half_overlap(self):
        a = mask_of_rect(10, 10, 0, 0, 4, 9)
        b = mask_of_rect(10, 10, 5, 0, 9, 9)
        # 50 px each, no intersection
        self.assertEqual(maskmod.mask_iou(a, b), 0.0)
        c = mask_of_rect(10, 10, 0, 0, 4, 4)
        d = mask_of_rect(10, 10, 3, 3, 7, 7)
        # 25 px each, 4 px shared, union 46 px
        self.assertAlmostEqual(maskmod.mask_iou(c, d), 4 / 46.0)

    def test_iou_empty_masks(self):
        empty = maskmod.Mask(4, 4)
        self.assertEqual(maskmod.mask_iou(empty, empty), 1.0)

    def test_iou_size_mismatch(self):
        with self.assertRaises(maskmod.MaskError):
            maskmod.mask_iou(maskmod.Mask(4, 4), maskmod.Mask(5, 5))


# --- view / projection tests ------------------------------------------------
class ViewTests(unittest.TestCase):
    def test_projection_origin_is_centre(self):
        view = make_view(size=1024)
        u, v = view.project_point((0.0, 0.0, 0.0))
        self.assertAlmostEqual(u, 512.0)
        self.assertAlmostEqual(v, 512.0)

    def test_projection_x_offset(self):
        view = make_view(size=1024)
        u, v = view.project_point((0.5, 0.0, 0.0))
        self.assertAlmostEqual(u, 1024.0)
        self.assertAlmostEqual(v, 512.0)

    def test_projection_z_offset_flips_v(self):
        view = make_view(size=1024)
        u, v = view.project_point((0.0, 0.0, 0.25))
        self.assertAlmostEqual(u, 512.0)
        self.assertAlmostEqual(v, 256.0)

    def test_projection_is_unit_invariant(self):
        metres = make_view(size=1024, ortho=1.0)
        centimetres = make_view(size=1024, ortho=0.01)  # 1 cm expressed in metres
        u_m, v_m = metres.project_point((0.5, 0.0, 0.0))
        u_c, v_c = centimetres.project_point((0.005, 0.0, 0.0))
        self.assertAlmostEqual(u_m, u_c)
        self.assertAlmostEqual(v_m, v_c)

    def test_manifest_units_are_converted(self):
        entry = {
            "view_id": "front",
            "kind": "calibrated",
            "image": {"width_px": 1024, "height_px": 1024},
            "projection": {
                "type": "orthographic",
                "location_m": [0.0, -400.0, 0.0],
                "look_at_m": [0.0, 0.0, 0.0],
                "up_world": [0.0, 0.0, 1.0],
                "ortho_scale_m": 100.0,
            },
        }
        view = maskmod.View.from_manifest(entry, "cm")
        self.assertAlmostEqual(view.ortho_scale_m, 1.0)
        self.assertAlmostEqual(view.location_m[1], -4.0)
        u, v = view.project_point((0.5, 0.0, 0.0))
        self.assertAlmostEqual(u, 1024.0)
        self.assertAlmostEqual(v, 512.0)

    def test_shift_is_rejected(self):
        with self.assertRaises(maskmod.ShiftNotSupportedError):
            maskmod.View("front", (0, -4, 0), (0, 0, 0), (0, 0, 1), 1.0, 64, 64,
                         shift_x=0.5)

    def test_manifest_shift_is_rejected(self):
        entry = {
            "view_id": "front",
            "image": {"width_px": 64, "height_px": 64},
            "projection": {
                "type": "orthographic",
                "location_m": [0, -4, 0],
                "look_at_m": [0, 0, 0],
                "up_world": [0, 0, 1],
                "ortho_scale_m": 1.0,
                "shift_y": 0.25,
            },
        }
        with self.assertRaises(maskmod.ShiftNotSupportedError):
            maskmod.View.from_manifest(entry, "m")

    def test_unproject_pixel_round_trip(self):
        view = make_view(size=1024)
        point = (0.1, 0.0, -0.2)
        u, v = view.project_point(point)
        back = view.unproject_pixel(u, v)
        for a, b in zip(point, back):
            self.assertAlmostEqual(a, b, places=9)

    def test_unproject_pixel_round_trip_non_square(self):
        # Non-square images use the same pixel scale on both axes
        # (width_px / ortho_scale_m); the visible height follows from the aspect.
        view = maskmod.View("front", (0.0, -4.0, 0.0), (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), 1.0, 512, 256)
        point = (0.1, 0.0, -0.2)
        u, v = view.project_point(point)
        self.assertAlmostEqual(u, (0.5 + 0.1) * 512)
        self.assertAlmostEqual(v, (0.5 + 0.2) * 512)
        back = view.unproject_pixel(u, v)
        for a, b in zip(point, back):
            self.assertAlmostEqual(a, b, places=9)
        bbox = view.pixel_bbox_to_camera((100, 50, 300, 150))
        self.assertAlmostEqual(bbox[0], (100 / 512 - 0.5))
        self.assertAlmostEqual(bbox[3], (0.5 - 50 / 512))


# --- carve tests ------------------------------------------------------------
class CarveTests(unittest.TestCase):
    def setUp(self):
        self.front = make_view("front", (0, -4, 0), (0, 0, 0), (0, 0, 1), 1.0, 64)
        self.right = make_view("right", (4, 0, 0), (0, 0, 0), (0, 0, 1), 1.0, 64)
        self.top = make_view("top", (0, 0, 4), (0, 0, 0), (0, 1, 0), 1.0, 64)

    def test_three_views_recover_a_box(self):
        box_min = (-0.2, -0.2, -0.2)
        box_max = (0.2, 0.2, 0.2)
        pairs = [
            (self.front, mask_of_box(self.front, box_min, box_max)),
            (self.right, mask_of_box(self.right, box_min, box_max)),
            (self.top, mask_of_box(self.top, box_min, box_max)),
        ]
        lo, hi = carve.derive_hull_bbox(pairs)
        for axis in range(3):
            self.assertAlmostEqual(lo[axis], box_min[axis], delta=0.01)
            self.assertAlmostEqual(hi[axis], box_max[axis], delta=0.01)
        grid, _ = carve.make_grid(lo, hi, 32)
        hull = carve.carve(grid, pairs)
        self.assertGreater(hull.count(), 0)
        blo, bhi = hull.bbox()
        voxel = grid.voxel_size
        for axis in range(3):
            self.assertLess(abs(blo[axis] - box_min[axis]), voxel * 1.5)
            self.assertLess(abs(bhi[axis] - box_max[axis]), voxel * 1.5)

    def test_two_views_carve_a_slab(self):
        pairs = [
            (self.front, mask_of_rect(64, 64, 19, 19, 44, 44)),
            (self.right, mask_of_rect(64, 64, 19, 19, 44, 44)),
        ]
        lo, hi = carve.derive_hull_bbox(pairs)
        grid, _ = carve.make_grid(lo, hi, 16)
        hull = carve.carve(grid, pairs)
        self.assertGreater(hull.count(), 0)
        blo, bhi = hull.bbox()
        self.assertAlmostEqual(bhi[0] - blo[0], hi[0] - lo[0], delta=grid.voxel_size)
        self.assertAlmostEqual(bhi[1] - blo[1], hi[1] - lo[1], delta=grid.voxel_size)

    def test_single_view_is_unbounded(self):
        pairs = [(self.front, mask_of_rect(64, 64, 19, 19, 44, 44))]
        with self.assertRaises(carve.CarveError):
            carve.derive_hull_bbox(pairs)

    def test_parallel_views_are_unbounded(self):
        second = make_view("front2", (0, 4, 0), (0, 0, 0), (0, 0, 1), 1.0, 64)
        pairs = [
            (self.front, mask_of_rect(64, 64, 19, 19, 44, 44)),
            (second, mask_of_rect(64, 64, 19, 19, 44, 44)),
        ]
        with self.assertRaises(carve.CarveError):
            carve.derive_hull_bbox(pairs)

    def test_empty_mask_is_rejected(self):
        pairs = [(self.front, maskmod.Mask(64, 64))]
        with self.assertRaises(carve.CarveError):
            carve.derive_hull_bbox(pairs)

    def test_grid_resolution_and_caps(self):
        grid, used = carve.make_grid((0, 0, 0), (1, 1, 1), 64)
        self.assertEqual(used, 64)
        self.assertEqual(grid.dims, (64, 64, 64))
        _, capped = carve.make_grid((0, 0, 0), (1, 1, 1), 1000)
        self.assertEqual(capped, carve.MAX_RESOLUTION)

    def test_voxel_set_bbox(self):
        voxels = solid_voxel_set(2, 3, 4, 0.5)
        lo, hi = voxels.bbox()
        self.assertEqual(lo, (0.0, 0.0, 0.0))
        self.assertEqual(hi, (1.0, 1.5, 2.0))


# --- supplement tests -------------------------------------------------------
class SupplementTests(unittest.TestCase):
    def setUp(self):
        self.grid = carve.Grid((0.0, 0.0, 0.0), (1.0, 1.0, 1.0), (4, 4, 4), 0.25)
        self.solid = solid_voxel_set(4, 4, 4, 0.25)
        self.solid = carve.VoxelSet(self.grid, frozenset(range(self.grid.count())))

    def test_thickness_hollows_the_interior(self):
        hull, outcomes, diagnostics = supp.apply_supplements(
            [{"supplement_id": "t", "kind": "thickness", "geometry": {"value_m": 0.25}}],
            self.solid, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "applied")
        self.assertLess(hull.count(), self.solid.count())
        distance = supp._surface_distance(self.solid)
        self.assertEqual(max(distance.values()), 2)
        # the outermost layer survives, the centre does not
        self.assertTrue(hull.contains(0, 0, 0))
        self.assertFalse(hull.contains(1, 1, 1))

    def test_cavity_removes_only_interior(self):
        hull, outcomes, _ = supp.apply_supplements(
            [{"supplement_id": "c", "kind": "cavity",
              "geometry": {"bbox_m": [0.5, 0.5, 0.5]}}],
            self.solid, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "applied")
        self.assertLess(hull.count(), self.solid.count())
        self.assertTrue(hull.contains(0, 0, 0))
        self.assertFalse(hull.contains(1, 1, 1))

    def test_closure_adds_a_capsule(self):
        before = self.solid.count()
        hull, outcomes, _ = supp.apply_supplements(
            [{"supplement_id": "k", "kind": "closure",
              "geometry": {"from": [-1.0, 0.5, 0.5], "to": [2.0, 0.5, 0.5],
                           "radius_m": 0.2}}],
            self.solid, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "applied")
        self.assertEqual(outcomes[0].mode, "added")
        self.assertGreater(outcomes[0].voxels.count(), 0)
        # the hull itself is untouched by an additive rule
        self.assertEqual(hull.count(), before)

    def test_curve_adds_a_chain(self):
        hull, outcomes, _ = supp.apply_supplements(
            [{"supplement_id": "cv", "kind": "curve",
              "geometry": {"points_m": [[-1.0, 0.5, 0.5], [0.5, 0.5, 0.5],
                                         [2.0, 0.5, 0.5]], "radius_m": 0.2}}],
            self.solid, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "applied")
        self.assertGreater(outcomes[0].voxels.count(), 0)
        self.assertEqual(hull.count(), self.solid.count())

    def test_symmetry_intersects(self):
        symmetric = carve.VoxelSet(
            self.grid,
            frozenset(
                self.grid.index(i, j, k)
                for i in range(4)
                for j in range(4)
                for k in range(4)
                if abs((i + 0.5) - 2.0) == abs((k + 0.5) - 2.0)
            ),
        )
        hull, outcomes, _ = supp.apply_supplements(
            [{"supplement_id": "s", "kind": "symmetry", "geometry": {"axis": "x"}}],
            symmetric, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "applied")
        self.assertLessEqual(hull.count(), symmetric.count())
        for index in hull.indices:
            i, j, k = self.grid.coords(index)
            mirror = self.grid.index(3 - i, j, k)
            self.assertIn(mirror, hull.indices)

    def test_unsupported_kind_is_reported(self):
        hull, outcomes, diagnostics = supp.apply_supplements(
            [{"supplement_id": "s", "kind": "section",
              "geometry": {"points_m": [[0, 0], [1, 1]]}}],
            self.solid, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "unsupported")
        self.assertEqual(hull.count(), self.solid.count())
        self.assertTrue(any(d["code"] == "SUPPLEMENT_UNSUPPORTED_KIND"
                            for d in diagnostics))

    def test_invalid_geometry_is_rejected(self):
        hull, outcomes, diagnostics = supp.apply_supplements(
            [{"supplement_id": "s", "kind": "thickness", "geometry": {}}],
            self.solid, self.grid, ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
        )
        self.assertEqual(outcomes[0].status, "rejected")
        self.assertEqual(hull.count(), self.solid.count())
        self.assertTrue(any(d["code"] == "SUPPLEMENT_GEOMETRY_INVALID"
                            for d in diagnostics))

    def test_rule_allowed_unknown_is_resolved_by_a_supplement(self):
        stubs, results, diagnostics = supp.resolve_unknowns(
            [{"id": "u_rule", "policy": "rule_allowed", "affected_parts": ["body"]}],
            ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
            {"body": ["wall"]},
        )
        self.assertEqual(stubs, [])
        self.assertEqual(results[0]["status"], "resolved")
        self.assertIn("wall", results[0]["detail"])

    def test_rule_allowed_unknown_without_rule_is_unresolved(self):
        stubs, results, diagnostics = supp.resolve_unknowns(
            [{"id": "u_rule", "policy": "rule_allowed", "affected_parts": ["body"]}],
            ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
            {},
        )
        self.assertEqual(results[0]["status"], "unresolved")

    def test_unknown_stub_and_unresolved(self):
        stubs, results, diagnostics = supp.resolve_unknowns(
            [
                {"id": "u_stub", "policy": "stub_allowed", "affected_parts": []},
                {"id": "u_author", "policy": "author_required", "affected_parts": []},
            ],
            ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0)),
            {},
        )
        self.assertEqual(len(stubs), 1)
        self.assertEqual(stubs[0][0], "u_stub")
        self.assertEqual(results[0]["status"], "stub")
        self.assertEqual(results[1]["status"], "unresolved")
        self.assertTrue(any(d["code"] == "UNKNOWN_UNRESOLVED" for d in diagnostics))


# --- mesh tests -------------------------------------------------------------
class MeshTests(unittest.TestCase):
    def test_unit_cube_has_six_quads(self):
        voxels = box_voxel_set(1, 1, 1, 1.0)
        vertices, faces = meshmod.voxel_surface(voxels)
        self.assertEqual(len(faces), 6)
        self.assertEqual(len(vertices), 8)
        self.assertTrue(all(len(face) == 4 for face in faces))

    def test_vertices_are_deduplicated(self):
        voxels = solid_voxel_set(2, 2, 2, 1.0)
        vertices, faces = meshmod.voxel_surface(voxels)
        self.assertEqual(len(vertices), len(set(vertices)))
        # 8 voxels, the 12 hidden faces are not emitted
        self.assertEqual(len(faces), 24)

    def test_obj_parses_and_has_object_name(self):
        with tempfile.TemporaryDirectory() as tmp:
            voxels = box_voxel_set(1, 1, 1, 1.0)
            path = os.path.join(tmp, "blockout.obj")
            meshmod.write_obj(path, "blockout", voxels)
            with open(path, "r", encoding="utf-8") as handle:
                lines = handle.read().strip().split("\n")
            self.assertEqual(lines[1], "o blockout")
            vertex_lines = [l for l in lines if l.startswith("v ")]
            face_lines = [l for l in lines if l.startswith("f ")]
            self.assertEqual(len(vertex_lines), 8)
            self.assertEqual(len(face_lines), 6)
            for line in face_lines:
                indices = [int(value) for value in line.split()[1:]]
                self.assertEqual(len(indices), 4)
                for index in indices:
                    self.assertGreaterEqual(index, 1)
                    self.assertLessEqual(index, len(vertex_lines))

    def test_winding_is_outward(self):
        voxels = box_voxel_set(1, 1, 1, 1.0)
        vertices, faces = meshmod.voxel_surface(voxels)
        centre = (0.5, 0.5, 0.5)
        for face in faces:
            p0 = vertices[face[0]]
            p1 = vertices[face[1]]
            p2 = vertices[face[2]]
            u = [p1[a] - p0[a] for a in range(3)]
            v = [p2[a] - p1[a] for a in range(3)]
            normal = (
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            )
            mid = [(p0[a] + p1[a] + p2[a]) / 3.0 for a in range(3)]
            outward = [mid[a] - centre[a] for a in range(3)]
            dot = sum(normal[a] * outward[a] for a in range(3))
            self.assertGreater(dot, 0.0, "face %s is not wound outwards" % (face,))

    def test_box_mesh_has_six_quads(self):
        vertices, faces = meshmod.box_mesh((0, 0, 0), (1, 2, 3))
        self.assertEqual(len(vertices), 8)
        self.assertEqual(len(faces), 6)


# --- compare tests ----------------------------------------------------------
class CompareTests(unittest.TestCase):
    def test_identical_masks_iou_one(self):
        mask = mask_of_rect(16, 16, 4, 4, 11, 11)
        metrics = _metrics(mask, mask)
        self.assertEqual(metrics["iou"], 1.0)
        self.assertEqual(metrics["intersection_px"], 64)
        self.assertEqual(metrics["union_px"], 64)

    def test_disjoint_masks_iou_zero(self):
        a = mask_of_rect(16, 16, 0, 0, 3, 3)
        b = mask_of_rect(16, 16, 10, 10, 15, 15)
        self.assertEqual(_metrics(a, b)["iou"], 0.0)

    def test_half_overlap_exact(self):
        a = mask_of_rect(16, 16, 0, 0, 7, 15)
        b = mask_of_rect(16, 16, 8, 0, 15, 15)
        self.assertEqual(_metrics(a, b)["iou"], 0.0)
        c = mask_of_rect(16, 16, 0, 0, 7, 15)
        d = mask_of_rect(16, 16, 4, 0, 15, 15)
        # 128 px each, 64 px shared, union 256 px
        self.assertAlmostEqual(_metrics(c, d)["iou"], 64 / 256.0)

    def test_alpha_threshold_of_renders(self):
        rows = [bytes([0, 0, 0, 0, 0, 0, 0, 127, 0, 0, 0, 128])]
        image = pngread.read_png_bytes(encode_png(3, 1, 6, rows))
        mask = pngread.mask_from_alpha(image)
        self.assertEqual(mask.count(), 1)


def _metrics(mask, render):
    from mask import mask_counts

    intersection, union, mask_px, render_px = mask_counts(mask, render)
    return {
        "iou": 1.0 if union == 0 else intersection / union,
        "intersection_px": intersection,
        "union_px": union,
        "mask_px": mask_px,
        "render_px": render_px,
    }


# --- solver CLI tests -------------------------------------------------------
class SolverCliTests(unittest.TestCase):
    def test_solve_produces_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(os.path.isfile(os.path.join(out, "blockout.obj")))
            self.assertTrue(os.path.isfile(os.path.join(out, "build_report.json")))
            with open(os.path.join(out, "build_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
            self.assertEqual(report["schema"], "michelangelo.build-report/0")
            self.assertEqual(report["package"]["package_id"], "solver-test-fixture")

    def test_report_top_level_keys(self):
        expected = {
            "basis", "components", "diagnostics", "limits", "measurements",
            "package", "schema", "solver", "supplements", "unknowns",
        }
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            out = os.path.join(tmp, "out")
            subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                capture_output=True, text=True, check=True,
            )
            with open(os.path.join(out, "build_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        self.assertEqual(set(report.keys()), expected)

    def test_determinism(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            out_a = os.path.join(tmp, "out_a")
            out_b = os.path.join(tmp, "out_b")
            for out in (out_a, out_b):
                subprocess.run(
                    [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                    capture_output=True, text=True, check=True,
                )
            for name in ("blockout.obj", "build_report.json"):
                with open(os.path.join(out_a, name), "rb") as handle:
                    first = handle.read()
                with open(os.path.join(out_b, name), "rb") as handle:
                    second = handle.read()
                self.assertEqual(first, second, "%s is not deterministic" % name)

    def test_measurement_pass_and_fail(self):
        measurements = [
            {"id": "height", "kind": "height", "value_m": 0.5, "tolerance_m": 0.01},
            {"id": "width", "kind": "width", "value_m": 9.0, "tolerance_m": 0.01},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp, measurements=measurements)
            out = os.path.join(tmp, "out")
            subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                capture_output=True, text=True, check=True,
            )
            with open(os.path.join(out, "build_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        by_id = {entry["id"]: entry for entry in report["measurements"]}
        self.assertTrue(by_id["height"]["within_tolerance"])
        self.assertFalse(by_id["width"]["within_tolerance"])
        self.assertGreater(by_id["width"]["relative_error"], 0.05)

    def test_measurement_in_centimetres(self):
        measurements = [
            {"id": "height", "kind": "height", "value_m": 0.5, "tolerance_m": 0.01},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp, units="cm", measurements=measurements)
            out = os.path.join(tmp, "out")
            subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                capture_output=True, text=True, check=True,
            )
            with open(os.path.join(out, "build_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        entry = report["measurements"][0]
        self.assertTrue(entry["within_tolerance"])
        self.assertAlmostEqual(entry["measured"], 0.005, places=6)

    def test_supplements_and_unknowns_are_reported(self):
        supplements = [
            {"supplement_id": "wall", "kind": "thickness",
             "geometry": {"value_m": 0.1}, "part_id": "body"},
            {"supplement_id": "sec", "kind": "section",
             "geometry": {"points_m": [[0, 0], [1, 1]]}},
        ]
        unknowns = [
            {"id": "u_stub", "policy": "stub_allowed", "affected_parts": []},
            {"id": "u_author", "policy": "author_required", "affected_parts": []},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp, supplements=supplements, unknowns=unknowns)
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(os.path.isfile(os.path.join(out, "supplement_wall.obj")))
            self.assertTrue(os.path.isfile(os.path.join(out, "stub_u_stub.obj")))
            with open(os.path.join(out, "build_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        status = {entry["supplement_id"]: entry["status"]
                  for entry in report["supplements"]}
        self.assertEqual(status["wall"], "applied")
        self.assertEqual(status["sec"], "unsupported")
        unknown_status = {entry["id"]: entry["status"] for entry in report["unknowns"]}
        self.assertEqual(unknown_status["u_stub"], "stub")
        self.assertEqual(unknown_status["u_author"], "unresolved")

    def test_invalid_package_exits_two(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            with open(os.path.join(package, "package.json"), "r", encoding="utf-8") as h:
                manifest = json.load(h)
            manifest["schema"] = "something/9"
            with open(os.path.join(package, "package.json"), "w", encoding="utf-8") as h:
                json.dump(manifest, h)
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out],
                capture_output=True, text=True,
            )
        self.assertEqual(result.returncode, 2)

    def test_missing_package_exits_two(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(
                [sys.executable, SOLVE_PY, "--package",
                 os.path.join(tmp, "nope"), "--out", os.path.join(tmp, "out")],
                capture_output=True, text=True,
            )
        self.assertEqual(result.returncode, 2)

    def test_json_flag_prints_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, SOLVE_PY, "--package", package, "--out", out, "--json"],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(result.stdout)
        self.assertEqual(report["schema"], "michelangelo.build-report/0")


# --- comparator CLI tests ---------------------------------------------------
class CompareCliTests(unittest.TestCase):
    def test_compare_passes_for_identical_renders(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            renders = build_renders(tmp, package)
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, COMPARE_PY, "--package", package,
                 "--renders", renders, "--out", out, "--json"],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            with open(os.path.join(out, "quality_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        self.assertEqual(report["schema"], "michelangelo.quality-report/0")
        self.assertEqual(report["thresholds"]["status"], "pass")
        self.assertAlmostEqual(report["source"]["mean_iou"], 1.0)
        self.assertAlmostEqual(report["source"]["min_iou"], 1.0)
        self.assertIsNone(report["evaluation"])

    def test_compare_fails_for_empty_renders(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            renders = build_renders(tmp, package, rect=(0, 0, 0, 0))
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, COMPARE_PY, "--package", package,
                 "--renders", renders, "--out", out],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 1)
            with open(os.path.join(out, "quality_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        self.assertEqual(report["thresholds"]["status"], "fail")

    def test_compare_with_evaluation_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            renders = build_renders(tmp, package)
            eval_package = build_package(os.path.join(tmp, "eval"))
            eval_renders = build_renders(os.path.join(tmp, "eval"), eval_package)
            out = os.path.join(tmp, "out")
            result = subprocess.run(
                [sys.executable, COMPARE_PY, "--package", package,
                 "--renders", renders, "--out", out,
                 "--evaluation-package", eval_package,
                 "--evaluation-renders", eval_renders, "--json"],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(result.stdout)
        self.assertIsNotNone(report["evaluation"])
        self.assertAlmostEqual(report["evaluation"]["mean_iou"], 1.0)
        self.assertEqual(len(report["evaluation"]["views"]), 2)

    def test_quality_report_keys(self):
        expected = {
            "schema", "source", "evaluation", "measurements", "thresholds",
            "diagnostics",
        }
        with tempfile.TemporaryDirectory() as tmp:
            package = build_package(tmp)
            renders = build_renders(tmp, package)
            out = os.path.join(tmp, "out")
            subprocess.run(
                [sys.executable, COMPARE_PY, "--package", package,
                 "--renders", renders, "--out", out],
                capture_output=True, text=True, check=True,
            )
            with open(os.path.join(out, "quality_report.json"), "r", encoding="utf-8") as h:
                report = json.load(h)
        self.assertEqual(set(report.keys()), expected)
        view = report["source"]["views"][0]
        self.assertEqual(
            set(view.keys()),
            {"view_id", "iou", "mask_px", "render_px", "intersection_px",
             "union_px", "mask_coverage", "render_coverage"},
        )


if __name__ == "__main__":
    unittest.main()
