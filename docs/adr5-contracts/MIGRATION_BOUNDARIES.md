# ADR 0005 — Migration Boundaries (GLM-1, draft)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Статус:** DRAFT. Документ фиксирует границы миграции: что должно остаться неизменным, что
  требует новых targets, и какие блокирующие решения владельца (ADR 0005 §13) влияют на потоки.
  Сам документ миграций не выполняет.

## 1. Неизменяемое (backward compatibility)

| Сущность | Правило |
|---|---|
| `michelangelo.input-package/0` | Схема и семантика не меняются; reconstruction-ветка продолжает работать как сейчас |
| `michelangelo.geometry-snapshot/0` | Не расширяется полями family/asset-bundle |
| `michelangelo.evaluation-report/0` | Enum результатов не расширяется (`not_required` живёт только в новом отчёте) |
| Accept gate IT-8D (коды, порядок проверок) | Не изменяется; новые пути не «облегчают» его |
| Verification state (current.json, run.lock, append-only history) | Инварианты сохраняются во всех новых путях |
| Существующие CLI-команды и JSONL-методы | Не переименовываются, не меняют envelope/exit codes |
| Существующие 18 Worker ops | Не расширяются этим gate; новые ops — отдельный proposal |
| SQLite `projects`/`jobs` | Существующие таблицы/столбцы не ALTER-ятся; новые данные — новые таблицы |
| Исторические reconstruction runs | Не мигрируются автоматически; читаются как раньше |

## 2. Новые targets (требуют миграционного этапа, не входят в раунд 1)

1. **Asset registry / canonical publication path** для `CandidateAssetBundle` → `accepted`
   AssetRevision (ADR §5.3): один authoritative pointer на asset identity, no-clobber create,
   CAS-обновление. Предложение структуры — `INTEGRATION_NOTES.md` §2.
2. **Durable batch lifecycle storage**: batch/item/attempt records поверх persisted jobs
   (ADR §7.2.1). In-memory Scheduler остаётся средством исполнения.
3. **CLI-команды** `asset.*`, `family.*`, `batch.*` (draft, CONTRACT_GATE §8) и имя команды
   приёмки rich target — согласовать отдельно, не меняя старый транспорт.
4. **Recipe/lock/profile реестр** (immutable документы + registry-индексы по образцу
   `evaluations/index.json`).
5. **Регистрация зарегистрированных шаблонов** (`registered_blender_template_v0`): список
   разрешённых templates с review/registration до исполнения (ADR §10).

## 3. Блокирующие решения владельца (ADR §13) и их влияние на потоки

| # | Вопрос владельца | Влияние, пока не решён |
|---|---|---|
| B1 (§13.1) | Rich bundle target и единый canonical publisher | GLM-3/GLM-6 не публикуют `accepted`; bundle остаётся `proposed`. Не обходить через Library Adapter. |
| B2 (§13.2) | Безопасный приём `.blend`/templates (confinement) | GLM-2 ограничивает вход `glb/gltf/obj/stl/ply`; GLM-5 не загружает произвольный `.blend`. `.blend` — только output. FBX не поддержан. |
| B3 (§13.3) | Lifecycle/ресурсы: persist attempts, fencing, cancel, admission | GLM-3 исполняет последовательно, один тяжёлый процесс; `wait=false` не считается batch; cross-process admission не заявляется. |
| B4 (§13.4) | Профиль качества/форматы: сохранность данных, renderer, клипы | GLM-6 фиксирует применимость профиля до генерации; недоступная обязательная проверка = `unavailable` (блокирует), не pass. |
| OP-3 | Canonical JSON (RFC 8785) для новых документов | Digest-код не пишется до резолюции; документы хранят declared digests, пересчёт — после резолюции. |
| OP-4..OP-7 | Реестры check_id / roles / статусы / `not_required` | Потоки используют состав CONTRACT_GATE §5–6 как draft; расширения — только новые ревизии схем. |

## 4. Правило конфликта (ADR §12 / dispatch README)

Если реализация потока требует изменения общего файла (core dispatcher, общая схема, существующие
specs, Worker API): **не редактировать**. Зафиксировать proposed diff или точку расширения в
`docs/adr5-streams/<lane>/INTEGRATION_NOTES.md` и продолжить частные тесты. Merge/rebase —
запрещены в раундах 1–2; интеграция — отдельный владелец после контракта.

## 5. Совместимость старых данных с новыми путями

- Reconstruction run может быть источником `SourceSnapshot` (origin_kind =
  `michelangelo_reconstruction`), но его OBJ-компоненты и evaluation не считаются приёмкой
  rich bundle: нужен новый snapshot/bundle + validation по профилю.
- Историческая accepted revision остаётся authoritative для своей identity; перенос принятого
  варианта в существующую сцену — самостоятельное интеграционное изменение с собственным
  base/scope/gate (ADR §5.3).
- Библиотечный индекс восстановим из авторитетных документов; миграция индекса не создаёт
  новых author approvals (ADR §7.2.9).
