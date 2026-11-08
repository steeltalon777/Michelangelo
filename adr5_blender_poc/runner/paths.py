"""Проверки путей PoC-раннера (F13: path escape).

Все пути — workspace-relative, '/'-сепаратор, без '..'-сегментов, абсолютных
форм, backslash, drive-букв и NUL. Дополнительная фактическая проверка
контейнмента (resolve + relative_to) выполняется на реальном каталоге
staging — здесь только строковые инварианты (pure, тестируемые без Blender).
"""

from __future__ import annotations

import re
from pathlib import Path, PurePosixPath

from . import errors

# Базовое имя элемента: строчные латинские/цифры/дефис/подчёркивание.
_SAFE_ITEM_ID = re.compile(r"\A[a-z0-9][a-z0-9_-]{0,63}\Z")

# Сегмент относительного пути внутри staging (без '/', '..' и т.д.).
_SAFE_SEGMENT = re.compile(r"\A[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}\Z")

_WINDOWS_DRIVE = re.compile(r"\A[A-Za-z]:")

MAX_POC_PATH_CHARS = 512


def is_safe_item_id(value: str) -> bool:
    """Стабильный item/candidate/asset id (`idToken`-подобный, строчный)."""
    return isinstance(value, str) and bool(_SAFE_ITEM_ID.match(value))


def is_safe_output_stem(value: str) -> bool:
    """Безопасное базовое имя выходного файла варианта (без расширения)."""
    if not isinstance(value, str) or not value or len(value) > 128:
        return False
    if value != value.rstrip(". "):
        return False
    return bool(_SAFE_SEGMENT.match(value))


def validate_relative_path(value: str) -> str:
    """Workspace-relative posix-путь без эскейпа; возвращает его же или raising.

    Отклоняет: пустое/не-строковое, абсолютные пути, drive/UNC формы,
    backslash, '..'-сегменты, сегменты '.', NUL, ведущий '/'.
    """
    if not isinstance(value, str) or not value:
        raise errors.PoCError(
            errors.INVALID_OUTPUT_PATH, "path must be a non-empty string"
        )
    if len(value) > MAX_POC_PATH_CHARS:
        raise errors.PoCError(
            errors.INVALID_OUTPUT_PATH,
            f"path too long ({len(value)} > {MAX_POC_PATH_CHARS})",
        )
    if "\x00" in value:
        raise errors.PoCError(
            errors.INVALID_OUTPUT_PATH, "path contains NUL"
        )
    if "\\" in value:
        raise errors.PoCError(
            errors.INVALID_OUTPUT_PATH,
            "path must use '/' separators (backslash rejected)",
        )
    if value.startswith("/") or _WINDOWS_DRIVE.match(value) or value.startswith("//"):
        raise errors.PoCError(
            errors.INVALID_OUTPUT_PATH,
            f"absolute/drive/UNC paths are rejected: {value!r}",
        )
    pure = PurePosixPath(value)
    for segment in pure.parts:
        if segment == ".." or segment == ".":
            raise errors.PoCError(
                errors.INVALID_OUTPUT_PATH,
                f"parent/self path segments are rejected: {value!r}",
            )
        if not _SAFE_SEGMENT.match(segment):
            raise errors.PoCError(
                errors.INVALID_OUTPUT_PATH,
                f"unsafe path segment {segment!r} in {value!r}",
            )
    return value


def ensure_contained(path: Path, root: Path) -> Path:
    """Фактический контейнмент после resolve (symlink-aware). Рост ошибки."""
    resolved_root = root.resolve()
    try:
        resolved = path.resolve(strict=True)
    except (OSError, RuntimeError) as exc:
        raise errors.PoCError(
            errors.ARTIFACT_MISSING, f"path does not resolve: {path} ({exc})"
        ) from exc
    try:
        resolved.relative_to(resolved_root)
    except ValueError:
        raise errors.PoCError(
            errors.ARTIFACT_PATH_ESCAPE,
            f"resolved path escapes the staging root: {resolved}",
        ) from None
    return resolved


def sha256_file(path: Path) -> str:
    """SHA-256 по фактическим байтам файла."""
    import hashlib

    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def is_sha256_hex(value: str) -> bool:
    return isinstance(value, str) and bool(re.fullmatch(r"[0-9a-f]{64}", value))
