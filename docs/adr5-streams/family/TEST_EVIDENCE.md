# TEST_EVIDENCE — GLM-4 Family Builder / Pure Planner V0–V1

- **Дата:** 2026-11-08. **Toolchain:** cargo/rustc 1.96.0 (stable-x86_64-pc-windows-msvc). **Окружение:** локальный Windows 10 (10.0.26100), standalone crate, zero dependencies.
- Все команды выполнялись из `crates/michelangelo_family` в worktree `I:/prog/michelangelo-adr5-family`, ветка `feat/adr5-family`. Это **не** production-заявление: pure planning PoC, без Blender, без реальных payload.

## 1. Сборка

```
cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.55s   (первая сборка, 0 warnings после правок)
```

Корневой workspace не затронут: crate собирается standalone (`[workspace]` в локальном Cargo.toml); корневые `Cargo.toml`/`Cargo.lock` не изменялись (см. `git status` ниже).

## 2. Тесты (unit + негативные) — фактический прогон 2026-11-08

Команда: `cargo test`

```
running 7 tests                                   # lib unit tests (sha256, json)
test result: ok. 7 passed; 0 failed; 0 ignored
running 37 tests                                  # tests/negative_tests.rs
test result: ok. 37 passed; 0 failed; 0 ignored
running 10 tests                                  # tests/unit_tests.rs
test result: ok. 10 passed; 0 failed; 0 ignored
running 0 tests                                   # doc-tests
test result: ok. 0 passed; 0 failed
```

**Итого 54 passed / 0 failed.** Ни один негатив не помечен PASS; каждый проверяет точный код отказа (`FamilyErrorCode`).

### Ключевые позитивные проверки (10)
- SHA-256: NIST векторы (в т.ч. 1,000,000 × 'a' — перекрёстно сверен с Python hashlib и Windows certutil: `cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0`).
- Seed derivation: детерминизм, чувствительность к item_id и lock digest, порядок-независимость, 53-бит JSON-safe маска.
- Комбинационный движок: произведение 1944, валидных 720 (арифметика fixture задокументирована), парные/сокетные/высотные фильтры.
- Sampling: 15 детерминированных, попарно различных requests; валидны против рецепта.
- Dry plan: побайтово идентичен при повторном планировании; 15 приватных непересекающихся output namespace; fingerprints различаются по параметрам.
- Round-trip: committed fixtures на диске == кодовые DTO (защита от дрейфа) и committed `dry_plan_15.json` == канонический вывод планировщика.
- Wire round-trip VariantRequest.

### Негативы (37) — по матрице GLM-1
- F6 ROLE_INVALID: неизвестная роль/usage строка; module_donor без allowed_parts; style_reference с build_input.
- HELDOUT_LEAKAGE: independent_validation_reference не held_out; held_out + build роль; generator потребляет held-out источник (plan time).
- F7 SCOPE_WIDENING: wire-инъекция `scope` в variant_request; binding на роль вне allowed_roles.
- F9-аналог PROTECTED_ROLE_TARGET: binding на protected роль (`entry_anchor.*`); overlap allowed/protected.
- F10 ARBITRARY_CODE: `allow_arbitrary_code:true` top-level и в scope; script-словарь в binding (`exec.run_script`); свободный путь (`../../python_entry.main`).
- F12 FLOATING_VERSION: `latest`, `^4.2`, `~4.2`, `>=4.2`, `<5`, `*`, пустая; отсутствие michelangelo/blender; wire `floating_latest_allowed:true`.
- F17 PARAMETER_INVALID: unknown/extra, missing, вне диапазона, неверный тип, enum miss (unit + wire).
- CAPABILITY_DENIED: kinds `parametric_modifier_v2`, `rule_based_construction_v3`, `agent_proposal_v4`, unknown kind.
- SOCKET_MISMATCH: `roof=hip` требует отсутствующий `roof_hip_socket`; require-socket без профиля.
- DUPLICATE_IDENTITY: дубликат source_id; дубликат item_id в одном плане.
- FOREIGN_SKELETON: skinned build-источники с разными skeleton digests.
- STALE_DEPENDENCY: donor re-export (hash сменился), ротация hash зависимости, сменившийся source_set digest. LOCK_MISMATCH: другая ревизия/чужой рецепт.
- INSUFFICIENT_UNIQUE_COMBINATIONS: 1 валидная комбинация, запрос 15 → честный отказ ("refusing to fabricate").
- COMBINATORIAL_CAP_EXCEEDED: продукт 1944 против cap 100; превышение budget.max_items_per_plan.
- NOT_PLANNABLE: NullBatchExecutor отказывает план (граница pure planning).
- SCHEMA_INVALID: unknown top-level keys, duplicate JSON keys, trailing content, пустой план.

## 3. Dry plan на 15 requests — фактический прогон

```
cargo run --quiet -- --regen fixtures          # запись wire-fixture документов (3 файла)
cargo run --quiet -- fixtures > docs/adr5-streams/family/examples/dry_plan_15.json
# stderr: dry plan ok: 15 items, recipe urban-house-family@1, lock digest a6054305...
#         executor port refused (expected): NOT_PLANNABLE: ...
```

Проверка уникальности (Python поверх committed JSON):

```
plan items: 15
unique ids: 15
unique param sets: 15
unique seeds: 15
seeds 53-bit safe: True
```

Примеры item (полный документ — `examples/dry_plan_15.json`, canonical JSON одной строкой):

```
house-000  {bays:2, floors:1, palette:brick_dark,  roof:gable, trim:none, wear_permille:"0"}   seed 8747271564...
house-001  {bays:2, floors:1, palette:plaster_warm, roof:gable, trim:iron, wear_permille:"300"} seed ...
house-002  {bays:2, floors:2, palette:brick_dark,  roof:gable, trim:iron, wear_permille:"200"} ...
```

Состав комбинаций честный: hip-крыша исключена (socket `roof_hip_socket` отсутствует в pinned профиле), flat+carved и timber_grey+flat исключены матрицей несовместимости, 3 этажа+gable исключены height model (>10.5 м). Разнообразие — из различных допустимых значений, не только seed (ADR §9.3: seed не гарантирует визуального разнообразия; визуальная проверка — не в этом потоке).

## 4. SKIPPED / BLOCKED (честно, не PASS)

| Что | Статус | Причина |
|---|---|---|
| variant.generate (исполнение) | NOT_PLANNABLE | pure planning по заданию; порт отказывает |
| Blender smoke | SKIPPED | нет вызовов Blender в потоке |
| Реальные SHA-256 payload источников | SUBSTITUTED-FIXTURE | Intake-поток даст фактические hashes; fixture хеширует fixture-контент |
| Полный F9 evidence (эффективный результат через зависимости) | DELEGATED | GLM-5/GLM-6; здесь только planning-аналог |
| Риги/клипы/корабль (§11.2/11.3) | SKIPPED | вне V0–V1 static; только FOREIGN_SKELETON/sockets аналоги |
| Wire-валидация по JSON Schema (check_schemas.py GLM-1) | SKIPPED | схемы GLM-1 в чужом worktree; read-only ориентир, мой wire-парсер проверяется тестами |

## 5. Что должен перепроверить ревьюер

1. Повторить `cargo test` и `cargo run --quiet -- fixtures` — byte-identical план.
2. Арифметику 1944/720 и состав исключений (см. `combination_engine_counts_and_filters`).
3. Seed derivation против INTEGRATION_NOTES.md §5.1.
