# TEST_EVIDENCE — GLM-4 Family Builder / Pure Planner V0–V1

- **Дата:** 2026-11-09 (fix-раунд по итогам независимого ревью DSH wave-1). **Toolchain:** cargo/rustc 1.96.0 (stable-x86_64-pc-windows-msvc). **Окружение:** локальный Windows 10 (10.0.26100), standalone crate, zero dependencies.
- Все команды выполнялись из `crates/michelangelo_family` в worktree `I:/prog/michelangelo-adr5-family`, ветка `feat/adr5-family`. Это **не** production-заявление: pure planning PoC, без Blender, без реальных payload.
- **Что изменено в этом раунде:** (L) переход на собственные provisional-теги `michelangelo.family.<doc>/draft-0` + явное поле `provisional: true` (были теги GLM-1 `michelangelo.<doc>/draft-0.1` на документах, схемам GLM-1 не соответствующих) + регенерация фикстур и примера кодом; (K) `cargo fmt` по всему crate — `cargo fmt --check` теперь exit 0. Детали — INTEGRATION_NOTES.md §1a, SCOPE.md.

## 1. Сборка и gate

```
cargo build                    # exit 0
cargo test                     # exit 0 — 59 passed / 0 failed (7 lib + 41 negative + 11 unit)
cargo fmt --check              # exit 0   (было exit 1: ~126 мест rustfmt)
cargo clippy --all-targets -- -D warnings   # exit 0, 0 warnings
```

Корневой workspace не затронут: crate собирается standalone (`[workspace]` в локальном Cargo.toml); корневые `Cargo.toml`/`Cargo.lock` не изменялись (см. `git status` ниже).

## 2. Тесты (unit + негативные) — фактический прогон 2026-11-09

Команда: `CARGO_NET_OFFLINE=true cargo test`

```
running 7 tests                                   # lib unit tests (sha256, json)
test result: ok. 7 passed; 0 failed; 0 ignored
running 41 tests                                  # tests/negative_tests.rs
test result: ok. 41 passed; 0 failed; 0 ignored
running 11 tests                                  # tests/unit_tests.rs
test result: ok. 11 passed; 0 failed; 0 ignored
running 0 tests                                   # doc-tests
test result: ok. 0 passed; 0 failed; 0 ignored
```

**Итого 59 passed / 0 failed** (было 54; +5 — покрытие finding L, см. §2.1). Ни один негатив не помечен PASS; каждый проверяет точный код отказа (`FamilyErrorCode`).

### 2.1 Новое покрытие: provisional-теги и маркер (finding L)

- **Позитив (`unit_tests`):** все пять констант `WIRE_SCHEMA_*` лежат в приватном пространстве `michelangelo.family.*` и оканчиваются `/draft-0` (ни одна не содержит `/draft-0.1`); каждый выдаваемый документ (`source_set`, `family_recipe`, `recipe_lock`, `variant_request`, `batch_plan`) несёт `provisional: true`; `lock.digest() == sha256(canonical(lock.to_json()))` — digest покрывает ровно публикуемые байты.
- **Негативы (`negative_tests`, 4 новых):** GLM-1-теги (`michelangelo.source-set/draft-0.1`, `michelangelo.family-recipe/draft-0.1`, `michelangelo.recipe-lock/draft-0.1`) отвергаются на wire с `SCHEMA_INVALID` («schema constant mismatch»); документ без маркера `provisional` — `SCHEMA_INVALID`; `provisional: false` — `SCHEMA_INVALID` (противоречит provisional-тегу); `provisional: "yes"` (не булево) — `SCHEMA_INVALID`.

### Ключевые позитивные проверки (11)
- SHA-256: NIST векторы (в т.ч. 1,000,000 × 'a' — перекрёстно сверен с Python hashlib и Windows certutil: `cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0`).
- Seed derivation: детерминизм, чувствительность к item_id и lock digest, порядок-независимость, 53-бит JSON-safe маска.
- Комбинационный движок: произведение 1944, валидных 720 (арифметика fixture задокументирована), парные/сокетные/высотные фильтры.
- Sampling: 15 детерминированных, попарно различных requests; валидны против рецепта.
- Dry plan: побайтово идентичен при повторном планировании; 15 приватных непересекающихся output namespace; fingerprints различаются по параметрам.
- Round-trip: committed fixtures на диске == кодовые DTO (защита от дрейфа) и committed `dry_plan_15.json` == канонический вывод планировщика.
- Wire round-trip VariantRequest.
- Provisional namespace + маркер (см. §2.1).

### Негативы (41) — по матрице GLM-1
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
- PROVISIONAL-теги: GLM-1-теги на wire отвергаются; отсутствие/ложность/не-булево маркера `provisional` отвергаются (см. §2.1).

## 3. Dry plan на 15 requests — фактический прогон 2026-11-09

```
cargo run --quiet -- --regen fixtures          # запись wire-fixture документов (3 файла)
cargo run --quiet -- fixtures > docs/adr5-streams/family/examples/dry_plan_15.json
# stderr: dry plan ok: 15 items, recipe urban-house-family@1, lock digest 765a3541...
#         executor port refused (expected): NOT_PLANNABLE: ...
```

Регенерация выполнена **кодом** (не руками): теги и маркер берутся из `WIRE_SCHEMA_*`, digest'ы считаются из содержимого фикстур.

Проверка уникальности (Python поверх committed JSON):

```
plan items: 15
unique ids: 15
unique param sets: 15
unique seeds: 15
seeds 53-bit safe: True
```

**Детерминизм:** два последовательных прогона планировщика дают побайтово идентичный вывод (`cmp` — без расхождений), и он совпадает с committed `examples/dry_plan_15.json`.

**sha256 перегенерированного примера** (`docs/adr5-streams/family/examples/dry_plan_15.json`, 10571 байт, LF-нормализация как в git-блобе):

```
31b4cb74ec68f2ab1adb473f9084392d70cf9c7f71e8031d693a582287ce2cca
```

(до фикса тегов было `c44227fd6387d4539f65734088f1ef18b15ed5910fad737fb33de4a44effa663` — изменился, потому что тег и маркер входят в хешируемый canonical JSON.)

Новые digest'ы (все проверены на согласованность тестом round-trip):

```
source_set_sha256   ea514a75a1b2654fec3094fe82ec058c1e00129bd97245e973a6a008b5321c8f
recipe_sha256       dc4b38de87ea4d15dd142212982937d55b37adc288e9297a8e9d18791c9a8132
recipe_lock_digest  765a35415a16d5fdce4728cb7fdbce811ae2b002ffe817dc3f2f7b8f4974be5d
```

Примеры item (полный документ — `examples/dry_plan_15.json`, canonical JSON одной строкой):

```
house-000  {bays:2, floors:1, palette:brick_dark,  roof:gable, trim:none,  wear_permille:"0"}   seed 3534138763290489
house-001  {bays:2, floors:1, palette:plaster_warm, roof:gable, trim:carved, wear_permille:"0"} seed 5938223322133826
house-002  {bays:2, floors:1, palette:timber_grey, roof:gable, trim:iron,   wear_permille:"0"} seed 5839328532396979
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
| Wire-валидация по JSON Schema (check_schemas.py GLM-1) | SKIPPED (по замыслу) | документы потока **намеренно** не проходят схемы GLM-1: они несут собственные provisional-теги `michelangelo.family.*/draft-0` + `provisional: true` и не являются GLM-1-артефактами. Схемы GLM-1 живут в чужом worktree (read-only ориентир); мой wire-парсер проверяется своими тестами |
| Фиксация provisional-конвенции в контракте | DELEGATED | раунд 2: перенос тегов в утверждённое пространство GLM-1 (или расширение схем) — INTEGRATION_NOTES.md §1a/§3.0 |

## 5. Что должен перепроверить ревьюер

1. Повторить `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` и `cargo run --quiet -- fixtures` — byte-identical план, exit 0 по всем трём gate.
2. Проверить, что ни один документ потока не несёт тег `michelangelo.*/draft-0.1` и что все несут `provisional: true` (INTEGRATION_NOTES.md §1a).
3. Арифметику 1944/720 и состав исключений (см. `combination_engine_counts_and_filters`).
4. Seed derivation против INTEGRATION_NOTES.md §5.1 (внимание: `recipe_lock_digest` изменился из-за смены тега — энтропийный якорь прежний, значение новое).
5. Что dry plan остался побайтово детерминированным: два прогона — `cmp` без расхождений; sha256 примера `31b4cb74ec68f2ab1adb473f9084392d70cf9c7f71e8031d693a582287ce2cca`.
