# TEST EVIDENCE — GLM-6 Asset Validation (feat/adr5-validation)

- **Дата:** 2026-11-09 (fix-раунд поверх `592da2f`). **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556`.
- Все команды выполнялись в `I:/prog/michelangelo-adr5-validation/michelangelo_asset_validation`
  (standalone crate, offline). Ни один статус ниже не выдуман: вывод команд
  зафиксирован в этом документе.
- Fix-раунд закрывает findings независимого ревью §8: H1 (fmt/clippy gate),
  H2 (`not_required` на required), M1 (live reservation по диску), M2
  (условные проверки/optional), M3 (`deny_unknown_fields`), M4
  (`uv_preservation` без пина/порога).

## 1. Команды и фактические результаты

### 1.1. Полный тестовый прогон (unit + integration)

```
CARGO_NET_OFFLINE=true cargo test --offline
```

Фактический результат (exit 0):

```
running 0 tests  (unittests src/lib.rs)   test result: ok. 0 passed; 0 failed
running 14 tests (tests/adversarial.rs)   test result: ok. 14 passed; 0 failed
running 9 tests  (tests/positive_paths.rs) test result: ok. 9 passed; 0 failed
Doc-tests: test result: ok. 0 passed
```

### 1.2. Статический анализ — полный gate

```
cd michelangelo_asset_validation
CARGO_NET_OFFLINE=true cargo test --offline                 -> exit 0
cargo fmt --check                                           -> exit 0
cargo clippy --offline --all-targets -- -D warnings         -> exit 0
cargo run --offline --example dump_validation_report        -> exit 0 (SMOKE_OK)
```

Это исправление H1: прошлая запись «clippy -> 0 warnings» относилась к
lib-target и не включала `--all-targets -D warnings`; `cargo fmt --check`
в прошлом раунде не запускался (exit 1). Unused imports
(`examples/dump_validation_report.rs`, `tests/adversarial.rs`) удалены,
весь crate отформатирован; оба gate теперь exit 0.

### 1.3. Локальный smoke

```
cargo run --offline --example dump_validation_report
```

Фактический вывод (сокращённо):

```
report verdict: pass
report digest: dfeff0ac139bb5fd09fd10676eb20c81dc12da3bb8e52c116314076f54f024b5
  check payload_integrity          pass
  check dependency_closure         pass
  check path_containment           pass
  check asset_digest_full          pass
  check budget_geometry            pass         measured 8 triangles; limit le 100000
  check protected_roles            pass         1 protected components match the pinned bytes and effective transforms
  check connector_compatibility    pass         1 pinned connector pairs fit within tolerance
  check minimum_clearance          pass         the minimum entrance separation 4.000000 satisfies ge 1.2
  check maximum_dimension          pass         2 axis extents measured within thresholds
  check material_presence          pass
  check texture_integrity          pass
  check uv_preservation            pass         7 vt lines measured; pinned expectation total 7
  check format_loss_declared       not_required
registered: vr-smoke-0001 ... verified: digest ... verdict pass
second report registered alongside the first (no-clobber)
SMOKE_OK
```

### 1.4. Кросс-валидация против схем GLM-1 (draft-0.1, read-only)

Скрипт в OS TEMP (`python -I`, вне репозитория), схемы
`I:/prog/michelangelo-adr5-contracts/schemas/adr5/*.schema.json` (commit `4597c43`):

```
PROFILE_OK against GLM-1 draft-0.1
REPORT_OK against GLM-1 draft-0.1
verdict: pass
```

Emitted JSON отчёта и профиля проходит соответствующие draft-схемы без правок —
сильный аргумент для раунда 2 (единая спецификация DTO).

## 2. Позитивные пути (tests/positive_paths.rs)

| Тест | Что доказывает |
|---|---|
| `passthrough_profile_a_passes_honestly` | Profile A: payload/dependency/asset-digest по фактическим bytes, source-unchanged pin, declared losses, UV против `expected_uv_counts`-пина (M4: без пина/порога UV даёт unavailable), материалы/текстуры; optional protected_roles → `not_required` с applicability_ref; детерминированный digest; round-trip bytes→parse идентичен |
| `passthrough_with_declared_export_is_unavailable_without_backend` | требуемый export_roundtrip без backend → `unavailable` (BLOCKED), вердикт `unavailable`, не pass |
| `family_static_profile_b_passes` | Profile B: budget, protected roles (bytes + effective transforms), socket fit, clearance, dimensions, materials/textures/UV против пинов; not_required для optional при отсутствии контента |
| `family_rig_compatible_marks_clips_unavailable_without_backend` | rig_bindings против pinned skeleton; clips с верифицированными digest → `unavailable` без плеера (импорт+digest ≠ совместимость) |
| `reconstruction_kind_is_refused_not_reimplemented` | kind=reconstruction → `UNSUPPORTED_PROFILE_KIND` (IT-8D evaluator остаётся владельцем) |
| `unknown_required_check_refuses_with_gate_code` | F8 schema-слой: `ghost_check` → `VALIDATION_UNKNOWN_REQUIRED` |
| `registry_lifecycle_reserve_register_verify` | reservation до вычислений, конфликт reservation, idempotent re-register, append-only индекс |
| `changed_profile_invalidates_the_report_basis` | изменение threshold меняет digest профиля; verify с новым профилем → `VALIDATION_PROFILE_CHANGED` |
| `report_rejects_an_imported_approval_flag` | поле `approved` в acceptance JSON отказывается (deny_unknown_fields) |

## 3. Adversarial матрица (tests/adversarial.rs)

| Тест | F-код | Отказ |
|---|---|---|
| `f1_geometry_and_manifest_substitution_is_refused` | F1 | подмена geometry + «отремонтированный» манифест: payload проходит, `asset_digest_full` → `SNAPSHOT_MISMATCH` против пина до генерации; вердикт fail |
| `f2_input_changed_after_validation_is_detected` | F2 | PNG-swap после регистрации → `VALIDATION_INPUT_CHANGED` с именем файла |
| `f9_parent_transform_moves_protected_child` | F9 | разрешённый parent transform двигает protected child: bytes меша не менялись, effective world transform ≠ пин → `protected_roles` fail |
| `f9_shared_material_change_after_lock_is_refused` | F9/F19 | перекраска shared MTL + repair манифеста → `dependency_closure` fail `DEPENDENCY_CHANGED` (pinned aggregate) |
| `seam_refuses_missing_report_replay_and_stale_basis` | F14/F5/F19 | acceptance без зарегистрированного отчёта → `VALIDATION_REPORT_NOT_REGISTERED`; replay record id на чужой bundle → `ACCEPTANCE_REPLAY`; чужой bundle binding → `bundle_binding_matches=false`; смена профиля → `profile_binding_matches=false` + `VALIDATION_PROFILE_CHANGED`; unavailable-вердикт → `verdict_sufficient=false`; публикация → `ACCEPT_SEAM_NOT_AVAILABLE` |
| `f8_report_missing_required_check_is_refused` | F8 behavioral | отчёт без required-проверки → `MISSING_REQUIRED_CHECK` уже при регистрации; forged entry → тот же отказ при verify с профилем |
| `f16_not_required_and_unavailable_are_never_pass` | F16 | not_required без applicability_ref → `INVALID_NOT_REQUIRED`; противоречащий свёртке verdict → `INVALID_VERDICT_FOLD`; verdict `not_required` не парсится (enum без значения) |
| `f13_path_escape_is_refused` | F13-adj | `../outside.obj` в манифесте → `PATH_ESCAPE` до любого чтения |
| `registry_refuses_overwrite_and_unreserved_registration` | D1.3 | регистрация без reservation → `RESERVATION_REQUIRED`; тот же id с другим digest → `NO_CLOBBER_VIOLATION`; индекс хранит ровно один entry |
| `m1_registration_requires_a_live_on_disk_reservation` | M1 | in-memory `Reservation` (публичные поля) без файла на диске → `RESERVATION_REQUIRED`; live-reservation на другой bundle digest → `RESERVATION_REQUIRED`; реестр пуст |
| `m2_conditional_check_cannot_be_optional_when_condition_is_present` | M2 | `minimum_clearance` в optional при токене `constraints:clearance` → `VALIDATION_APPLICABILITY_CONFLICT`; matrix-required `protected_roles` в optional → конфликт; без токена optional легален |
| `m3_ghost_fields_are_rejected` | M3 | ghost-поле в report / check record / profile → `DOCUMENT_INVALID` (не отбрасывается молча) |
| `m4_uv_preservation_without_pin_or_threshold_is_unavailable` | M4 | vt-строки есть, но нет `expected_uv_counts` пина и `min_uv_count` порога → `unavailable`, вердикт `unavailable`, не pass |
| `h2_required_check_cannot_be_not_required` | H2 | required `payload_integrity: not_required` собирается структурно (verdict pass), но регистрация → `INVALID_NOT_REQUIRED`; forged entry: verify с профилем, verify без пина, verify с pinned-копией и seam (`check_prerequisites`) → `INVALID_NOT_REQUIRED`; accept-факты не выдаются |

## 4. Честные пределы этого evidence

- Все проверки измеряют fixture-уровень (OBJ/MTL + provisional metadata
  JSON). Реальные `.gltf` skins, `.blend` DNA, node groups, PBR — не
  проверяются и не заявляются; соответствующие провероки дают `unavailable`.
- `budget_geometry`/`clearance`/`maximum_dimension` считают OBJ-геометрию;
  bbox-metric clearance — провизорная метрика, не воксельная проверка.
- Smoke-прогон выполняется в disposable OS temp workspace; реальные
  пользовательские ассеты, `F:\AssetsStorage`, Vault, Workshop не
  затрагивались.
- Кросс-валидация проверяет совместимость **emitted documents** с draft-схемами
  GLM-1; она не утверждает контракты (OP-1..OP-7 открыты) и не заменяет
  независимый acceptance-аудит (раунд 3).
