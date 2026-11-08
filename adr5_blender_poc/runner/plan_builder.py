"""Сборка конечного job-spec'а из зарегистрированного шаблона и запроса.

Правила:
- операции берутся ТОЛЬКО из шаблона (base_operations + выбранные
  component_blocks); слоты подставляют уже валидированные значения
  (палитра из таблицы шаблона, параметры) в заранее описанные места;
- строки-значения не интерполируются и не склеиваются: подставляется
  атомарное значение (число/enum-строка/список цветов);
- путь импорта фиксируется в staging 'inputs/', выход — в
  'outputs/<output_stem>/'.

Результат — job.json для СУЩЕСТВУЮЩЕГО worker'а (18 ops, contract
blender/worker.py без изменений).
"""

from __future__ import annotations

import copy
import json

from . import errors

# Слот задаёт место в операции: {"op_index": int, "field": "color",
# "source": "palette", "target_object": "walls"} и т.п.
_ALLOWED_SLOT_FIELDS = frozenset({"op_index", "field", "source", "param", "target_object"})


def build_job_spec(template, resolved, inputs_dir_name: str = "inputs") -> dict:
    """Возвращает dict job-spec'а (поле 'spec' job.json)."""
    plan = template.manifest["job_plan"]
    operations = copy.deepcopy(plan.get("base_operations", []))

    # V1: вставить операции выбранных component-блоков (в объявленном порядке
    # параметров component_choice_parameters), затем tail-операции.
    # insert_at_index — позиция вставки в ТЕКУЩИЙ список операций; блоки в
    # порядке объявления параметров, каждая вставка сдвигает последующие.
    if resolved.component_choices:
        blocks = plan.get("component_blocks") or {}
        inserted = 0
        for param_name in template.manifest.get("component_choice_parameters", []):
            choice_id = resolved.component_choices.get(param_name)
            if choice_id is None:
                continue
            block = blocks[choice_id]
            block_ops = copy.deepcopy(block["operations"])
            insert_at = int(block.get("insert_at_index", len(operations))) + inserted
            if insert_at < 0 or insert_at > len(operations):
                raise errors.PoCError(
                    errors.TEMPLATE_INVALID,
                    f"component block '{choice_id}': insert_at_index {insert_at} "
                    f"out of range",
                )
            operations[insert_at:insert_at] = block_ops
            inserted += len(block_ops)

    # Подстановка параметров-констант хвостовых операций (output пути).
    output_dir = f"outputs/{resolved.output_stem}"
    tail = plan.get("tail_operations", [])
    operations.extend(copy.deepcopy(tail))
    _bind_output_paths(operations, output_dir)

    # Слоты V0: подстановка значений по op_index/field.
    for slot in template.manifest.get("slots", []):
        _apply_slot(operations, slot, resolved)

    return {"operations": operations}


def _apply_slot(operations: list, slot: dict, resolved) -> None:
    unknown = set(slot) - _ALLOWED_SLOT_FIELDS
    if unknown:
        raise errors.PoCError(
            errors.TEMPLATE_INVALID, f"slot has unknown fields {sorted(unknown)}"
        )
    op_index = slot.get("op_index")
    field = slot.get("field")
    if not isinstance(op_index, int) or not isinstance(field, str):
        raise errors.PoCError(errors.TEMPLATE_INVALID, "slot needs op_index/field")
    if op_index < 0 or op_index >= len(operations):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            f"slot op_index {op_index} out of range (spec has {len(operations)} ops)",
        )
    operation = operations[op_index]
    if slot["source"] == "palette":
        target = slot.get("target_object")
        # Двойная защита F9: реестр отклонил слоты на защищённые объекты при
        # загрузке; здесь проверяем, что op вообще адресует объявленный объект.
        if operation.get("object") != target:
            raise errors.PoCError(
                errors.TEMPLATE_INVALID,
                f"palette slot targets object '{target}' but operation "
                f"addresses '{operation.get('object')}'",
            )
        color = (resolved.palette or {}).get(target)
        if color is None:
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"palette table has no entry for object '{target}'",
            )
        operation[field] = list(color)
    elif slot["source"] == "param":
        param_name = slot.get("param")
        if param_name not in resolved.params:
            raise errors.PoCError(
                errors.INVALID_PARAMETER, f"slot param '{param_name}' was not resolved"
            )
        operation[field] = resolved.params[param_name]
    elif slot["source"] == "const":
        if "value" not in slot:
            raise errors.PoCError(errors.TEMPLATE_INVALID, "const slot needs 'value'")
        operation[field] = copy.deepcopy(slot["value"])
    else:  # component_choice — не используется в слотах V0
        raise errors.PoCError(
            errors.TEMPLATE_INVALID, f"unsupported slot source {slot['source']!r}"
        )


def _bind_output_paths(operations: list, output_dir: str) -> None:
    """Заменяет placeholder 'outputs/@stem@/...' на фактический staging dir."""
    for operation in operations:
        for field in ("path",):
            value = operation.get(field)
            if isinstance(value, str) and value.startswith("outputs/@stem@/"):
                suffix = value[len("outputs/@stem@/") :]
                operation[field] = f"{output_dir}/{suffix}"


def dump_job_spec(spec: dict) -> str:
    """Канонический JSON для job.json (отладка/evidence)."""
    return json.dumps(spec, indent=2, sort_keys=True)
