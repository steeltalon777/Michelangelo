# schemas/adr5 — draft JSON Schema для контрактов ADR 0005

- **Статус:** **DRAFT (`draft-0.1`).** Не утверждённый wire-протокол. Существование файла схемы
  не означает существования DTO/CLI/миграций в коде. Согласование — контрактный gate (ADR 0005 §12, этап 0).
- **Владелец:** GLM-1 (contracts lane). База: `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Соглашение об именах schema-тегов:** `michelangelo.<document>/draft-0.1` — по аналогии с
  существующим `michelangelo.input-package/0`. Суффикс `draft-` обязателен до принятия владельцем;
  после принятия тег переводится в `/0` новой ревизией схемы.

## Нормативные правила кодировки и digest (v0.1 draft)

1. **Байты документа:** UTF-8, без BOM, переводы строк LF (`\n`).
2. **Canonical JSON** (когда digest вычисляется по документу): **RFC 8785 (JCS)** — UTF-8,
   ключи объектов отсортированы по кодовым точкам, компактные разделители `,` и `:`,
   shortest round-trip представление чисел. Это осознанное отличие от legacy-контрактов
   (`VERIFICATION_STATE.md` фиксирует порядок полей кодировщиком); legacy digest'ы не
   пересчитываются по JCS. Открытый вопрос владельца: `CONTRACT_GATE.md` §9 OP-3.
3. **sha256:** lowercase hex, 64 символа, по **фактически прочитанным** bytes файла.
   Значение, заявленное в манифесте, не является доказательством: Core обязан пересчитывать
   (правило IT-8D D1.2, ADR §8.2).
4. **`asset_digest`:** SHA-256 над конкатенацией UTF-8 строк
   `<artifact_id>\u0000<path>\u0000<content_kind>\u0000<sha256>\n`, где строки отсортированы
   byte-wise lexicographic по `(artifact_id, path)`. Расширение агрегата geometry-snapshot
   (`<component>\0<sha256>\n`).
5. **`geometry_digest`** (опционален): тот же алгоритм, только артефакты с
   `content_kind == "geometry"`. Не заменяет `asset_digest` и не принимается вместо него.
6. **`dependency_digest`:** SHA-256 над строками `<dep_id>\u0000<revision>\u0000<sha256>\n`,
   отсортированными по `dep_id` (byte-wise).
7. **Сортировка массивов-идентификаторов** (sources, artifacts, dependencies, checks):
   по ключевому полю byte-wise; порядок файлов не определяет приоритет (ADR §5.2).
8. **Path containment:** все пути внутри документов — относительные, `/`-разделённые, без
   начального `/`, без сегмента `..`, без backslash, без drive letter, без NUL (паттерн
   `$defs.relPath`, унаследован от `input_package_v0.schema.json`). Абсолютные/внешние пути
   в wire-документах запрещены; нарушение — `PATH_ESCAPE`.
9. **Иммутабельность:** зарегистрированные документы не перезаписываются (no-clobber);
   новая версия = новый документ/revision. `expected_current_revision` — CAS-семантика
   публикации (см. `CONTRACT_GATE.md` §4).

## Состав (13 схем)

| Файл | Schema-тег | Владелец потока |
|---|---|---|
| `source_snapshot.schema.json` | `michelangelo.asset-source-snapshot/draft-0.1` | GLM-2 |
| `preparation_plan.schema.json` | `michelangelo.preparation-plan/draft-0.1` | GLM-2 |
| `asset_revision.schema.json` | `michelangelo.asset-revision/draft-0.1` | GLM-2 (produce), GLM-4 (reference) |
| `source_set.schema.json` | `michelangelo.source-set/draft-0.1` | GLM-4 |
| `family_recipe.schema.json` | `michelangelo.family-recipe/draft-0.1` | GLM-4 |
| `recipe_lock.schema.json` | `michelangelo.recipe-lock/draft-0.1` | GLM-4 |
| `variant_request.schema.json` | `michelangelo.variant-request/draft-0.1` | GLM-4 (produce), GLM-3 (consume) |
| `batch_plan.schema.json` | `michelangelo.batch-plan/draft-0.1` | GLM-3 |
| `batch_report.schema.json` | `michelangelo.batch-report/draft-0.1` | GLM-3 |
| `candidate_bundle.schema.json` | `michelangelo.candidate-asset-bundle/draft-0.1` | GLM-3/GLM-5 (produce), GLM-6 (consume) |
| `validation_profile.schema.json` | `michelangelo.validation-profile/draft-0.1` | GLM-6 |
| `validation_report.schema.json` | `michelangelo.validation-report/draft-0.1` | GLM-6 |
| `acceptance_record.schema.json` | `michelangelo.acceptance-record/draft-0.1` | GLM-6 / Core-side publisher |

`examples/positive/` — минимальные валидные экземпляры для schema-проверки;
`examples/negative/` — fixtures, отвергаемые **схемой** (часть негативных контрактов —
поведенческие и проверяется исполнителем потока, см. `docs/adr5-contracts/NEGATIVE_TEST_MATRIX.md`).

## Проверка схем

```bash
python docs/adr5-streams/contracts/tools/check_schemas.py   # требует python + jsonschema >= 4.18
```

Скрипт валидирует каждую схему meta-схемой draft 2020-12, затем прогоняет все
positive-примеры (обязаны проходить) и negative-примеры (обязаны падать).
