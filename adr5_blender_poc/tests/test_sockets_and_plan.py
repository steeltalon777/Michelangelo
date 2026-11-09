"""Сокеты/коннекторы (V1) и сборка плана: INVALID_SOCKET, bounds/counts."""

import json
import unittest

from helpers import load_registry

from runner import errors, plan_builder, socket_check
from runner.parameter_gate import validate_request


class TestSocketChecks(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = load_registry()
        cls.template = cls.registry.get("v1_roof_assembly")

    def request(self, roof_variant):
        return {
            "item_id": f"item-v1-{roof_variant}",
            "template_id": "v1_roof_assembly",
            "params": {"roof_variant": roof_variant},
            "output_stem": f"v1_{roof_variant}",
        }

    def test_all_choices_pass_sockets(self):
        for roof in ("flat", "gable", "none"):
            resolved = validate_request(self.template, self.request(roof))
            socket_check.check_request_sockets(self.template, resolved.component_choices)

    def test_narrow_roof_refused_invalid_socket(self):
        """INVALID_SOCKET: footprint компонента не накрывает сокет."""
        manifest = json.loads(json.dumps(self.template.manifest))
        manifest["job_plan"]["component_blocks"]["narrow_flat"] = {
            "insert_at_index": 1,
            "connector": {"width": 2.0, "depth": 1.5, "base_z": 2.4},
            "operations": [],
        }
        from runner.template_registry import Template

        template = Template(manifest, self.template.file_path, self.template.sha256)
        request = {
            "item_id": "item-v1-narrow",
            "template_id": "v1_roof_assembly",
            "params": {"roof_variant": "narrow_flat"},
            "output_stem": "v1_narrow",
        }
        with self.assertRaises(errors.PoCError) as ctx:
            socket_check.check_request_sockets(
                template,
                {"roof_variant": "narrow_flat"},
            )
        self.assertEqual(errors.INVALID_SOCKET, ctx.exception.code)

    def test_wrong_base_z_refused(self):
        manifest = json.loads(json.dumps(self.template.manifest))
        manifest["job_plan"]["component_blocks"]["flat"]["connector"]["base_z"] = 1.7
        from runner.template_registry import Template

        template = Template(manifest, self.template.file_path, self.template.sha256)
        with self.assertRaises(errors.PoCError) as ctx:
            socket_check.check_request_sockets(template, {"roof_variant": "flat"})
        self.assertEqual(errors.INVALID_SOCKET, ctx.exception.code)

    def test_unknown_socket_refused(self):
        manifest = json.loads(json.dumps(self.template.manifest))
        manifest["component_choice_parameters"] = {"roof_variant": "ghost_socket"}
        manifest["sockets"] = {}  # ссылка на несуществующий сокет
        from runner.template_registry import Template

        template = Template(manifest, self.template.file_path, self.template.sha256)
        with self.assertRaises(errors.PoCError) as ctx:
            socket_check.check_request_sockets(template, {"roof_variant": "flat"})
        self.assertEqual(errors.UNKNOWN_SOCKET, ctx.exception.code)

    def test_expected_bounds_and_counts(self):
        resolved_flat = validate_request(self.template, self.request("flat"))
        mins, maxs = socket_check.expected_bounds_for(
            self.template, resolved_flat.component_choices
        )
        self.assertEqual([-2.2, -1.7, 0.0], mins)
        self.assertEqual([2.2, 1.7, 2.65], maxs)
        vertices, triangles = socket_check.expected_scene_counts_for(
            self.template, resolved_flat.component_choices
        )
        self.assertEqual((32, 24), (vertices, triangles))

        resolved_gable = validate_request(self.template, self.request("gable"))
        mins, maxs = socket_check.expected_bounds_for(
            self.template, resolved_gable.component_choices
        )
        self.assertEqual([-2.3, -1.76, 0.0], mins)
        self.assertEqual([2.3, 1.76, 3.485423], maxs)
        vertices, triangles = socket_check.expected_scene_counts_for(
            self.template, resolved_gable.component_choices
        )
        self.assertEqual((40, 36), (vertices, triangles))

        resolved_none = validate_request(self.template, self.request("none"))
        mins, maxs = socket_check.expected_bounds_for(
            self.template, resolved_none.component_choices
        )
        self.assertEqual([-2.0, -1.5, 0.0], mins)
        self.assertEqual([2.0, 1.5, 2.4], maxs)


class TestPlanBuilder(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = load_registry()
        cls.v0 = cls.registry.get("v0_palette_master")
        cls.v1 = cls.registry.get("v1_roof_assembly")

    def test_v0_spec_contains_only_whitelisted_ops_and_palette(self):
        resolved = validate_request(
            self.v0,
            {
                "item_id": "item-v0-brick-dark",
                "template_id": "v0_palette_master",
                "params": {"palette_id": "brick-dark"},
                "output_stem": "v0_brick_dark",
            },
        )
        spec = plan_builder.build_job_spec(self.v0, resolved)
        ops = spec["operations"]
        self.assertEqual(8, len(ops))
        self.assertEqual("import_mesh", ops[0]["op"])
        self.assertEqual("inputs/house_master.glb", ops[0]["path"])
        self.assertEqual([0.3, 0.21, 0.17, 1.0], ops[2]["color"])
        self.assertEqual([0.24, 0.18, 0.15, 1.0], ops[3]["color"])
        self.assertEqual([0.14, 0.1, 0.08, 1.0], ops[4]["color"])
        self.assertEqual(
            "outputs/v0_brick_dark/variant.glb", ops[-1]["path"]
        )
        allowed = {
            "create_box", "set_material", "set_camera_set", "render_views",
            "export_glb", "save_blend", "get_scene_summary", "create_cylinder",
            "create_sphere", "import_mesh", "set_transform",
            "create_ortho_camera", "set_camera_pose", "set_camera_settings",
            "frame_camera", "set_render_settings", "render_camera",
            "create_component",
        }
        for op in ops:
            self.assertIn(op["op"], allowed)

    def test_v1_spec_inserts_block_and_binds_outputs(self):
        resolved = validate_request(
            self.v1,
            {
                "item_id": "item-v1-gable",
                "template_id": "v1_roof_assembly",
                "params": {"roof_variant": "gable"},
                "output_stem": "v1_gable",
            },
        )
        spec = plan_builder.build_job_spec(self.v1, resolved)
        ops = spec["operations"]
        self.assertEqual("import_mesh", ops[0]["op"])
        self.assertEqual("create_box", ops[1]["op"])
        self.assertEqual("roof_panel_south", ops[1]["name"])
        self.assertEqual([30.0, 0.0, 0.0], ops[1]["rotation_deg"])
        self.assertEqual("roof_panel_north", ops[2]["name"])
        self.assertEqual("get_scene_summary", ops[3]["op"])
        self.assertEqual("outputs/v1_gable/variant.glb", ops[-1]["path"])

    def test_v1_none_choice_has_no_roof_ops(self):
        resolved = validate_request(
            self.v1,
            {
                "item_id": "item-v1-none",
                "template_id": "v1_roof_assembly",
                "params": {"roof_variant": "none"},
                "output_stem": "v1_none",
            },
        )
        spec = plan_builder.build_job_spec(self.v1, resolved)
        names = [op.get("name") for op in spec["operations"]]
        self.assertNotIn("roof_slab", names)
        self.assertNotIn("roof_panel_south", names)


if __name__ == "__main__":
    unittest.main()
