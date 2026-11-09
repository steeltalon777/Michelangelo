"""Адресные коды ошибок PoC-исполнителя шаблонов.

Каждая ошибка — чётный ненулевой результат (не «успешная починка»).
Коды стабильны и используются в отчётах/тестах.

Маппинг на негативы GLM-1 (docs/adr5-streams/contracts/NEGATIVE_TEST_MATRIX.md):
- F9  -> SLOT_TARGETS_PROTECTED_ROLE / BOUNDS_MISMATCH
- F10 -> TEMPLATE_FORBIDDEN_KEY / UNKNOWN_PARAMETER (параметр вне реестра слотов)
- F13 -> INVALID_OUTPUT_PATH / ARTIFACT_PATH_ESCAPE
- F18 -> CAPABILITY_UNAVAILABLE
"""

from __future__ import annotations


class PoCError(Exception):
    """Отказ исполнения шаблона с адресным кодом.

    `code` — стабильный строковый код; `details` — машиночитаемые дополнения.
    """

    def __init__(self, code: str, message: str, details: dict | None = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = details or {}

    def to_dict(self) -> dict:
        return {"code": self.code, "message": self.message, "details": self.details}

    def __str__(self) -> str:  # pragma: no cover - presentation only
        return f"[{self.code}] {self.message}"


# --- реестр/шаблоны -------------------------------------------------------
TEMPLATE_HASH_MISMATCH = "TEMPLATE_HASH_MISMATCH"
TEMPLATE_INVALID = "TEMPLATE_INVALID"
TEMPLATE_FORBIDDEN_KEY = "TEMPLATE_FORBIDDEN_KEY"  # F10
TEMPLATE_NOT_FOUND = "TEMPLATE_NOT_FOUND"
SLOT_TARGETS_PROTECTED_ROLE = "SLOT_TARGETS_PROTECTED_ROLE"  # F9 (статический guard)

# --- параметры ------------------------------------------------------------
UNKNOWN_PARAMETER = "UNKNOWN_PARAMETER"  # F10 на уровне запроса
INVALID_PARAMETER = "INVALID_PARAMETER"
INVALID_OUTPUT_PATH = "INVALID_OUTPUT_PATH"  # F13
PARAMETER_SCOPE_PROTECTED = "PARAMETER_SCOPE_PROTECTED"  # F9

# --- входы/исходники ------------------------------------------------------
MISSING_MODULE = "MISSING_MODULE"  # отсутствует файл исходника/модуля
SOURCE_HASH_MISMATCH = "SOURCE_HASH_MISMATCH"  # nonmatching source hash
MISSING_TEXTURE = "MISSING_TEXTURE"

# --- план/сборка ----------------------------------------------------------
INVALID_SOCKET = "INVALID_SOCKET"
UNKNOWN_SOCKET = "UNKNOWN_SOCKET"
BOUNDS_MISMATCH = "BOUNDS_MISMATCH"  # эффективный результат не совпал с планом

# --- возможности/окружение ------------------------------------------------
CAPABILITY_UNAVAILABLE = "CAPABILITY_UNAVAILABLE"  # F18
BLENDER_NOT_FOUND = "BLENDER_NOT_FOUND"
RESOURCE_BUSY = "RESOURCE_BUSY"  # уже запущен другой Blender-процесс
WORKER_FAILED = "WORKER_FAILED"  # worker вернул failed/ненулевой exit
WORKER_TIMEOUT = "WORKER_TIMEOUT"

# --- выходы ---------------------------------------------------------------
ARTIFACT_PATH_ESCAPE = "ARTIFACT_PATH_ESCAPE"  # F13 (артефакт вне staging)
ARTIFACT_MISSING = "ARTIFACT_MISSING"

# --- evidence (диспозитивный каталог прогона) -----------------------------
EVIDENCE_DIR_UNAVAILABLE = "EVIDENCE_DIR_UNAVAILABLE"  # не удалось создать run-<ts>


ALL_CODES = (
    TEMPLATE_HASH_MISMATCH,
    TEMPLATE_INVALID,
    TEMPLATE_FORBIDDEN_KEY,
    TEMPLATE_NOT_FOUND,
    SLOT_TARGETS_PROTECTED_ROLE,
    UNKNOWN_PARAMETER,
    INVALID_PARAMETER,
    INVALID_OUTPUT_PATH,
    PARAMETER_SCOPE_PROTECTED,
    MISSING_MODULE,
    SOURCE_HASH_MISMATCH,
    MISSING_TEXTURE,
    INVALID_SOCKET,
    UNKNOWN_SOCKET,
    BOUNDS_MISMATCH,
    CAPABILITY_UNAVAILABLE,
    BLENDER_NOT_FOUND,
    RESOURCE_BUSY,
    WORKER_FAILED,
    WORKER_TIMEOUT,
    ARTIFACT_PATH_ESCAPE,
    ARTIFACT_MISSING,
    EVIDENCE_DIR_UNAVAILABLE,
)
