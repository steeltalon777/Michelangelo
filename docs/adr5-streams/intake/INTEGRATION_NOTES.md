# INTEGRATION NOTES — GLM-2 | Asset Intake PoC → общая интеграция (раунд 2)

Для интегратора / раунда 2 (сведение к контракту GLM-1) и негативной приёмки DeepSeek.
Ничего из перечисленного не применялось к общим файлам — только предложения.

## 1. Adapter seam под контракт GLM-1

Контракт GLM-1 в момент работы отсутствовал. Реализован **private internal DTO** с явной
пометкой provisional и единой точкой адаптации:

- `src/snapshot.rs` — `SnapshotManifest`, schema `michelangelo.asset-source-snapshot/draft-0`,
  поле `provisional: true`. Это хранимый документ снимка, НЕ wire protocol.
- `src/dto.rs` — `SourceSnapshotView` / `IntakeReportView`
  (schema `provisional/intake-source-snapshot/0`, `provisional/intake-report/0`) + функции
  `source_snapshot_view(manifest)` / `intake_report_view_from_rejection(rejection)`.
  При появлении контракта GLM-1 меняются **только** структуры и мапперы этого модуля.
- Соответствие ADR 0005 §5.2 (SourceSnapshot): identity → `snapshot_id`; формат → `format`;
  payload с фактическим SHA-256 → `payload`; зависимости → `dependencies[]`
  (declared_ref/role/acquisition/stored_name/sha256/size); происхождение → `provenance`;
  лицензия → `license_decision` (`unknown_no_permission` | `declared_not_evaluated`);
  неоднозначности → `unresolved_issues` (диагностики). Поля, которых ждёт ADR, но которых
  ещё нет: единицы/оси (не извлекаются PoC — нужен glTF `units`/axes-контракт от GLM-1),
  parent lineage (вне intake).

Вопросы к GLM-1 для раунда 2:
1. Имена и форма: `dependency_kind` сейчас JSON-tagged enum с абсолютными оффсетами
   (`glb_bin_chunk{byte_offset,byte_length}` и т.п.) — оставить в контракте или абстрагировать?
2. Статусы: `ReportStatus { clean, with_warnings, rejected }` — provisional; не пересекать с
   IT-8 enum и не превращать в pass/unavailable-семантику.
3. `identity` (SHA-256 канонической строки формат+source+sorted deps) — нормализовать алгоритм
   в контракте или оставить приватным?

## 2. Физическое размещение (proposed, НЕ применено)

Фактический путь: `adr5-lanes/intake/` — standalone crate вне корневого workspace
(пустая секция `[workspace]` в локальном Cargo.toml, собственный Cargo.lock).
Корневые `Cargo.toml`/`Cargo.lock` не изменялись.

Предлагаемые интеграционные варианты (решение владельца):
- **Вариант A (минимальный diff):** оставить `adr5-lanes/intake/` как изолированный пакет;
  CLI-интеграция позже отдельным патчем с path-зависимостью.
- **Вариант B (proposed diff к корню, для интегратора):** добавить в root `Cargo.toml`
  в `[workspace.members]` строку `"adr5-lanes/intake"` и удалить пустую `[workspace]`
  секцию из `adr5-lanes/intake/Cargo.toml`. Других правок корня не требуется
  (deps `serde`/`serde_json` уже в workspace; `tempfile` уже dev-dep elsewhere).
- При переносе в `crates/michelangelo_asset_intake` — не забыть поправить path-dep на
  `michelangelo_input` на `"../michelangelo_input"`.

## 3. Точки стыка с остальными потоками (ничего не менять с их стороны сейчас)

| Поток | Стык |
|---|---|
| GLM-1 Contracts | настоящий файл, §1 — предлагаю принять `SnapshotManifest` как черновик SourceSnapshot |
| GLM-3 Batch | `IntakePlan` + `identity` — кандидат на вход item-а батча; plan/execute двухфазно согласуется с attempt-семантикой |
| GLM-4 Family | `SnapshotManifest` — неизменяемый источник для SourceSet-ролей; hash-pinning уже обеспечен |
| GLM-5 Blender | smoke-валидация GLB-фикстуры импортом в Blender (headless) — то, что intake сознательно не делал; контракт на `<workspace>/snapshots/<id>/` |
| GLM-6 Validation | `verify_snapshot_integrity()` — переиспользуемая проверка целостности входов профиля |

## 4. Известные ограничения / риски для независимой приёмки (DeepSeek)

1. **Конкурентный intake одной identity** из двух процессов: no-clobber есть
   (`create_dir` атомарен), сериализации нет — один из процессов получит `SnapshotDirConflict`
   или reuse. Для батча с параллельными items этого достаточно, но формулировку нужно
   подтвердить на уровне Batch (GLM-3).
2. **TOCTOU окно** между проверкой хеша зависимости и записью копии в снапшот:
   копия берётся из уже прочитанных и верифицированных байтов, так что снимок консистентен;
   но исходник на диске может измениться после чтения — это ловится повторным
   `plan_intake` (новая identity), не маскируется.
3. **Percent-encoded ссылки отклоняются консервативно** — часть легитимных ассетов
   с пробелами в путях будет отвергнута. Решение о декодировании (с повторной
   проверкой escape) — отдельное согласованное изменение, не одностороннее.
4. **Лимиты фиксированы в манифесте**, но не в identity: одинаковый контент при разных
   лимитах даёт одинаковый identity (лимиты не влияют на байты). Считаю это корректным;
   спорно — вынести лимиты в identity, если контракт потребует.
5. **OBJ**: геометрия не разбирается и не валидируется (инфо-диагностика `obj_parse_note`);
   ссылки могут быть неполными, если MTL ссылается дальше (вложенные mtllib) — не поддержано.
6. **GLB**: чанки после JSON допустимы только BIN; JSON-чанк обязан быть первым;
   более одного buffer без uri — ошибка. Экзотика (multiple scenes, skins, спарс-аксессуары)
   не разбирается — intake не читает ничего, кроме buffers/images/extensions, это осознанно.
7. **Windows**: `symlink_escape`-тест выполняется только на unix; junction-покрытие —
   через общий canonicalize-код. Негативный Windows-тест — желательный довесок для CI.
8. **serde-модель glTF игнорирует неизвестные поля** — осознанный компромисс для intake
   (не валидатор). Полная schema-валидация — зона Validation (GLM-6) при необходимости.

## 5. Что должно быть проверено приёмкой (сводка)

- Honest PASS/SKIP: `negative_symlink_escape` на Windows — SKIPPED, не PASS (см. TEST_EVIDENCE.md §SKIPPED).
- Отсутствие скрытой модификации источника: единственные записи — в disposable workspace
  (`snapshots/`, `passthrough/`); `convert()` всегда `ConversionNotImplemented`.
- Unknown license ≠ permission: манифест не содержит утверждений о разрешённом использовании.
- Повторное использование снапшота только при полном совпадении identity и целостности байт.
