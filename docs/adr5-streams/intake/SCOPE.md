# SCOPE — GLM-2 | Asset Intake / Preservation PoC (ADR 0005, stream A)

- Дата: 2026-11-08. База: `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- Ветка: `feat/adr5-intake` (worktree `I:/prog/michelangelo-adr5-intake`). Merge/push не выполнялись.
- Статус: **выполнено в заявленном ограниченном объёме** (см. «Статус реализации»); это PoC, не production.

## 1. Фактический путь владения

Предложенное имя `michelangelo_asset_intake` реализовано как **standalone crate вне корневого
workspace** (корневой `Cargo.toml`/`Cargo.lock` не изменялись):

```
adr5-lanes/intake/            # фактический путь (зафиксирован здесь)
├── Cargo.toml                # пакет michelangelo_asset_intake 0.1.0, edition 2021
│                             # содержит пустую секцию [workspace] — сознательный выход
│                             # из корневого workspace; собственный Cargo.lock
├── src/*.rs                  # 14 модулей (см. ниже)
├── tests/{intake.rs, common/mod.rs}
└── .gitignore                # /target
docs/adr5-streams/intake/     # этот документ, TEST_EVIDENCE.md, INTEGRATION_NOTES.md
```

Зависимости: `serde`, `serde_json`, `tempfile` (dev) + **read-only path-dependency на
`crates/michelangelo_input`** (переиспользование аудированной in-repo SHA-256 FIPS 180-4;
сам крейт не изменялся). Внешних API, Provider SDK, сети — нет.

## 2. Входы (поддерживаются явно)

| Формат | Что принимается |
|---|---|
| GLB | glTF 2.0 binary: magic, version 2, chunk-структура, встроенный BIN chunk, изображения в bufferView внутри BIN, `data:` URI |
| GLTF | plain `.gltf` c **относительными локальными** зависимостями: внешние `.bin`, изображения, `data:` URI |
| OBJ | текст + `mtllib` → MTL → текстурные map-ссылки (`map_Kd` и др.) на локальные файлы |

Явно НЕ поддерживаются и отклоняются отдельной ошибкой: произвольный `.blend`
(gate ADR 0005 §13.2 остаётся открытым), remote/network URI как источник и как
зависимость (никогда не скачиваются), абсолютные пути, `..`-traversal,
percent-encoding (консервативно), symlink/junction escape за пределы каталога
источника, `extensionsRequired` вне поддерживаемого набора, расширения,
меняющие кодирование payload буферов.

## 3. Выходы

1. `IntakePlan` — план с фактическими SHA-256 всех прочитанных байт (source + closure),
   диагностиками, лимитами, лицензионным решением. Контент-производный `identity`
   (SHA-256 над канонической строкой: формат + hash источника + сортированные
   зависимости; без путей/времени/версий).
2. `SnapshotManifest` (`michelangelo.asset-source-snapshot/draft-0`, `provisional: true`)
   на диске: `<workspace>/snapshots/<identity>/{manifest.json, original/, dependencies/}`.
   Оригинал копируется байт-в-байт, ничего не конвертируется.
3. `PassthroughExport` (`michelangelo.asset-passthrough-export/draft-0`) — byte-for-byte
   копия проверенного снапшота, no-clobber, с пустым (явным) loss report.
4. Provisional DTO views (`src/dto.rs`) — seam под будущий контракт GLM-1.

## 4. Гарантии, реализованные и покрытые тестами

- Хеши считаются по **фактически прочитанным байтам**, не по декларациям (инвариант IT-8D D1.2).
- Подмена зависимости/источника после планирования → `InputChanged`, не ложный успех (D1.2).
- Резервирование каталога снапшота атомарным `create_dir` (no-overwrite), reuse только при
  полной проверке целостности существующего снапшота; повреждённый снапшот → `SnapshotDirConflict`/
  `SnapshotCorrupt`, никогда не перезапись (аналог D1.3/D1.4 на уровне intake).
- Идемпотентность по content identity; разный контент → другой каталог, старый не трогается.
- Unknown license → warning + решение `unknown_no_permission`; декларированная лицензия →
  `declared_not_evaluated` (термины SPDX здесь не интерпретируются). Коммерческое разрешение
  из неизвестной лицензии не выводится никогда.
- Диагностика: missing texture, path traversal, absolute path, network URI,
  percent-encoding, symlink escape, path collision (warning), truncation GLB
  (заявленная длина/чанки), превышение лимитов (размер источника/зависимости/сумма/количество/длина ссылки),
  несоответствие magic, неподдерживаемые расширения, неоднозначная кодировка текстовых ссылок.
- Никаких скрытых remesh/retopo/decimation: единственный API `convert()` возвращает
  `ConversionNotImplemented`; passthrough — только байтовая копия с проверкой.
- `#![forbid(unsafe_code)]`.

## 5. Допущения

- Source — локальный regular file; «сведения об источнике» (supplier/external_id/license/notes)
  поставляются вызывающей стороной в `IntakeRequest` (нет каталога пользователей и нет
  автоопределения лицензии).
- Snapshot пишется в disposable workspace, выделенный вызывающей стороной; cleanup — её зона.
- Лимиты по умолчанию консервативны (источник 512 MiB, зависимость 256 MiB, сумма 1 GiB,
  4096 зависимостей) и фиксируются в манифесте.
- Одна attempt на identity: конкурентная запись одного и того же снапшота из двух процессов
  не сериализуется межпроцессной блокировкой (в ADR-терминах это работа уровня Batch/Executor);
  no-clobber гарантирует отсутствие перезаписи, но не атомарную координацию.

## 6. Статус реализации (честно)

| Пункт задания | Статус |
|---|---|
| GLB/GLTF/OBJ intake + явное отклонение прочего | DONE (тесты) |
| Immutable snapshot + closure + SHA-256 + provenance/license + ошибки | DONE (тесты) |
| Диагностика (missing/traversal/absolute/network/symlink/collision/truncation/limits) | DONE (тесты) |
| INPUT_CHANGED при подмене после планирования | DONE (тест) |
| Идемпотентность + collision-safe target | DONE (тесты) |
| Passthrough экспорт с проверкой + явный loss report | DONE (тесты) |
| Conversion | NOT IMPLEMENTED (явно: `ConversionNotImplemented`) |
| Local smoke на реальном GLB | DONE на **синтетическом минимальном GLB**, собранном фикстурой кода (валидный контейнер glTF 2.0); валидация внешним вьюером/Blender НЕ выполнялась — вне зоны потока, см. TEST_EVIDENCE.md |
| Windows junction escape (негативный тест) | SKIPPED на Windows (нужны привилегии на junction); unix-вариант с `symlink(2)` реализован как `#[cfg(unix)]`; логика containment покрыта каноническим префикс-сравнением |
| Интеграция Asset Library / approve-publish / каталог | OUT OF SCOPE (запрещено заданием) |
| Произвольный `.blend` | NOT SUPPORTED (явная ошибка) |

## 7. Блокеры / открытые вопросы для интегратора

1. Контракт GLM-1 (SourceSnapshot/IntakeReport wire DTO) — текущий манифест provisional;
   адаптация — одна точка `src/dto.rs` + `INTEGRATION_NOTES.md`.
2. Место standalone-крейта при интеграции: `adr5-lanes/intake/` вне workspace или перенос
   в `crates/` с изменением корневого `Cargo.toml` — решение владельца (proposed diff в
   INTEGRATION_NOTES.md).
3. Межпроцессная координация конкурентного intake одной identity — не в этом потоке.
