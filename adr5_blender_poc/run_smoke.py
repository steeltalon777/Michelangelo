#!/usr/bin/env python3
"""Headless smoke GLM-5 PoC: V0 (палитры) + V1 (сборки) + негативы.

Реальный Blender (если доступен); каждый прогон — один тяжёлый
Blender-процесс с tasklist-guard. Пишет evidence:
adr5_blender_poc/evidence/<variant>/{variant.glb,variant.blend,bundle.json,report.json}
+ adr5_blender_poc/evidence/evidence_summary.json.

Результат — exit code 0 и summary с флагами, либо адресная ошибка.
Отсутствие Blender — честный SKIPPED (exit 3), не фикция executed.
"""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import time
from pathlib import Path

POC_ROOT = Path(__file__).resolve().parent
REPO_ROOT = POC_ROOT.parent
sys.path.insert(0, str(POC_ROOT))
sys.path.insert(0, str(POC_ROOT / "tests"))

from runner import bundle_writer, errors, paths, resource_monitor  # noqa: E402
from runner.template_executor import ExecutionLimits, execute_variant  # noqa: E402
from runner.template_registry import Template, TemplateRegistry  # noqa: E402

from helpers import CANDIDATE_BUNDLE_SCHEMA, FIXTURES_DIR, WORKER_PY  # noqa: E402

EVIDENCE_DIR = POC_ROOT / "evidence"

V0_PALETTES = ["brick-dark", "plaster-warm", "paint-teal"]
V1_ROOFS = ["flat", "gable", "none"]

POC_ISSUES = [
    "PoC: appearance is verified only via glTF baseColorFactor of the "
    "exported GLB; no EEVEE/Cycles render validation was performed",
    "No UE readiness is claimed: no engine import/playback validation exists",
    "--factory-startup is not a sandbox; only known trusted fixtures were "
    "loaded, no untrusted .blend intake is supported",
]

V1_ISSUES = [
    "V1 socket compatibility is verified statically from registered template "
    "data plus a planned-bounds runtime check; not a physics/fit simulation",
]


def _dependency_digests(template: Template, worker_py: Path) -> dict:
    return {
        "worker_blender_py": paths.sha256_file(worker_py),
        "template_file": template.sha256,
        **{
            source["source_id"]: source["sha256"]
            for source in template.sources
        },
    }


def _provenance(template: Template) -> list:
    return [
        {
            "stage": "variant_generation",
            "ref": {
                "path": f"adr5_blender_poc/templates/{template.file_path.name}",
                "sha256": template.sha256,
            },
            "actor_role": "core",
        }
    ]


def build_bundle_for(template: Template, result, fixture_sha: str) -> dict:
    artifacts = []
    for artifact in result.artifacts:
        kind = "scene_layout" if artifact["type"] == "glb" else "other"
        artifacts.append(
            {
                "artifact_id": (
                    "variant_glb" if artifact["type"] == "glb" else "variant_blend"
                ),
                "path": artifact["path"],
                "sha256": artifact["sha256"],
                "size_bytes": artifact["size_bytes"],
                "content_kind": kind,
                "source": "generated",
            }
        )
    issues = list(POC_ISSUES) + (
        V1_ISSUES if template.level == "V1" else []
    )
    return bundle_writer.build_bundle(
        candidate_id=f"cand-{result.item_id}",
        asset_id=(
            "v0-house-master-palette" if template.level == "V0"
            else "v1-walls-roof-assembly"
        ),
        item_id=result.item_id,
        artifacts=artifacts,
        dependency_digests=_dependency_digests(template, WORKER_PY),
        base_ref={
            "kind": "source_snapshot",
            "id": template.sources[0]["source_id"],
            "sha256": fixture_sha,
        },
        provenance_chain=_provenance(template),
        unresolved_issues=issues,
        diff_to_base={
            "changed": [],
            "added": [a["artifact_id"] for a in artifacts],
            "removed": [],
        },
    )


def run_positive(registry: TemplateRegistry, workspace: Path) -> list[dict]:
    records = []
    plans = [
        ("v0_palette_master", V0_PALETTES, "palette_id"),
        ("v1_roof_assembly", V1_ROOFS, "roof_variant"),
    ]
    for template_id, values, param_name in plans:
        template = registry.get(template_id)
        fixture_sha = template.sources[0]["sha256"]
        previous = None
        for value in values:
            time.sleep(1.5)  # пауза на освобождение памяти между прогонами
            item_id = f"item-{template_id.replace('_', '-')}-{value}"
            request = {
                "item_id": item_id,
                "template_id": template_id,
                "params": {param_name: value},
                "output_stem": f"{template_id}_{value}",
            }
            result = execute_variant(
                template,
                request,
                workspace,
                FIXTURES_DIR,
                POC_ROOT / "fixtures",
                WORKER_PY,
                limits=ExecutionLimits(timeout_s=240.0),
            )
            # Геометрия вариантов V0 идентична source и друг другу (факт GLB).
            source_glb_report = None
            for source in template.sources:
                fixture_glb = FIXTURES_DIR / Path(source["file"]).name
                source_glb_report = glb_report_safe(fixture_glb)
            checks = {}
            if template.level == "V0" and source_glb_report is not None:
                for artifact_path, report in result.glb_reports.items():
                    equal, reason = glb_equal_safe(source_glb_report, report)
                    checks["geometry_preserved_vs_source"] = {
                        "equal": equal,
                        "reason": reason,
                    }
            distinct = None
            if template.level == "V0" and previous is not None:
                distinct, reason = glb_materials_distinct_safe(previous["glb"], result.glb_reports)
                checks["materials_distinct_vs_prev"] = {"distinct": distinct, "reason": reason}
            bundle = build_bundle_for(template, result, fixture_sha)
            evidence_dir = EVIDENCE_DIR / result.item_id
            evidence_dir.mkdir(parents=True, exist_ok=True)
            for artifact in result.artifacts:
                staged = _locate_staging_artifact(workspace, artifact["path"])
                shutil.copyfile(staged, evidence_dir / Path(artifact["path"]).name)
            bundle_sha = bundle_writer.write_bundle(
                bundle, evidence_dir / "bundle.json"
            )
            glb_report_data = next(iter(result.glb_reports.values()), None)
            record = {
                "item_id": result.item_id,
                "template_id": template_id,
                "level": template.level,
                "variant_value": value,
                "status": "executed",
                "elapsed_s": result.elapsed_s,
                "artifacts": result.artifacts,
                "source_hashes": result.source_hashes,
                "source_hashes_unchanged": result.source_hashes_unchanged,
                "expected_bounds": result.expected_bounds,
                "actual_bounds": result.actual_bounds,
                "scene_summary": result.scene_summary,
                "glb_report": glb_report_data,
                "checks": checks,
                "bundle_path": str(evidence_dir / "bundle.json"),
                "bundle_sha256": bundle_sha,
                "resources": result.resources,
                "schema_validation": bundle_writer.validate_bundle_against_schema(
                    bundle, CANDIDATE_BUNDLE_SCHEMA
                )
                if CANDIDATE_BUNDLE_SCHEMA.is_file()
                else ["SKIPPED: GLM-1 contracts worktree not available"],
            }
            records.append(record)
            previous = {"glb": result.glb_reports}
            print(
                f"[smoke] {result.item_id}: executed, "
                f"{len(result.artifacts)} artifacts, "
                f"ram={result.resources.get('peak_blender_ram_mib')} "
                f"gpu={result.resources.get('gpu_used_max_mib')}"
            )
    return records


def glb_report_safe(path: Path):
    try:
        from runner import glb_report as glb

        return glb.report_glb(path)
    except Exception as exc:  # noqa: BLE001 - evidence only
        return {"error": str(exc)}


def glb_equal_safe(a, b):
    from runner import glb_report as glb

    return glb.geometry_equal(a, b)


def glb_materials_distinct_safe(previous_reports, current_reports):
    from runner import glb_report as glb

    return glb.materials_distinct(
        list(previous_reports.values()) + list(current_reports.values())
    )


def _locate_staging_artifact(workspace: Path, artifact_path: str) -> Path:
    staging_root = workspace / "staging"
    for attempt_dir in sorted(staging_root.iterdir()):
        candidate = attempt_dir / artifact_path
        if candidate.is_file():
            return candidate
    raise errors.PoCError(
        errors.ARTIFACT_MISSING, f"cannot locate staged artifact {artifact_path}"
    )


def run_negatives(registry: TemplateRegistry, workspace: Path) -> list[dict]:
    results = []

    def expect(code: str, fn, label: str):
        try:
            fn()
        except errors.PoCError as exc:
            results.append(
                {
                    "negative": label,
                    "expected_code": code,
                    "actual_code": exc.code,
                    "ok": exc.code == code,
                    "message": exc.message,
                }
            )
            return
        results.append(
            {
                "negative": label,
                "expected_code": code,
                "actual_code": None,
                "ok": False,
                "message": "no error raised (BAD: failure was masked)",
            }
        )

    probe = registry.get("v0_textured_probe")
    request = {
        "item_id": "item-neg-capability",
        "template_id": "v0_textured_probe",
        "params": {},
        "output_stem": "neg_capability",
    }

    # F18: недоступная capability блокирует item до запуска.
    expect(
        errors.CAPABILITY_UNAVAILABLE,
        lambda: execute_variant(
            probe, dict(request), workspace, FIXTURES_DIR, POC_ROOT / "fixtures",
            WORKER_PY,
        ),
        "F18_capability_unavailable",
    )

    # MISSING_TEXTURE: capability выдана, файл текстуры отсутствует.
    from runner.template_executor import SUPPORTED_CAPABILITIES

    expect(
        errors.MISSING_TEXTURE,
        lambda: execute_variant(
            probe,
            dict(request),
            workspace,
            FIXTURES_DIR,
            POC_ROOT / "fixtures",
            WORKER_PY,
            capabilities=set(SUPPORTED_CAPABILITIES) | {"texture_binding"},
        ),
        "missing_texture",
    )

    # TEMPLATE_HASH_MISMATCH: изменённый шаблон против реестра.
    def tampered_template():
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            shutil.copytree(POC_ROOT / "templates", tmp / "templates")
            target = tmp / "templates" / "v0_palette_master.json"
            manifest = json.loads(target.read_text(encoding="utf-8"))
            manifest["description"] = "tampered"
            target.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
            TemplateRegistry.load(tmp / "templates")

    expect(errors.TEMPLATE_HASH_MISMATCH, tampered_template, "template_hash_mismatch")

    # SOURCE_HASH_MISMATCH: fixture подменён, пиннинг ловит до запуска.
    def tampered_source():
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            fixtures = tmp / "fixtures"
            fixtures.mkdir()
            original = FIXTURES_DIR / "house_master.glb"
            data = bytearray(original.read_bytes())
            data[-1] ^= 0xFF
            (fixtures / "house_master.glb").write_bytes(bytes(data))
            template = registry.get("v0_palette_master")
            execute_variant(
                template,
                {
                    "item_id": "item-neg-hash",
                    "template_id": "v0_palette_master",
                    "params": {"palette_id": "brick-dark"},
                    "output_stem": "neg_hash",
                },
                workspace,
                fixtures,
                POC_ROOT / "fixtures",
                WORKER_PY,
            )

    expect(errors.SOURCE_HASH_MISMATCH, tampered_source, "source_hash_mismatch")

    # MISSING_MODULE: fixture отсутствует.
    def missing_module():
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            fixtures = tmp / "fixtures"
            fixtures.mkdir()
            template = registry.get("v0_palette_master")
            execute_variant(
                template,
                {
                    "item_id": "item-neg-missing",
                    "template_id": "v0_palette_master",
                    "params": {"palette_id": "brick-dark"},
                    "output_stem": "neg_missing",
                },
                workspace,
                fixtures,
                POC_ROOT / "fixtures",
                WORKER_PY,
            )

    expect(errors.MISSING_MODULE, missing_module, "missing_module")

    # INVALID_SOCKET: узкая крыша не накрывает сокет (поведенческий, V1).
    def invalid_socket():
        template = registry.get("v1_roof_assembly")
        manifest = json.loads(json.dumps(template.manifest))
        manifest["parameters"]["roof_variant"]["values"].append("narrow_flat")
        manifest["job_plan"]["component_blocks"]["narrow_flat"] = {
            "insert_at_index": 1,
            "connector": {"width": 2.0, "depth": 1.5, "base_z": 2.4},
            "operations": [],
        }
        narrow = Template(manifest, template.file_path, template.sha256)
        execute_variant(
            narrow,
            {
                "item_id": "item-neg-socket",
                "template_id": "v1_roof_assembly",
                "params": {"roof_variant": "narrow_flat"},
                "output_stem": "neg_socket",
            },
            workspace,
            FIXTURES_DIR,
            POC_ROOT / "fixtures",
            WORKER_PY,
        )

    expect(errors.INVALID_SOCKET, invalid_socket, "F_invalid_socket")

    # WORKER_FAILED: битый GLB проходит hash-пиннинг (свой фактический hash),
    # но Worker обязан вернуть failed на импорте мусора — без «починки».
    def worker_failed():
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            fixtures = tmp / "fixtures"
            fixtures.mkdir()
            garbage = fixtures / "garbage.glb"
            garbage.write_bytes(b"this is not a glb file at all")
            manifest = {
                "template_schema": "michelangelo.adr5-poc.template/draft-0",
                "template_id": "neg_worker_fail",
                "revision": 1,
                "level": "V0",
                "capabilities_required": ["worker_job"],
                "sources": [
                    {
                        "source_id": "garbage",
                        "file": "fixtures/garbage.glb",
                        "sha256": paths.sha256_file(garbage),
                        "role": "base_candidate",
                    }
                ],
                "protected_objects": [],
                "parameters": {},
                "source_bounds": {"min": [0, 0, 0], "max": [0, 0, 0]},
                "source_counts": {"vertices": 0, "triangles": 0},
                "job_plan": {
                    "base_operations": [
                        {"op": "import_mesh", "path": "inputs/garbage.glb"}
                    ],
                    "component_blocks": {},
                    "tail_operations": [],
                },
                "slots": [],
            }
            bad_template = Template(manifest, Path("in-memory"), "0" * 64)
            execute_variant(
                bad_template,
                {
                    "item_id": "item-neg-worker",
                    "template_id": "neg_worker_fail",
                    "params": {},
                    "output_stem": "neg_worker",
                },
                workspace,
                fixtures,
                POC_ROOT / "fixtures",
                WORKER_PY,
            )

    expect(errors.WORKER_FAILED, worker_failed, "worker_failed_nonzero")

    return [r for r in results]


def main() -> int:
    blender_bin = resource_monitor.resolve_blender_bin()
    if not blender_bin:
        print("SKIPPED: Blender binary not found (set BLENDER_BIN); "
              "smoke not executed, no fake results", file=sys.stderr)
        return 3
    resource_monitor.assert_no_blender_running()

    registry = TemplateRegistry.load(POC_ROOT / "templates")
    if EVIDENCE_DIR.exists():
        shutil.rmtree(EVIDENCE_DIR)
    EVIDENCE_DIR.mkdir(parents=True)

    with tempfile.TemporaryDirectory(prefix="adr5poc_smoke_") as tmp_name:
        workspace = Path(tmp_name)
        records = run_positive(registry, workspace)
        negatives = run_negatives(registry, workspace)

    summary = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "blender_bin": str(blender_bin),
        "blender_version": _blender_version(blender_bin),
        "worker_py_sha256": paths.sha256_file(WORKER_PY),
        "registry": {
            entry["template_id"]: entry["sha256"]
            for entry in json.loads(
                (POC_ROOT / "templates" / "registry.json").read_text(encoding="utf-8")
            )["templates"]
        },
        "positives": records,
        "negatives": negatives,
        "all_negatives_ok": all(n["ok"] for n in negatives),
    }
    summary_path = EVIDENCE_DIR / "evidence_summary.json"
    summary_path.write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"[smoke] summary: {summary_path}")
    print(f"[smoke] positives: {len(records)}, negatives ok: "
          f"{sum(1 for n in negatives if n['ok'])}/{len(negatives)}")
    if not summary["all_negatives_ok"]:
        print("[smoke] FAIL: some negatives were masked", file=sys.stderr)
        return 1
    for record in records:
        if record["schema_validation"] not in ([], None) and not all(
            v.startswith("SKIPPED") for v in record["schema_validation"]
        ):
            print(f"[smoke] FAIL: bundle schema errors for {record['item_id']}: "
                  f"{record['schema_validation']}", file=sys.stderr)
            return 1
    return 0


def _blender_version(blender_bin: str) -> str:
    import subprocess

    try:
        output = subprocess.run(
            [blender_bin, "--version"], capture_output=True, text=True, timeout=60
        )
        if isinstance(output.stdout, bytes):
            return output.stdout.decode("utf-8", errors="ignore").splitlines()[0]
        return (output.stdout or "").splitlines()[0]
    except Exception:  # noqa: BLE001
        return "unknown"


if __name__ == "__main__":
    sys.exit(main())
