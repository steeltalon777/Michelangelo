# ADR 0005 — Ownership / Adapter Matrix (GLM-1, draft)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Статус:** DRAFT. Таблица сопоставляет новые контракты ADR 0005 с фактически существующими
  подсистемами Michelangelo. Формулировки «переиспользовать/миграция/запрещено» — правила для
  GLM-2..6 на раунд 1; изменения общих файлов оформляются только как предложения
  (`INTEGRATION_NOTES.md`), не правками.

## 1. Легенда

- **REUSE** — можно вызывать/читать напрямую из изолированного модуля потока без изменений общих файлов.
- **ADAPT** — требуется новый изолированный адаптер/модуль, который читает существующие документы/данные, но не меняет их формат.
- **MIGRATE** — требует нового target/таблицы/метода; реализация = отдельное интеграционное изменение после контракта (предложение в INTEGRATION_NOTES).
- **FORBIDDEN** — прямое использование/вызов в новых путях запрещено этим gate.

## 2. Сопоставление по подсистемам

| Существующая часть (b861547) | ADR 0005 контракт | Режим | Правила для потоков |
|---|---|---|---|
| JSONL stdio transport (`docs/specs/CORE_PROTOCOL.md`) | Управление CLI-first (§7.1) | REUSE (транспорт), MIGRATE (методы) | Новые методы `asset.*`, `family.*`, `batch.*` в общий dispatcher **не добавлять**; имена — draft из CONTRACT_GATE §8. Envelope/exit codes согласовать отдельно. Старые методы не менять. |
| Core project/workspace (`crates/michelangelo_workspace`) | Output namespaces, private staging (§7.2) | REUSE (layout), MIGRATE (резервирование) | Резервирование batch/item/attempt каталогов — в новой подсистеме; атомарность через create-dir-no-overwrite. Не менять корневой Cargo.toml. |
| SQLite `projects`/`jobs` (`storage.rs`) | Durable attempt lifecycle (§7.2.1) | REUSE (jobs как дочерние execution records), MIGRATE (новые таблицы) | ALTER/CREATE общих таблиц запрещён. Новые таблицы (asset_registry, recipes, plans, attempts) — только предложение миграции. BatchReport ссылается на job_id, не заменяет его. |
| Scheduler in-process, concurrency 1 (`michelangelo_jobs`) | Исполнение попыток (§7.2) | REUSE (identity-safe dequeue), FORBIDDEN (как общий admission) | Scheduler — средство исполнения одного процесса; межпроцессный лимит RAM/VRAM им не обеспечивается. `wait=false` — не batch. |
| Blender adapter + 18 safe ops (`docs/specs/BLENDER_WORKER.md`) | Registered templates (§6), Variant Generator | REUSE (существующие ops), FORBIDDEN (расширение ops без gate) | GLM-5: PoC templates строятся **только** на существующих 18 ops. Новые ops/drivers/GN — отдельный proposal. `save_blend` ≠ безопасный intake произвольного `.blend`. |
| Artifact verification (workspace-relative, типы blend/glb/png) | Path containment всех новых документов | REUSE (прецедент и правила) | Паттерн `relPath` в schemas/adr5 унаследован от `input_package_v0`; нарушение → `PATH_ESCAPE`. |
| `michelangelo.input-package/0` + views/cameras | SourceSnapshot (новый документ) | ADAPT | Input-package остаётся входом reconstruction-ветки. SourceSnapshot — sibling-документ, не расширение v0; общий envelope-стиль (strict schema, resourcePath, sha256) сохранён. |
| Geometry snapshot / evaluation / accept gate (IT-8D) | Canonical accept/reject (§5.3, §8.2) | REUSE (инварианты), FORBIDDEN (переиспользование `edit accept` для asset bundles) | `edit accept` остаётся приёмкой reconstruction-геометрии. Для CandidateAssetBundle нужен отдельный canonical publisher (вопрос владельца OP-1). Делать `edit accept` «универсальным» запрещено. |
| Evaluation enum `pass/fail/unavailable` (`michelangelo_eval`) | ValidationReport (§5) | ADAPT | Новый отчёт использует `pass/fail/unavailable/not_required` в своей схеме. `not_required` не вносится в существующий enum; `unavailable` не превращается в `pass`. |
| Verification state (current.json, run.lock, bundle verification) | Authoritative pointer + no-clobber (§4) | REUSE (модель), MIGRATE (новый pointer на asset identity) | Один pointer на identity; библиотечный индекс/batch summary/job status указателями не являются. CAS через `expected_current_revision`. |
| Unknowns/binding/resolution (`UNKNOWNS.md`, IT-8D D2) | AssetRevision.unknowns (§2.2) | REUSE (контракт), ADAPT | Приёмка геометрии и закрытие unknown раздельны; скопированный unknown ID не переносит подтверждение. Evidence references переносятся в новый bundle hash-pinned. |
| Assistant MCP bounded tasks (`ASSISTANT_MCP.md`) | V4 agent proposals (§4) | REUSE (граница), FORBIDDEN (новые полномочия) | Deny-by-default apply сохраняется. Агент предлагает в bounded task; регистрация — доверенная Core-side сторона. V4 в v0.1 не реализуется. |
| Reconstruction pipeline (input/recon/eval) | Сценарий A/C: без принудительной реконструкции | REUSE (параллельная ветка) | Reconstruction — отдельный producer, не внутренний шаг Intake. Сценарий A обязан работать без invocation реконструктора (ADR §11.4). |

## 3. Явные запреты для всех потоков (сводка)

1. Не изменять: корневой `Cargo.toml`/`Cargo.lock`, `michelangelo_core` dispatcher, общую SQLite-схему, существующие `docs/specs/**` и `docs/adr/**`, действующий IT-8D accept/eval, публичный CLI.
2. Не вызывать `edit accept`/`edit reject` для CandidateAssetBundle; не выдавать локальные private DTO за утверждённый wire protocol.
3. Не объявлять `.blend`-intake, FBX, UE-ready, production-ready реализованными.
4. Не создавать competing authoritative pointers (library index, batch summary, job status).
5. Не расширять scope/protected/capabilities через recipe/request/agent поля.
6. Не запускать тяжёлые параллельные Blender-процессы «для проверки» на целевой машине: тяжёлый runtime — вне параллельного раунда (ADR §12).

## 4. Что нужно от интегратора (раунд 3)

1. Создать canonical publication path для rich asset bundle (OP-1) — или явно отложить этап 1.
2. Зарезервировать CLI-пространство `asset.*`, `family.*`, `batch.*` (draft имена CONTRACT_GATE §8) без реализации.
3. Принять решение по OP-2 (`.blend` intake) — от него зависит входной набор GLM-2.
4. Определить миграцию storage: новые таблицы/документы asset registry (предложение — INTEGRATION_NOTES.md §2).
