#!/usr/bin/env python3
"""Генерация source-фикстур PoC через СУЩЕСТВУЮЩИЙ Blender Worker.

Одноразовая подготовка (не часть исполнения вариантов):
1. disposable temp-workspace, job.json на существующих операциях Worker'а;
2. headless прогоны `--background --factory-startup` (по одному Blender-
   процессу, tasklist-guard перед каждым, пауза между запусками);
3. копирование .glb/.blend в adr5_blender_poc/fixtures/;
4. пересчёт фактических SHA-256 и обновление FIXTURES_MANIFEST.json,
   SHA-256 пиннинга источников в шаблонах и registry.json.

Побитовая воспроизводимость .blend НЕ обещается (внутренние поля Blender);
.glb на зафиксированной версии Blender 5.2.3 детерминирован на практике,
но контракт Pinning — фактический hash зафиксированного файла.

Запуск: python adr5_blender_poc/fixtures/make_fixtures.py [--blender-bin X]
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
import time
from pathlib import Path

POC_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = POC_ROOT.parent
sys.path.insert(0, str(POC_ROOT))

from runner import errors, paths, worker_client, resource_monitor  # noqa: E402

FIXTURES_DIR = POC_ROOT / "fixtures"
TEMPLATES_DIR = POC_ROOT / "templates"

FIXTURE_JOBS = {
    "house_master": {
        "operations": [
            {"op": "create_box", "name": "walls", "size": [4.0, 3.0, 2.4], "location": [0.0, 0.0, 1.2]},
            {"op": "create_box", "name": "roof", "size": [4.4, 3.4, 0.25], "location": [0.0, 0.0, 2.525]},
            {"op": "create_box", "name": "door", "size": [0.9, 0.1, 2.0], "location": [0.0, -1.51, 1.0]},
            {"op": "get_scene_summary"},
            {"op": "save_blend", "path": "outputs/house_master.blend"},
            {"op": "export_glb", "path": "outputs/house_master.glb"},
        ]
    },
    "walls_module": {
        "operations": [
            {"op": "create_box", "name": "walls_base", "size": [4.0, 3.0, 2.4], "location": [0.0, 0.0, 1.2]},
            {"op": "get_scene_summary"},
            {"op": "save_blend", "path": "outputs/walls_module.blend"},
            {"op": "export_glb", "path": "outputs/walls_module.glb"},
        ]
    },
}


def make_fixture(name: str, spec: dict, blender_bin: str, worker_py: Path) -> dict:
    resource_monitor.assert_no_blender_running()
    with tempfile.TemporaryDirectory(prefix=f"adr5poc_fixture_{name}_") as tmp:
        workspace = Path(tmp)
        job_id = f"fixture_{name}"
        job_json = workspace / f"{job_id}.job.json"
        result_json = workspace / f"{job_id}.result.json"
        job_json.write_text(
            json.dumps(
                {
                    "job_id": job_id,
                    "spec": spec,
                    "workspace_root": str(workspace.resolve()),
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        outcome = worker_client.run_worker(
            blender_bin, worker_py, job_json, result_json, workspace / "logs"
        )
        result = worker_client.ensure_worker_exit_ok(outcome)
        status = result.get("status")
        if status != "completed":
            raise errors.PoCError(
                errors.WORKER_FAILED, f"fixture '{name}': worker status {status}"
            )
        glb_src = workspace / "outputs" / f"{name}.glb"
        blend_src = workspace / "outputs" / f"{name}.blend"
        if not glb_src.is_file() or not blend_src.is_file():
            raise errors.PoCError(
                errors.ARTIFACT_MISSING, f"fixture '{name}': missing outputs"
            )
        FIXTURES_DIR.mkdir(parents=True, exist_ok=True)
        glb_dst = FIXTURES_DIR / f"{name}.glb"
        blend_dst = FIXTURES_DIR / f"{name}.blend"
        shutil.copyfile(glb_src, glb_dst)
        shutil.copyfile(blend_src, blend_dst)
        return {
            "glb_sha256": paths.sha256_file(glb_dst),
            "glb_size": glb_dst.stat().st_size,
            "blend_sha256": paths.sha256_file(blend_dst),
            "blend_size": blend_dst.stat().st_size,
            "scene_summary": result.get("scene_summary"),
        }


def patch_template_source_hashes(glb_hashes: dict[str, str]) -> None:
    """Прописывает фактические sha256 фикстур в 'sources' шаблонов.

    Файлы шаблонов пишутся с LF-переводами строк (независимо от платформы),
    а их digest-пины считаются по LF-байтам (см. runner/paths).
    """
    for template_path in sorted(TEMPLATES_DIR.glob("*.json")):
        if template_path.name == "registry.json":
            continue
        manifest = json.loads(template_path.read_text(encoding="utf-8"))
        changed = False
        for source in manifest.get("sources", []):
            file_name = Path(source["file"]).name
            if file_name in glb_hashes:
                new_sha = glb_hashes[file_name]
                if source["sha256"] != new_sha:
                    source["sha256"] = new_sha
                    changed = True
        if changed:
            template_path.write_text(
                json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
                encoding="utf-8",
                newline="\n",
            )
            print(f"[make_fixtures] pinned source hashes in {template_path.name}")


def write_registry() -> None:
    """registry.json: пересчитанные SHA-256 всех файлов шаблонов.

    Пин — digest_rule 'sha256-lf': SHA-256 по LF-нормализованным байтам,
    т.е. равен git-blob'у текстового шаблона на любом checkout.
    """
    entries = []
    for template_path in sorted(TEMPLATES_DIR.glob("v*.json")):
        entries.append(
            {
                "template_id": json.loads(
                    template_path.read_text(encoding="utf-8")
                )["template_id"],
                "file": template_path.name,
                "sha256": paths.sha256_lf_normalized_file(template_path),
                "digest_rule": "sha256-lf",
            }
        )
    registry = {
        "registry_schema": "michelangelo.adr5-poc.template-registry/draft-0",
        "note": (
            "Registered templates for the GLM-5 PoC. sha256 pins the template "
            "file bytes under digest_rule 'sha256-lf' (SHA-256 over CRLF->LF "
            "normalized bytes), so the pin equals the git blob of the text "
            "file and survives CRLF/LF checkouts; the executor refuses a "
            "template whose bytes do not match (TEMPLATE_HASH_MISMATCH). "
            "Binary fixtures are pinned by raw bytes. Data-only manifests: "
            "the registry loader rejects code vectors (F10)."
        ),
        "templates": entries,
    }
    registry_path = TEMPLATES_DIR / "registry.json"
    registry_path.write_text(
        json.dumps(registry, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    for entry in entries:
        print(
            f"[make_fixtures] registered {entry['template_id']} "
            f"({entry['sha256'][:12]}...) [{entry['digest_rule']}]"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--blender-bin", default=None)
    args = parser.parse_args()

    blender_bin = resource_monitor.resolve_blender_bin(args.blender_bin)
    if not blender_bin:
        print("BLENDER_NOT_FOUND: set BLENDER_BIN; fixtures not generated", file=sys.stderr)
        return 2
    worker_py = REPO_ROOT / "blender" / "worker.py"
    if not worker_py.is_file():
        print(f"worker.py not found: {worker_py}", file=sys.stderr)
        return 2

    glb_hashes: dict[str, str] = {}
    manifest: dict = {"fixtures": {}}
    for index, (name, spec) in enumerate(FIXTURE_JOBS.items()):
        if index > 0:
            time.sleep(2.0)  # пауза на освобождение памяти между прогонами
        info = make_fixture(name, spec, blender_bin, worker_py)
        glb_hashes[f"{name}.glb"] = info["glb_sha256"]
        manifest["fixtures"][name] = {
            "file_glb": f"{name}.glb",
            "glb_sha256": info["glb_sha256"],
            "glb_size_bytes": info["glb_size"],
            "file_blend": f"{name}.blend",
            "blend_sha256": info["blend_sha256"],
            "blend_size_bytes": info["blend_size"],
            "scene_summary": info["scene_summary"],
            "blender_version": _blender_version(blender_bin),
            # .glb/.blend — бинарные файлы: пин по сырым байтам (= git blob).
            "digest_rule": "sha256-bytes",
        }
        print(f"[make_fixtures] {name}: glb {info['glb_sha256']}")

    manifest_path = FIXTURES_DIR / "FIXTURES_MANIFEST.json"
    manifest["manifest_note"] = (
        "Pinned source fixtures for the GLM-5 PoC. Binary inputs (.glb/.blend) "
        "are pinned by digest_rule 'sha256-bytes' (SHA-256 over the file bytes "
        "as stored, which equals the git blob); LF normalization is NOT "
        "applied to binary inputs. Text inputs (templates, blender/worker.py) "
        "use 'sha256-lf' pins in their own registries. .blend "
        "byte-reproducibility is not claimed."
    )
    manifest_path.write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )

    patch_template_source_hashes(glb_hashes)
    write_registry()
    print("[make_fixtures] done")
    return 0


def _blender_version(blender_bin: str) -> str:
    try:
        output = subprocess_run([blender_bin, "--version"])
        return output.splitlines()[0] if output else "unknown"
    except Exception:  # noqa: BLE001 - best effort
        return "unknown"


def subprocess_run(command: list[str]) -> str:
    import subprocess

    return subprocess.run(command, capture_output=True, text=True, timeout=60).stdout


if __name__ == "__main__":
    sys.exit(main())
