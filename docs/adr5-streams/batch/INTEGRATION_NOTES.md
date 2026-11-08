# INTEGRATION NOTES — GLM-3 batch executor → будущий интегратор

Статус: предложения. Ничего из этого не реализовано в общих модулях и не
требует реализации без отдельного решения владельца (ADR 0005 §12, gate
закрыт). Контракт GLM-1 на момент работы отсутствовал — все DTO provisional.

## 1. Adapter seam для общих контрактов (GLM-1)

Точки, где provisional DTO заменяются утверждённой спецификацией:

| Provisional (здесь) | Будущий контракт | Seam |
|---|---|---|
| `michelangelo.batch-plan/provisional-0` (`BatchPlan`, `PlanItem`, `WorkerSpec`) | `ResolvedBatchPlan` из ADR §5.2 | `plan::BatchPlan::load/validate/to_canonical_json` — единственное место парсинга; CLI принимает файл, executor работает только с типом. |
| `michelangelo.batch-report/provisional-0` (`BatchReport`) | машинный stdout-формат будущих `batch run/status/resume` | `report::BatchReport::build/to_json`; таксономия исходов (`built/awaiting_review/validation_failed/execution_failed/blocked/cancelled`) уже различна и готова к маппингу. |
| `WorkerInvocation`/`WorkerResult` + exit-коды 0/3/4/5/21/70 | операции существующего Blender Worker / будущие typed ops | `worker::run_worker` и spawn в `executor.rs` (Command::new(current_exe)) — заменяется на реальный адаптер без изменения durable-логики. |
| exit codes CLI 0/1/2/3/4/70 | точные exit semantics будущих CLI-команд (ADR §7.1) | `exit_codes` в `lib.rs`, одно место. |

## 2. Точки интеграции с существующим Michelangelo

1. **Persisted attempt lifecycle → Workspace storage.** Схема `mb_*` —
   отдельная disposable БД. Для production предлагается миграция с теми же
   таблицами в Workspace SQLite и теми же CAS-семантиками (ADR §7.2.1:
   «persisted attempt lifecycle принадлежит workspace storage»). Существующий
   `jobs` scheduler остаётся как есть; batch attempt может порождать дочерний
   job (execution record), но не обязан в PoC.
2. **CLI-роутинг.** Команды `mbatch run/resume/retry/status/cancel` — PoC.
   Интеграция — новые подкоманды в michelangelo_cli без изменения старого
   транспорта (ADR §7.1). Не менять `wait=false` семантику jobs: здесь её нет.
3. **Canonical accept.** Терминальное `awaiting_review` — это граница потока.
   Дальше должен вызываться общий canonical accept/reject (IT-8D инварианты);
   в PoC приёмки нет, и ни один артефакт не публикуется за пределы private
   attempt-директории.
4. **Resource admission.** Файловый слот (limit 1) реализует «один тяжёлый
   Blender процесс» (ADR §7.3) без отдельного сервера. При будущем расширении:
   N слотов = N lock-файлов `heavy_worker.slot.<i>.lock`; RAM/VRAM-телеметрия —
   отдельная задача, сюда не включена.
5. **Blender worker.** Замена fake worker: реальный адаптер должен (а) держать
   attempt.lock, (б) писать result.json + артефакты только в staging, (в)
   соблюдать exit-протокол. `--factory-startup`, allowlist ops и confinement —
   по ADR §10, вне этого PoC.
6. **Crash fencing для реального дерева процессов.** Windows: создавать worker
   внутри Job Object с `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` и наследуемым
   handle, чтобы cancel гарантированно убивал всё дерево и PID reuse не
   влиял. `taskkill /T /F` корректен для дерева без внуков (fake worker).
7. **Event log.** `mb_events` (append-only) — кандидата на объединение с
   EventEnvelope-потоком jobs (маппинг kind → типизированные события); при
   интеграции согласовать с GLM-1 единый формат envelope.

## 3. Предлагаемые негативные проверки интегратору (Round 3)

- CAS-арбитраж при одновременном `cancel` и финализации attempt (здесь
  покрыто частично: attempt_already_finalized_by_other).
- Отказ SQLite (busy_timeout > 5s, блокировка файла антивирусом) — поведение
  честного отказа, а не silent pass.
- Two runners + cancel одновременно (интерливинг трёх процессов).
- Подмена входного файла между `pin_inputs` и первым запуском (здесь покрыто
  resume-проверкой; между этими фазами — окно, закрываемое только тем, что
  fingerprint вычисляется до первой попытки).
- Большие планы (30 items) — производительность topo/CLI вывода не
  профилировалась.

## 4. Что НЕ предлагается менять

- Существующий in-process Scheduler, его очередь и события (используются как
  есть для будущих дочерних job records).
- Workspace migrations и глобальную БД в этой смене.
- IT-8D accept/eval, Canonical Publisher.
- Корневой Cargo workspace: `michelangelo_batch` — standalone; при слиянии
  интегратор решает, добавлять ли crate в members (потребует правки корневого
  `Cargo.toml` — отдельное согласованное изменение).
