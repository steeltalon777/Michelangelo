"""Запуск СУЩЕСТВУЮЩЕГО Blender Worker (`blender/worker.py`) headless.

- бинарник Blender: `--blender-bin`/BLENDER_BIN/PATH/типовые пути;
- режим: `--background --factory-startup` (заводские настройки, без
  пользовательских аддонов; НЕ является security sandbox);
- перед КАЖДЫМ запуском — tasklist-guard: только ОДИН blender.exe;
- stdout/stderr — в файлы лога попытки (никогда не пайпы);
- таймаут: процесс убивается (Windows taskkill /T /F) и фиксируется
  WORKER_TIMEOUT;
- worker.py НЕ изменяется: используется по абсолютному пути из worktree.
"""

from __future__ import annotations

import json
import subprocess
import time
from pathlib import Path

from . import errors, resource_monitor

DEFAULT_TIMEOUT_S = 300.0


class WorkerRunOutcome:
    def __init__(self, exit_code: int, result: dict | None, timed_out: bool, log_dir: Path):
        self.exit_code = exit_code
        self.result = result
        self.timed_out = timed_out
        self.log_dir = log_dir


def run_worker(
    blender_bin: str,
    worker_py: Path,
    job_json: Path,
    result_json: Path,
    log_dir: Path,
    timeout_s: float = DEFAULT_TIMEOUT_S,
) -> WorkerRunOutcome:
    """Один headless-прогон worker'а; возвращает исход, никогда не «чинит» ошибки."""
    resource_monitor.assert_no_blender_running()

    log_dir.mkdir(parents=True, exist_ok=True)
    command = [
        blender_bin,
        "--background",
        "--factory-startup",
        "--python",
        str(worker_py),
        "--",
        str(job_json),
        str(result_json),
    ]
    stdout_log = log_dir / "stdout.log"
    stderr_log = log_dir / "stderr.log"

    timed_out = False
    start = time.monotonic()
    with open(stdout_log, "wb") as out_f, open(stderr_log, "wb") as err_f:
        process = subprocess.Popen(
            command,
            stdout=out_f,
            stderr=err_f,
            stdin=subprocess.DEVNULL,
        )
        try:
            exit_code = process.wait(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            timed_out = True
            _kill_process_tree(process)
            exit_code = process.wait(timeout=60)
    elapsed_s = time.monotonic() - start

    result = None
    if result_json.is_file():
        try:
            result = json.loads(result_json.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            result = None

    if timed_out:
        raise errors.PoCError(
            errors.WORKER_TIMEOUT,
            f"blender worker exceeded {timeout_s:.0f}s (elapsed {elapsed_s:.1f}s); "
            f"process tree killed; logs: {log_dir}",
            {"log_dir": str(log_dir), "elapsed_s": round(elapsed_s, 1)},
        )

    outcome = WorkerRunOutcome(exit_code, result, timed_out, log_dir)
    outcome.elapsed_s = round(elapsed_s, 1)
    return outcome


def _kill_process_tree(process: subprocess.Popen) -> None:
    import os
    import signal

    if os.name == "nt":
        subprocess.run(
            ["taskkill", "/T", "/F", "/PID", str(process.pid)],
            capture_output=True,
            timeout=30,
        )
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            process.kill()


def ensure_worker_exit_ok(outcome: WorkerRunOutcome) -> dict:
    """Требует completed-результат; иначе адресная ошибка WORKER_FAILED."""
    result = outcome.result
    if result is None:
        raise errors.PoCError(
            errors.WORKER_FAILED,
            f"worker did not produce a result.json (exit={outcome.exit_code}); "
            f"logs: {outcome.log_dir}",
            {"exit_code": outcome.exit_code, "log_dir": str(outcome.log_dir)},
        )
    status = result.get("status")
    if status != "completed" or outcome.exit_code != 0:
        error = result.get("error") or result.get("message") or "unknown worker error"
        raise errors.PoCError(
            errors.WORKER_FAILED,
            f"worker reported status={status!r} exit={outcome.exit_code}: {error}",
            {
                "worker_status": status,
                "worker_error": error,
                "log_dir": str(outcome.log_dir),
            },
        )
    return result
