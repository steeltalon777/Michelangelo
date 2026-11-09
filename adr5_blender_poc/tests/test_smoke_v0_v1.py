"""Blender smoke V0/V1 + негативы (SKIPPED честно, если Blender недоступен).

Запускает полный драйвер adr5_blender_poc/run_smoke.py: 6 позитивных
headless-прогонов (3 палитры V0, 3 сборки V1) и 7 негативов. Каждый прогон —
ОДИН тяжёлый Blender-процесс с tasklist-guard; между прогонами паузы.
Evidence: новый no-clobber adr5_blender_poc/evidence/run-<ts>/ (tracked
evidence/ — замороженный снимок, smoke его не перезаписывает).

Запуск всех тестов потока:
    cd adr5_blender_poc/tests && python -m unittest discover -p "test_*.py" -v
"""

import hashlib
import unittest
from pathlib import Path

from helpers import WORKER_PY

from runner import resource_monitor


BLENDER_BIN = resource_monitor.resolve_blender_bin()


def _frozen_evidence_snapshot(evidence_root: Path) -> dict[str, str]:
    """SHA-256 файлов tracked-снимка evidence/ (без disposable run-<ts>/)."""
    snapshot: dict[str, str] = {}
    if not evidence_root.is_dir():
        return snapshot
    for path in sorted(evidence_root.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(evidence_root)
        if relative.parts and relative.parts[0].startswith("run-"):
            continue
        snapshot[relative.as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return snapshot


@unittest.skipUnless(
    BLENDER_BIN is not None and WORKER_PY.is_file(),
    "Blender binary or blender/worker.py not available: smoke is SKIPPED, "
    "not faked (no dry-run presented as executed)",
)
class TestSmokeV0V1(unittest.TestCase):
    def test_full_smoke_driver(self):
        import run_smoke

        # HIGH-1: tracked evidence — замороженный снимок; smoke обязан писать
        # только в новый evidence/run-<ts>/ и ничего не удалять/перезаписывать.
        frozen_before = _frozen_evidence_snapshot(run_smoke.EVIDENCE_ROOT)

        exit_code = run_smoke.main()
        run_dir = run_smoke.LAST_RUN_DIR

        self.assertIsNotNone(run_dir)
        self.assertNotEqual(run_smoke.EVIDENCE_ROOT, run_dir)
        self.assertEqual(run_smoke.EVIDENCE_ROOT, run_dir.parent)
        self.assertRegex(run_dir.name, r"^run-\d{8}T\d{6}Z(-\d+)?$")
        self.assertEqual(
            frozen_before,
            _frozen_evidence_snapshot(run_smoke.EVIDENCE_ROOT),
            "smoke modified/deleted files of the frozen tracked evidence snapshot",
        )

        self.assertEqual(
            0,
            exit_code,
            f"run_smoke reported failures; see {run_dir / 'evidence_summary.json'}",
        )

        # Повторная сверка summary независимо от exit code.
        import json

        summary_path = run_dir / "evidence_summary.json"
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
        self.assertEqual(str(run_dir), summary["evidence_run_dir"])
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
