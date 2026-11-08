# TEST EVIDENCE — GLM-2 | Asset Intake / Preservation PoC

Дата прогона: 2026-11-08. Всё выполнено локально на этой машине; никаких внешних
сервисов, сети при тестировании и Provider SDK не использовалось.

## Окружение (реальные версии)

| Компонент | Версия |
|---|---|
| База | `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D), ветка `feat/adr5-intake` |
| rustc / cargo | 1.96.0 (30a34c682 / ac68faa20 2026-05-25) |
| OS | Windows 10.0.26100 x64, Git Bash |
| Crate | `michelangelo_asset_intake` 0.1.0, standalone (`adr5-lanes/intake/`, свой `Cargo.lock`) |
| Зависимости | serde 1.0.229, serde_json 1.0.151, tempfile 3.27.0 (dev), path-dep `michelangelo_input` (read-only, in-repo SHA-256) |

## Команды и фактические результаты

```
$ cargo test                                    # в adr5-lanes/intake/
running 8 tests   ... test result: ok. 8 passed; 0 failed; 0 ignored    (unit, src/*)
running 27 tests  ... test result: ok. 27 passed; 0 failed; 0 ignored   (integration, tests/intake.rs)
running 0 tests   ... test result: ok. 0 passed; 0 failed; 0 ignored    (doc-tests)
```

Итого **35 passed / 0 failed / 0 ignored**.

```
$ cargo clippy --all-targets   → 0 warnings (после исправлений; финальный счётчик: 0)
$ cargo fmt --check            → clean
```

## Unit-тесты (8) — PASS

| Тест | Что фиксирует |
|---|---|
| `base64::tests::rfc4648_vectors` | декодер RFC 4648 (эталонные векторы) |
| `base64::tests::rejects_invalid` | неканонические паддинги/биты/символы отклоняются |
| `pathsafe::tests::normalization_rejects_escape` | `..`-escape → Traversal |
| `pathsafe::tests::absolute_and_network_forms` | `/...`, `C:\...`, UNC, `https:`/`file:` распознаются; относительные — нет |
| `pathsafe::tests::flatten_is_stable` | детерминированное имя хранения |
| `obj::tests::extracts_mtllib_and_maps` | mtllib + map_Kd/map_Ks/bump с опциями (-clamp, -bm) |
| `obj::tests::lossy_encoding_flags_ambiguous_filename` | не-UTF-8 текст → warning; битое имя → AmbiguousEncoding (error) |
| `license::tests::unknown_license_is_never_a_permission` | unknown ≠ permission, решение UnknownNoPermission |

## Интеграционные тесты (27) — PASS

Позитивные / smoke:

| Тест | Фиксирует |
|---|---|
| `smoke_glb_full_pipeline` | Полный проход GLB: план → 2 зависимости (BIN chunk + embedded image view с хешем байтов PNG) → execute → снимок на диске (manifest/original/dependencies), повторная проверка хешей копий, round-trip манифеста. **Local smoke на реальном файле GLB на диске** |
| `passthrough_export_is_byte_identical_and_collision_safe` | byte-identical passthrough, пустой loss report, 3 файла, no-clobber повтор → `OutputExists`, `convert()` → `ConversionNotImplemented` |
| `happy_gltf_with_external_dependencies` | .gltf + внешние .bin/текстура; роли buffer/image; имена хранения |
| `happy_gltf_with_data_uri_buffer` | data: URI → EmbeddedDataUri, декодированные байты == оригинальному .bin |
| `happy_glb_with_data_uri_buffer_instead_of_bin` | GLB без BIN chunk, buffer как data: URI |
| `happy_obj_with_mtl_and_texture` | OBJ → mtllib → MTL → текстура; роли material_library/texture_map; только info-диагностика по геометрии |
| `identity_is_content_derived_and_portable` | одинаковый контент в разных каталогах → одинаковый identity; другой контент → другой identity, старый снапшот не тронут |
| `execute_is_idempotent_by_identity` | повторный execute → reused=true, тот же каталог |
| `provisional_dto_view_maps_the_manifest` | DTO seam маппит манифест |
| `negative_symlink_escape` (не-unix ветка) | помечен SKIPPED-замечанием для Windows: проверяется обычный happy-path; негативный symlink-тест выполняется только на unix (ниже) |

Негативные (каждый — диагностируемый отказ, не PASS-маска):

| Тест | Ожидаемый отказ (фактически получен) |
|---|---|
| `negative_missing_texture` | PlanRejected: `missing_dependency` |
| `negative_path_traversal` (`../escape.bin`) | PlanRejected: `path_traversal` |
| `negative_absolute_references` (`/etc/passwd`, `C:/Windows/abs.png`) | PlanRejected: `absolute_reference` |
| `negative_network_references` (`https:`, `file:`) | PlanRejected: `network_reference` |
| `negative_percent_encoded_reference` | PlanRejected: `percent_encoded_reference` |
| `negative_symlink_escape` (unix) | PlanRejected: `symlink_escape` — **на Windows SKIPPED** (см. ниже) |
| `negative_truncated_glb` | PlanRejected: `truncated_glb` |
| `negative_glb_declared_length_mismatch` | PlanRejected: `truncated_glb` (заявленная длина > факта) |
| `negative_glb_magic_mismatch` | `UnsupportedFormat` (`.glb` без magic) |
| `negative_dependency_count_limit` | PlanRejected: `too_many_dependencies` |
| `negative_dependency_size_limit` | PlanRejected: `dependency_too_large` |
| `negative_unsupported_blend_is_explicit` | `UnsupportedFormat`, причина называет `.blend`; снапшот-каталог НЕ создан |
| `negative_remote_source_uri_is_never_fetched` | `Io` (URL как путь не скачивается) |
| `negative_required_extension_unsupported` | PlanRejected: `unsupported_required_extension` |

Ключевые контракты устойчивости:

| Тест | Фиксирует |
|---|---|
| `dependency_substitution_after_planning_is_input_changed` | подмена зависимости ИЛИ источника после планирования → `InputChanged` с ожидаемым/фактическим SHA-256 — **не ложный успех** |
| `tampered_existing_snapshot_is_a_conflict_not_a_success` | битый manifest → `SnapshotDirConflict`; битая копия зависимости → `SnapshotCorrupt`; после восстановления → reuse |
| `path_collision_between_distinct_references_is_a_warning` | `textures/a.png` и `./textures/./a.png` → `path_collision` warning, оба пина, снапшот создаётся |
| `unknown_license_is_recorded_and_never_a_permission` | unknown → warning + `unknown_no_permission` в манифесте; declared CC-BY-4.0 → `declared_not_evaluated`; в манифесте нет `"permitted"` |

## SKIPPED / не выполнено (не PASS)

1. **Windows junction escape (негативный тест)** — SKIPPED на этой платформе:
   создание junction требует привилегий, которых у тестовой среды может не быть.
   На unix соответствующий тест `negative_symlink_escape` реализован (`#[cfg(unix)]`)
   и должен выполняться в CI на Linux. Containment-логика (canonicalize + префикс-сравнение,
   strip `\\?\`) покрыта unit-тестами формы ссылок и общим кодом резолва.
2. **Внешняя валидация GLB-фикстуры** — smoke выполняется собственным сканером crate
   над валидным минимальным GLB, собранным кодом фикстуры (корректный заголовок, чанки,
   JSON glTF 2.0, PNG). Валидация внешним инструментом (glTF-валидатор, Blender import)
   не запускалась — это зона GLM-5/интегратора, не поток intake. Заявление «import-safe GLB»
   из этого не следует.
3. **Произвольный .blend, remote URI** — не поддерживаются по заданию (явные ошибки есть,
   тесты фиксируют отказ).
4. **Реальные пользовательские ассеты** — не использовались; все фикстуры синтетические
   в disposable temp-каталогах, исходники проекта не изменялись.

## Read-only regression-анализ IT-8D (база b861547)

Коммит b861547 (corrective pass IT-8D, docs/tz/TZ_IT8_ACCEPTANCE_INTEGRITY.md §12) закрыл
дыры accept-integrity в reconstruction/edit-цепочке. Read-only сверка показала, что новые
пути intake воспроизводят его инварианты на своём уровне, не дублируя и не ослабляя
действующий accept/eval:

| Инвариант IT-8D | Как отражён в intake PoC | Тест |
|---|---|---|
| D1.2 фактические входы: accept перечитывает и перехеширует реальные байты; изменение PNG после оценки → `INPUT_CHANGED`, не pass | `execute_plan` перечитывает source и каждую внешнюю зависимость и сверяет с пинами плана; подмена → `IntakeError::InputChanged` | `dependency_substitution_after_planning_is_input_changed` |
| D1.2 хеши реально прочитанных байт, а не деклараций манифеста | все SHA-256 считаются над прочитанными байтами (source, deps, data: URI payload, BIN chunk, bufferView-срезы) | `smoke_glb_full_pipeline` (срез изображения = хеш PNG) |
| D1.3 атомарное резервирование / no-overwrite | каталог снапшота создаётся `create_dir` (ошибка при существовании); reuse только после полной проверки; export — no-clobber | `tampered_existing_snapshot_is_a_conflict_not_a_success`, `passthrough_export_...` |
| D1.4 полная проверка bundle до commit | перед записью перечитываются и верифицируются все входы; после записи каждая копия перечитывается и сверяется (`write_verified`); export верифицирует снапшот до копирования | `smoke...`, `passthrough_export...` |
| Retry ≠ новый approval; recovery только при совпадении полного результата | reuse снапшота допускается только при совпадении identity и целостности всех копий; повреждённый снапшот не переиспользуется | `tampered_existing_snapshot_...` |
| Не ослаблять действующий accept/eval | crate не трогает `michelangelo_edit/eval/evidence`, не вводит accept/reject; никаких публикаций | (архитектурно; git diff ограничен новыми файлами) |

Изменений в существующих крейтах, корневом Cargo.toml/Cargo.lock, IT-8D тестах — нет
(`git status` до коммита: только новые файлы в `adr5-lanes/intake/` и
`docs/adr5-streams/intake/`).
