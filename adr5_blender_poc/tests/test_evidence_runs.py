"""HIGH-1: evidence-каталоги прогонов — no-clobber, tracked снимок не трогается.

Pure-тесты (без Blender): проверяют выделение evidence/run-<ts>/ и то, что
существующий каталог/файл никогда не очищается и не перезаписывается.
"""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import helpers  # noqa: F401  (добавляет adr5_blender_poc в sys.path)

import run_smoke


class TestEvidenceRunDir(unittest.TestCase):
    def test_run_dirs_are_no_clobber(self):
        with tempfile.TemporaryDirectory() as tmp_name:
            root = Path(tmp_name) / "evidence"
            root.mkdir()
            frozen = root / "evidence_summary.json"
            frozen.write_text("frozen snapshot\n", encoding="utf-8")
            # Имитируем совпадение timestamp: занятый run-каталог с артефактом.
            occupied = root / "run-20260101T000000Z"
            occupied.mkdir()
            keeper = occupied / "variant.glb"
            keeper.write_bytes(b"existing run evidence")

            with mock.patch.object(
                run_smoke.time, "strftime", return_value="20260101T000000Z"
            ):
                first = run_smoke.create_run_dir(root)
                second = run_smoke.create_run_dir(root)

            self.assertEqual(root / "run-20260101T000000Z-2", first)
            self.assertEqual(root / "run-20260101T000000Z-3", second)
            self.assertEqual(root, first.parent)
            self.assertEqual(root, second.parent)
            # Никакой rmtree: занятый каталог и frozen-файл сохранены.
            self.assertEqual(b"existing run evidence", keeper.read_bytes())
            self.assertEqual("frozen snapshot\n", frozen.read_text(encoding="utf-8"))

    def test_live_evidence_root_is_never_a_run_dir(self):
        """LAST_RUN_DIR (если был прогон) всегда лежит ВНУТРИ evidence/, не равен ему."""
        self.assertNotEqual(run_smoke.EVIDENCE_ROOT, run_smoke.LAST_RUN_DIR)
        if run_smoke.LAST_RUN_DIR is not None:
            self.assertEqual(run_smoke.EVIDENCE_ROOT, run_smoke.LAST_RUN_DIR.parent)
            self.assertTrue(run_smoke.LAST_RUN_DIR.name.startswith("run-"))


if __name__ == "__main__":
    unittest.main()
