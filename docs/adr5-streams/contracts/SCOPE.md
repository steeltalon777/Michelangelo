# SCOPE — GLM-1 Contracts lane (ADR 0005 parallel dispatch)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Worktree:** `I:/prog/michelangelo-adr5-contracts`, ветка `feat/adr5-contracts` (создана и checked out до работ).
- **Роль:** Contract Lead. Контрактный gate для GLM-2..6. Production-код Core не тронут.

## 1. Фактическая зона владения (все файлы созданы этой сменой)

- `docs/adr5-contracts/CONTRACT_GATE.md` — границы 13 контрактов (v0.1 vs deferred), canonical digest, authority boundaries, порты, реестр кодов/check_id, решения владельца (OP-1..OP-7).
- `docs/adr5-contracts/OWNERSHIP_MATRIX.md` — adapter/ownership mapping на существующий Michelangelo (REUSE/ADAPT/MIGRATE/FORBIDDEN).
- `docs/adr5-contracts/MIGRATION_BOUNDARIES.md` — неизменяемое, новые targets, блокирующие решения §13, правило конфликта.
- `docs/adr5-contracts/VALIDATION_MATRIX.md` — применимость проверок по профилям, потери форматов, сохранение инвариантов IT-8D.
- `docs/adr5-contracts/NEGATIVE_TEST_MATRIX.md` — F1–F20: schema-фикстуры + behavioral-требования к потокам.
- `schemas/adr5/README.md` — правила кодировки/digest/sort, реестр схем.
- `schemas/adr5/*.schema.json` — 13 draft-схем `draft-0.1`.
- `schemas/adr5/examples/positive/*.json` — 13 минимальных валидных экземпляров.
- `schemas/adr5/examples/negative/*.json` — 15 fixtures, обязанные отвергаться схемой (fix-раунд 2026-11-09: +5 — held-out per-item ×2, `not_required` без `applicability_ref`, `tool_versions="latest"`, bundle без CAS-объявления).
- `docs/adr5-streams/contracts/SCOPE.md` (этот файл), `HANDOFF.md`, `INTEGRATION_NOTES.md`.
- `docs/adr5-streams/contracts/tools/check_schemas.py` — исполняемая schema-проверка.

Вне зоны ничего не изменено. Dispatch-документы из Downloads в репозиторий не копировались.

## 2. Вход

- ADR 0005 (read-only, из dispatch-архива; в базовом commit b861547 отсутствует).
- ADR 0004, specs: CORE_PROTOCOL, BLENDER_WORKER, EVALUATION, VERIFICATION_STATE, INPUT_PACKAGE, UNKNOWNS, LOCAL_EDIT, ASSISTANT_MCP; TZ IT-8D §12; исходники: `michelangelo_workspace/storage.rs`, `michelangelo_jobs/lib.rs`, `michelangelo_edit/scene.rs`, `michelangelo_eval/assemble.rs`, `michelangelo_cli/main.rs`; схема `input_package_v0.schema.json`.

## 3. Выход

Contract gate: draft-схемы + матрицы + handoff для GLM-2..6 (HANDOFF.md) + предложения интегратору (INTEGRATION_NOTES.md). Commit в ветке `feat/adr5-contracts` только из зоны владения. Никаких merge/push.

## 4. Допущения

1. Имена контрактов и schema-теги `michelangelo.<document>/draft-0.1` — предложения; финальные теги утверждает владелец (после принятия суффикс `draft-` снимается).
2. Rust-сигнатуры портов — иллюстративные; физическое размещение модулей определяет будущее ТЗ (ADR §5.1).
3. Canonical JSON = RFC 8785 для новых документов — открытое решение владельца (OP-3); legacy digest-правила не затронуты.
4. Паттерн `relPath` унаследован без изменений от `input_package_v0.schema.json`.

## 5. Блокеры / открытые пункты

- ADR 0005 отсутствует в базовом commit — читался из dispatch-архива (дано заданием).
- OP-1..OP-7 (CONTRACT_GATE §9) и B1–B4 (MIGRATION_BOUNDARIES §3) не решены — потоки не должны считать их утверждёнными.
- Проверки производительности/Blender smoke в этой смене не выполнялись (docs+schemas lane; тяжёлый runtime вне параллельного раунда, ADR §12).

## 6. Статус реализации (честно)

- Schema/meta-schema check: **PASS** (13/13 meta, 13/13 positive, 15/15 negative — факт в §7).
- Behavioral negative tests (F1–F5, F9, F17–F20): **NOT IMPLEMENTED** — обязательны для GLM-2..6 (см. NEGATIVE_TEST_MATRIX.md §3); здесь не заявляются пройденными.
- Rust код в этом потоке не создавался и не собирался (не требуется для docs+schemas зоны).

## 7. Выполненные проверки (факты)

```text
$ python docs/adr5-streams/contracts/tools/check_schemas.py
OK meta   <13 schema files>          -> каждый валиден по draft 2020-12 meta-schema
OK pos    <13 positive examples>     -> все валидны
OK neg    <15 negative fixtures>     -> каждая отвергнута по своему правилу (NEGATIVE_EXPECTED_PATH)
RESULT: all schema/example checks passed   (exit=0, 41 OK-строк)
```

Fix-раунд 2026-11-09 (DSH review §3, findings A–E): per-item held-out/if-then в `source_set`,
per-check `not_required`/`applicability_ref` в `validation_report`, запрет `latest` в
`tool_versions` (`recipe_lock`), условно-обязательный CAS (`candidate_bundle` при
`base_ref.kind=asset_revision`), 64-hex digest'ы во всех негативных фикстурах и таблица
fixture → expected path в `check_schemas.py`.

Повторяемо локально: `python docs/adr5-streams/contracts/tools/check_schemas.py`
(требуется python + `jsonschema>=4.18`; фактически использован jsonschema 4.26.0, Python 3.13.15).
