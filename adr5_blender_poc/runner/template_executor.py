"""Оркестратор `template.execute(bindings, capabilities, limits)` (PoC-порт).

Соответствие CONTRACT_GATE §7.4 (draft): pinned template + typed bindings +
capabilities allowlist + лимиты -> артефакты в private staging. Поверх
СУЩЕСТВУЮЩЕГО worker'а; 18 ops не расширяются.

Порядок исполнения одного варианта:
  1. capabilities: заявленные требования шаблона против доступных
     (CAPABILITY_UNAVAILABLE, F18) — до запуска;
  2. параметры: гейт запроса (UNKNOWN_PARAMETER / INVALID_PARAMETER /
     PARAMETER_SCOPE_PROTECTED / INVALID_OUTPUT_PATH) — до запуска;
  3. сокеты: статическая проверка выбранных компонентов (INVALID_SOCKET);
  4. входы: копия fixture в staging + hash по фактическим байтам
     (MISSING_MODULE / SOURCE_HASH_MISMATCH / MISSING_TEXTURE);
  5. job-spec: только операции шаблона + подстановка слотов;
  6. headless прогон (`--background --factory-startup`), guard tasklist;
  7. worker result: completed обязателен (WORKER_FAILED иначе);
  8. артефакты: существование, контейнмент, размер, SHA-256 (F13 guard);
  9. эффективный результат: scene_summary bounds/counts против плана
     (BOUNDS_MISMATCH) + сохранность исходника (rehash);
  10. GLB-отчёт: фактические имена/геометрия/UV/материалы.
"""

from __future__ import annotations

import json
import shutil
import time
from pathlib import Path

from . import bundle_writer, errors, glb_report, paths, plan_builder, resource_monitor
from . import socket_check, worker_client
from .parameter_gate import validate_request

# Capabilities, которые PoC может честно подтвердить на стенде.
SUPPORTED_CAPABILITIES = frozenset(
    {
        "worker_job",          # 18-op worker, headless --factory-startup
        "gltf_export",         # export_glb op
        "blend_save",          # save_blend op
        "material_variation",  # set_material op (V0)
        "module_assembly",     # component blocks + socket check (V1)
        "workbench_render",    # render_camera (Workbench only)
        "scene_summary",       # get_scene_summary op
    }
)

# Операции, требуемые шаблоном, маппим на capability для F18-проверки.
OP_CAPABILITY = {
    "export_glb": "gltf_export",
    "save_blend": "blend_save",
    "set_material": "material_variation",
    "render_camera": "workbench_render",
    "get_scene_summary": "scene_summary",
}


class ExecutionLimits:
    """Лимиты PoC-исполнения (адресные, проверяются до запуска)."""

    def __init__(
        self,
        timeout_s: float = 300.0,
        max_output_bytes: int = 512 * 1024 * 1024,
        bounds_tolerance: float = 1e-3,
    ):
        self.timeout_s = timeout_s
        self.max_output_bytes = max_output_bytes
        self.bounds_tolerance = bounds_tolerance


class ExecutionResult:
    """Полный результат одной попытки (успешной или нет — решает caller)."""

    def __init__(self):
        self.item_id: str = ""
        self.template_id: str = ""
        self.attempt_dir: Path | None = None
        self.job_spec: dict = {}
        self.worker_result: dict = {}
        self.scene_summary: dict | None = None
        self.artifacts: list[dict] = []      # verified artifacts (staging-relative)
        self.artifact_hashes: dict[str, str] = {}
        self.source_hashes: dict[str, str] = {}
        self.source_hashes_unchanged: bool = True
        self.glb_reports: dict[str, dict] = {}
        self.expected_bounds: tuple[list, list] | None = None
        self.actual_bounds: tuple[list, list] | None = None
        self.resources: dict = {}
        self.elapsed_s: float | None = None


def execute_variant(
    template,
    request: dict,
    workspace: Path,
    fixtures_dir: Path,
    textures_dir: Path,
    worker_py: Path,
    capabilities: set[str] | None = None,
    limits: ExecutionLimits | None = None,
    blender_bin: str | None = None,
) -> ExecutionResult:
    """Исполняет один вариант; при отказе — PoCError (никогда не маскируется)."""
    limits = limits or ExecutionLimits()
    capabilities = capabilities if capabilities is not None else set(SUPPORTED_CAPABILITIES)
    workspace = Path(workspace)

    # --- 0. запрос/параметры (до любых файловых действий) -------------------
    resolved = validate_request(template, request)

    # --- 1. capabilities (F18) ----------------------------------------------
    required = set(template.capabilities_required)
    for operation in _all_operations(template):
        if operation.get("op") in OP_CAPABILITY:
            required.add(OP_CAPABILITY[operation["op"]])
    missing_caps = sorted(required - set(capabilities))
    if missing_caps:
        raise errors.PoCError(
            errors.CAPABILITY_UNAVAILABLE,
            f"template '{template.template_id}' requires capabilities "
            f"{missing_caps} that are not available in this execution context; "
            f"item is blocked (other items continue)",
            {"missing_capabilities": missing_caps, "available": sorted(capabilities)},
        )

    # --- 2. сокеты (статическая плановая проверка V1) ------------------------
    socket_check.check_request_sockets(template, resolved.component_choices)

    # --- 3. входы: копия + hash (F9/F13 guard: копия, не оригинал) ----------
    staging = workspace / "staging" / f"attempt-{int(time.time() * 1000):013d}"
    staging.mkdir(parents=True, exist_ok=True)
    inputs_dir = staging / "inputs"
    inputs_dir.mkdir(exist_ok=True)

    source_hashes: dict[str, str] = {}
    for source in template.sources:
        expected_sha = source["sha256"]
        fixture_path = fixtures_dir / Path(source["file"]).relative_to("fixtures")
        if not fixture_path.is_file():
            raise errors.PoCError(
                errors.MISSING_MODULE,
                f"source fixture '{source['source_id']}' is missing: "
                f"{fixture_path}",
                {"source_id": source["source_id"]},
            )
        actual_sha = paths.sha256_file(fixture_path)
        if actual_sha != expected_sha:
            raise errors.PoCError(
                errors.SOURCE_HASH_MISMATCH,
                f"source fixture '{source['source_id']}' does not match the "
                f"pinned hash (expected {expected_sha}, actual {actual_sha}); "
                f"execution refused",
                {"source_id": source["source_id"]},
            )
        copy_path = inputs_dir / Path(source["file"]).name
        shutil.copyfile(fixture_path, copy_path)
        copy_sha = paths.sha256_file(copy_path)
        if copy_sha != expected_sha:
            raise errors.PoCError(
                errors.SOURCE_HASH_MISMATCH,
                f"staged copy of '{source['source_id']}' does not match its "
                f"pinned hash (copy integrity failure)",
            )
        source_hashes[source["source_id"]] = expected_sha

    for texture in template.manifest.get("textures", []):
        texture_path = textures_dir / Path(texture["file"]).relative_to("textures")
        if not texture_path.is_file():
            raise errors.PoCError(
                errors.MISSING_TEXTURE,
                f"template requires texture '{texture['texture_id']}' which is "
                f"not available: {texture_path}",
                {"texture_id": texture["texture_id"]},
            )

    # --- 4. job-spec ---------------------------------------------------------
    job_spec = plan_builder.build_job_spec(template, resolved)
    _validate_spec_ops(job_spec)

    run_id = f"{resolved.item_id}-{resource_monitor.resolve_attempt_token()}"
    attempt_dir = workspace / "jobs" / run_id
    attempt_dir.mkdir(parents=True, exist_ok=True)
    job_json = attempt_dir / "job.json"
    result_json = attempt_dir / "result.json"
    job_json.write_text(
        json.dumps(
            {
                "job_id": f"job_{run_id}".replace("-", "_"),
                "spec": job_spec,
                "workspace_root": str(staging.resolve()),
            },
            indent=2,
        ),
        encoding="utf-8",
    )

    # --- 5. headless прогон --------------------------------------------------
    blender_binary = blender_bin or resource_monitor.resolve_blender_bin()
    if not blender_binary:
        raise errors.PoCError(
            errors.BLENDER_NOT_FOUND,
            "Blender binary not found (set BLENDER_BIN); smoke would be "
            "SKIPPED, not faked",
        )

    result = ExecutionResult()
    result.item_id = resolved.item_id
    result.template_id = template.template_id
    result.job_spec = job_spec
    result.source_hashes = source_hashes

    with resource_monitor.ResourceMonitor() as monitor:
        outcome = worker_client.run_worker(
            blender_binary,
            worker_py,
            job_json,
            result_json,
            attempt_dir,
            timeout_s=limits.timeout_s,
        )
        worker_result = worker_client.ensure_worker_exit_ok(outcome)
        result.resources = monitor.report()
    result.elapsed_s = getattr(outcome, "elapsed_s", None)
    result.worker_result = worker_result
    result.scene_summary = worker_result.get("scene_summary")
    result.attempt_dir = attempt_dir

    # --- 6. артефакты: верификация (не ослаблять) ---------------------------
    outputs_root = staging / "outputs"
    outputs_root.mkdir(parents=True, exist_ok=True)
    for artifact in worker_result.get("artifacts", []):
        rel_path = artifact.get("path", "")
        paths.validate_relative_path(rel_path)
        if not rel_path.startswith("outputs/"):
            raise errors.PoCError(
                errors.ARTIFACT_PATH_ESCAPE,
                f"worker artifact outside the outputs area: {rel_path!r}",
            )
        artifact_path = staging / rel_path
        contained = paths.ensure_contained(artifact_path, staging)
        if not contained.is_file():
            raise errors.PoCError(
                errors.ARTIFACT_MISSING,
                f"worker artifact is not a regular file: {rel_path!r}",
            )
        size = contained.stat().st_size
        declared = artifact.get("size_bytes")
        if declared is not None and int(declared) != size:
            raise errors.PoCError(
                errors.ARTIFACT_MISSING,
                f"artifact size mismatch for {rel_path!r}: "
                f"declared {declared}, actual {size}",
            )
        if size <= 0 or size > limits.max_output_bytes:
            raise errors.PoCError(
                errors.ARTIFACT_MISSING,
                f"artifact size out of limits for {rel_path!r}: {size} bytes",
            )
        sha = paths.sha256_file(contained)
        result.artifacts.append(
            {
                "path": rel_path,
                "size_bytes": size,
                "sha256": sha,
                "type": artifact.get("type"),
            }
        )
        result.artifact_hashes[rel_path] = sha
    if not result.artifacts:
        raise errors.PoCError(
            errors.ARTIFACT_MISSING,
            "worker reported no artifacts; nothing to accept as a candidate",
        )

    # --- 7. эффективный результат: bounds/counts против плана ---------------
    mins, maxs = socket_check.expected_bounds_for(template, resolved.component_choices)
    result.expected_bounds = (mins, maxs)
    summary = result.scene_summary or {}
    if "bounds" in summary:
        actual_min = [float(v) for v in summary["bounds"]["min"]]
        actual_max = [float(v) for v in summary["bounds"]["max"]]
        result.actual_bounds = (actual_min, actual_max)
        tol = limits.bounds_tolerance
        for axis, axis_name in enumerate("xyz"):
            if abs(actual_min[axis] - mins[axis]) > tol:
                raise errors.PoCError(
                    errors.BOUNDS_MISMATCH,
                    f"actual min {axis_name}={actual_min[axis]} differs from the "
                    f"planned {mins[axis]} beyond {tol} (effective result check)",
                )
            if abs(actual_max[axis] - maxs[axis]) > tol:
                raise errors.PoCError(
                    errors.BOUNDS_MISMATCH,
                    f"actual max {axis_name}={actual_max[axis]} differs from the "
                    f"planned {maxs[axis]} beyond {tol} (effective result check)",
                )
    exp_vertices, exp_triangles = socket_check.expected_scene_counts_for(
        template, resolved.component_choices
    )
    if "vertices" in summary and int(summary["vertices"]) != exp_vertices:
        raise errors.PoCError(
            errors.BOUNDS_MISMATCH,
            f"actual vertices {summary['vertices']} != planned {exp_vertices}",
        )
    if "triangles" in summary and int(summary["triangles"]) != exp_triangles:
        raise errors.PoCError(
            errors.BOUNDS_MISMATCH,
            f"actual triangles {summary['triangles']} != planned {exp_triangles}",
        )

    # --- 8. исходники не тронуты (rehash) ------------------------------------
    for source in template.sources:
        fixture_path = fixtures_dir / Path(source["file"]).relative_to("fixtures")
        if paths.sha256_file(fixture_path) != source["sha256"]:
            result.source_hashes_unchanged = False
            raise errors.PoCError(
                errors.SOURCE_HASH_MISMATCH,
                f"source fixture '{source['source_id']}' changed during the run",
            )

    # --- 9. GLB-отчёт по факту экспорта --------------------------------------
    for artifact in result.artifacts:
        if (artifact.get("type") or "").lower() == "glb":
            glb_path = staging / artifact["path"]
            result.glb_reports[artifact["path"]] = glb_report.report_glb(glb_path)

    return result


def _all_operations(template) -> list[dict]:
    plan = template.manifest["job_plan"]
    operations = list(plan.get("base_operations", []))
    for block in (plan.get("component_blocks") or {}).values():
        operations.extend(block.get("operations", []))
    operations.extend(plan.get("tail_operations", []))
    return operations


def _validate_spec_ops(job_spec: dict) -> None:
    """Последний рубеж: в финальном spec только 18 ops (guard от сборки)."""
    from .template_registry import WORKER_SAFE_OPS

    for index, operation in enumerate(job_spec["operations"]):
        if operation.get("op") not in WORKER_SAFE_OPS:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"assembled spec contains non-whitelisted op at {index}: "
                f"{operation.get('op')!r}",
            )


__all__ = [
    "ExecutionLimits",
    "ExecutionResult",
    "SUPPORTED_CAPABILITIES",
    "execute_variant",
    "bundle_writer",
]
