"""Реестр шаблонов: пиннинг байтов, запрет кода (F10), защита ролей (F9)."""

import json
import shutil
import tempfile
import unittest
from pathlib import Path

from helpers import TEMPLATES_DIR

from runner import errors, paths
from runner.template_registry import TemplateRegistry, validate_template_manifest


def make_registry(tmp: Path, mutate_registry=None, mutate_template=None):
    """Копия реального каталога шаблонов с опциональными мутациями."""
    shutil.copytree(TEMPLATES_DIR, tmp / "templates")
    registry_path = tmp / "templates" / "registry.json"
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    if mutate_registry is not None:
        mutate_registry(registry)
        registry_path.write_text(json.dumps(registry, indent=2), encoding="utf-8")
    if mutate_template is not None:
        mutate_template(tmp / "templates")
        # Пересчитывать хэши нельзя: мутация должна ловиться как mismatch,
        # либо тест сам обновляет реестр под свои нужды.
    return registry_path


class TestRegistryPinning(unittest.TestCase):
    def test_real_registry_loads(self):
        registry = TemplateRegistry.load(TEMPLATES_DIR)
        self.assertIn("v0_palette_master", registry.template_ids)
        self.assertIn("v1_roof_assembly", registry.template_ids)
        self.assertIn("v0_textured_probe", registry.template_ids)

    def test_tampered_template_file_refused(self):
        """TEMPLATE_HASH_MISMATCH: байты шаблона не совпали с реестром."""
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)

            def mutate(templates_dir: Path):
                path = templates_dir / "v0_palette_master.json"
                manifest = json.loads(path.read_text(encoding="utf-8"))
                manifest["description"] = "tampered by test"
                path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

            make_registry(tmp, mutate_template=mutate)
            with self.assertRaises(errors.PoCError) as ctx:
                TemplateRegistry.load(tmp / "templates")
            self.assertEqual(errors.TEMPLATE_HASH_MISMATCH, ctx.exception.code)

    def test_missing_template_file(self):
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)

            def mutate(registry):
                registry["templates"] = [
                    entry for entry in registry["templates"]
                    if entry["template_id"] != "v0_palette_master"
                ] + [
                    {
                        "template_id": "v0_palette_master",
                        "file": "ghost.json",
                        "sha256": "0" * 64,
                        "digest_rule": "sha256-lf",
                    }
                ]

            make_registry(tmp, mutate_registry=mutate)
            with self.assertRaises(errors.PoCError) as ctx:
                TemplateRegistry.load(tmp / "templates")
            self.assertEqual(errors.MISSING_MODULE, ctx.exception.code)

    def test_registry_file_escape_refused(self):
        """Имя файла шаблона в реестре не может быть путём вне каталога."""
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)

            def mutate(registry):
                registry["templates"][0]["file"] = "../../secrets/template.json"

            make_registry(tmp, mutate_registry=mutate)
            with self.assertRaises(errors.PoCError) as ctx:
                TemplateRegistry.load(tmp / "templates")
            self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)


class TestLfNormalizedPins(unittest.TestCase):
    """MEDIUM-2: пины шаблонов — по LF-нормализованным байтам (git blob)."""

    def test_crlf_worktree_bytes_match_lf_pin(self):
        """CRLF-байты worktree не дают ложный TEMPLATE_HASH_MISMATCH."""
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            shutil.copytree(TEMPLATES_DIR, tmp / "templates")
            target = tmp / "templates" / "v0_palette_master.json"
            lf_bytes = target.read_bytes().replace(b"\r\n", b"\n")
            target.write_bytes(lf_bytes.replace(b"\n", b"\r\n"))
            registry = TemplateRegistry.load(tmp / "templates")
            self.assertIn("v0_palette_master", registry.template_ids)
            # Digest шаблона = sha256 LF-байтов, независимо от checkout.
            self.assertEqual(
                paths.sha256_lf_normalized_file(target),
                registry.get("v0_palette_master").sha256,
            )

    def test_registry_entry_requires_lf_digest_rule(self):
        """Реестр без явного digest_rule='sha256-lf' не принимается."""
        for broken in (None, "sha256-bytes", "sha256"):
            with self.subTest(digest_rule=broken):
                with tempfile.TemporaryDirectory() as tmp_name:
                    tmp = Path(tmp_name)

                    def mutate(registry):
                        entry = registry["templates"][0]
                        if broken is None:
                            entry.pop("digest_rule", None)
                        else:
                            entry["digest_rule"] = broken

                    make_registry(tmp, mutate_registry=mutate)
                    with self.assertRaises(errors.PoCError) as ctx:
                        TemplateRegistry.load(tmp / "templates")
                    self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)


class TestTemplateStructure(unittest.TestCase):
    def load_manifest(self, name="v0_palette_master"):
        return json.loads(
            (TEMPLATES_DIR / f"{name}.json").read_text(encoding="utf-8")
        )

    def test_real_manifests_valid(self):
        for name in ("v0_palette_master", "v1_roof_assembly", "v0_textured_probe"):
            validate_template_manifest(self.load_manifest(name))

    def test_forbidden_code_vector_refused(self):
        """F10: python/exec/shell ключи в шаблоне запрещены на любом уровне."""
        for key in ("python", "exec", "shell_command", "script_path", "allow_arbitrary_code"):
            manifest = self.load_manifest()
            manifest[key] = "print('pwned')"
            with self.assertRaises(errors.PoCError) as ctx:
                validate_template_manifest(manifest)
            self.assertEqual(errors.TEMPLATE_FORBIDDEN_KEY, ctx.exception.code)

    def test_nested_code_vector_refused(self):
        manifest = self.load_manifest()
        manifest["job_plan"]["base_operations"][0]["python_expression"] = "1+1"
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.TEMPLATE_FORBIDDEN_KEY, ctx.exception.code)

    def test_non_whitelisted_worker_op_refused(self):
        manifest = self.load_manifest()
        manifest["job_plan"]["base_operations"].append(
            {"op": "run_custom_bpy_script"}
        )
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)

    def test_import_mesh_outside_inputs_refused(self):
        manifest = self.load_manifest()
        manifest["job_plan"]["base_operations"][0]["path"] = "../../etc/passwd.glb"
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)

    def test_slot_targeting_protected_role_refused(self):
        """F9: слот не может адресовать защищённый объект."""
        manifest = self.load_manifest()
        manifest["protected_objects"] = ["walls"]
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.SLOT_TARGETS_PROTECTED_ROLE, ctx.exception.code)

    def test_slot_with_undeclared_parameter_refused(self):
        manifest = self.load_manifest()
        manifest["slots"].append(
            {"op_index": 0, "field": "path", "source": "param", "param": "evil_path"}
        )
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)

    def test_unpinned_source_hash_refused(self):
        manifest = self.load_manifest()
        manifest["sources"][0]["sha256"] = "not-a-hash"
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)

    def test_free_color_parameter_refused_via_unknown_spec_field(self):
        """Свободный цвет через параметр не проходит: типы ограничены."""
        manifest = self.load_manifest()
        manifest["parameters"]["evil_rgb"] = {"type": "color"}
        with self.assertRaises(errors.PoCError) as ctx:
            validate_template_manifest(manifest)
        self.assertEqual(errors.TEMPLATE_INVALID, ctx.exception.code)


if __name__ == "__main__":
    unittest.main()
