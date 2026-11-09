"""Гейт параметров: неизвестные параметры (F10), enum/диапазоны, F13 пути."""

import unittest

from helpers import load_registry

from runner import errors, parameter_gate


class TestParameterGateV0(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = load_registry()
        cls.template = cls.registry.get("v0_palette_master")

    def valid_request(self, **overrides):
        request = {
            "item_id": "item-v0-brick-dark",
            "template_id": "v0_palette_master",
            "params": {"palette_id": "brick-dark"},
            "output_stem": "v0_brick_dark",
        }
        request.update(overrides)
        return request

    def test_valid_request_resolves_palette(self):
        resolved = parameter_gate.validate_request(self.template, self.valid_request())
        self.assertEqual(
            {"walls": [0.3, 0.21, 0.17, 1.0], "roof": [0.24, 0.18, 0.15, 1.0],
             "door": [0.14, 0.1, 0.08, 1.0]},
            resolved.palette,
        )

    def test_unknown_parameter_refused(self):
        """F10: параметр вне зарегистрированного маппинга отвергается."""
        request = self.valid_request()
        request["params"]["python_source"] = "import os; os.system('rm -rf /')"
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(self.template, request)
        self.assertEqual(errors.UNKNOWN_PARAMETER, ctx.exception.code)

    def test_invalid_enum_value_refused(self):
        request = self.valid_request()
        request["params"]["palette_id"] = "magenta-neon"
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(self.template, request)
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)

    def test_missing_parameter_refused(self):
        request = self.valid_request()
        request["params"] = {}
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(self.template, request)
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)

    def test_wrong_template_id_refused(self):
        request = self.valid_request()
        request["template_id"] = "other_template"
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(self.template, request)
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)

    def test_output_path_escape_refused(self):
        """F13: '..'/absolute/backslash/drive в output_stem/path отвергаются."""
        bad_outputs = [
            "../escape",
            "a/../b",
            "abs/path",
            "back\\slash",
            "C:drive",
            ".hidden-stem",
            "-leading-dash",
        ]
        for stem in bad_outputs:
            request = self.valid_request(output_stem=stem)
            with self.assertRaises(errors.PoCError) as ctx:
                parameter_gate.validate_request(self.template, request)
            self.assertEqual(
                errors.INVALID_OUTPUT_PATH, ctx.exception.code, f"stem={stem!r}"
            )

    def test_unsafe_item_id_refused(self):
        request = self.valid_request(item_id="ITEM WITH SPACES")
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(self.template, request)
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)


class TestParameterGateNumbers(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = load_registry()
        cls.template = cls.registry.get("v0_palette_master")

    def test_number_out_of_range_refused(self):
        """Числовые параметры: диапазон из манифеста обязателен."""
        template_manifest = self.template.manifest
        template_manifest["parameters"]["wear"] = {
            "type": "number",
            "minimum": 0.0,
            "maximum": 0.35,
        }
        from runner.template_registry import Template

        template = Template(template_manifest, self.template.file_path, self.template.sha256)
        request = {
            "item_id": "item-wear",
            "template_id": "v0_palette_master",
            "params": {"palette_id": "brick-dark", "wear": 0.9},
            "output_stem": "v0_wear",
        }
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(template, request)
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)

    def test_nan_string_number_refused(self):
        from runner.template_registry import Template

        template_manifest = dict(self.template.manifest)
        template_manifest["parameters"] = dict(template_manifest["parameters"])
        template_manifest["parameters"]["wear"] = {
            "type": "number",
            "minimum": 0.0,
            "maximum": 0.35,
        }
        template = Template(template_manifest, self.template.file_path, self.template.sha256)
        request = {
            "item_id": "item-wear",
            "template_id": "v0_palette_master",
            "params": {"palette_id": "brick-dark", "wear": "0.2; import os"},
            "output_stem": "v0_wear",
        }
        with self.assertRaises(errors.PoCError) as ctx:
            parameter_gate.validate_request(template, request)
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)


if __name__ == "__main__":
    unittest.main()
