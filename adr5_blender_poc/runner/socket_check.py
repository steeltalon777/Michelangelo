"""Статическая проверка сокетов/коннекторов компонентов (V1).

PoC-уровень: проверка выполняется по данным зарегистрированного шаблона
(плановая), а не измерением сцены. Фактическая проверка эффективного
результата — отдельная сверка bounds/counts из scene_summary после прогона
(BOUNDS_MISMATCH) в template_executor.

Правило совместимости PoC: коннектор компонента должен накрывать сокет
(footprint width/depth >= сокетных), base_z должен совпадать с z сокета,
перекрытие footprint по центрам должно быть не пустым.
"""

from __future__ import annotations

from . import errors


def check_choice_against_socket(socket: dict, block: dict, label: str) -> dict:
    """Возвращает connector блока или raising INVALID_SOCKET."""
    connector = block.get("connector")
    if connector is None:
        # Блок без коннектора (например, выбор "none") не участвует в стыковке.
        return {}
    for field in ("width", "depth", "base_z"):
        if field not in connector:
            raise errors.PoCError(
                errors.INVALID_SOCKET,
                f"{label}: connector missing '{field}'",
                {"choice": label},
            )
    socket_width = float(socket["footprint"]["width"])
    socket_depth = float(socket["footprint"]["depth"])
    socket_z = float(socket["z"])
    width = float(connector["width"])
    depth = float(connector["depth"])
    base_z = float(connector["base_z"])
    if width + 1e-9 < socket_width or depth + 1e-9 < socket_depth:
        raise errors.PoCError(
            errors.INVALID_SOCKET,
            f"{label}: component footprint {width}x{depth} does not cover the "
            f"socket {socket_width}x{socket_depth} (socket coverage required)",
            {
                "connector": {"width": width, "depth": depth},
                "socket": {"width": socket_width, "depth": socket_depth},
            },
        )
    if abs(base_z - socket_z) > 1e-6:
        raise errors.PoCError(
            errors.INVALID_SOCKET,
            f"{label}: component base_z {base_z} does not match socket z {socket_z}",
            {"connector_base_z": base_z, "socket_z": socket_z},
        )
    return connector


def check_request_sockets(template, component_choices: dict) -> None:
    """Проверяет все выбранные компоненты против сокетов шаблона.

    component_choices: {param_name: choice_id}; соответствие параметра сокету
    задаёт dict component_choice_parameters в манифесте.
    """
    sockets = template.manifest.get("sockets") or {}
    choice_params = template.manifest.get("component_choice_parameters") or {}
    if not isinstance(choice_params, dict):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            "'component_choice_parameters' must map parameter -> socket id",
        )
    blocks = template.manifest["job_plan"].get("component_blocks") or {}

    for param_name, choice_id in component_choices.items():
        socket_id = choice_params.get(param_name)
        if socket_id is None:
            continue
        socket = sockets.get(socket_id)
        if socket is None:
            raise errors.PoCError(
                errors.UNKNOWN_SOCKET,
                f"parameter '{param_name}' references unknown socket '{socket_id}'",
            )
        block = blocks.get(choice_id)
        if block is None:
            raise errors.PoCError(
                errors.INVALID_SOCKET,
                f"choice '{choice_id}' has no registered component block",
            )
        check_choice_against_socket(
            socket, block, f"component '{choice_id}' vs socket '{socket_id}'"
        )


def expected_bounds_for(template, component_choices: dict) -> tuple[list, list]:
    """Ожидаемые world bounds из данных шаблона (для runtime-сверки).

    Берутся из поля 'expected_bounds' выбранного блока (константы, выведенные
    из геометрии при регистрации шаблона), объединённые с bounds источника
    ('source_bounds' манифеста). Возврат (min[3], max[3]).
    """
    source_bounds = template.manifest.get("source_bounds")
    if (
        not isinstance(source_bounds, dict)
        or not isinstance(source_bounds.get("min"), list)
        or not isinstance(source_bounds.get("max"), list)
    ):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            "manifest must pin 'source_bounds' {min, max} for runtime checks",
        )
    mins = [float(v) for v in source_bounds["min"]]
    maxs = [float(v) for v in source_bounds["max"]]

    blocks = template.manifest["job_plan"].get("component_blocks") or {}
    for _param, choice_id in component_choices.items():
        block = blocks.get(choice_id) or {}
        block_bounds = block.get("expected_bounds")
        if not block_bounds:
            continue
        for axis in range(3):
            mins[axis] = min(mins[axis], float(block_bounds["min"][axis]))
            maxs[axis] = max(maxs[axis], float(block_bounds["max"][axis]))
    return mins, maxs


def expected_scene_counts_for(template, component_choices: dict) -> tuple[int, int]:
    """Ожидаемые (vertices, triangles) для выбранной конфигурации."""
    source_counts = template.manifest.get("source_counts")
    if (
        not isinstance(source_counts, dict)
        or "vertices" not in source_counts
        or "triangles" not in source_counts
    ):
        raise errors.PoCError(
            errors.TEMPLATE_INVALID,
            "manifest must pin 'source_counts' {vertices, triangles}",
        )
    vertices = int(source_counts["vertices"])
    triangles = int(source_counts["triangles"])
    blocks = template.manifest["job_plan"].get("component_blocks") or {}
    for _param, choice_id in component_choices.items():
        block = blocks.get(choice_id) or {}
        counts = block.get("expected_counts") or {}
        vertices += int(counts.get("vertices", 0))
        triangles += int(counts.get("triangles", 0))
    return vertices, triangles
