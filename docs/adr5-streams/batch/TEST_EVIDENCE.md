# TEST EVIDENCE — GLM-3 Durable Batch Executor (fake worker)

Дата: 2026-11-08. Стенд: Windows 10.0.26100 x64, Git Bash, rustc/cargo 1.96.0.
Crate: `michelangelo_batch` (standalone), bin `mbatch`. База среза: b861547.
GPU/Blender smoke не выполнялся и не заявляется — worker фейковый по заданию.

## 1. Команды

```
cd michelangelo_batch
CARGO_NET_OFFLINE=true cargo test          # весь набор (unit + integration)
CARGO_NET_OFFLINE=true cargo build         # ноль warnings
```

Сборка standalone: корневой workspace не используется (пустая секция
`[workspace]` в `michelangelo_batch/Cargo.toml`); зависимости взяты из
локального кэша cargo (offline): rusqlite 0.31 (bundled), serde/serde_json,
sha2 0.10, tempfile (dev).

## 2. Итог `cargo test` (финальный прогон)

| Suite | Результат |
|---|---|
| unittests src/lib.rs (unit: plan/store/locks) | **ok. 13 passed; 0 failed** (0.12s) |
| unittests src/bin/mbatch.rs | ok. 0 passed (нет unit-тестов в bin) |
| tests/crash_recovery.rs | **ok. 4 passed; 0 failed** (10.22s) |
| tests/policies.rs | **ok. 7 passed; 0 failed** (0.59s) |
| tests/two_process.rs | **ok. 4 passed; 0 failed** (9.10s) |

Итого 28 тестов, 0 упавших, 0 warnings. Дополнительно: 10 последовательных
прогонов `two_process + crash_recovery + policies` — 10/10 без сбоев (после
устранения двух гонок, см. §6).

## 3. Покрытие свойств задания

| Требование | Тест(ы) | Что зафиксировано |
|---|---|---|
| Immutable plan id / item ids | `plan_identity_is_immutable_and_registration_idempotent`, unit `register_is_idempotent...` | повторная регистрация с тем же payload — идемпотентна; мутированный payload под тем же plan_id отклонён (exit 4, «immutable») |
| Persist batch/item/attempt переходы (отдельная disposable SQLite) | все интеграционные тесты; unit store | таблицы `mb_batches/mb_items/mb_attempts/mb_events`, CAS-переходы, WAL |
| Atomic reservation attempt-директорий, без overwrite; retry = новый attempt | `runner_crash_after_reserve_recovers_with_new_attempt`, `transient_budget_exhausted_then_explicit_retry_gets_new_attempts` | после crash директория attempt_001 сохранена нетронутой; артефакт только в attempt_002; 6 attempt-директорий при 2 циклах по 3 попытки |
| Успех одной задачи не откатывается чужой ошибкой | `permanent_failure_no_retry_dependent_blocked_independent_success_kept`, `io_failure_stops_launches_and_keeps_successes` | `execution_failed` у одного item, `blocked` у зависимого, `awaiting_review` + артефакт у независимого сохранены |
| Retry только transient, bounded attempts/backoff | `transient_retry_bounded_and_recovers`, `transient_budget_exhausted_then_explicit_retry_gets_new_attempts`, `worker_crash_before_finalize_retries_and_never_promotes_old_artifact` | 2 transient-провала → успех на 3-й; исчерпание бюджета ровно после max_attempts; crash артефакта не публикуется (attempt failed_transient, без result.json) |
| Invalid input / quality fail без бесконечных повторов | `permanent_failure_no_retry...` (attempts=1), `validation_failed_is_distinct_and_not_retryable` (attempts=1, retry отклонён) | перманентные вердикты не ретраятся |
| Resume сверяет fingerprints и commit state; старое accepted не сбрасывается | `resume_verifies_input_fingerprints_and_success_is_stable` | подмена байтов входа → `blocked(input_drift)`, попыток нет; успешный item остаётся awaiting_review с 1 attempt |
| Cross-process ownership/admission (лимит 1), не in-memory mutex | `heavy_worker_slot_is_exclusive_across_processes`, `two_concurrent_runners_execute_each_item_exactly_once` | OS файл-лок: занятый слот во время исполнения подтверждён из второго процесса; второй runner — честный отказ; 4 items × 4 параллельных runner-процесса — ровно 1 attempt на item, по 1 директории |
| Fencing против позднего старого attempt | unit CAS (`attempt_reservation_and_transitions`), `attempt_already_finalized_by_other`-путь в cancel-тесте | двойная финализация невозможна: второй CAS running→terminal = 0 строк |
| Cancel: остановка process tree, запрет зависимых стадий | `cancel_from_second_process_stops_worker_and_dependents` | worker-pid мёртв после cancel; зависимый item cancelled с 0 попыток; batch cancelled; явный resume завершает пачку (2 попытки у первого item, директории не переиспользованы) |
| Crash/restart без двойной записи и без второго worker при живом первом | `orphan_worker_blocks_second_worker_until_cancel` | убийство ТОЛЬКО runner: осиротевший worker держит OS-лок; resume честно предупреждает «appears live» и не создаёт второй attempt; cancel убивает осиротевший pid (проверка живости через лок); resume завершает пачку |
| Crash injection around reservation/finalize | `runner_crash_after_reserve...` (exit 70 после резерва), `worker_crash_before_finalize...` (exit 70 worker после записи артефакта) | оба случая восстановлены, дубликатов нет |
| Delayed finish, статус во время исполнения | `delayed_finish_status_from_second_process` | второй процесс видит `live_attempt {attempt_no, state, owner_pid}` пока worker жив |
| Различные исходы, не «файл есть = accepted» | `validation_failed_is_distinct_and_not_retryable`, mixed-smoke §4, `event_log_records_lifecycle` | built / awaiting_review / validation_failed / execution_failed / blocked / cancelled — разные статусы; event log полный |
| Idempotency | `plan_identity...` (re-run без новых попыток) | повторный run: те же 1 attempt на item |
| Disk full / IO error | `fail_io` в `io_failure_stops_launches_and_keeps_successes` | симуляция IO-отказа worker: запуск новых items остановлен (batch failed, предупреждение в отчёте), успехи целы |

## 4. Ручной fake-worker batch smoke (машиночитаемый отчёт)

Сценарий 4 items (`a-flaky` transient×1→успех, `b-invalid` permanent,
`c-dependent` от b-invalid, `d-review` успех):

```json
{"batch_status": "completed",
 "counts": {"items_total": 4, "awaiting_review": 2, "execution_failed": 1,
            "blocked": 1, "attempts_total": 2},
 "items": [{"id": "a-flaky", "status": "awaiting_review", "attempts": 2},
           {"id": "b-invalid", "status": "execution_failed", "attempts": 1},
           {"id": "c-dependent", "status": "blocked", "attempts": 0},
           {"id": "d-review", "status": "awaiting_review", "attempts": 1}]}
```
Exit code процесса: 2 (PARTIAL_FAILURE). Проверка цепочки `w1→w2`:
зависимости, отдельные исходы (awaiting_review vs built), sha256 артефактов в
result.json.

## 5. Двухпроцессная демонстрация lifecycle (обжатый вывод)

```
--- status DURING execution (second process) ---
{"batch": "running",
 "w1": {"item_id": "w1", "status": "running", "attempts_used": 1,
        "live_attempt": {"attempt_no": 1, "state": "running", "owner_pid": 11400}},
 "w2_attempts": 0}
--- cancel (third process) ---
{"batch": "cancelled", "w1": "cancelled", "w2": "cancelled"}
runner EXIT=2
attempt dirs of w1: attempt_001            <- сохранён, не переиспользован
--- resume (explicit) ---
{"batch": "completed", "w1": "awaiting_review", "w1_attempts": 2,
 "w2": "built", "w2_attempts": 1}          EXIT=0
```

## 6. Найденные и устранённые в ходе прогона дефекты

1. Флак «slot busy»-теста: при параллельной нагрузке worker успевал завершиться
   до проверки → увеличены интервалы симуляции (9s) и бюджет ожидания в тестах.
2. Флак `status` под параллельной нагрузкой: `init_schema` брал write-лок на
   каждое открытие БД → busy_timeout исчерпывался. Исправлено: схема
   создаётся только при отсутствии таблиц (read-проба sqlite_master), плюс
   тестовый poll_status переживает transient-отказы status в рамках таймаута.
3. Retry-бюджет считался от общего attempt_no → после исчерпания явный retry
   делал только 1 попытку. Исправлено: бюджет на цикл исполнения
   (fixed at first claim of each run/resume/retry invocation).

## 7. Честные пометки

- SKIPPED/BLOCKED: GPU smoke, Blender smoke, RAM/VRAM admission, canonical
  accept, multi-worker (>1) — не реализованы и не помечены как PASS.
- Восстановление «crash между mkdir и insert» покрыто кодом
  (`orphan_attempt_dir_renamed`), отдельного теста-инъекции именно этого окна
  нет (проверено unit-логикой ветки AlreadyExists); помечено для Round 3.
- `pid_alive` в тестах использует `tasklist` (Windows), с повторной пробой.
