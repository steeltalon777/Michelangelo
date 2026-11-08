"""Парсер GLB (binary glTF 2.0) — отчёт о ФАКТИЧЕСКОМ экспорте.

Читает JSON-chunk GLB и собирает фактический отчёт: меши/примитивы, число
вершин/треугольников (accessor counts), наличие UV (TEXCOORD_0), материалы с
baseColorFactor, имена объектов (nodes). Используется для:
- проверки визуальной различимости вариантов (материалы различны);
- проверки сохранности геометрии (counts/локальные bounds совпадают);
- честного отчёта «фактическая геометрия/UV/материалы/имена» без заявлений
  о UE-ready (UV в GLB не проверяет импорт в движок).

Только стандартная библиотека; разбирается сам файл, никакой сети.
"""

from __future__ import annotations

import json
import struct

_GLTF_MAGIC = 0x46546C67  # 'glTF'
_JSON_CHUNK_TYPE = 0x4E4F534A  # 'JSON'


def parse_glb(path) -> dict:
    """Возвращает словарь glTF JSON из GLB-файла."""
    with open(path, "rb") as handle:
        header = handle.read(12)
        if len(header) < 12:
            raise ValueError("not a GLB: file too small")
        magic, _version, _length = struct.unpack("<III", header)
        if magic != _GLTF_MAGIC:
            raise ValueError("not a GLB: bad magic")
        chunk_header = handle.read(8)
        if len(chunk_header) < 8:
            raise ValueError("not a GLB: missing chunk header")
        chunk_len, chunk_type = struct.unpack("<II", chunk_header)
        if chunk_type != _JSON_CHUNK_TYPE:
            raise ValueError("not a GLB: first chunk is not JSON")
        payload = handle.read(chunk_len)
        return json.loads(payload)


def _accessor_count(gltf: dict, index) -> int | None:
    accessors = gltf.get("accessors") or []
    if not isinstance(index, int) or index < 0 or index >= len(accessors):
        return None
    return int(accessors[index].get("count", 0))


def _accessor_minmax(gltf: dict, index) -> tuple[list, list] | None:
    accessors = gltf.get("accessors") or []
    if not isinstance(index, int) or index < 0 or index >= len(accessors):
        return None
    accessor = accessors[index]
    if accessor.get("type") != "VEC3" or "min" not in accessor or "max" not in accessor:
        return None
    return [float(v) for v in accessor["min"]], [float(v) for v in accessor["max"]]


def _material_summary(gltf: dict) -> list[dict]:
    materials = []
    for index, material in enumerate(gltf.get("materials") or []):
        pbr = material.get("pbrMetallicRoughness") or {}
        base_color = pbr.get("baseColorFactor")
        materials.append(
            {
                "index": index,
                "name": material.get("name"),
                "base_color_factor": (
                    [round(float(c), 6) for c in base_color]
                    if isinstance(base_color, list)
                    else None
                ),
            }
        )
    return materials


def report_glb(path) -> dict:
    """Фактический отчёт о GLB: имена объектов, меши, UV, материалы.

    Ключи: file_sha256_placeholder нет — hash считает вызывающая сторона;
    здесь только контент. `bounds_local_*` — объединённые accessor min/max
    локальных координат мешей (без node-трансформов; PoC-пометка).
    """
    gltf = parse_glb(path)
    nodes = gltf.get("nodes") or []
    meshes = gltf.get("meshes") or []

    node_reports = []
    for node in nodes:
        mesh_index = node.get("mesh")
        if not isinstance(mesh_index, int) or mesh_index >= len(meshes):
            continue
        mesh = meshes[mesh_index]
        primitives = []
        vertices = 0
        triangles = 0
        has_uv_all = True
        mins, maxs = None, None
        for primitive in mesh.get("primitives") or []:
            attributes = primitive.get("attributes") or {}
            position = attributes.get("POSITION")
            position_count = _accessor_count(gltf, position) or 0
            indices = primitive.get("indices")
            if indices is not None:
                index_count = _accessor_count(gltf, indices) or 0
                tri = index_count // 3
            else:
                tri = position_count // 3
            vertices += position_count
            triangles += tri
            has_uv = "TEXCOORD_0" in attributes
            has_uv_all = has_uv_all and has_uv
            minmax = _accessor_minmax(gltf, position)
            if minmax is not None:
                lo, hi = minmax
                mins = lo if mins is None else [min(a, b) for a, b in zip(mins, lo)]
                maxs = hi if maxs is None else [max(a, b) for a, b in zip(maxs, hi)]
            primitives.append(
                {
                    "vertices": position_count,
                    "triangles": tri,
                    "has_uv_texcoord_0": has_uv,
                    "material_index": primitive.get("material"),
                    "mode": primitive.get("mode", 4),
                }
            )
        node_reports.append(
            {
                "node_name": node.get("name"),
                "mesh_name": mesh.get("name"),
                "translation": [
                    float(v) for v in (node.get("translation") or [0.0, 0.0, 0.0])
                ],
                "rotation_present": "rotation" in node,
                "primitives": primitives,
                "vertices_total": vertices,
                "triangles_total": triangles,
                "has_uv_all_primitives": has_uv_all,
                "bounds_local_min": mins,
                "bounds_local_max": maxs,
            }
        )

    total_vertices = sum(node["vertices_total"] for node in node_reports)
    total_triangles = sum(node["triangles_total"] for node in node_reports)
    materials = _material_summary(gltf)

    return {
        "generator": (gltf.get("asset") or {}).get("generator"),
        "glTF_version": (gltf.get("asset") or {}).get("version"),
        "nodes": node_reports,
        "meshes_count": len(meshes),
        "materials": materials,
        "materials_count": len(materials),
        "total_vertices": total_vertices,
        "total_triangles": total_triangles,
        "bounds_local_min": mins,
        "bounds_local_max": maxs,
        "note": (
            "bounds_local_* are accessor min/max in local mesh coordinates "
            "(no node transforms applied); authoritative world bounds come "
            "from the worker scene_summary"
        ),
    }


def summarize_geometry(gltf_report: dict) -> dict:
    """Компактный геометрический профиль для сравнения вариантов."""
    return {
        "total_vertices": gltf_report["total_vertices"],
        "total_triangles": gltf_report["total_triangles"],
        "bounds_local_min": gltf_report["bounds_local_min"],
        "bounds_local_max": gltf_report["bounds_local_max"],
        "node_names": [node["node_name"] for node in gltf_report["nodes"]],
        "uv_all": all(node["has_uv_all_primitives"] for node in gltf_report["nodes"]),
    }


def geometry_equal(a: dict, b: dict, tolerance: float = 1e-4) -> tuple[bool, str]:
    """Фактическая равность геометрии двух GLB-отчётов (counts + bounds)."""
    sa, sb = summarize_geometry(a), summarize_geometry(b)
    if sorted(x for x in sa["node_names"] if x) != sorted(
        x for x in sb["node_names"] if x
    ):
        return False, f"node names differ: {sa['node_names']!r} vs {sb['node_names']!r}"
    for key in ("total_vertices", "total_triangles", "uv_all"):
        if sa[key] != sb[key]:
            return False, f"{key} differs: {sa[key]!r} vs {sb[key]!r}"
    for key in ("bounds_local_min", "bounds_local_max"):
        va, vb = sa[key], sb[key]
        if (va is None) != (vb is None):
            return False, f"{key} presence differs"
        if va is not None:
            for x, y in zip(va, vb):
                if abs(x - y) > tolerance:
                    return False, f"{key} differs beyond {tolerance}: {va} vs {vb}"
    return True, "geometry profile identical"


def materials_distinct(reports: list[dict]) -> tuple[bool, str]:
    """Фактическая различимость вариантов: наборы материалов попарно различны."""
    signatures = []
    for report in reports:
        signature = tuple(
            (m["name"], tuple(m["base_color_factor"] or ()))
            for m in report["materials"]
        )
        signatures.append(signature)
    for i in range(len(signatures)):
        for j in range(i + 1, len(signatures)):
            if signatures[i] == signatures[j]:
                return False, f"variants {i} and {j} have identical material sets"
    return True, f"{len(signatures)} variants have pairwise distinct materials"
