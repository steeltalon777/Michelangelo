"""Общие хелперы тестов PoC (пути репо/контрактов, загрузка реестра)."""

from __future__ import annotations

import os
import sys
from pathlib import Path

POC_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = POC_ROOT.parent
TEMPLATES_DIR = POC_ROOT / "templates"
FIXTURES_DIR = POC_ROOT / "fixtures"
WORKER_PY = REPO_ROOT / "blender" / "worker.py"
# Contracts-worktree GLM-1 параметризован окружением: GLM1_CONTRACTS_DIR
# (по умолчанию — текущий путь стенда; на другой машине задаётся env-переменной).
GLM1_CONTRACTS_DIR = Path(
    os.environ.get("GLM1_CONTRACTS_DIR", "I:/prog/michelangelo-adr5-contracts")
)
CANDIDATE_BUNDLE_SCHEMA = (
    GLM1_CONTRACTS_DIR / "schemas" / "adr5" / "candidate_bundle.schema.json"
)


def _ensure_import_path() -> None:
    if str(POC_ROOT) not in sys.path:
        sys.path.insert(0, str(POC_ROOT))


_ensure_import_path()


def load_registry():
    from runner.template_registry import TemplateRegistry

    return TemplateRegistry.load(TEMPLATES_DIR)
