"""GLB-отчёт: синтетический GLB, geometry_equal, materials_distinct, F13."""

import hashlib
import json
import struct
import tempfile
import unittest
from pathlib import Path

from runner import errors, glb_report, paths


def build_glb(gltf: dict) -> bytes:
    payload = json.dumps(gltf, separators=(",", ":")).encode("utf-8")
    padding = b" " * ((4 - len(payload) % 4) % 4)
    payload += padding
    json_header = struct.pack("<II", len(payload), 0x4E4F534A)
    total = 12 + 8 + len(payload)
    header = struct.pack("<III", 0x46546C67, 2, total)
    return header + json_header + payload


def sample_gltf(base_color=(0.3, 0.2, 0.1, 1.0), name="cube_a"):
    return {
        "asset": {"version": "2.0", "generator": "test"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0, "translation": [0.0, 0.0, 0.0]}],
        "meshes": [
            {
                "name": "mesh_a",
                "primitives": [
                    {
                        "attributes": {"POSITION": 0, "TEXCOORD_0": 1},
                        "indices": 2,
                        "material": 0,
                        "mode": 4,
                    }
                ],
            }
        ],
        "materials": [
            {
                "name": f"Mat_{name}",
                "pbrMetallicRoughness": {"baseColorFactor": list(base_color)},
            }
        ],
        "accessors": [
            {
                "type": "VEC3",
                "count": 8,
                "min": [-1.0, -1.0, -1.0],
                "max": [1.0, 1.0, 1.0],
            },
            {"type": "VEC2", "count": 8},
            {"type": "SCALAR", "count": 36},
        ],
    }


class TestGlbReport(unittest.TestCase):
    def write(self, gltf: dict) -> Path:
        with tempfile.NamedTemporaryFile(suffix=".glb", delete=False) as handle:
            handle.write(build_glb(gltf))
            return Path(handle.name)

    def test_report_fields(self):
        path = self.write(sample_gltf())
        try:
            report = glb_report.report_glb(path)
            self.assertEqual(1, report["meshes_count"])
            self.assertEqual(8, report["total_vertices"])
            self.assertEqual(12, report["total_triangles"])
            self.assertTrue(report["nodes"][0]["has_uv_all_primitives"])
            self.assertEqual("Mat_cube_a", report["materials"][0]["name"])
            self.assertEqual([0.3, 0.2, 0.1, 1.0], report["materials"][0]["base_color_factor"])
            self.assertEqual([-1.0, -1.0, -1.0], report["bounds_local_min"])
        finally:
            path.unlink()

    def test_uv_missing_detected(self):
        gltf = sample_gltf()
        del gltf["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        path = self.write(gltf)
        try:
            report = glb_report.report_glb(path)
            self.assertFalse(report["nodes"][0]["has_uv_all_primitives"])
        finally:
            path.unlink()

    def test_geometry_equal_and_materials_distinct(self):
        a = glb_report.report_glb(self.write(sample_gltf((0.1, 0.2, 0.3, 1.0))))
        b = glb_report.report_glb(self.write(sample_gltf((0.9, 0.8, 0.7, 1.0))))
        equal, reason = glb_report.geometry_equal(a, b)
        self.assertTrue(equal, reason)
        distinct, _reason = glb_report.materials_distinct([a, b])
        self.assertTrue(distinct)

        c = glb_report.report_glb(self.write(sample_gltf((0.1, 0.2, 0.3, 1.0))))
        distinct, reason = glb_report.materials_distinct([a, c])
        self.assertFalse(distinct)

    def test_geometry_difference_detected(self):
        a = glb_report.report_glb(self.write(sample_gltf()))
        modified = sample_gltf()
        modified["accessors"][0]["max"] = [1.0, 1.0, 2.0]  # геометрия изменилась
        b = glb_report.report_glb(self.write(modified))
        equal, reason = glb_report.geometry_equal(a, b)
        self.assertFalse(equal)
        self.assertIn("bounds_local_max", reason)


class TestPaths(unittest.TestCase):
    """F13 на уровне строковых инвариантов путей."""

    def test_valid_paths(self):
        self.assertEqual("outputs/a/variant.glb", paths.validate_relative_path("outputs/a/variant.glb"))
        self.assertEqual("inputs/house_master.glb", paths.validate_relative_path("inputs/house_master.glb"))

    def test_escape_refused(self):
        bad = [
            "../escape.glb",
            "a/../b.glb",
            "/abs.glb",
            "C:\\x.glb",
            "C:x.glb",
            "//server/share.glb",
            "back\\slash.glb",
            "a/b/../../c.glb",
            "",
        ]
        for value in bad:
            with self.assertRaises(errors.PoCError) as ctx:
                paths.validate_relative_path(value)
            self.assertEqual(errors.INVALID_OUTPUT_PATH, ctx.exception.code, value)

    def test_nul_refused(self):
        with self.assertRaises(errors.PoCError):
            paths.validate_relative_path("out\x00put.glb")

    def test_sha256_hex(self):
        self.assertTrue(paths.is_sha256_hex("a" * 64))
        self.assertFalse(paths.is_sha256_hex("A" * 64))
        self.assertFalse(paths.is_sha256_hex("a" * 63))

    def test_sha256_lf_normalization(self):
        """MEDIUM-2: текстовый digest одинаков для CRLF- и LF-байтов."""
        lf_bytes = b'{\n  "a": 1\n}\n'
        with tempfile.TemporaryDirectory() as tmp_name:
            path = Path(tmp_name) / "file.json"
            path.write_bytes(lf_bytes)
            lf_sha = paths.sha256_lf_normalized_file(path)
            # LF-нормализованный digest = sha256 LF-байтов (git blob).
            self.assertEqual(hashlib.sha256(lf_bytes).hexdigest(), lf_sha)

            path.write_bytes(lf_bytes.replace(b"\n", b"\r\n"))
            self.assertEqual(lf_sha, paths.sha256_lf_normalized_file(path))
            # Сырой digest CRLF-файла отличается — поэтому пины только LF.
            self.assertNotEqual(lf_sha, paths.sha256_file(path))


if __name__ == "__main__":
    unittest.main()
