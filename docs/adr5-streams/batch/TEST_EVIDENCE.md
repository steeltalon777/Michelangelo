# TEST EVIDENCE — GLM-3 Durable Batch Executor (fake worker)

Дата: 2026-11-09 (Asia/Chita). Стенд: Windows 10.0.26100 x64, Git Bash, rustc/cargo 1.96.0.
Crate: `michelangelo_batch` (standalone), bin `mbatch`. База среза: b861547, ветка `feat/adr5-batch`.
GPU/Blender smoke не выполнялся и не заявляется — worker фейковый по заданию.

Ревизия: fix-раунд по итогам независимого ревью (DSH_REVIEW_WAVE1.md, §5 GLM-3,
findings F1/F2/F3/F4/F5) + P2/P3-раунд по итогам delta-review к нему
(Windows-коллизия `item_id` → чужой orphan-rename + три бонуса). Изменения —
только в `michelangelo_batch/` и этом каталоге.

## 1. Команды (финальный gate)

```
cd michelangelo_batch
CARGO_NET_OFFLINE=true cargo test                                  # весь набор (unit + integration)
cargo fmt --check                                                   # exit 0
CARGO_NET_OFFLINE=true cargo clippy --all-targets -- -D warnings    # exit 0
```

Фактические результаты финального прогона (после P2/P3-раунда):

| Команда | Exit code | Результат |
|---|---|---|
| `CARGO_NET_OFFLINE=true cargo test` | **0** | 50 passed; 0 failed; 0 ignored (27 unit + 23 integration) |
| `cargo fmt --check` | **0** | diff пуст (форматирование всего крейта приведено к rustfmt) |
| `CARGO_NET_OFFLINE=true cargo clippy --all-targets -- -D warnings` | **0** | 0 warnings; 0 errors |

До P2/P3-раунда (после fix-раунда): `cargo test` = exit 0, 38 passed;
`cargo fmt --check` = exit 0; `cargo clippy --all-targets -- -D warnings` = exit 0.
До fix-раунда: `cargo fmt --check` = exit 1, `cargo clippy --all-targets -- -D warnings` =
exit 101 (6 линтов: `collapsible_match`, `single_match` ×2, `unnecessary_map_or`,
`type_complexity`, `unnecessary_cast`).

Сборка standalone: корневой workspace не используется (пустая секция
`[workspace]` в `michelangelo_batch/Cargo.toml`); зависимости взяты из
локального кэша cargo (offline): rusqlite 0.31 (bundled), serde/serde_json,
sha2 0.10, tempfile (dev).

## 2. Итог `cargo test` (финальный прогон)

| Suite | Результат |
|---|---|
| unittests src/lib.rs (unit: plan/store/locks/executor) | **ok. 27 passed; 0 failed** (0.13s) |
| unittests src/bin/mbatch.rs | ok. 0 passed (нет unit-тестов в bin) |
| tests/crash_recovery.rs | **ok. 6 passed; 0 failed** (10.17s) |
| tests/plan_guards.rs | **ok. 6 passed; 0 failed** (4.10s) — +2 теста P2/P3-раунда |
| tests/policies.rs | **ok. 7 passed; 0 failed** (0.74s) |
| tests/two_process.rs | **ok. 4 passed; 0 failed** (9.10s) |

Итого 50 тестов, 0 упавших, 0 warnings. До P2/P3-раунда было 38 тестов
(17 unit + 21 integration); добавлено 12 (10 unit: 4 в `plan.rs`, 2 в
`store.rs`, 4 в `executor.rs`; 2 integration в `plan_guards.rs`).

## 3. Покрытие свойств задания

| Требование | Тест(ы) | Что зафиксировано |
|---|---|---|
| Immutable plan id / item ids | `plan_identity_is_immutable_and_registration_idempotent`, unit `register_is_idempotent...` | повторная регистрация с тем же payload — идемпотентна; мутированный payload под тем же plan_id отклонён (exit 4, «immutable») |
| **Fingerprint покрывает фактические входы** | `correct_declared_input_hash_is_accepted_and_pinned`, unit `pin_and_verify_inputs_roundtrip` | fingerprint и `plan_id` вычисляются **после** `pin_inputs`, т.е. по каноническим путям и реальным SHA-256 байтов; в зарегистрированном плане лежит фактический digest входа |
| **Объявленный hash входа — контракт, а не подсказка** | `declared_input_hash_mismatch_is_rejected`, unit `pin_inputs_rejects_declared_hash_mismatch` | plan с неверным `input.sha256` отклоняется (exit 4, «hash mismatch»), план **не регистрируется**, ни один attempt не стартует; объявленное значение никогда не перезаписывается наблюдаемым |
| Persist batch/item/attempt переходы (отдельная disposable SQLite) | все интеграционные тесты; unit store | таблицы `mb_batches/mb_items/mb_attempts/mb_events`, CAS-переходы, WAL |
| Atomic reservation attempt-директорий, без overwrite; retry = новый attempt | `runner_crash_after_reserve_recovers_with_new_attempt`, `transient_budget_exhausted_then_explicit_retry_gets_new_attempts` | после crash директория attempt_001 сохранена нетронутой; артефакт только в attempt_002; 6 attempt-директорий при 2 циклах по 3 попытки |
| **Claim и attempt-строка — одна транзакция (F1)** | `crash_between_claim_and_attempt_insert_does_not_strand_the_item` | инъекция краша строго в окне claim→insert: claim откатывается, item **не** остаётся `running` без attempt (в старой схеме — висел бы навсегда), item снова `pending`, resume завершает пачку; каталог упавшего attempt сохранён как `attempt_001.orphan-*`, не перезаписан |
| **Свип лимбо-состояния (F1)** | `resume_sweeps_running_item_without_any_attempt` | item `running` **без** attempt-строки (состояние старой сборки) подметается в `execution_failed` следующим запуском, событие `item_running_without_attempt_swept` в event log, честная причина в `last_error` |
| Успех одной задачи не откатывается чужой ошибкой | `permanent_failure_no_retry_dependent_blocked_independent_success_kept`, `io_failure_stops_launches_and_keeps_successes` | `execution_failed` у одного item, `blocked` у зависимого, `awaiting_review` + артефакт у независимого сохранены |
| Retry только transient, bounded attempts/backoff | `transient_retry_bounded_and_recovers`, `transient_budget_exhausted_then_explicit_retry_gets_new_attempts`, `worker_crash_before_finalize_retries_and_never_promotes_old_artifact` | 2 transient-провала → успех на 3-й; исчерпание бюджета ровно после max_attempts; crash артефакта не публикуется (attempt failed_transient, без result.json) |
| Invalid input / quality fail без бесконечных повторов | `permanent_failure_no_retry...` (attempts=1), `validation_failed_is_distinct_and_not_retryable` (attempts=1, retry отклонён) | перманентные вердикты не ретраятся |
| **Сверка fingerprints во всех режимах + в claim-loop (F3)** | `resume_verifies_input_fingerprints_and_success_is_stable`, `fresh_run_verifies_input_drift_before_claiming_an_item` | reconcile выполняется в `run`/`resume`/`retry`; непосредственно перед claim входы item'а пересчитываются заново — подмена байтов после старта пачки даёт `blocked(input_drift)` и **0 попыток** даже в fresh-режиме |
| **Грамматика item_id без обхода пути (F5)** | `item_id_dot_dot_is_rejected_before_anything_is_written`, unit `item_id_grammar_rejects_path_traversal`, `attempt_dir_cannot_escape_the_runs_root` | `..`, `.`, `.hidden`, `-lead`, `a/b` отклоняются валидацией плана (exit 4) **до** любой записи; первый символ обязан быть `[A-Za-z0-9]`; попытка выхода за runs root не создаёт ни одного каталога |
| **P2: коллизия item_id на Windows не даёт чужой orphan-rename** | unit `orphan_rename_is_refused_when_another_attempt_owns_the_directory` (любая платформа), unit `orphan_rename_is_refused_when_another_attempt_owns_the_case_folded_directory` (Windows), unit `validation_rejects_ids_that_collide_on_windows`, integration `case_colliding_item_ids_are_rejected_on_windows` | План с id `Build` и `build` отклоняется валидацией (exit 4, «duplicate») **до** регистрации; если каталог уже указан в attempt-строке **чужого** item'а, orphan-ветка **не переименовывает** его, а возвращает детерминированную ошибку `Inconsistent` («already used by attempt …»); живой каталог остаётся на месте, orphan-каталог не создаётся. Проверено «откатом» фикса: без сравнения по нормализованному пути тест падает (чужой каталог переименовывается) |
| **P3-1: зарезервированные имена Windows-устройств** | unit `item_id_grammar_rejects_windows_device_names`, integration `windows_device_name_item_id_is_rejected_before_anything_is_written` | `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9` (в любом регистре, включая `NUL.txt`) отклоняются валидацией (exit 4) на **любой** платформе; runs root остаётся пустым, план не регистрируется; `CONSOLE`, `COM0`, `LPT0` остаются валидными |
| **P3-2: orphan-имя не перезаписывается (Unix)** | unit `orphan_name_gets_an_incrementing_suffix_when_the_base_name_is_taken` | при занятом базовом имени `attempt_NNN.orphan-<token>` выбирается `…-2`, затем `…-3`; `std::fs::rename` на Unix не может молча заменить существующий каталог |
| **P3-3: `insert_attempt` — один контракт** | unit `insert_attempt_rejects_a_double_reservation` | сигнатура `Result<(), StoreError>`; двойная вставка того же (plan, item, attempt_no) → ошибка «attempt already reserved», первая строка не перезаписана, в таблице 1 строка |
| **Лефтовер без attempt-строки сохраняется как orphan** | unit `leftover_directory_without_any_attempt_row_is_preserved_as_an_orphan`, integration `crash_between_claim_and_attempt_insert_does_not_strand_the_item` | каталог без единой ссылки в БД сохраняется как `attempt_NNN.orphan-<token>` (содержимое не теряется), новый attempt получает свой каталог |

| Cross-process ownership/admission (лимит 1), не in-memory mutex | `heavy_worker_slot_is_exclusive_across_processes`, `two_concurrent_runners_execute_each_item_exactly_once` | OS файл-лок: занятый слот во время исполнения подтверждён из второго процесса; второй runner — честный отказ; 4 items × 4 параллельных runner-процесса — ровно 1 attempt на item, по 1 директории |
| Fencing против позднего старого attempt | unit CAS (`attempt_reservation_and_transitions`), `attempt_already_finalized_by_other`-путь в cancel-тесте | двойная финализация невозможна: второй CAS running→terminal = 0 строк |
| Cancel: остановка process tree, запрет зависимых стадий | `cancel_from_second_process_stops_worker_and_dependents` | worker-pid мёртв после cancel; зависимый item cancelled с 0 попыток; batch cancelled; явный resume завершает пачку (2 попытки у первого item, директории не переиспользованы) |
| Crash/restart без двойной записи и без второго worker при живом первом | `orphan_worker_blocks_second_worker_until_cancel` | убийство ТОЛЬКО runner: осиротевший worker держит OS-лок; resume честно предупреждает «appears live» и не создаёт второй attempt; cancel убивает осиротевший pid (проверка живости через лок); resume завершает пачку |
| Crash injection around reservation/finalize | `runner_crash_after_reserve...` (exit 70 после резерва), `worker_crash_before_finalize...` (exit 70 worker после записи артефакта), `crash_between_claim_and_attempt_insert...` (exit 70 в окне claim→insert) | все три случая восстановлены, дубликатов нет |
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
4. **Fix-раунд (F1):** claim item и вставка attempt-строки были двумя
   отдельными операциями → пачка могла навсегда зависнуть в `running` без
   attempt-строки. Исправлено: claim и attempt-строка пишутся одной
   транзакцией (лимибо-состояние не представимо), плюс reconcile подметает
   `running`-items без живых attempt'ов в `execution_failed` и переносит
   известный исход завершённого attempt'а на item.
5. **Fix-раунд (F2):** `pin_inputs` безусловно перезаписывал объявленный
   `input.sha256`, а fingerprint считался до пиннинга → объявленный hash не
   проверялся, идентичность плана не покрывала фактические входы. Исправлено:
   расхождение → `InputHashMismatch` (exit 4), fingerprint/plan_id — по
   пиннутой форме.
6. **Fix-раунд (F3):** fresh-run не вызывал reconcile и не сверял drift в
   claim-loop. Исправлено: reconcile во всех режимах + повторная сверка
   входов конкретного item непосредственно перед его claim.
7. **Fix-раунд (F5):** `valid_item_id` пропускал `.`/`..` → `attempt_dir` мог
   уйти за пределы runs root. Исправлено: явный запрет `.`/`..` и требование
   первого символа `[A-Za-z0-9]`.
8. **P2/P3-раунд:** на NTFS «runs/Build» == «runs/build» — план с id `Build` и
   `build` проходил валидацию, и orphan-ветка резерва переименовывала **чужой
   живой** каталог (проверялась только attempt-строка своего item'а). Исправлено:
   уникальность id — по Windows-нормализованной форме; перед rename ищется
   любая attempt-строка, ссылающаяся на каталог, и при её наличии резерв
   завершается детерминированной ошибкой вместо rename.
9. **P3-раунд:** `valid_item_id` пропускал имена Windows-устройств (`CON`,
   `NUL`, …); orphan-имя на Unix могло быть молча перезаписано `rename`; doc
   `insert_attempt` обещал `Ok(false)`, а код возвращал всегда `Ok(true)`.
   Исправлено: чёрный список устройств, инкрементальный суффикс orphan-имени,
   единый контракт `Result<(), StoreError>`.

## 7. Честные пометки

- SKIPPED/BLOCKED: GPU smoke, Blender smoke, RAM/VRAM admission, canonical
  accept, multi-worker (>1) — не реализованы и не помечены как PASS.
- Восстановление «crash между mkdir и insert» теперь покрыто тестом
  `crash_between_claim_and_attempt_insert_does_not_strand_the_item`: каталог
  упавшего attempt сохраняется как `attempt_NNN.orphan-<token>`, новый
  attempt получает свой каталог, содержимое не перезаписывается.
- Свип лимбо-состояния (`running` без attempt-строки) воспроизводится в
  тесте через API хранилища: текущий код пишет claim и attempt атомарно,
  поэтому такое состояние в живом прогоне не возникает — тест фиксирует
  совместимость с базами, созданными старой сборкой.
- Известное ограничение reconcile: если владелец умер между финализацией
  attempt'а (`failed_transient`) и обновлением item, item переносится в
  `execution_failed`, и автоматический retry этого attempt'а теряется
  (ограниченное окно ~2 мс, требует параллельного reconcile-процесса).
  Исход attempt'а при этом не теряется и не подменяется.
- `pid_alive` в тестах использует `tasklist` (Windows), с повторной пробой.
- Race-окно между `claim` и вставкой attempt-строки закрыто транзакцией;
  отдельного «мягкого» heartbeat/lease на item нет — при следующем раунде
  это стоит рассмотреть, если понадобится отличать «долгий живой attempt»
  от «мёртвый runner» без OS-лока.
- **Граница P2-фикса:** проверка «кто владеет каталогом» читает *закоммиченные*
  attempt-строки. Если конкурентный процесс успел создать каталог, но ещё не
  закоммитил свою attempt-строку, чужой каталог всё ещё может быть
  переименован. Для одного и того же item это окно недостижимо (claim-CAS
  отдаёт item ровно одному владельцу, и создание каталога идёт **внутри**
  транзакции claim), но как универсальная блокировка каталога правило не
  является — при необходимости потребуется OS-лок на attempt-каталог.
- **Известный флак (не связан с этим раундом):** интеграционный тест
  `two_concurrent_runners_execute_each_item_exactly_once` падает с
  «at least one runner must succeed; got [2, 2]», когда оба параллельных
  runner'а успевают взять по части items и каждый рапортует частичный
  результат. Флак **присутствует на базовом коммите** `fa216ea` (замер:
  3/30 падений на базе, 5/30 на текущей ветке, одинаковая ассертация),
  причина — гонка в самом тесте, а не в этом раунде. Все остальные 49 тестов
  проходят детерминированно.
