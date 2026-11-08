# INTEGRATION_NOTES — GLM-4 Family → раунд 2 / интегратор / DeepSeek

- **Статус:** draft-0, всё provisional. Читалось read-only: `I:/prog/michelangelo-adr5-contracts` (`schemas/adr5/*.schema.json`, `docs/adr5-contracts/CONTRACT_GATE.md`, `NEGATIVE_TEST_MATRIX.md`, `HANDOFF.md`). Ничего не копировано в зону владения кроме ориентира по полям; ни один документ GLM-1 не изменён.

## 1. Маппинг provisional DTO → GLM-1 draft-0.1 (для раунда 2)

| Мой DTO/поле | GLM-1 schema | Совпадение | Расхождение / предложение |
|---|---|---|---|
| `SourceRole` (6 значений) | `source_set.schema.json` roles enum | 1:1 | нет |
| `SourceUsage` (build_input/reference_only/held_out) | usage enum | 1:1 | нет |
| `SourceSet.validate()` held-out инвариант | allOf (independent_validation_reference ⇒ held_out) | совпадает по смыслу | я добавил зеркальное правило: held_out ⇒ никаких build-ролей |
| `SourceEntry.allowed_parts` | allowed_parts | 1:1 | я трактую отсутствие = нет права (как в description) |
| `FamilyRecipe.parameters` (integer/enum/boolean + number-спека) | parameterSpec | почти 1:1 | у меня `number` не энумерируем (cardinality error) — предложить явный `step` в схеме либо запретить number в комбинационных семьях |
| `Generator.kind` (`registered_blender_template_v0`, `static_variant_set_v0`) | тот же enum | 1:1 | я даю явные имена v2/v3/v4 kinds для честного `CAPABILITY_DENIED`; схема их не знает (additionalProperties=false) — proposal: включить в схему как enum с `"deferred"` семантикой, либо отвергнуть и оставить parse-level |
| `parameter_bindings` (`param -> "role.token"`) | parameter_bindings (free string) | сужение | я enforce `role.token` (ровно 2 сегмента, lowercase токены); предложение: зафиксировать этот формат в CONTRACT_GATE §7.3 |
| `scope.allow_arbitrary_code` const false | const false | 1:1 | у меня поля нет в struct; wire true → `ARBITRARY_CODE` |
| `RecipeLock` (exact deps, tool_versions, prng, floating_latest_allowed=false) | recipe_lock.schema.json | ~1:1 | у меня добавлен `source_set_sha256` и per-source pin `source:<id>` (stale donor detection); в draft-0.1 это покрывается только через dependencies — proposal добавить явное поле |
| `VariantRequest` (item_id, parameters, purpose, export_targets) | variant_request.schema.json | 1:1 | seed: у меня отсутствует в struct — он всегда derived (`family-seed-v1`); draft-0.1 позволяет explicit seed. Proposal: derive-only для генерируемых вариантов, explicit seed — отдельное решение владельца |
| `ResolvedBatchPlan` (batch_id, items, admission, cycle_check) | batch_plan.schema.json | ~ | я не пишу `variant_request_sha256`/`asset_input` в items (держу fingerprints: recipe/lock/source-set/resolved_parameters); proposal согласовать набор fingerprint-ключей |
| `errors::FamilyErrorCode` | NEGATIVE_TEST_MATRIX behavioral-коды | совпадают для F6/F7/F10/F12/F17 | мои коды: ROLE_INVALID, SCOPE_WIDENING, PROTECTED_ROLE_TARGET (F9-аналог), ARBITRARY_CODE, FLOATING_VERSION, PARAMETER_INVALID, HELDOUT_LEAKAGE, SOCKET_MISMATCH, DUPLICATE_IDENTITY, FOREIGN_SKELETON, CAPABILITY_DENIED, STALE_DEPENDENCY, LOCK_MISMATCH, INSUFFICIENT_UNIQUE_COMBINATIONS, COMBINATORIAL_CAP_EXCEEDED, NOT_PLANNABLE — на раунд 2 сверить с реестром OP-4/OP-6 |

**Ничего из этого не утверждено.** Раунд 2 (GLM-1) — arbiter.

## 2. Порты (CONTRACT_GATE §7.3, provisional имена)

```rust
// family.validate
FamilyPlanner::validate -> (recipe.validate() + sources.validate() + lock.check_against())
// family.plan
FamilyPlanner::plan(batch_id, &recipe, &sources, &lock, &[VariantRequest]) -> FamilyResult<ResolvedBatchPlan>
// variant.generate — НЕ реализован (pure planning). см. port::BatchExecutorPort
trait BatchExecutorPort { fn accept_plan(&self, plan: &ResolvedBatchPlan) -> FamilyResult<PlanAck>; }
```

- `NullBatchExecutor` — дефолт, отказ `NOT_PLANNABLE`: граница pure planning видна в рантайме, GLM-3 подменит реализацией.
- Для `family.plan` и Batch Executor: `derive_seed(lock_digest, item_id)` — единственная точка seed derivation; Batch должен НЕ переизобретать.

## 3. Что изменить в моём коде при принятии draft-0.1 (минимальный план раунда 2)

1. Переименовать/переставить поля под `family_recipe.schema.json` (например `source_set_ref.{source_set_id,revision,sha256}` — уже совпадает).
2. Добавить `created_at` (RFC3339) во все документы — у меня нет времени создания (determinism важнее, но wire требует).
3. Решить судьбу моих приватных расширений: `height_model`, `sockets`, `source_inputs` — либо включить в schema v0.2, либо вынести в `constraints`/`dependencies`.
4. Согласовать `INSUFFICIENT_UNIQUE_COMBINATIONS` как доменный результат плана (не PASS, не ошибка executor) — это из ADR §9.3.

## 4. Негативы GLM-1, закреплённые здесь

- **F6** ROLE_INVALID: unknown role string; donor без allowed_parts; style_reference не reference_only.
- **F7** SCOPE_WIDENING: `scope`-инъекция в variant_request wire; binding на роль вне allowed_roles.
- **F9 (аналог)** PROTECTED_ROLE_TARGET: binding на protected role; overlap allowed/protected. Полный F9 (эффективный результат через зависимости) — GLM-5/6.
- **F10** ARBITRARY_CODE: `allow_arbitrary_code:true` (top-level и в scope); script-словарь в binding (`exec.` и пр.); свободный путь `../../python_entry.main`.
- **F12** FLOATING_VERSION: `latest`, `*`, `^`, `~`, `>`, `<`, пустая версия; `floating_latest_allowed:true` на wire; отсутствие обязательных tool versions.
- **F17** PARAMETER_INVALID: unknown/extra параметр, missing параметр, вне диапазона, неверный тип, enum miss.
- Дополнительно: HELDOUT_LEAKAGE (3 варианта), SOCKET_MISMATCH, DUPLICATE_IDENTITY (source_id, item_id), FOREIGN_SKELETON, STALE_DEPENDENCY (donor re-export, dep rotation), LOCK_MISMATCH, CAPABILITY_DENIED (V2–V4 kinds), INSUFFICIENT_UNIQUE_COMBINATIONS, COMBINATORIAL_CAP_EXCEEDED, NOT_PLANNABLE, SCHEMA_INVALID (unknown keys, duplicate keys, trailing).

## 5. Для DeepSeek (независимая negative acceptance)

1. **Seed derivation:** `family-seed-v1` = `SHA-256("michelangelo/family-seed-v1" || 0x00 || lock_digest_hex || 0x00 || item_id)[0..8] BE & (2^53-1)`. Проверить: достаточен ли lock_digest как энтропийный якорь; не создаёт ли 53-битная маска коллизий на практике (30 items — нет, но зафиксировать).
2. **Hold-out инвариант:** мои правила строже draft-0.1 allOf (зеркальное направление + запрет build-ролей у held_out). Контрпример, который это ломает, приветствуется.
3. **Стратегия ошибок check_against:** порядок identity → dep-hash (STALE) → recipe digest (LOCK_MISMATCH) → source_set digest (STALE). Обосновать/оспорить приоритеты диагностики.
4. **План без `created_at` и без `creator`:** my ResolvedBatchPlan не содержит автора плана; в batch_plan.schema.json есть creator. Proposal: creator живёт в CLI-слое, не в planner — согласовать.
5. **Float-хеширование:** canonical JSON сейчас форматирует f64 shortest-round-trip Rust; в fixture float-значений в хешируемых местах нет (wear — enum). Требуется решение (op-3-подобное): запретить float в hash-области либо зафиксировать формат.
6. **INSUFFICIENT_UNIQUE_COMBINATIONS:** честная ошибка при 720 валидных комбинациях и запросе, например, 1000 — подтверждает ли владелец, что план обязан падать, а не уменьшать count?

## 6. Изменения общих файлов

**Ноль.** Корневой `Cargo.toml` не тронут: новый crate standalone со своей пустой `[workspace]`-секцией. Существующие crates не собираются вместе с ним (root members list явный). Патч-предложение не требуется — при интеграции достаточно добавить одну строку в `members` (это делает интегратор, не этот поток).
