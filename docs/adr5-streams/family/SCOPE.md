# SCOPE — GLM-4 Family Builder / Pure Planner V0–V1

- **Поток:** GLM-4 (`feat/adr5-family`), worktree `I:/prog/michelangelo-adr5-family`, база `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Статус:** реализация V0–V1 pure planning завершена в пределах своей зоны; исполняемая часть (Blender, публикация) отсутствует by design.
- **Все DTO и порты: PROVISIONAL** до контракта GLM-1 (`feat/adr5-contracts`, `schemas/adr5`) и раунда 2. Никакой wire-формат не считается утверждённым.
- **Собственный namespace тегов (fix-раунд, finding L):** документы потока несут **свои** provisional-теги `michelangelo.family.<doc>/draft-0` и явное поле `provisional: true`. Теги GLM-1 (`michelangelo.<doc>/draft-0.1`) **не используются и отвергаются парсером**: документы потока схемам GLM-1 не соответствуют, и потребитель, диспетчеризующий по тегу, не должен принять их за GLM-1-артефакты. Теги provisional — до раунда 2.

## 1. Фактические файлы в зоне владения (новые, ничего из корня не изменено)

```
crates/michelangelo_family/            standalone crate (свой Cargo.toml, [workspace], zero deps)
  Cargo.toml                           пустая [workspace] секция; crate ВНЕ корневого workspace
  Cargo.lock                           локальный, корневой Cargo.lock не тронут
  src/lib.rs                           корень модуля, все экспорты помечены provisional
  src/sha256.rs                        чистый SHA-256 (FIPS 180-4), без зависимостей
  src/json.rs                          минимальный JSON (парсер + canonical writer)
  src/errors.rs                        типизированные коды отказов (FamilyErrorCode)
  src/model.rs                         provisional DTO: SourceSet, SourceRole, FamilyRecipe,
                                       RecipeLock, VariantRequest (+ParamSpec/ParamValue,
                                       Scope, Generator, Constraint, HeightModel, SocketProfile)
  src/wire.rs                          строгие wire-адаптеры (additionalProperties:false аналог)
  src/plan.rs                          планировщик: seed derivation, комбинационный движок,
                                       ResolvedBatchPlan
  src/port.rs                          provisional BatchExecutorPort + NullBatchExecutor (отказ)
  src/fixture.rs                       зафиксированный тестовый дом "urban-house-family"
  src/bin/family_dry_plan_15.rs        dry-plan demo (regen fixtures / plan 15 requests)
  fixtures/source_set.json             wire-образные fixture-документы с реальными digest
  fixtures/family_recipe.json
  fixtures/recipe_lock.json
  tests/unit_tests.rs                  позитивные unit-тесты
  tests/negative_tests.rs              негативы F6/F7/F9/F10/F12/F17 и остальные
docs/adr5-streams/family/
  SCOPE.md                             этот документ
  TEST_EVIDENCE.md                     команды и фактические результаты
  INTEGRATION_NOTES.md                 маппинг на GLM-1 (черновик), порты, расхождения
  examples/dry_plan_15.json            dry plan на 15 уникальных requests (canonical JSON)
```

Зона владения не пересекается ни с одним существующим файлом репозитория. Корневые `Cargo.toml`/`Cargo.lock`, Core/CLI/Worker/DB — не изменялись (проверяется `git status`).

## 2. Вход / выход потока

- **Вход:** `SourceSet` + `FamilyRecipe` + `RecipeLock` + `VariantRequest[]` (provisional private DTO; wire-документы собственного тега `michelangelo.family.<doc>/draft-0` с маркером `provisional: true` читаются `wire.rs`).
- **Выход:** диагностика рецепта (`FamilyError` с типизированными кодами) и immutable `ResolvedBatchPlan` (canonical JSON). План показывает, что БУДЕТ создано; ничего не создаёт.
- **Точка связи с Batch Executor (GLM-3):** `port::BatchExecutorPort::accept_plan(&ResolvedBatchPlan) -> PlanAck`; дефолт `NullBatchExecutor` отказывает с `NOT_PLANNABLE` — чистая граница pure planning.

## 3. Реализовано (V0–V1)

- Single- и multi-source `SourceSet`: 6 закрытых ролей, usage (`build_input`/`reference_only`/`held_out`), donor `allowed_parts` (нет grant — нет права), held-out отделён от generator inputs.
- `FamilyRecipe` описывает **допустимое множество**: typed parameters (integer/enum/boolean; number — только как спека, не энумерируем), bounds, явная incompatibility matrix, socket requirements, height model + `maximum_dimension`, combinatorial cap. Generator kinds только V0/V1; V2–V4 — `CAPABILITY_DENIED`.
- `RecipeLock`: точные hash зависимостей (включая manifests каждого источника — stale donor детектируется), точные версии инструментов (floating `latest`/`*`/`^`/`~`/`>`/`<` — отказ), pinned PRNG алгоритм+версия; digest по canonical JSON.
- `VariantRequest`: полный типизированный набор значений; незвестный/лишний/вне диапазона/не того типа параметр — отказ. По конструкции не содержит scope/capability/validation полей; wire-инъекция таких полей = `SCOPE_WIDENING`.
- Seed derivation `family-seed-v1` (точный алгоритм зафиксирован в `plan.rs` doc): SHA-256(prefix||0x00||lock_digest||0x00||item_id), первые 8 байт BE, маска 53 бита (JSON-safe). Зависит только от lock digest + стабильного item_id.
- Dry plan: комбинационный движок (canonical order, odometer), stride-sampling N уникальных допустимых комбинаций; при нехватке — честная ошибка `INSUFFICIENT_UNIQUE_COMBINATIONS`, не фальшивые варианты.
- **Own tag namespace + явный маркер:** все пять документов потока (`source_set`, `family_recipe`, `recipe_lock`, `variant_request`, `batch_plan`) несут приватные теги `michelangelo.family.<doc>/draft-0` и поле `provisional: true`. Wire-парсер отвергает GLM-1-теги `michelangelo.<doc>/draft-0.1`, документ без маркера и документ с `provisional: false` (все три — `SCHEMA_INVALID`). Единый источник тегов — константы `WIRE_SCHEMA_*` в `model.rs`; фикстуры и пример генерируются кодом (`family_dry_plan_15 --regen` / прогон планировщика), руками не правятся.

## 4. Не реализовано (честно, не PASS)

- **variant.generate — SKIPPED/NOT_PLANNABLE:** исполнение вариантов (Blender) вне pure planning; порт принимает план и отказывает.
- **F9 в полном объёме (эффективное изменение защищённого через parent transform/shared material/modifier/GN):** реализован только planning-аналог `PROTECTED_ROLE_TARGET` (binding на protected role). Полный F9 — зона GLM-5/GLM-6 evidence.
- **Wire-совместимость с GLM-1:** частичная (см. INTEGRATION_NOTES.md §2); мои схемы полей differ (например `height_model`/`sockets` — приватные расширения, их нет в GLM-1 schema). Не утверждено. Документы несут собственные provisional-теги `michelangelo.family.<doc>/draft-0` + `provisional: true`, поэтому валидировать их по схемам GLM-1 нельзя (и не нужно: это не GLM-1-артефакты).
- **Number-параметры:** не энумерируются (dense); fixture использует integer/enum. Float-хеширование — открытый вопрос (INTEGRATION_NOTES.md §5).
- **Creature/ship профили (§11.2/11.3):** только analogue-проверки (FOREIGN_SKELETON, sockets); pinned clips/hardpoints/keep-out — не реализовано.

## 5. Допущения

- Нет доступа к общему workspace/SQLite —(plan работает на in-memory DTO; persistence — зона GLM-3/интегратора).
- `dependency hash` источников в fixture — runtime-хеши fixture-контента, не реальных payload файлов (Intake-поток даст фактические SHA-256).
- `admission_profile: single_heavy_process_v0` в плане — константная пометка из ADR §7.3, не измеренная гарантия.

## 6. Блокеры / открытые пункты

1. Контрактный gate GLM-1 — round 2 сведёт мои private DTO со `schemas/adr5` (расхождения в INTEGRATION_NOTES.md). До раунда 2 документы остаются provisional: теги `michelangelo.family.*/draft-0` + `provisional: true` (см. INTEGRATION_NOTES.md §1a).
2. `RecipeLock` fixture-хеши зависимостей — до Intake фиксируются программно; в интеграции должны приходить из `SourceSnapshot`.
3. Раунд 2: определить, кто владеет seed derivation (я реализовал `family-seed-v1`; Batch/GLM-3 должен переиспользовать ту же функцию).
