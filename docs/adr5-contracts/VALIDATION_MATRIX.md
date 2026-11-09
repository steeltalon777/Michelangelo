# ADR 0005 — Validation Applicability и Format Loss Matrix (GLM-1, draft)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Статус:** DRAFT. Матрица применимости проверок по профилям (ADR §8.1) и потерь форматов.
  Применяется через `validation_profile.schema.json` (check_id реестр — CONTRACT_GATE §6).

## 1. Матрица применимости проверок

Легенда: **R** = required (обязательна, отсутствие = `unavailable` и блокирует вердикт),
**O** = optional (может отсутствовать без блокировки), **–** = не применима
(в отчёте возможен `not_required` со ссылкой на applicability профиля),
**H** = human review (не автоматический скор, результат — отдельная авторская процедура).

| check_id | reconstruction | passthrough_preservation | family_static | family_rig_compatible | anatomy_change |
|---|---|---|---|---|---|
| payload_integrity | R | R | R | R | R |
| dependency_closure | R | R | R | R | R |
| path_containment | R | R | R | R | R |
| asset_digest_full | R | R | R | R | R |
| budget_geometry | R | R | R | R | R |
| protected_roles | R (protected parts пакета) | O | R | R | R |
| connector_compatibility | – | – | R (при connector constraints) | R (при socket profile) | O (новый scope) |
| minimum_clearance | – | – | R (при объявленном constraint) | R (при объявленном) | O |
| maximum_dimension | – | – | R (при объявленном constraint) | R (при объявленном) | O |
| material_presence | O | R (если материалы объявлены к сохранению) | R | R | R |
| texture_integrity | O | R (если текстуры объявлены) | R (если объявлены) | R (если объявлены) | O |
| export_roundtrip | R (когда экспорт в outputs) | R (когда экспорт объявлен) | R (когда экспорт в outputs) | R (когда экспорт в outputs) | R (когда экспорт в outputs) |
| uv_preservation | O | R (если UV объявлены) | R (если UV объявлены) | R | O |
| rig_bindings | – | – | – | R | R |
| animation_clips | – | – | – | R (при pinned клипах) | R (новые клипы/позы) |
| silhouette_conformance | R (calibrated references) | – | **–** (намеренно изменённый силуэт не сравнивается с мастер-силуэтом) | O (только если silhouette зафиксирован профилем) | – |
| style_review | – | – | H | H | H |
| format_loss_declared | – | R | O | O | O |

Правила применения:

1. Применимость фиксируется **до генерации** (ValidationProfile в RecipeLock/плане). Требовать от
   намеренно изменённого силуэта совпадения с мастер-моделью нельзя; фиктивный
   `silhouette_conformance=pass` — отказ (`CONTRACT_GATE` §5).
2. Неизвестная обязательная проверка (id вне реестра) блокирует принятие: `VALIDATION_UNKNOWN_REQUIRED`.
3. «Не требуется по утверждённой постановке» выражается значением `not_required` нового отчёта со
   ссылкой на applicability профиля. Это **не** новое значение IT-8 enum (вопрос владельца OP-7).
4. Отсутствующий GT/анимационный тест = `unavailable`, не pass. Один contact sheet помогает автору,
   но не заменяет индивидуальные evidence references на каждый вариант (ADR §8.1).
5. Проверка в Blender не доказывает совместимость импорта в Unreal; UE-ready требует отдельного
   engine-отчёта (ADR §4.2, §10).
6. Риг-профиль: совпадение имён костей или успешный импорт без проигрывания клипов не являются
   доказательством совместимости (ADR §8.1).

## 2. Матрица потерь форматов (intake → внутренние → exchange)

Основание: фактические возможности Worker на b861547 (import `.glb/.gltf/.obj/.stl/.ply`,
export `.glb`, `save_blend`; рендер только Workbench; конвенции импорта — дефолты Blender,
`BLENDER_WORKER.md` §Import format notes). «?» = зависит от содержимого файла, фиксируется
IntakeReport, не предполагается.

| Формат входа | Geometry | UV | Materials | PBR textures | Rig/skin | Animation | Morphs | Node groups | Hierarchy | Статус v0.1 |
|---|---|---|---|---|---|---|---|---|---|---|
| `.glb` | да | да | да (PBR metal-rough) | да (embedded/external) | да (glTF skins) | да | да (targets) | нет | да | **поддержан** |
| `.gltf` | да | да | да (PBR) | да (external) | да | да | да | нет | да | **поддержан** (внешние зависимости обязательны к closure) |
| `.obj` | да | да | частично (MTL: diffuse/spec; без PBR-графа) | частично (map_* каналы) | нет | нет | нет | нет | частично (o/g группы) | **поддержан** (потери PBR обязательны к объявлению) |
| `.stl` | да (только меш) | нет | нет | нет | нет | нет | нет | нет | нет | **поддержан** (потери максимальны, фиксируются) |
| `.ply` | да | частично (vertex attributes) | нет (vertex colors?) | нет | нет | нет | нет | нет | нет | **поддержан** («?» фиксируется intake) |
| `.fbx` | — | — | — | — | — | — | — | — | — | **не поддержан** (Worker не импортирует; ADR §2). Snapshot с fbx = отказ схемы |
| `.blend` (вход) | — | — | — | — | — | — | — | — | — | **BLOCKED**: `save_blend` ≠ безопасный intake; ожидание решения B2 (§13.2) |
| `.blend` (выход registered template) | да | да | да | да | да (если создан) | да (если создан) | да (если создан) | да | да | **output**; byte-равенство исходнику не обещается |

Exchange `.glb` из `.blend`: теряет node groups/драйверы и части графов материалов; процедуры,
риги, анимация переносятся в пределах glTF 2.0. Любая потеря процедурности/materials/rig при
конвертации обязана быть обнаружена/объявлена и разрешена, а не скрыта (ADR §8.3) — канал:
`declared_losses` (PreparationPlan) + `format_loss_declared` (passthrough-профиль).

Дополнительно:

- Roundtrip-проверка (`export_roundtrip`) обязательна, когда экспорт заявлен в outputs бандла;
  экспорт создаётся в private staging и проверяется до commit (ADR §8.3).
- Byte-preserving копия возможна для native archive; пересохранённый `.blend`/GLB не обязаны быть
  побитово равны исходнику. Честные уровни воспроизводимости — ADR §9.3.

## 3. Сохранение инвариантов IT-8D в новых путях

Каждый инвариант IT-8D (TZ §12) имеет прямого наследника в новых контрактах:

| IT-8D | Наследник в ADR 0005 контракте |
|---|---|
| D1.1 Snapshot identity + полный diff | `SourceSnapshot` immutable; `asset_digest` поверх фактических bytes всех артефактов; `diff_to_base` + проверка unchanged-артефактов против base bytes; подмена манифеста не легитимизируется новой evaluation (`SNAPSHOT_MISMATCH`) |
| D1.2 Фактические bytes входов | `validation_report.input_digests` — digests реально прочитанных bytes; `VALIDATION_INPUT_CHANGED` при изменении после отчёта |
| D1.3 Reservation + immutability evaluation | Реестр отчётов no-clobber: reservation id до вычислений, тяжёлая работа вне lock, отчёт не перезаписывается |
| D1.4 Полная проверка bundle перед commit | AcceptanceRecord ссылается на bundle digest + digest зарегистрированного отчёта; публикация = CAS (`expected_current_revision`) под lock |
| D1.5 Решения под lock | Accept/reject/publish сериализуются одним canonical механизмом; reject-vs-accept интерливинг и crash-after-pointer — обязательные негативные тесты |
| D1.6 Held-out independence | Per-item schema-правило в `source_set`: каждый источник с `independent_validation_reference` обязан `usage: held_out` (F21); каждый `held_out`-источник может нести только эту роль — без build/reference (F22); не используются при построении |
| D2.7 Перенос evidence references | `provenance_chain` + hash-pinned docRefs; resolution/confirmation переносится в новый bundle со всеми ссылками |
| D2.8 Исторические bindings | Bundle выбирает версии документов своей ревизии (docRefs), не глобально последние |
| D2.9 Сужение задачи не расширяет | `VariantRequest` структурно не содержит scope-полей; `SCOPE_WIDENING` на валидаторе |
| D2.10 Consistency policy override | `approval.policy_ref` hash-pinned в RecipeLock и AcceptanceRecord; смена policy = новый lock |
| Негативы IT-8D (тампер, PNG-after-eval, конкуренция, копия отчёта, interleaving, crash, перенос refs, rebind+rollback, widening, override, opposite held-out) | Повторяются в NEGATIVE_TEST_MATRIX.md как F1–F10 с новыми кодами; суть и ожидания не ослаблены |
