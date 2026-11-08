"""Blender smoke V0/V1 + негативы (SKIPPED честно, если Blender недоступен).

Запускает полный драйвер adr5_blender_poc/run_smoke.py: 6 позитивных
headless-прогонов (3 палитры V0, 3 сборки V1) и 7 негативов. Каждый прогон —
ОДИН тяжёлый Blender-процесс с tasklist-guard; между прогонами паузы.
Evidence: adr5_blender_poc/evidence/ (перезаписывается).

Запуск всех тестов потока:
    cd adr5_blender_poc/tests && python -m unittest discover -p "test_*.py" -v
"""

import unittest

from helpers import WORKER_PY

from runner import resource_monitor


BLENDER_BIN = resource_monitor.resolve_blender_bin()


@unittest.skipUnless(
    BLENDER_BIN is not None and WORKER_PY.is_file(),
    "Blender binary or blender/worker.py not available: smoke is SKIPPED, "
    "not faked (no dry-run presented as executed)",
)
class TestSmokeV0V1(unittest.TestCase):
    def test_full_smoke_driver(self):
        import run_smoke

        exit_code = run_smoke.main()
        self.assertEqual(
            0, exit_code, "run_smoke reported failures; see "
            "adr5_blender_poc/evidence/evidence_summary.json"
        )

        # Повторная сверка summary независимо от exit code.
        import json
        from pathlib import Path

        summary_path = Path(run_smoke.EVIDENCE_DIR) / "evidence_summary.json"
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
        self.assertTrue(summary["all_negatives_ok"])
        positives = summary["positives"]
        self.assertEqual(6, len(positives))

        v0 = [r for r in positives if r["level"] == "V0"]
        v1 = [r for r in positives if r["level"] == "V1"]
        self.assertEqual(3, len(v0))
        self.assertEqual(3, len(v1))
        for record in positives:
            self.assertEqual("executed", record["status"])
            self.assertTrue(record["source_hashes_unchanged"])
            self.assertGreaterEqual(len(record["artifacts"]), 2)
            self.assertEqual([], record["schema_validation"])
            # Телеметрия ресурсов присутствует (значение или честный
            # NOT_MEASURED, но не отсутствие поля).
            self.assertIn("peak_blender_ram_mib", record["resources"])

        # V0: геометрия сохранена, материалы попарно различимы.
        for record in v0:
            check = record["checks"].get("geometry_preserved_vs_source")
            self.assertIsNotNone(check)
            self.assertTrue(check["equal"], check["reason"])

        # V1: bounds совпали с плановыми в допуске (проверено исполнителем;
        # здесь — контроль, что actual_bounds вообще получены).
        for record in v1:
            self.assertIsNotNone(record["actual_bounds"])


if __name__ == "__main__":
    unittest.main()
