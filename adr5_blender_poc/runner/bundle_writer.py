"""Writer CandidateAssetBundle (draft-0.1, GLM-1 schema — ориентир).

Форма: schemas/adr5/candidate_bundle.schema.json
`michelangelo.candidate-asset-bundle/draft-0.1`. Статус — константа
`proposed` (bundle никогда не повышает себя до accepted).

Digest-правила — schemas/adr5/README.md (draft):
- asset_digest: SHA-256 над конкатенацией UTF-8 строк
  `<artifact_id>\u0000<path>\u0000<content_kind>\u0000<sha256>\n`,
  отсортированных byte-wise по (artifact_id, path);
- sha256 — по фактически прочитанным байтам (пересчитывается здесь);
- пути — workspace-relative POSIX без эскейпа (PATH_ESCAPE при нарушении).

bundle draft PoC НЕ является утверждённым wire: только ориентир для
интегратора; валидация экземпляров — против схемы GLM-1 отдельной командой.
"""

from __future__ import annotations

import datetime
import hashlib
import json
from pathlib import Path

from . import errors, paths

BUNDLE_SCHEMA_TAG = "michelangelo.candidate-asset-bundle/draft-0.1"
STATUS_PROPOSED = "proposed"  # константа v0.1

# content_kind из candidate_bundle.schema.json
CONTENT_KINDS = frozenset(
    {
        "geometry",
        "material",
        "texture",
        "rig_definition",
        "animation",
        "scene_layout",
        "node_group",
        "linked_library",
        "metadata",
        "other",
    }
)


def compute_asset_digest(artifacts: list[dict]) -> str:
    """README схем §4: сортировка byte-wise по (artifact_id, path)."""
    lines = []
    for artifact in sorted(artifacts, key=lambda a: (a["artifact_id"], a["path"])):
        lines.append(
            f"{artifact['artifact_id']}\u0000{artifact['path']}\u0000"
            f"{artifact['content_kind']}\u0000{artifact['sha256']}\n"
        )
    return hashlib.sha256("".join(lines).encode("utf-8")).hexdigest()


def build_bundle(
    *,
    candidate_id: str,
    asset_id: str,
    item_id: str,
    artifacts: list[dict],
    dependency_digests: dict[str, str],
    base_ref: dict | None,
    provenance_chain: list[dict],
    unresolved_issues: list[str],
    expected_current_revision: int | None = None,
    parents: list[dict] | None = None,
    diff_to_base: dict | None = None,
    recipe_ref: dict | None = None,
    created_at: str | None = None,
) -> dict:
    """Собирает bundle-документ draft-0.1 и валидирует локальные инварианты."""
    if not paths.is_safe_item_id(candidate_id):
        raise errors.PoCError(
            errors.INVALID_PARAMETER, f"unsafe candidate_id {candidate_id!r}"
        )
    if not paths.is_safe_item_id(asset_id):
        raise errors.PoCError(errors.INVALID_PARAMETER, f"unsafe asset_id {asset_id!r}")
    if not paths.is_safe_item_id(item_id):
        raise errors.PoCError(errors.INVALID_PARAMETER, f"unsafe item_id {item_id!r}")
    if not artifacts:
        raise errors.PoCError(
            errors.ARTIFACT_MISSING, "bundle requires at least one artifact"
        )

    normalized_artifacts = []
    for artifact in artifacts:
        if artifact["content_kind"] not in CONTENT_KINDS:
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"artifact {artifact.get('artifact_id')!r}: unknown content_kind "
                f"{artifact.get('content_kind')!r}",
            )
        paths.validate_relative_path(artifact["path"])
        if not paths.is_sha256_hex(artifact["sha256"]):
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"artifact {artifact.get('artifact_id')!r}: sha256 must be 64 hex",
            )
        normalized_artifacts.append(
            {
                "artifact_id": artifact["artifact_id"],
                "path": artifact["path"],
                "sha256": artifact["sha256"],
                "size_bytes": int(artifact["size_bytes"]),
                "content_kind": artifact["content_kind"],
                **(
                    {"component_id": artifact["component_id"]}
                    if artifact.get("component_id")
                    else {}
                ),
                **({"source": artifact["source"]} if artifact.get("source") else {}),
            }
        )

    for dep_id, dep_sha in dependency_digests.items():
        if not paths.is_sha256_hex(dep_sha):
            raise errors.PoCError(
                errors.INVALID_PARAMETER, f"dependency '{dep_id}': bad sha256"
            )
    if not dependency_digests:
        raise errors.PoCError(
            errors.INVALID_PARAMETER, "dependency_digests must not be empty"
        )

    if base_ref is not None:
        if base_ref.get("kind") not in ("source_snapshot", "asset_revision"):
            raise errors.PoCError(
                errors.INVALID_PARAMETER, "base_ref.kind must be a known kind"
            )
        if not paths.is_sha256_hex(base_ref.get("sha256", "")):
            raise errors.PoCError(errors.INVALID_PARAMETER, "base_ref.sha256 bad")

    created_at = created_at or _rfc3339_now()
    bundle = {
        "schema": BUNDLE_SCHEMA_TAG,
        "candidate_id": candidate_id,
        "status": STATUS_PROPOSED,
        "created_at": created_at,
        "asset_id": asset_id,
        "artifacts": normalized_artifacts,
        "dependency_digests": dict(dependency_digests),
        "asset_digest": compute_asset_digest(normalized_artifacts),
        "provenance_chain": provenance_chain,
        "unresolved_issues": list(unresolved_issues),
    }
    if expected_current_revision is not None:
        bundle["expected_current_revision"] = expected_current_revision
    if base_ref is not None:
        bundle["base_ref"] = base_ref
    if recipe_ref is not None:
        bundle["recipe_ref"] = recipe_ref
    if item_id:
        bundle["item_id"] = item_id
    if parents:
        bundle["parents"] = parents
    if diff_to_base is not None:
        bundle["diff_to_base"] = diff_to_base
    return bundle


def write_bundle(bundle: dict, output_path) -> str:
    """Пишет bundle как canonical-подобный JSON (sorted keys, LF, UTF-8, no BOM).

    Возвращает SHA-256 записанных байт. Экземпляр остаётся `proposed`.
    """
    output_path = Path(output_path)
    payload = json.dumps(bundle, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    data = payload.encode("utf-8")
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(data)
    return hashlib.sha256(data).hexdigest()


def validate_bundle_against_schema(bundle: dict, schema_path) -> list[str]:
    """Валидация экземпляра против схемы GLM-1 (jsonschema >= 4.18).

    Возвращает список ошибок (пустой = валидно); если jsonschema недоступна,
    возвращает единственную строку-причину SKIPPED (не PASS).
    """
    try:
        import jsonschema
    except ImportError:
        return ["SKIPPED: jsonschema is not available in this Python"]
    schema = json.loads(Path(schema_path).read_text(encoding="utf-8"))
    validator = jsonschema.Draft202012Validator(schema)
    return [
        f"{error.json_path}: {error.message}" for error in sorted(validator.iter_errors(bundle), key=lambda e: e.json_path)
    ]


def _rfc3339_now() -> str:
    now = datetime.datetime.now(datetime.timezone.utc)
    return now.strftime("%Y-%m-%dT%H:%M:%S") + "Z"
