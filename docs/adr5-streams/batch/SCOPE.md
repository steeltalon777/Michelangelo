# SCOPE — GLM-3 / Durable Batch Executor (stream C, ADR 0005)

Дата: 2026-11-08 (fix-раунд: 2026-11-09). База: `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
Ветка: `feat/adr5-batch` (worktree `I:/prog/michelangelo-adr5-batch`). Push/merge не выполнялись.

## Статус

Implemented (PoC, fake worker first). Все заявленные свойства покрыты
прогоняемыми тестами, включая двухпроцессные и crash-инъекции. GPU/Blender
smoke не запускался и не требуется (fake worker). Ничего здесь не является
production-ready, UE-ready или безопасным запуском недоверенных файлов.

Fix-раунд (по DSH_REVIEW_WAVE1.md §5, findings F1/F2/F3/F4/F5) меняет
поведение в четырёх местах — см. §«Изменения fix-раунда» ниже.

## Фактические пути владения (единоличные)

| Путь | Назначение |
|---|---|
| `michelangelo_batch/` | Новый standalone crate: `michelangelo_batch` (lib) + bin `mbatch`. Собственный `Cargo.toml` с пустой секцией `[workspace]` — crate НЕ входит в корневый workspace (корневой `Cargo.toml`/`Cargo.lock` не изменялись). |
| `michelangelo_batch/src/` | `lib.rs`, `plan.rs`, `store.rs`, `locks.rs`, `executor.rs`, `worker.rs`, `report.rs`, `timeutil.rs`, `bin/mbatch.rs` |
| `michelangelo_batch/tests/` | `common/mod.rs`, `two_process.rs`, `crash_recovery.rs`, `policies.rs`, `plan_guards.rs` |
| `docs/adr5-streams/batch/` | Этот SCOPE, TEST_EVIDENCE.md, INTEGRATION_NOTES.md |

Disposable state в рантайме: тесты используют `tempfile::TempDir` (OS TEMP);
ручные прогоны — `michelangelo_batch/target/tmp/**` (git-ignored). SQLite-база
создаётся по `--db <path>` и никогда не совпадает с базой Workspace проекта.

## Что реализовано

1. **Immutable resolved plan identity.** `plan_id` задан или выведен из
   SHA-256 канонического JSON плана; повторная регистрация того же payload
   идемпотентна, другой payload под тем же id отклоняется
   (`StoreError::PlanImmutable`). Входы пинруются: абсолютные пути + фактические
   SHA-256 файлов (`pin_inputs`) при первой регистрации. **Fingerprint и
   `plan_id` вычисляются по пиннутой форме**, то есть идентичность плана
   покрывает фактические входы, а не только объявленные пути. Объявленный
   входом `sha256` — контракт: расхождение с фактическими байтами →
   `PlanError::InputHashMismatch` (exit 4), объявленное значение никогда не
   перезаписывается наблюдаемым.
2. **Durable state в отдельной disposable SQLite.** Таблицы `mb_batches`,
   `mb_items`, `mb_attempts`, `mb_events` (append-only event log). WAL +
   busy_timeout; все переходы — CAS `UPDATE ... WHERE status IN (...)`, поэтому
   runner/cancel/status из разных процессов арбитрируются без двойных записей.
3. **Атомарное резервирование attempt-директорий.** `<runs>/<item_id>/attempt_<NNN>`
   создаётся `create_dir` (AlreadyExists → конфликт); crash между mkdir и DB
   insert обрабатывается честно: старая директория переименовывается в
   `attempt_NNN.orphan-<token>` (без overwrite содержимого), резервируется
   новая. Каждый retry — новый attempt_no и новая директория. Идентификатор
   item'а — строгий: первый символ `[A-Za-z0-9]`, символы `[A-Za-z0-9._-]`,
   `.`/`..` запрещены, поэтому `attempt_dir` физически не может выйти за
   пределы runs root.
4. **Cross-process admission тяжёлого worker (лимит 1).** Файловая блокировка
   `heavy_worker.slot.lock` через `std::fs::File::try_lock` (LockFileEx на
   Windows), освобождается ядром при смерти владельца — crash-safe, без
   in-memory mutex и без отдельного Resource Manager сервера. Занятый слот →
   честный отказ (`SlotBusy`, exit 3), второй worker не стартует.
5. **Fencing против поздних/осиротевших attempt.** Каждый attempt держит
   `attempt.lock` всё время жизни worker; финализация возможна только CAS
   `running → terminal` в БД — проигравший арбитраж не публикует ничего.
   Проверка «жив ли владелец» = попытка захвата OS-лока (не возраст записи).
   Cancel убивает pid только если лок реально удерживается (PID reuse не
   приводит к убийству постороннего процесса по одной записи в БД).
6. **Retry-политика.** Авто-retry только для transient/crash/timeout, bounded
   (per-cycle budget = `max_attempts`, backoff base*2^n с cap). Invalid input,
   validation fail — перманентные вердикты без повторов; IO/storage failure
   останавливает запуск новых items (успехи ранее выполненных сохраняются).
7. **Resume/Retry.** Reconciliation выполняется **во всех режимах**
   (`run`/`resume`/`retry`): фактические SHA-256 входов сверяются до
   запуска чего бы то ни было; drift → item `blocked(input_drift)`, не
   выполняется; непосредственно перед claim входы конкретного item'а
   пересчитываются ещё раз (свежая проверка на момент claim). Успешные items
   никогда не сбрасываются и не перезапускаются; прерванные владельцы (мёртвые
   по OS-локу) помечаются `interrupted`. `retry --items` даёт новый
   bounded-цикл выбранным items; `validation_failed` повторам не подлежит.
   Items, оставленные упавшим runner'ом в `running` без живой attempt-строки,
   подметаются в `execution_failed` (иначе пачка висела бы вечно: claim
   разрешён только из `pending/execution_failed/cancelled/blocked`, cancel —
   только из `pending/blocked`). Исход уже завершённого attempt'а
   переносится на item, а не выбрасывается.
8. **Cancel.** Отдельный процесс `cancel`: убивает process tree живого worker
   (`taskkill /T /F`), ждёт освобождения лока, CAS-переводит attempt/item в
   `cancelled`; pending/blocked → `cancelled` (зависимые стадии не стартуют).
   Успешные исходы не откатываются. Статус доступен во время исполнения
   (`status` в отдельном процессе, `live_attempt.owner_pid` в отчёте).
9. **Различные доменные исходы.** `built`, `awaiting_review`,
   `validation_failed`, `execution_failed`, `blocked`, `cancelled` (+ оси
   исполнения `pending/running`). «Файл существует» никогда не равен success:
   worker crash после записи артефакта, но без result.json даёт
   `failed_transient`, артефакт не публикуется.
10. **Fake worker как отдельный процесс.** `mbatch worker` получает spec-файл,
    держит attempt.lock, пишет только в свою attempt-директорию (staging),
    протокол exit-кодов 0/3/4/5/21/70. К БД worker не обращается.

## Явные границы PoC (не заявлено и не сделано)

- `heavy_workers > 1` отклоняется валидацией плана: сериализация тяжёлой
  работы — единственный реализованный admission.
- Kill process tree: `taskkill /T /F`; для реального Blender-дерева с
  внуками нужен Windows Job Object (см. INTEGRATION_NOTES.md).
- Randomness/owner_token — pid+time+RandomState, не криптография; hashes и
  advisory locks не защита от владельца ОС.
- Real RAM/VRAM admission, GPU, Blender, canonical accept/reject (IT-8D),
  Asset Library — вне зоны потока.
- Контракт GLM-1 отсутствовал: DTO (`BatchPlan`, `WorkerInvocation`,
  `WorkerResult`, `BatchReport`) — private provisional c маркерами
  `provisional-0` и adapter seam (см. INTEGRATION_NOTES.md).

## Изменения fix-раунда (2026-11-09, findings DSH_REVIEW_WAVE1 §5)

| Finding | Что изменено | Где |
|---|---|---|
| F1 (HIGH) | Claim item (`running`) и вставка attempt-строки выполняются **одной транзакцией** → состояние «running без attempt'а» не представимо. Дополнительно reconcile подметает такие items в `execution_failed` и переносит исход завершённого attempt'а на item. Добавлена инъекция краша в окне claim→insert (`--inject-crash-before-insert`) и 2 теста | `src/store.rs` (`claim_item_with_attempt`), `src/executor.rs` (`reconcile_state`), `src/bin/mbatch.rs`, `tests/crash_recovery.rs` |
| F2 (HIGH) | `pin_inputs` проверяет объявленный `sha256` → `PlanError::InputHashMismatch`; fingerprint и `plan_id` считаются по пиннутой форме. 2 теста | `src/plan.rs`, `src/executor.rs` (`load_or_register`), `tests/plan_guards.rs` |
| F3 (MEDIUM) | Reconciliation выполняется во всех режимах; входы item'а пересчитываются непосредственно перед claim. 1 тест | `src/executor.rs`, `tests/plan_guards.rs` |
| F4 (MEDIUM, gate) | `cargo fmt --check` → exit 0, `cargo clippy --all-targets -- -D warnings` → exit 0; фактические результаты в TEST_EVIDENCE.md | весь крейт |
| F5 (MEDIUM) | `valid_item_id`: явный запрет `.`/`..`, первый символ `[A-Za-z0-9]`. 1 интеграционный + 2 unit-теста | `src/plan.rs`, `tests/plan_guards.rs` |

Новый файл: `michelangelo_batch/tests/plan_guards.rs`. Прочие каталоги репозитория
не тронуты.

## Не изменённые (запрещённые) файлы

Корневые `Cargo.toml`, `Cargo.lock`; `crates/michelangelo_jobs`,
`crates/michelangelo_workspace`, `michelangelo_cli`, core, eval/edit/evidence,
migrations, CLI-роутинг — не тронуты. `git status` перед коммитом это
подтверждает.

## Допущения

- Standalone crate с пустой секцией `[workspace]` — согласованное обходное
  решение запрета на изменение корневого Cargo.toml; фактический путь
  зафиксирован здесь.
- Правило AGENTS.md «работать только на dev» относится к общему чекауту; для
  этого параллельного dispatch действуют инструкции ADR-0005 dispatch: работа
  в выделенном worktree и коммит в `feat/adr5-batch` (staging только явными
  pathspec).
- Тесты используют `env!("CARGO_BIN_EXE_mbatch")` и реальный спавн процессов;
  Windows-специфика: `tasklist` для pid-liveness, `taskkill` для tree-kill.
