# ADR 0005 — Contract Gate (GLM-1, draft)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Статус:** **DRAFT contract gate.** Ничего из описанного здесь wire-формата не существует в коде.
  Все схемы в `schemas/adr5/` помечены `draft-0.1` и не являются утверждённым wire-протоколом.
  Принятие каждого контракта — отдельное решение владельца (ADR 0005 §12, этап 0).
- **Владелец документа:** GLM-1 (contracts lane). Потребители: GLM-2 (intake), GLM-3 (batch),
  GLM-4 (family), GLM-5 (blender), GLM-6 (validation) — см. `docs/adr5-streams/contracts/HANDOFF.md`.
- **Граница:** этот документ и `schemas/adr5/**` ничего не меняют в Core, CLI, SQLite-схеме,
  существующих schemas, Worker ops и действующем IT-8D accept/eval.

## 1. Факты read-only аудита на b861547 (не предложения)

Зафиксировано чтением исходников и specs (пути от корня репозитория):

| Факт | Основание |
|---|---|
| Транспорт: однопоточный JSONL over stdio; методы Phase 1/2 (`system.ping`, `project.*`, `job.*`, `blender.get_capabilities`, `blender.run_job`); новых команд family/batch/asset нет | `docs/specs/CORE_PROTOCOL.md` |
| Scheduler: in-process, FIFO, max concurrency 1, один running slot, identity-safe dequeue; не является межпроцессным ограничителем | `crates/michelangelo_jobs/src/lib.rs` |
| SQLite: только таблицы `projects`, `jobs`; нет asset/family/batch/attempt таблиц | `crates/michelangelo_workspace/src/storage.rs` |
| Worker: ровно 18 safe ops; import `.glb/.gltf/.obj/.stl/.ply`; `save_blend` (сохранение) есть, безопасного приёма произвольного `.blend` нет; export `.glb`; рендер только Workbench | `docs/specs/BLENDER_WORKER.md` |
| Edit/eval-цепочка: `michelangelo.geometry-snapshot/0`, `michelangelo.evaluation-report/0`, verdict `pass/fail/unavailable`, `AuthorDecision` `pending/accepted/rejected`; accept gate с кодами `EVALUATION_MISSING/UNREGISTERED/STALE`, `INPUT_CHANGED`, `VISIBLE_FAILED`, `AUTHOR_CONDITION_FAILED`, `REGRESSION`, `NO_GROUND_TRUTH` | `docs/specs/EVALUATION.md`, `crates/michelangelo_eval/src/assemble.rs` |
| Verification state: один authoritative pointer (`current.json`), immutable evidence, advisory lock `run.lock`, агрегат geometry digest = sha256 по строкам `<component>\0<sha256>\n`, отсортированным по имени | `docs/specs/VERIFICATION_STATE.md` |
| IT-8D: snapshot identity + полный diff, фактические bytes входов, reservation/immutability evaluation, полная проверка bundle перед commit, решения под lock, held-out independence (`|dot| <= 0.9`) | `docs/tz/TZ_IT8_ACCEPTANCE_INTEGRITY.md` §12 |
| Input package: `michelangelo.input-package/0`, strict `additionalProperties: false`, resourcePath-паттерн (относительный, без `..`, backslash, drive letter, NUL) | `docs/specs/schemas/input_package_v0.schema.json` |
| CLI: только `core/input/reconstruct/edit/evaluate/assistant/mcp/unknown` | `crates/michelangelo_cli/src/main.rs` |
| `SceneComponent` хранит OBJ (bbox, sha256, protected), не полный материализованный ассет | `crates/michelangelo_edit/src/scene.rs` |

Правило: при расхождении этого документа с исходниками исходники правы; сообщить GLM-1.

## 2. Границы контрактов: v0.1 минимум vs отложенное

Тринадцать документов. Каждый — отдельный immutable документ со своим `schema`-тегом.
Полные draft JSON Schema — в `schemas/adr5/`. Здесь — смысловая граница.

| # | Контракт | v0.1 (минимально необходимо) | Отложено за v0.1 |
|---|---|---|---|
| 1 | `SourceSnapshot` | snapshot_id; исходный формат из поддерживаемого набора; payload files: относительный путь + фактический sha256 + size_bytes; dependency closure (declared + scanned, каждый с digest); provenance/лицензия как зафиксированное состояние (`unknown` не превращается в разрешение); units/axes; ambiguities. Иммутабелен; изменение внешнего файла после приёма не меняет snapshot | Семантическое извлечение формата (rig/morph/node groups), лицензионная верификация как сервис, авто-классификация содержимого |
| 2 | `AssetRevision` | revision_id; asset_id; multi-artifact manifest (см. §4): artifact_id, path, sha256, size, content_kind (`geometry/material/texture/rig_definition/animation/scene_layout/node_group/linked_library/metadata/other`); stable component IDs; transforms как отдельный `scene_layout` artifact; parent lineage; validation/evidence references; unknowns. Это manifest вокруг native-данных, не сериализация Blender DNA в JSON | Полная семантическая модель рига, morph correspondence, иерархические кости как типизированная модель |
| 3 | `SourceSet` | source_set_id; revision; immutable список источников: source_id, asset_id, revision, manifest digest, roles; role = один из закрытого набора (`base_candidate`, `module_donor`, `variation_exemplar`, `style_reference`, `geometric_constraint_reference`, `independent_validation_reference`); allowed_parts/usage per source; alignment/correspondence profile ref (hash-pinned документ); conflict policy (`require_explicit_resolution`); held-out источники отделены от generator/agent inputs | Автоматический расчёт correspondences, авто-разрешение конфликтов, число источников > 16 |
| 4 | `PreparationPlan` | plan_id; ссылка на SourceSnapshot (digest); явный список разрешённых изменений (каждое: kind, target role/component, tool, параметры, before/after digests); preservation requirements (что обязано сохраниться); declared losses; tool versions; бюджеты. Отсутствие плана = нет изменений, а не «улучшить автоматически» | Автоматические repair-предложения, авто-remesh/retopo (запрещены ADR §5.1 и в v0.1) |
| 5 | `FamilyRecipe` | recipe_id; revision; source_set ref (digest); style_profile ref (hash-pinned, только ссылка в v0.1); typed parameters (integer/number/enum/boolean с диапазонами); generator: kind ∈ {`registered_blender_template_v0`, `static_variant_set_v0`} + template_ref + parameter_bindings; scope (allowed_roles, protected_roles, `allow_arbitrary_code: false` — константа); constraints (закрытый набор: `connector_compatibility`, `minimum_clearance`, `maximum_dimension`); randomness (алгоритм, seed derivation version); validation profile ref; approval policy ref; budget; outputs. Исполняемая ревизия immutable; редактирование = новая revision | V2/V3 capability-типы (modifiers, GN graphs, shape keys), StyleProfile как полный проверяемый документ, полный constraint DSL |
| 6 | `RecipeLock` | lock_id; recipe ref (digest конкретной revision); exact digests всех транзитивных зависимостей (модели SourceSet, шаблоны, style/validation/policy профили, текстуры, node groups, rigs, clips); tool versions (blender, michelangelo); PRNG algorithm + derivation version; policy/profile digests; `floating_latest_allowed: false` — константа. Floating `latest` не попадает в исполняемый план | Полная platform/render/export setting матрица пинов (v0.1 — явная карта declared settings) |
| 7 | `VariantRequest` | item_id (стабильный, независим от порядка запуска); recipe ref + recipe_lock digest; полный разрешённый набор значений параметров (все параметры рецепта, без пропусков); seed (или явная ссылка на derivation); target purpose/export. Не может расширять recipe/workspace scope — структурно: в схеме нет полей scope/capabilities/validation | Ссылки на внешние провайдеры, агентские поля |
| 8 | `ResolvedBatchPlan` | batch_id; plan_revision; immutable item list: item_id, resolved parameters (вычислены и записаны до execution), input fingerprints (digests), dependency edges между стадиями, output namespace (private, reserved), validation profile ref; limits (admission profile, budgets); результат cycle-check (`acyclic: true`, полный список корней); creator (роль, не аутентификация). План показывает, что будет создано; не утверждает формы | Параллельные workers, RAM/VRAM телеметрия в плане, межпроцессный admission (этап 1+) |
| 9 | `BatchReport` | batch_id; per-item доменный статус из закрытого набора: `built`, `validation_failed`, `awaiting_review`, `accepted`, `rejected`, `execution_failed`, `cancelled`, `blocked`; partial-failure counts; attempt references. Доменные статусы не смешиваются с execution `JobStatus` (это разные оси) | Экономика/тайминги как контрактные поля (сейчас — измерения, не контракт) |
| 10 | `CandidateAssetBundle` | candidate_id; status = `proposed` (константа в v0.1: publication в `accepted` — только canonical publisher); полный artifact set с digests фактических bytes (не только изменённая деталь); `asset_digest` (см. §4); `dependency_digests` транзитивного замыкания (linked libraries, materials, textures, node groups, rigs, clips); diff к базе; provenance chain (DAG, multi-parent); unresolved issues | Derived artifacts (LOD/baked), вторичные экспорты как часть bundle |
| 11 | `ValidationProfile` | profile_id; revision; kind ∈ {`reconstruction`, `passthrough_preservation`, `family_static`, `family_rig_compatible`, `anatomy_change`}; applicability conditions; required/optional checks: check_id из реестра §6; thresholds; evaluator versions; coverage. Применимость фиксируется до генерации; неизвестная обязательная проверка блокирует принятие | Полная библиотека rig/clip/appearance проверок, renderer-матрица |
| 12 | `ValidationReport` | report_id; bundle ref (digest); profile ref + profile digest; входные digests фактических прочитанных bytes; per-check результат: `pass` / `fail` / `unavailable` / `not_required` (закрытый набор, см. §5); измеренные значения; evidence references (hash-pinned); verdict (aggregate по тем же правилам свёртки, что IT-8: fail > unavailable > pass); `not_required` оправдан ссылкой на applicability профиля. Отчёты immutable, регистрируются no-clobber | Style-оценка как автоматический скор; LLM-вердикт как ground truth |
| 13 | `AcceptanceRecord` | record_id; решение `accepted`/`rejected`; candidate bundle digest; validation report digest + факт регистрации; decision basis (ссылки на policy/profilе); actor + role (audit attribution, не аутентификация); timestamp; scope решения (ровно один candidate digest, не «семейство целиком»). Self-reported `approved: true` или hash во входном JSON не авторизует приёмку — поля авторизации в wire-схемах кандидатов отсутствуют | Массовое подтверждение по политике (перечисление точных digests, не «всё будущее»), подписи |

## 3. Кодировка, сортировка и canonical digest

Опирается на прецедент `VERIFICATION_STATE.md` §3 (sha256 over canonical bytes, computed by Core,
never self-declared) и расширяет его на multi-artifact ассеты. Полная редакция —
`schemas/adr5/README.md`; здесь — нормативная выжимка.

1. **Байты документов:** UTF-8, без BOM, LF. Файлы immutable после регистрации (no-clobber).
2. **Canonical JSON** (для полей, чей digest вычисляется по документу): RFC 8785 (JCS) — отсортированные
   ключи, компактные разделители, shortest round-trip для чисел. Отличие от legacy-контрактов
   (там фиксируется порядок полей кодировщиком) — сознательное: новый реестр документов живёт
   в новой подсистеме и не переопределяет старые digest'ы.
3. **sha256:** lowercase hex, 64 символа; вычисляется по фактическим прочитанным bytes файла.
   Декларация в манифесте — не доказательство; Core обязан пересчитывать (правило IT-8D D1.2).
4. **`asset_digest`:** SHA-256 над UTF-8 строками
   `<artifact_id>\u0000<path>\u0000<content_kind>\u0000<sha256>\n`, отсортированными лексикографически
   (byte-wise) по `(artifact_id, path)`. Форма продолжение агрегата geometry-snapshot
   (`<component>\0<sha256>\n`), расширенная path + content_kind.
5. **`geometry_digest`** (опционален, вычислим из artifact set): тот же алгоритм, но только артефакты
   с `content_kind == "geometry"`. Он **не заменяет** `asset_digest` и не принимается вместо него.
6. **`dependency_digest`:** SHA-256 над строками `<dep_id>\u0000<revision>\u0000<sha256>\n`, отсортированными
   по `dep_id`.
7. **Сортировка:** везде byte-wise lexicographic по UTF-8 байтам. Порядок файлов в массиве
   источника не определяет приоритет (ADR §5.2).

### 3.1. Анти-маскировка: material/transform/rig не прячутся за geometry hash

Требование ADR §5.3: изменение текстуры, bind matrix или node group не может остаться невидимым
за прежним geometry hash. Механизм контракта:

1. `asset_digest` покрывает **все** артефакты, включая `content_kind`; строка с kind входит в hash —
   добавление/удаление артефакта любого вида меняет digest.
2. Transforms — отдельный артефакт `scene_layout` (component → transform mapping) со своим sha256:
   изменение трансформа меняет digest layout-артефакта и `asset_digest`, даже если geometry bytes
   прежние.
3. Rig — артефакт `rig_definition` + `skeleton_profile_digest` в RecipeLock; изменение bind pose,
   иерархии или весов меняет digest.
4. Транзитивные зависимости (materials, textures, linked libraries, node groups) входят в
   `dependency_digests` бандла; изменение любой зависимости создаёт новый lock (ADR §6) и
   инвалидирует зависимый cache/evidence (ADR §9.2).
5. Fixture `F9_EFFECTIVE_PROTECTED_CHANGE` (см. NEGATIVE_TEST_MATRIX.md): разрешённый parent
   transform / shared material / modifier / GN-зависимость косвенно меняет защищённую роль —
   приёмка обязана отказать, даже если bytes mesh защищённой роли не изменились; evidence
   охватывает эффективный результат и влияющие зависимости (ADR §11.4).
6. Проверка структуры: `validation_profile.required_checks` для каждого kind профиля обязан
   включать `asset_digest_full` (пересчёт по фактическим bytes всех артефактов), когда профиль
   применяется к CandidateAssetBundle.

## 4. Authority boundaries и идентичность

1. **Один authoritative pointer на asset/scene identity.** Указатели: bundle publication. Библиотечный
   индекс, batch summary и SQLite job status указателями не являются (ADR §5.3).
2. **`expected_current_revision` / no-clobber.** Первая публикация identity: `expected_current_revision = null`
   (создание только при отсутствии; существующая identity не перехватывается из-за совпадения ID).
   Обновление: значение = ожидаемая текущая revision; сравнение под lock перед switch (CAS-семантика
   аналогична `STALE_BASE`-проверке VERIFICATION_STATE §4).
3. **Sibling-варианты.** ancestry — DAG (multi-parent), transactional base — одна identity/одна
   ожидаемая revision. 15 sibling-кандидатов не принимаются последовательно в один изменяемый run.
4. **Стабильные ID:** `asset_id`, `component_id`, `item_id` — значения `^[a-z0-9]([a-z0-9._-]{0,62}[a-z0-9])?$`
   (как `package_id` в input-package/0), выдаются до исполнения, не зависят от порядка запуска,
   worker number или номера retry.
5. **Seed:** `per_item_seed = PRNG(recipe_lock_digest, item_id)` при `randomness.per_item_seed =
   "from_locked_recipe_and_stable_item_id"`; сохраняются также фактические выбранные параметры —
   seed не заменяет историю (ADR §6).
6. **Разделение решений и измерений.** `ValidationReport` — измерения; `AcceptanceRecord` — решение.
   `approved: true`/hash в кандидатском JSON не авторизует. Регистрация — доверенная Core-side
   сторона; Worker/агент пишет только в private staging (ADR §7.2).
7. **Scope не расширяется.** Recipe/request/агент могут сужать scope/budget, но не расширять
   защищённую область или системные полномочия (IT-8, ADR §8.2). В wire-схемах полей расширения
   нет; violations — на уровне валидатора.
8. **Приёмка геометрии и закрытие unknown раздельны** (IT-8D D2): скопированный unknown ID не
   переносит подтверждение на изменённую геометрию другого варианта.

## 5. Коды отказов и результат проверки (v0.1 реестр)

Новые коды отказов контрактного слоя (не изменяют существующие коды accept gate):

| Код | Слой | Смысл |
|---|---|---|
| `SNAPSHOT_MISMATCH` | intake/publication | пересчитанный digest snapshot'а не совпал с заявленным / полный diff не сошёлся |
| `DEPENDENCY_MISSING` | intake/batch/validation | элемент транзитивного замыкания отсутствует или нечитаем |
| `DEPENDENCY_CHANGED` | validation/publication | digest фактической зависимости не совпал с lock/bundle |
| `PATH_ESCAPE` | intake/worker | путь вне контейнера (прецедент: artifact verification в CORE_PROTOCOL) |
| `REVISION_STALE` | publication | `expected_current_revision` не совпал с текущей (CAS-отказ) |
| `NO_CLOBBER_VIOLATION` | publication | попытка перезаписать существующую identity/зарегистрированный документ |
| `ROLE_INVALID` | family | роль вне закрытого набора или не разрешена для данного источника |
| `SCOPE_WIDENING` | family/batch/assistant | request/recipe пытается расширить scope/protected/capabilities |
| `PARAMETER_INVALID` | family/batch | неизвестный параметр, выход за диапазон, неполный набор значений |
| `VALIDATION_UNKNOWN_REQUIRED` | validation | обязательная проверка вне реестра check_id |
| `VALIDATION_INPUT_CHANGED` | validation | входные bytes изменились после evaluation (аналог IT-8D `INPUT_CHANGED`) |
| `ACCEPTANCE_REPLAY` | publication | повторное использование acceptance record для другого candidate digest |
| `CAPABILITY_UNAVAILABLE` | batch | требуемая capability не объявлена (блокирует item, не весь batch) |

Результат проверки (`ValidationReport.checks[].result`): закрытый набор
`pass | fail | unavailable | not_required`. Свёртка verdict: любой `fail` → `fail`; иначе любой
`unavailable` → `unavailable`; иначе `pass`. **`not_required` — новое значение, которого нет в
IT-8 enum** (`pass/fail/unavailable`); оно допустимо только в новом отчёте и только со ссылкой
на applicability профиля; его запрещено вносить в существующий `michelangelo.evaluation-report/0`
и превращать `unavailable` в `pass` (ADR §8.1).

## 6. Реестр check_id (v0.1)

Закрытый enum в `validation_profile.schema.json`. Расширение — новая ревизия схемы, не приёмка
неизвестного id.

| check_id | Применимость (обязателен для kind) |
|---|---|
| `payload_integrity` | все |
| `dependency_closure` | все |
| `path_containment` | все |
| `asset_digest_full` | все (когда применяется к bundle) |
| `budget_geometry` | все |
| `protected_roles` | family_static, family_rig_compatible, anatomy_change |
| `connector_compatibility` | family_static, family_rig_compatible |
| `minimum_clearance` | family_static (когда объявлен constraint) |
| `maximum_dimension` | family_static (когда объявлен constraint) |
| `material_presence` | family_static, family_rig_compatible, passthrough_preservation (когда материалы объявлены) |
| `texture_integrity` | passthrough_preservation, family_static, family_rig_compatible (когда текстуры объявлены) |
| `export_roundtrip` | все, когда экспорт объявлен в outputs |
| `uv_preservation` | passthrough_preservation, family_static, family_rig_compatible |
| `rig_bindings` | family_rig_compatible |
| `animation_clips` | family_rig_compatible (при pinned клипах) |
| `silhouette_conformance` | reconstruction (только там); для family — не применяется (ADR §8.1) |
| `style_review` | family_static, family_rig_compatible — human/авторская процедура, не автоматический скор |
| `format_loss_declared` | passthrough_preservation |

## 7. Порты (interface contracts) для GLM-2..6

Порты — ссылки на контракты §2; Rust-сигнатуры иллюстративны, реализация в изолированных
модулях потоков, не в shared Core. Полный handoff: `docs/adr5-streams/contracts/HANDOFF.md`.

### 7.1. GLM-2 Asset Intake

- **Порт:** `intake.snapshot(input: IntakeInput) -> (SourceSnapshot, IntakeReport)`.
- **Вход:** пути к файлам (disposable workspace), сведения об источнике, объявленные внешние зависимости.
- **Выход:** `SourceSnapshot` (schema §2.1) + `IntakeReport` (диагностика формата/безопасности, сканирование зависимостей).
- **Гарантии:** оригиналы не изменяются; hashes по фактическим bytes; path containment (`PATH_ESCAPE`); missing/unsupported зависимости — адресная ошибка, не молчаливый пропуск.
- **Запреты:** не исправляет и не перезаписывает оригинал; не вызывает реконструктор; не публикует.

### 7.2. GLM-3 Batch Executor

- **Порт:** `batch.execute(plan: ResolvedBatchPlan) -> BatchReport`; `batch.resume(batch_id, only_failed) -> BatchReport`; `batch.cancel(batch_id, item_id?)`.
- **Вход:** immutable `ResolvedBatchPlan` (проверен на ацикличность/полноту до запуска).
- **Выход:** attempts + per-item доменные статусы; `BatchReport`.
- **Гарантии:** persisted attempt lifecycle (существующие jobs — дочерние execution records); retry = новая attempt, inputs readonly; отказ item блокирует только зависимые стадии; resume только при совпадающих входах/hashes; invalid input / failed quality не исправляется бесконечным retry; OOM не снижает детализацию/пороги молча.
- **Запреты:** не меняет дизайн и пороги проверки; не публикует и не утверждает; не использует in-memory Scheduler как общий admission.

### 7.3. GLM-4 Family Builder / Variant Generator

- **Порты:** `family.validate(recipe) -> [diagnostics]`; `family.plan(recipe, lock, requests[]) -> ResolvedBatchPlan`; `variant.generate(lock, VariantRequest) -> CandidateAssetBundle` (приватный staging).
- **Вход:** `FamilyRecipe` + `SourceSet` + `RecipeLock` + `VariantRequest[]`.
- **Гарантии:** рецепт описывает допустимое множество результатов; параметры типизированы и проверены (диапазон, размер произведения, совместимость); неизвестный параметр — отказ; исполняемая ревизия immutable; item seed зависит от стабильной identity; генератор не публикует, не утверждает, не расширяет scope.
- **Запреты:** `allow_arbitrary_code: false`; shell/Python/eval в рецепте; свободный путь к исполняемому скрипту; floating `latest` в плане.

### 7.4. GLM-5 Blender adapter (registered templates)

- **Порт:** `template.execute(bindings, capabilities, limits) -> artifacts[]` поверх существующего ограниченного subprocess; параметры через typed bindings рецепта.
- **Вход:** pinned template (registered), параметр-биндинги, capabilities allowlist, лимиты.
- **Гарантии:** только зарегистрированные шаблоны (не автозапуск любого `.blend`); `--factory-startup`; без пользовательского startup/addons и авто-исполнения embedded Python/drivers; загрузка копии; выход только в private staging; проверка эффективного результата (лимиты геометрии).
- **Запреты:** не меняет текущие 18 Worker ops и `BLENDER_WORKER.md` без отдельного gate; не объявляет `.blend`-intake поддержанным входом (блокер ADR §13.2); не ослабляет artifact verification.

### 7.5. GLM-6 Validation

- **Порты:** `validation.evaluate(bundle, profile, pinned_inputs) -> ValidationReport` (immutable, no-clobber registry); `validation.verify(report) -> ok|код отказа`.
- **Вход:** candidate bundle snapshot + pinned `ValidationProfile` + фактические bytes входов.
- **Гарантии:** отчёты immutable; reservation до вычислений, тяжёлая работа вне lock; hashes по фактическим прочитанным bytes; `unavailable` для недоступных проверок; применимость фиксируется до генерации; неизвестная обязательная проверка блокирует (`VALIDATION_UNKNOWN_REQUIRED`); отчёты раздельны с решениями.
- **Запреты:** не подменяет авторское решение; не возвращает `not_required` без ссылки на applicability; не меняет IT-8D enum и gate реконструкции.

## 8. Draft CLI-поверхность (не реализована)

Эскиз из ADR §7.1 воспроизводится без изменений и остаётся **эскизом UX**: команд нет, JSONL-envelope
и exit codes подлежат отдельному согласованию без изменения старого транспорта. Машиный режим:
структурированные документы/events только в stdout, diagnostics в stderr; процесс, создавший файл,
не означает успешную проверку или принятие.

```text
michelangelo asset inspect --source <model> --json          # DRAFT
michelangelo asset prepare --asset <revision> --plan <preparation-plan> --json   # DRAFT
michelangelo family validate --recipe <recipe> --json       # DRAFT
michelangelo family plan --recipe <recipe> --count 15 --seed 1742 --out <batch-plan>  # DRAFT
michelangelo batch plan --items <independent-assets> --out <batch-plan>     # DRAFT
michelangelo batch run --plan <batch-plan> --max-workers 1 --jsonl          # DRAFT
michelangelo batch status --batch <id> --json               # DRAFT
michelangelo batch resume --batch <id> --only-failed --json # DRAFT
michelangelo batch cancel --batch <id> --item <id> --json   # DRAFT
```

Имя команды приёмки rich asset target отдельно согласуется (ADR §7.1); существующий `edit accept`
сохраняет свой смысл и не переиспользуется как универсальная приёмка (см. OWNERSHIP_MATRIX.md §3).

## 9. Решения владельца (отдельно от фактов)

Собраны также в MIGRATION_BOUNDARIES.md §4. Здесь — только контрактные:

1. **OP-1 (rich bundle target):** принять/отклонить трактовку `AssetRevision`+`CandidateAssetBundle`
   как отдельного asset target в общем механизме snapshot/evidence/accept, с новым canonical
   publisher; `edit accept` не расширяется. Блокирует этап 1.
2. **OP-2 (`.blend` intake):** поддерживается ли в v0.1 приём `.blend` как входа (при `--factory-startup`,
   копия, без drivers/addons) или ограничиться `.glb/.gltf/.obj/.stl/.ply` + `.blend` только как
   output registered templates. Пока не решено: GLM-2 ограничивает входной набор; GLM-5 не принимает
   произвольный `.blend`.
3. **OP-3 (canonical JSON):** RFC 8785 для новых документов vs поле-фиксирующий кодировщик legacy
   (совместимость внутри новых документов не нарушает старые). Нужна явная резолюция до реализации
   digest-кода.
4. **OP-4 (check_id реестр):** утвердить состав §6 для v0.1; расширение — новая ревизия схемы.
5. **OP-5 (source roles enum):** утвердить 6 ролей ADR §4.4 как закрытый набор v0.1.
6. **OP-6 (batch доменные статусы):** утвердить 8 статусов §2.9 как contract enum для BatchReport.
7. **OP-7 (`not_required`):** подтвердить, что новое значение результата остаётся вне IT-8 enum.

До резолюции все потоки считают эти пункты **не утверждёнными** и не строят код на их «фактическом»
существовании.

## 10. Что этот gate НЕ делает

- Не создаёт crates, методы JSONL, миграции SQLite, Worker ops, CLI-команды, провайдеров.
- Не изменяет `michelangelo.input-package/0`, `michelangelo.geometry-snapshot/0`,
  `michelangelo.evaluation-report/0`, verification state, IT-8D.
- Не объявляет V2–V4 реализованными: V2–V4 — расширение без реализации сейчас (порты §7
  покрывают V0/V1 + сценарии A/C).
- Не заменяет ADR 0005: при расхождении приоритет у ADR и решения владельца.
