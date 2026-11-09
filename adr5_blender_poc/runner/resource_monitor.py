"""Телеметрия ресурсов для тяжёлого Blender-процесса (RTX 3050 / 32 GB).

PoC-уровень, честные измерения:
- RAM: polling PeakWorkingSetSize процесса blender.exe (ctypes psapi,
  Windows; без спавна процессов — интервал опроса не срывается медленным
  запуском PowerShell). PeakWorkingSet — пик resident-памяти за жизнь.
- VRAM: polling `nvidia-smi --query-gpu=memory.used` — СУММАРНО по GPU
  (все процессы), не только Blender; в отчёте помечается явно.
- Если инструмент недоступен — поле получает NOT_MEASURED (никаких
  выдуманных чисел).

Guard: перед запуском Blender — `tasklist /FI "IMAGENAME eq blender.exe"`;
одновременно допускается только один тяжёлый Blender-процесс.
"""

from __future__ import annotations

import os
import subprocess
import threading
import time
from pathlib import Path

NOT_MEASURED = "NOT_MEASURED"
RAM_METHOD = "ctypes psapi GetProcessMemoryInfo PeakWorkingSetSize (polling)"
VRAM_METHOD = "nvidia-smi memory.used (GPU total, all processes)"


def assert_no_blender_running() -> None:
    """Отказ RESOURCE_BUSY, если blender.exe уже запущен (одна тяжёлая задача)."""
    try:
        # bytes-режим: консольный вывод tasklist в русской Windows — CP866,
        # декодирование UTF-8 падает; ищем только ASCII-префикс blender.exe.
        raw = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq blender.exe", "/NH", "/FO", "CSV"],
            capture_output=True,
            timeout=30,
        ).stdout or b""
        output = raw.decode("latin-1", errors="ignore")
    except (OSError, subprocess.SubprocessError):
        # tasklist недоступен (не Windows?) — на целевом стенде это Windows;
        # отсутствие проверки фиксируем в отчёте, а не притворяемся.
        return
    for line in output.splitlines():
        if line.strip().lower().startswith('"blender.exe'):
            from . import errors

            raise errors.PoCError(
                errors.RESOURCE_BUSY,
                "another blender.exe process is already running; only one "
                "heavy Blender process is allowed on the RTX 3050 stand",
            )


class ResourceMonitor:
    """Фоновый сборщик RAM/VRAM на время одного Blender-прогона."""

    def __init__(self, poll_interval_s: float = 0.5):
        self.poll_interval_s = poll_interval_s
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self.peak_blender_ram_bytes = 0
        self.gpu_used_max_mib: int | None = None
        self.ram_method = NOT_MEASURED
        self.vram_method = NOT_MEASURED

    # --- lifecycle ---------------------------------------------------------
    def __enter__(self) -> "ResourceMonitor":
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.stop()

    def start(self) -> None:
        self._thread = threading.Thread(target=self._poll_loop, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=5)

    # --- polling -----------------------------------------------------------
    def _poll_loop(self) -> None:
        while not self._stop.is_set():
            self._poll_ram_once()
            self._poll_vram_once()
            self._stop.wait(self.poll_interval_s)

    def _poll_ram_once(self) -> None:
        try:
            value = _blender_peak_working_set_bytes()
        except Exception:  # noqa: BLE001 - телеметрия не должна ломать прогон
            return
        if value is None:
            return
        if value > self.peak_blender_ram_bytes:
            self.peak_blender_ram_bytes = value
        self.ram_method = RAM_METHOD

    def _poll_vram_once(self) -> None:
        try:
            output = subprocess.run(
                [
                    "nvidia-smi",
                    "--query-gpu=memory.used",
                    "--format=csv,noheader,nounits",
                ],
                capture_output=True,
                text=True,
                timeout=20,
            ).stdout.strip()
            first = output.splitlines()[0].strip() if output else ""
            if first.isdigit():
                value = int(first)
                if self.gpu_used_max_mib is None or value > self.gpu_used_max_mib:
                    self.gpu_used_max_mib = value
                self.vram_method = VRAM_METHOD
        except (OSError, IndexError, subprocess.SubprocessError):
            return

    # --- отчёт -------------------------------------------------------------
    def report(self) -> dict:
        peak_mib = (
            round(self.peak_blender_ram_bytes / (1024 * 1024), 1)
            if self.peak_blender_ram_bytes > 0
            else None
        )
        return {
            "peak_blender_ram_mib": (
                peak_mib if peak_mib is not None else NOT_MEASURED
            ),
            "ram_method": self.ram_method,
            "gpu_used_max_mib": (
                self.gpu_used_max_mib
                if self.gpu_used_max_mib is not None
                else NOT_MEASURED
            ),
            "vram_method": self.vram_method,
            "vram_scope_note": (
                "GPU total across all processes, not Blender-only"
                if self.gpu_used_max_mib is not None
                else "no VRAM telemetry available"
            ),
        }


def _blender_peak_working_set_bytes() -> int | None:
    """Peak Working Set процесса blender.exe (ctypes, без спавна процессов).

    Возвращает None, если процесс не найден или платформа не Windows.
    """
    import ctypes

    if os.name != "nt":
        return None
    import ctypes.wintypes

    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000

    class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.wintypes.DWORD),
            ("PageFaultCount", ctypes.wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    count = ctypes.wintypes.DWORD(4096)
    pids = (ctypes.wintypes.DWORD * 4096)()
    if not psapi.EnumProcesses(pids, ctypes.sizeof(pids), ctypes.byref(count)):
        return None
    listed = count.value // ctypes.sizeof(ctypes.wintypes.DWORD)

    name_buffer = ctypes.create_unicode_buffer(1024)
    size = ctypes.wintypes.DWORD(1024)
    peak = None
    for index in range(listed):
        pid = pids[index]
        if not pid:
            continue
        handle = kernel32.OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION, False, pid
        )
        if not handle:
            continue
        try:
            size.value = 1024
            if not kernel32.QueryFullProcessImageNameW(
                handle, 0, name_buffer, ctypes.byref(size)
            ):
                continue
            image = name_buffer.value
            if not image.lower().endswith("\\blender.exe"):
                continue
            counters = PROCESS_MEMORY_COUNTERS()
            counters.cb = ctypes.sizeof(PROCESS_MEMORY_COUNTERS)
            if psapi.GetProcessMemoryInfo(
                handle, ctypes.byref(counters), counters.cb
            ):
                value = int(counters.PeakWorkingSetSize)
                if peak is None or value > peak:
                    peak = value
        finally:
            kernel32.CloseHandle(handle)
    return peak


def resolve_attempt_token() -> str:
    """Короткий уникальный суффикс для run/attempt id."""
    import uuid

    return uuid.uuid4().hex[:12]


def resolve_blender_bin(explicit: str | None = None) -> str | None:
    """BLENDER_BIN -> PATH -> типовые пути установки; None если не найдено."""
    import shutil

    if explicit:
        return explicit if Path(explicit).is_file() else None
    env = os.environ.get("BLENDER_BIN")
    if env and Path(env).is_file():
        return env
    found = shutil.which("blender")
    if found:
        return found
    candidates = [
        r"C:\Program Files\Blender Foundation\Blender\blender.exe",
        r"I:\Tools\Blender\blender.exe",
        r"D:\Program Files\Blender Foundation\Blender\blender.exe",
    ]
    for candidate in candidates:
        if Path(candidate).is_file():
            return candidate
    return None
