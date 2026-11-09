# HANDOFF — контракты GLM-1 → GLM-2..6

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556`.
- **Статус:** DRAFT (`draft-0.1`). Ничего из перечисленного не существует как DTO/CLI/миграций в коде.
  Раунд 2 (после контракта): привести локальные draft-DTO к одной спецификации, без merge.

## 0. Общие правила для всех потоков

1. Читай: `docs/adr5-contracts/CONTRACT_GATE.md` (границы §2, digest §3, authority §4, коды §5,
   check_id §6, порты §7), `schemas/adr5/README.md` (canonical правила), свою строку в
   `OWNERSHIP_MATRIX.md`, свою часть `NEGATIVE_TEST_MATRIX.md`.
2. Все wire-примеры помечай draft; локальные private DTO не выдавай за утверждённый протокол.
3. Изменение общих файлов запрещено; предложение — в свой `docs/adr5-streams/<lane>/INTEGRATION_NOTES.md`.
4. Свежие schema-файлы и примеры: `schemas/adr5/**`. Проверка схем доступна всем:
   `python docs/adr5-streams/contracts/tools/check_schemas.py`.
5. Нерешённые OP-1..OP-7 и B1–B4 не считать утверждёнными.

## 1. GLM-2 — Asset Intake (`feat/adr5-intake`)

**Получает:**
- `schemas/adr5/source_snapshot.schema.json` — `michelangelo.asset-source-snapshot/draft-0.1` (главный документ потока).
- `schemas/adr5/preparation_plan.schema.json` — `michelangelo.preparation-plan/draft-0.1`.
- Порт: `intake.snapshot(input) -> (SourceSnapshot, IntakeReport)` — CONTRACT_GATE §7.1.
- Негативы: F1 (tamper), F3 (missing dependency), F13 (path escape), F15 (формат вне набора).
- Правила: входной набор v0.1 = `glb/gltf/obj/stl/ply` (`.blend` — BLOCKED на B2, fbx — не поддержан);
  hashes по фактическим bytes; missing/unsupported зависимости фиксируются, не выбрасываются;
  оригиналы неизменны.

## 2. GLM-3 — Batch Executor (`feat/adr5-batch`)

**Получает:**
- `schemas/adr5/batch_plan.schema.json` — `michelangelo.batch-plan/draft-0.1` (вход потока).
- `schemas/adr5/batch_report.schema.json` — `michelangelo.batch-report/draft-0.1` (выход потока; доменные статусы — вопрос владельца OP-6).
- `schemas/adr5/variant_request.schema.json` — как входной документ items.
- Порты: `batch.execute/resume/cancel` — CONTRACT_GATE §7.2.
- Негативы: F4 (stale base/CAS), F11 (циклы), F18 (capability), F20 (crash/retry/fencing).
- Правила: план immutable до исполнения; retry = новая attempt, inputs readonly; отказ item не
  валит независимые items; Scheduler — не общий admission; публикация/приёмка — не в этом потоке.

## 3. GLM-4 — Family Builder / Variant Generator (`feat/adr5-family`)

**Получает:**
- `schemas/adr5/source_set.schema.json` — `michelangelo.source-set/draft-0.1` (6 ролей — OP-5; held-out правило зашито в схему).
- `schemas/adr5/family_recipe.schema.json` — `michelangelo.family-recipe/draft-0.1` (главный документ потока; V0/V1 kinds только).
- `schemas/adr5/recipe_lock.schema.json` — `michelangelo.recipe-lock/draft-0.1`.
- `schemas/adr5/variant_request.schema.json` — производит и валидирует.
- Порты: `family.validate/plan`, `variant.generate` — CONTRACT_GATE §7.3.
- Негативы: F6 (роль), F7 (scope widening), F9 (эффективное изменение защищённого), F10 (arbitrary code), F12 (floating latest), F17 (параметры).
- Правила: рецепт описывает допустимое множество; params типизированы и полные в request;
  seed derivation от стабильной identity; исполняемая ревизия immutable.

## 4. GLM-5 — Blender adapter / registered templates (`feat/adr5-blender`)

**Получает:**
- `schemas/adr5/candidate_bundle.schema.json` — `michelangelo.candidate-asset-bundle/draft-0.1` (форма выхода в staging; статус `proposed` — константа).
- Порты: `template.execute(bindings, capabilities, limits)` — CONTRACT_GATE §7.4; artifact set и content_kind из схем `asset_revision`/`candidate_bundle`.
- Негативы: F9 (эффективное изменение защищённого через разрешённую зависимость), F10 (код), F13 (path escape), F18 (capability).
- Правила: PoC только на существующих 18 ops (SAFE_WORKER_OPS); template = зарегистрированный,
  не автозапуск `.blend`; `--factory-startup`, копия входа, выход в private staging; artifact
  verification не ослаблять; новый op/GN — только proposal в INTEGRATION_NOTES.

## 5. GLM-6 — Validation (`feat/adr5-validation`)

**Получает:**
- `schemas/adr5/validation_profile.schema.json` — `michelangelo.validation-profile/draft-0.1` (реестр check_id — CONTRACT_GATE §6, OP-4).
- `schemas/adr5/validation_report.schema.json` — `michelangelo.validation-report/draft-0.1` (главный документ потока).
- `schemas/adr5/acceptance_record.schema.json` — `michelangelo.acceptance-record/draft-0.1` (решения отдельно от измерений).
- `schemas/adr5/candidate_bundle.schema.json` + `asset_revision.schema.json` — документы-потребители.
- Матрица применимости: `docs/adr5-contracts/VALIDATION_MATRIX.md` §1; IT-8D наследование — §3.
- Порты: `validation.evaluate/verify` — CONTRACT_GATE §7.5.
- Негативы: F2 (input changed), F5 (replay), F8 (unknown required check), F14 (acceptance без отчёта), F16 (`not_required`/unavailable→pass), F19 (stale approval).
- Правила: отчёты immutable/no-clobber, reservation до вычислений, тяжёлая работа вне lock;
  `not_required` не вносится в IT-8 enum; IT-8D accept/eval реконструкции не изменяется.

## 6. Что остаётся у GLM-1 (раунд 2)

- Разбор расхождений локальных draft-DTO потоков со схемами `schemas/adr5/**` и единая правка схем.
- Арбитраж конфликтов имён/полей; приём предложений изменений check_id/ролей/статусов как новых ревизий draft.

## 7. Для DeepSeek (независимый reviewer) — приоритет проверки

1. `CONTRACT_GATE.md` §3–4: достаточен ли анти-маскинг (asset_digest + scene_layout/rig как артефакты + dependency closure) против «материал/трансформ/риг прячутся за geometry hash» — контрпримеры приветствуются (fixture F9).
2. `CONTRACT_GATE.md` §5–6: корректность свёртки verdict и реестра check_id; не маскирует ли `not_required` будущие `unavailable`.
3. `source_set.schema.json`: held-out-инвариант (allOf/contains) — реально ли ловит сочетание `independent_validation_reference` + `usage != held_out`.
4. `family_recipe.schema.json`: условная валидация параметров (min/max vs values) — не слишком ли слаба; совместимость с ADR §6.
5. `candidate_bundle.schema.json` + `acceptance_record.schema.json`: CAS/no-clobber семантика `expected_current_revision`; нет ли обхода однопутности публикации.
6. `schemas/adr5/examples/negative/`: покрыть дополнительные негативы (например, дубликат artifact_id — сейчас behavioral).
7. Canonical JSON RFC 8785 vs legacy кодировщик (OP-3) — подтвердить/отклонить.
