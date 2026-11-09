"""Реестр зарегистрированных шаблонов PoC.

Шаблон = статический JSON-манифест + статический job-spec НА СУЩЕСТВУЮЩИХ
18 операциях Worker'а. Никакого кода внутри шаблона: только данные.

Реестр (`registry.json`) пиннит SHA-256 файла каждого шаблона; исполнитель
проверяет фактические байты шаблона перед использованием
(TEMPLATE_HASH_MISMATCH при расхождении).

Digest-правило шаблонов — `sha256-lf`: SHA-256 по LF-нормализованным байтам
(CRLF -> LF), чтобы пин совпадал с git-blob'ом на любом checkout
(core.autocrlf=true / LF-checkout) и не давал ложный TEMPLATE_HASH_MISMATCH.
Каждая запись реестра обязана явно объявить `digest_rule: "sha256-lf"`;
бинарные входы (.glb/.blend) пиннятся сырыми байтами (sha256_file), не LF.

Структурная валидация шаблона включает:
- F10: запрет полей-векторов произвольного кода (python/eval/shell/script...);
- F9: слоты не могут адресовать защищённые объекты/роли;
- слоты ссылаются только на объявленные параметры/объекты;
- component-block'и V1 содержат только операции из 18-op whitelist.
"""

from __future__ import annotations

import json
from pathlib import Path

from . import errors, paths

# Версия локальной схемы манифеста шаблона (draft PoC, не утверждённый wire).
TEMPLATE_SCHEMA = "michelangelo.adr5-poc.template/draft-0"

# 18 существующих операций Worker'а (blender/worker.py SAFE_OPS_ORDER,
# crates/michelangelo_protocol::SAFE_WORKER_OPS). Исполнитель отказывает
# шаблону с любой другой операцией — новый op требует отдельного gate.
WORKER_SAFE_OPS = frozenset(
    {
        "create_box",
        "set_material",
        "set_camera_set",
        "render_views",
        "export_glb",
        "save_blend",
        "get_scene_summary",
        "create_cylinder",
        "create_sphere",
        "import_mesh",
        "set_transform",
        "create_ortho_camera",
        "set_camera_pose",
        "set_camera_settings",
        "frame_camera",
        "set_render_settings",
        "render_camera",
        "create_component",
    }
)

# F10: ключи, наличие которых в манифесте шаблона (на любом уровне) означает
# попытку провести произвольный код/скрипт через шаблон.
FORBIDDEN_MANIFEST_KEYS = frozenset(
    {
        "python",
        "python_expression",
        "python_code",
        "exec",
        "eval",
        "shell",
        "shell_command",
        "command",
        "script",
        "script_path",
        "code",
        "expression",
        "driver",
        "drivers",
        "allow_arbitrary_code",
        "cmd",
        "argv",
    }
)

ALLOWED_PARAMETER_TYPES = frozenset({"enum", "number", "boolean"})
ALLOWED_SLOT_SOURCES = frozenset({"const", "param", "palette", "component_choice"})

# Единственное разрешённое digest-правило для текстовых шаблонов: LF-байты.
DIGEST_RULE_LF = "sha256-lf"
ALLOWED_TEMPLATE_DIGEST_RULES = frozenset({DIGEST_RULE_LF})

PARAMETER_SCHEMA_FIELDS = frozenset({"type", "values", "minimum", "maximum", "description"})


class Template:
    """Загруженный и структурно проверенный шаблон."""

    def __init__(self, manifest: dict, file_path: Path, sha256: str):
        self.manifest = manifest
        self.file_path = file_path
        self.sha256 = sha256

    @property
    def template_id(self) -> str:
        return self.manifest["template_id"]

    @property
    def revision(self) -> int:
        return int(self.manifest["revision"])

    @property
    def level(self) -> str:
        return self.manifest["level"]

    @property
    def capabilities_required(self) -> list[str]:
        return list(self.manifest.get("capabilities_required", []))

    @property
    def sources(self) -> list[dict]:
        return list(self.manifest.get("sources", []))

    @property
    def protected_objects(self) -> list[str]:
        return list(self.manifest.get("protected_objects", []))

    def parameter_spec(self, name: str) -> dict | None:
        return self.manifest.get("parameters", {}).get(name)


class TemplateRegistry:
    """Каталог зарегистрированных шаблонов с SHA-256 пиннингом."""

    def __init__(self, templates: dict[str, Template]):
        self._templates = templates

    def get(self, template_id: str) -> Template:
        template = self._templates.get(template_id)
        if template is None:
            raise errors.PoCError(
                errors.TEMPLATE_NOT_FOUND,
                f"template '{template_id}' is not registered",
                {"registered": sorted(self._templates)},
            )
        return template

    @property
    def template_ids(self) -> list[str]:
        return sorted(self._templates)

    @classmethod
    def load(cls, templates_dir: Path) -> "TemplateRegistry":
        registry_path = templates_dir / "registry.json"
        if not registry_path.is_file():
            raise errors.PoCError(
                errors.TEMPLATE_NOT_FOUND,
                f"template registry not found: {registry_path}",
            )
        registry = json.loads(registry_path.read_text(encoding="utf-8"))
        entries = registry.get("templates")
        if not isinstance(entries, list) or not entries:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                "registry.json must contain a non-empty 'templates' array",
            )

        templates: dict[str, Template] = {}
        for entry in entries:
            template_id = entry.get("template_id")
            file_name = entry.get("file")
            expected_sha = entry.get("sha256")
            digest_rule = entry.get("digest_rule")
            if not isinstance(template_id, str) or not template_id:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"registry entry without template_id: {entry!r}",
                )
            if not paths.is_sha256_hex(expected_sha):
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"registry entry '{template_id}': sha256 must be 64 hex chars",
                )
            # Пины обязаны быть по LF-нормализованным байтам: реестр с иным
            # правилом не принимается (иначе CRLF/LF checkout даёт ложный
            # TEMPLATE_HASH_MISMATCH).
            if digest_rule not in ALLOWED_TEMPLATE_DIGEST_RULES:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"registry entry '{template_id}': digest_rule must be "
                    f"'{DIGEST_RULE_LF}', got {digest_rule!r}",
                )
            # Имя файла реестра ограничено простым базовым именем — путь из
            # реестра не может указать за пределы каталога шаблонов.
            if (
                not isinstance(file_name, str)
                or Path(file_name).name != file_name
                or not file_name.endswith(".json")
            ):
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"registry entry '{template_id}': 'file' must be a plain "
                    f"*.json base name, got {file_name!r}",
                )
            file_path = templates_dir / file_name
            if not file_path.is_file():
                raise errors.PoCError(
                    errors.MISSING_MODULE,
                    f"registered template file is missing: {file_path}",
                    {"template_id": template_id},
                )
            actual_sha = paths.sha256_lf_normalized_file(file_path)
            if actual_sha != expected_sha:
                raise errors.PoCError(
                    errors.TEMPLATE_HASH_MISMATCH,
                    f"template '{template_id}' bytes do not match the registry "
                    f"(digest_rule={DIGEST_RULE_LF}, expected {expected_sha}, "
                    f"actual {actual_sha})",
                    {"template_id": template_id, "file": str(file_path)},
                )
            manifest = json.loads(file_path.read_text(encoding="utf-8"))
            validate_template_manifest(manifest)
            if manifest["template_id"] != template_id:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"template_id mismatch: registry='{template_id}', "
                    f"manifest='{manifest['template_id']}'",
                )
            templates[template_id] = Template(manifest, file_path, actual_sha)

        return cls(templates)


def _reject_forbidden_keys(node, location: str) -> None:
    """F10: рекурсивный поиск векторов произвольного кода в манифесте."""
    if isinstance(node, dict):
        for key, value in node.items():
            if isinstance(key, str) and key.lower() in FORBIDDEN_MANIFEST_KEYS:
                raise errors.PoCError(
                    errors.TEMPLATE_FORBIDDEN_KEY,
                    f"template manifest contains forbidden key '{key}' "
                    f"at {location}: templates are data-only, no code vectors",
                    {"location": location},
                )
            _reject_forbidden_keys(value, f"{location}.{key}")
    elif isinstance(node, list):
        for index, value in enumerate(node):
            _reject_forbidden_keys(value, f"{location}[{index}]")


def _validate_operation(operation, location: str) -> None:
    if not isinstance(operation, dict):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID, f"{location}: operation must be an object"
        )
    op_name = operation.get("op")
    if op_name not in WORKER_SAFE_OPS:
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            f"{location}: operation '{op_name}' is not one of the 18 "
            f"worker safe ops; new ops require a separate gate",
        )
    if op_name == "import_mesh":
        path = operation.get("path")
        if not isinstance(path, str) or not path.startswith("inputs/"):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"{location}: import_mesh.path must reference the private "
                f"staging 'inputs/' area, got {path!r}",
            )


def validate_template_manifest(manifest: dict) -> None:
    """Полная структурная проверка манифеста (данные, без кода)."""
    if not isinstance(manifest, dict):
        raise errors.PoCError(errors.TEMPLATE_INVALID, "manifest must be an object")
    if manifest.get("template_schema") != TEMPLATE_SCHEMA:
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            f"template_schema must be '{TEMPLATE_SCHEMA}', "
            f"got {manifest.get('template_schema')!r}",
        )
    for field in ("template_id", "revision", "level", "job_plan"):
        if field not in manifest:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, f"manifest missing required field '{field}'"
            )
    if manifest["level"] not in ("V0", "V1"):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            f"level must be 'V0' or 'V1' for this PoC, got {manifest['level']!r}",
        )
    if not isinstance(manifest["revision"], int) or manifest["revision"] < 1:
        raise errors.PoCError(errors.TEMPLATE_INVALID, "revision must be a positive int")

    _reject_forbidden_keys(manifest, "manifest")

    # --- параметры: типизированные, с enum/range ---------------------------
    parameters = manifest.get("parameters", {})
    if not isinstance(parameters, dict):
        raise errors.PoCError(errors.TEMPLATE_INVALID, "'parameters' must be an object")
    for name, spec in parameters.items():
        if not paths.is_safe_item_id(name):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, f"unsafe parameter name {name!r}"
            )
        if not isinstance(spec, dict):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, f"parameter '{name}': spec must be an object"
            )
        unknown_fields = set(spec) - PARAMETER_SCHEMA_FIELDS
        if unknown_fields:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"parameter '{name}': unknown spec fields {sorted(unknown_fields)}",
            )
        ptype = spec.get("type")
        if ptype not in ALLOWED_PARAMETER_TYPES:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"parameter '{name}': type must be one of "
                f"{sorted(ALLOWED_PARAMETER_TYPES)}",
            )
        if ptype == "enum":
            values = spec.get("values")
            if not isinstance(values, list) or not values:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID, f"parameter '{name}': enum needs values"
                )
            for value in values:
                if not isinstance(value, str) or not paths.is_safe_item_id(value):
                    raise errors.PoCError(
                        errors.TEMPLATE_INVALID,
                        f"parameter '{name}': unsafe enum value {value!r}",
                    )
        elif ptype == "number":
            minimum, maximum = spec.get("minimum"), spec.get("maximum")
            if not isinstance(minimum, (int, float)) or not isinstance(
                maximum, (int, float)
            ):
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"parameter '{name}': number needs minimum/maximum",
                )

    # --- источники: пиннинг SHA-256 обязателен ------------------------------
    for source in manifest.get("sources", []):
        if not isinstance(source, dict):
            raise errors.PoCError(errors.TEMPLATE_INVALID, "source must be an object")
        for field in ("source_id", "file", "sha256", "role"):
            if field not in source:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID, f"source missing '{field}'"
                )
        if not paths.is_safe_item_id(source["source_id"]):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, f"unsafe source_id {source['source_id']!r}"
            )
        if not paths.is_sha256_hex(source["sha256"]):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"source '{source['source_id']}': sha256 must be pinned",
            )
        validate_relative_staging_path(source["file"], prefix="fixtures/")

    # --- текстуры (для негатива missing texture) -----------------------------
    for texture in manifest.get("textures", []):
        for field in ("texture_id", "file"):
            if field not in texture:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID, f"texture missing '{field}'"
                )
        validate_relative_staging_path(texture["file"], prefix="textures/")

    # --- палитра: таблица объект -> RGBA, без свободных цветов --------------
    palette_table = manifest.get("palette_table")
    if palette_table is not None:
        if not isinstance(palette_table, dict) or not palette_table:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, "'palette_table' must be a non-empty object"
            )
        for palette_id, objects in palette_table.items():
            if not paths.is_safe_item_id(palette_id):
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID, f"unsafe palette_id {palette_id!r}"
                )
            if not isinstance(objects, dict) or not objects:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"palette '{palette_id}': must map objects to colors",
                )
            for obj_name, color in objects.items():
                if (
                    not isinstance(color, list)
                    or len(color) not in (3, 4)
                    or not all(
                        isinstance(c, (int, float)) and not isinstance(c, bool) and 0.0 <= c <= 1.0
                        for c in color
                    )
                ):
                    raise errors.PoCError(
                        errors.TEMPLATE_INVALID,
                        f"palette '{palette_id}': object '{obj_name}' color must "
                        f"be 3-4 components in 0..=1",
                    )

    # --- job_plan: только 18 операций ---------------------------------------
    job_plan = manifest["job_plan"]
    if not isinstance(job_plan, dict):
        raise errors.PoCError(errors.TEMPLATE_INVALID, "'job_plan' must be an object")
    for index, operation in enumerate(job_plan.get("base_operations", [])):
        _validate_operation(operation, f"job_plan.base_operations[{index}]")
    for block_id, block in (job_plan.get("component_blocks") or {}).items():
        if not paths.is_safe_item_id(block_id):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, f"unsafe component block id {block_id!r}"
            )
        if not isinstance(block, dict) or not isinstance(
            block.get("operations"), list
        ):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"component block '{block_id}': needs 'operations' array",
            )
        for index, operation in enumerate(block["operations"]):
            _validate_operation(operation, f"job_plan.component_blocks.{block_id}[{index}]")
        if "connector" in block and not isinstance(block["connector"], dict):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"component block '{block_id}': connector must be an object",
            )

    # --- слоты: только объявленные параметры, без защищённых таргетов --------
    protected = set(manifest.get("protected_objects", []))
    for index, slot in enumerate(manifest.get("slots", [])):
        location = f"slots[{index}]"
        if not isinstance(slot, dict):
            raise errors.PoCError(errors.TEMPLATE_INVALID, f"{location}: must be object")
        if slot.get("source") not in ALLOWED_SLOT_SOURCES:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"{location}: source must be one of {sorted(ALLOWED_SLOT_SOURCES)}",
            )
        if slot["source"] == "param":
            param_name = slot.get("param")
            if param_name not in parameters:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"{location}: slot references undeclared parameter "
                    f"'{param_name}'",
                )
        if slot["source"] == "palette":
            if palette_table is None:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"{location}: palette slot but no palette_table",
                )
            target = slot.get("target_object")
            if target in protected:
                # F9: слот не может перекрашивать защищённую роль.
                raise errors.PoCError(
                    errors.SLOT_TARGETS_PROTECTED_ROLE,
                    f"{location}: slot targets protected object '{target}' "
                    f"(protected roles cannot be modified through slots)",
                    {"target_object": target},
                )
            if target not in manifest.get("palette_objects", []):
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"{location}: palette target '{target}' is not declared in "
                    f"'palette_objects'",
                )

    # --- sockets V1 ----------------------------------------------------------
    for socket_id, socket in (manifest.get("sockets") or {}).items():
        if not paths.is_safe_item_id(socket_id):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID, f"unsafe socket id {socket_id!r}"
            )
        for field in ("z", "footprint"):
            if field not in socket:
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID, f"socket '{socket_id}' missing '{field}'"
                )
        footprint = socket["footprint"]
        if (
            not isinstance(footprint, dict)
            or not isinstance(footprint.get("width"), (int, float))
            or not isinstance(footprint.get("depth"), (int, float))
        ):
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"socket '{socket_id}': footprint needs numeric width/depth",
            )


def validate_relative_staging_path(value, prefix: str) -> str:
    """Путь к fixture/texture: относительный, внутри фиксированного префикса."""
    if not isinstance(value, str):
        raise errors.PoCError(errors.TEMPLATE_INVALID, "path must be a string")
    paths.validate_relative_path(value)
    if not value.startswith(prefix):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            f"path must start with '{prefix}': {value!r}",
        )
    return value
