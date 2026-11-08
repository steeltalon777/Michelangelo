# INTEGRATION NOTES — GLM-1 contracts lane

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556`.
- **Статус:** предложения для интегратора (раунд 3). Ничего из этого не реализовано и не должно
  реализовываться до контракта/разрешения владельца. Этот поток не менял общих файлов — расхождений
  с реальным кодом нет; ниже — точки расширения, которые потребуются интеграции.

## 1. CLI / Core dispatcher (предложение, не реализовано)

Зарезервировать (не реализовывая) пространство команд для rich asset пути; draft-имена из ADR §7.1
воспроизведены в `CONTRACT_GATE.md` §8. Точки расширения:

- `crates/michelangelo_cli/src/main.rs`: новые варианты `Commands` (`asset`, `family`, `batch`) —
  только после согласования envelope/exit codes. Старый транспорт JSONL не меняется.
- Точное имя команды приёмки rich asset target согласовать отдельно; `edit accept` не переиспользуется.

## 2. Workspace storage / SQLite (предложение миграции, не реализовано)

Новые таблицы (пример — не миграция):

```sql
-- asset identity и authoritative pointer (одна строка на identity)
CREATE TABLE IF NOT EXISTS asset_identities (
    asset_id          TEXT PRIMARY KEY,
    current_revision  INTEGER,          -- NULL = not yet published (no-clobber create)
    bundle_sha256     TEXT,
    updated_at        TEXT NOT NULL
);
-- immutable реестры документов по образцу evaluations/index.json
CREATE TABLE IF NOT EXISTS asset_documents (
    doc_kind    TEXT NOT NULL,          -- source_snapshot | recipe | lock | plan | bundle | report | acceptance
    doc_id      TEXT NOT NULL,
    revision    INTEGER NOT NULL,
    path        TEXT NOT NULL,
    sha256      TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (doc_kind, doc_id, revision)
);
```

Альтернатива — file-based registry по образцу verification state (без SQLite-миграции вообще).
Решение за интегратором; оба варианта сохраняют «SQLite job status — не competing pointer».

## 3. Blender Worker (предложение, не реализовано)

- Новые typed ops для registered templates (например, `apply_template_bindings`) — отдельный
  review/registration; PoC GLM-5 обязан уложиться в существующие 18 ops.
- Безопасный `.blend` intake (B2): копия + `--factory-startup` + отключение drivers/addons +
  allowlist capabilities + лимиты — при положительном решении владельца; confinement как отдельный
  implementation gate (ADR §10).

## 4. Jobs / Scheduler

- Batch attempts используют существующие persisted jobs как дочерние записи (`BatchReport.items[].attempts[].job_id`).
- Межпроцессный admission (RAM/VRAM) — вне in-process Scheduler; вариант: файловый ресурсный
  счётчик/lockfile в workspace. Решение B3.

## 5. Canonical JSON

- Для новых документов предлагается RFC 8785 (OP-3). Если владелец выберет legacy-подход
  (фиксированный порядок полей кодировщиком) — схемы не меняются, меняется только правило
  пересчёта digest в `schemas/adr5/README.md` §2 и CONTRACT_GATE §3.2.

## 6. Известные ограничения draft

1. Уникальность `artifact_id` внутри bundle/revision — behavioral-проверка (валидатор), не схема;
   fixture для схемы (duplicates) — кандидат на следующую ревизию (приоритет DeepSeek, HANDOFF §7.6).
2. `parameters` в `variant_request` не связаны со списком параметров рецепта на уровне схемы
   (меж-документная связь) — обязанность валидатора GLM-4 (`PARAMETER_INVALID`).
3. `resolved_parameters` в `batch_plan` — свободный объект: связь с рецептом тоже behavioral.
4. Контент-агностичные enum (`content_kind`, роли, статусы) — закрытые наборы v0.1; расширение —
   новые ревизии схем (OP-4..OP-7).

## 7. Что не предлагается

- Не предлагается миграция существующих reconstruction runs в новый asset registry.
- Не предлагается расширение `edit accept`/evaluation enum.
- Не предлагается сетевое/провайдерное API, внешний catalog-сервер, vector DB (ADR §2, §3).
