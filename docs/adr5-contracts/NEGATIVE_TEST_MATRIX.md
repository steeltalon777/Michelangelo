# ADR 0005 — Negative Contract Test Matrix (GLM-1, draft)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Ревизия (fix-раунд 2026-11-09):** добавлены schema-фикстуры F21–F23, расширены F12/F16;
  негативов стало 15. `check_schemas.py` пинит для каждой фикстуры точный путь отказа
  (таблица fixture → expected path): отказ по другому правилу (например, по digest-паттерну
  раньше заявленного инварианта) есть провал, как и отсутствие записи в таблице.
- **Статус:** DRAFT. Негативные контракты обязательны для каждого потока. Часть фиксируется
  **схемой** (исполняется `tools/check_schemas.py` — fixtures в `schemas/adr5/examples/negative/`),
  часть — **поведением** исполнителя (unit/negative tests потока). Ни один из этих статусов
  сегодня не является пройденным тестом реализации: это обязательная матрица.

## 1. Сводная матрица

| ID | Негативный сценарий | Уровень | Ожидаемый отказ | Исполнитель |
|---|---|---|---|---|
| F1 | Tampered payload + подделанный манифест: перезаписанный mesh, пересчитанный declared digest | behavioral | `SNAPSHOT_MISMATCH` / digest-несовпадение при пересчёте Core по фактическим bytes; новая evaluation не легитимизирует | GLM-2 (intake), GLM-6 (bundle) |
| F2 | Вход изменён после evaluation (например, текстура) | behavioral | `VALIDATION_INPUT_CHANGED` (аналог IT-8D `INPUT_CHANGED`) | GLM-6 |
| F3 | Отсутствующая зависимость транзитивного замыкания (linked library/texture/clip) | behavioral | `DEPENDENCY_MISSING`; не заменяется пустым материалом/успешным статусом | GLM-2, GLM-6 |
| F4 | Два sibling-варианта на одной базе: второй принимает после первого приёма (stale base) | behavioral | `REVISION_STALE` (CAS по `expected_current_revision`); второй кандидат не перехватывает identity | GLM-3 (executor), GLM-6/publication |
| F5 | Replay approval: тот же AcceptanceRecord повторно применён к другому candidate digest | behavioral | `ACCEPTANCE_REPLAY`; запись привязана к одному bundle digest | GLM-6 / publisher |
| F6 | Invalid role: роль вне закрытого набора или donor без allowed_parts | **schema** + behavioral | schema: `source_set_invalid_role.json` отвергнут; behavioral: `ROLE_INVALID` | GLM-4 |
| F7 | Scope widening: request/recipe добавляет scope/protected/capability поля или расширяет allowed_roles | **schema** + behavioral | schema: `variant_request_scope_field.json` отвергнут; behavioral: `SCOPE_WIDENING`; task-уровень: аналог IT-8D `SCOPE_DENIED` | GLM-4, GLM-3, assistant-путь |
| F8 | Unknown mandatory validation: required check вне реестра check_id | **schema** + behavioral | schema: `profile_unknown_check.json` отвергнут; behavioral: `VALIDATION_UNKNOWN_REQUIRED` блокирует принятие | GLM-6 |
| F9 | Эффективное изменение защищённой роли через разрешённую зависимость (parent transform / shared material / modifier / GN) — bytes защищённого mesh не изменились | behavioral | Отказ проверки protected/effective result; evidence охватывает влияющие зависимости (ADR §11.4) | GLM-4, GLM-5, GLM-6 |
| F10 | Recipe с `allow_arbitrary_code: true` / shell/Python/eval / свободный путь к скрипту | **schema** + behavioral | schema: `recipe_arbitrary_code.json` отвергнут; behavioral: рецепт не исполняется | GLM-4, GLM-5 |
| F11 | Batch plan с циклом зависимостей / неполными зависимостями | **schema** + behavioral | schema: `batch_plan_not_acyclic.json` отвергнут; behavioral: план не исполняется без пройденного cycle-check | GLM-3 |
| F12 | Floating `latest` в lock / отсутствие точной версии | **schema** + behavioral | schema: `lock_floating_latest.json` (флаг `floating_latest_allowed:true`) и `lock_tool_version_latest.json` (`tool_versions.blender="latest"` при `floating_latest_allowed:false`) отвергнуты; behavioral: план не исполняется с плавающими версиями | GLM-4 |
| F13 | Path escape в артефактах (`.`, `..`, absolute, backslash, drive letter) | **schema** + behavioral | schema: `bundle_path_escape.json` отвергнут; behavioral: `PATH_ESCAPE` (прецедент artifact verification) | GLM-2, GLM-3, GLM-5 |
| F14 | Acceptance без зарегистрированного validation report | **schema** + behavioral | schema: `acceptance_missing_report.json` отвергнут; behavioral: публикация невозможна без отчёта (аналог `EVALUATION_MISSING`) | GLM-6 / publisher |
| F15 | Неподдержанный формат входа (fbx / произвольный .blend до решения B2) | **schema** | schema: `source_snapshot_fbx.json` отвергнут; intake не создаёт snapshot | GLM-2 |
| F16 | `verdict: not_required` / подмена `unavailable` на pass | **schema** + behavioral | schema: `report_not_required_verdict.json` (verdict) и `report_not_required_missing_ref.json` (not_required без `applicability_ref`, per-check) отвергнуты; behavioral: свёртка fail > unavailable > pass; `not_required` — только в check со ссылкой applicability | GLM-6 |
| F17 | Параметр вне диапазона / неизвестный параметр / неполный набор значений | behavioral | `PARAMETER_INVALID`; не молчаливое игнорирование | GLM-4 |
| F18 | Недоступная требуемая capability при исполнении | behavioral | `CAPABILITY_UNAVAILABLE` — блокирует item, не весь batch; остальные items продолжают | GLM-3, GLM-5 |
| F19 | Stale approval: приёмка по политике/profilе, чей digest больше не совпадает с lock | behavioral | Отказ; изменённое основание инвалидирует зависимые решения (IT-8) | GLM-6 / publisher |
| F20 | Crash/retry: публикация после crash, повторный запуск той же attempt, сирота-процесс | behavioral | Идемпотентность: recovery не создаёт вторую публикацию и не подменяет открытый unknown на resolved; fencing исключает позднюю публикацию прежней attempt | GLM-3 |
| F21 | Held-out утечка: источник с ролью `independent_validation_reference`, но `usage != held_out` (в т.ч. `IVR+build_input` рядом с `IVR+held_out`) | **schema** | schema: `source_set_ivr_not_held_out.json` отвергнут (per-item: каждый IVR-источник обязан `held_out`) | GLM-4 |
| F22 | Held-out утечка: источник с `usage: held_out`, но с build/reference ролью (напр. `[base_candidate, IVR]`) | **schema** | schema: `source_set_held_out_build_role.json` отвергнут (per-item: held_out-источник может нести только `independent_validation_reference`) | GLM-4 |
| F23 | CandidateBundle с transactional base (`base_ref.kind = asset_revision`) без объявления CAS | **schema** | schema: `bundle_missing_expected_current_revision.json` отвергнут (ADR §5.3 / CONTRACT_GATE §4.2: публикация требует явного `expected_current_revision`: null = no-clobber create, integer = CAS) | GLM-5 (produce), GLM-6 / publisher |

## 2. Schema-фикстуры (исполнимая часть)

15 negative fixtures в `schemas/adr5/examples/negative/` уже исполняются:
`python docs/adr5-streams/contracts/tools/check_schemas.py` — все отвергаются (см. SCOPE.md, факт
прогона 2026-11-09). Каждая фикстура закреплена в таблице `NEGATIVE_EXPECTED_PATH`
(`tools/check_schemas.py`) за точным путём отказа своего правила: отказ по другому правилу
(например, по digest-паттерну раньше заявленного инварианта) — провал, как и отсутствие записи
в таблице. Это проверка **схемного** слоя; behavioral-проверки F1–F5, F9, F17–F20 —
обязанность потоков GLM-2..6 в их изолированных тестах (unit + negative), не Core.

## 3. Правила

1. Поток обязан реализовать свою часть матрицы (HANDOFF.md) и честно пометить непокрытое как
   SKIPPED/BLOCKED с причиной — не PASS.
2. Ожидаемый результат любого негативного теста: адресная ошибка с кодом из CONTRACT_GATE §5,
   отсутствие публикации/приёмки, неизменность оригинала.
3. Расширение матрицы (новые fixtures) — через ревизию этого документа, не молча.
