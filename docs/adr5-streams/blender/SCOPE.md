# SCOPE — поток GLM-5 (Blender) ADR 0005, PoC V0–V1

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Ветка:** `feat/adr5-blender`, worktree `I:/prog/michelangelo-adr5-blender`.
- **Статус:** PoC выполнен на реальном Blender 5.2.3 LTS (headless); ничего не заявляется
  production-ready / UE-ready. Контракты GLM-1 (`draft-0.1`) использованы как ориентир, не как
  утверждённый wire.

## 1. Зона владения (фактически созданные пути)

Все новые файлы — в **новой изолированной директории** `adr5_blender_poc/` (единоличное владение
GLM-5 в этом worktree) и в `docs/adr5-streams/blender/`.

| Путь | Назначение |
|---|---|
| `adr5_blender_poc/runner/` | PoC-оркестратор зарегистрированных шаблонов поверх существующего Worker'а: `errors.py` (адресные коды), `paths.py` (F13-инварианты, sha256), `template_registry.py` (пиннинг байтов, F10/F9-валидация), `parameter_gate.py` (типизированные параметры, палитры), `socket_check.py` (сокеты V1 + плановые bounds/counts), `plan_builder.py` (сборка job-spec из шаблона), `glb_report.py` (парсер GLB: фактические имена/геометрия/UV/материалы), `resource_monitor.py` (tasklist-guard, RAM/VRAM телеметрия), `worker_client.py` (headless запуск СУЩЕСТВУЮЩЕГО worker'а), `template_executor.py` (порт `template.execute` PoC), `bundle_writer.py` (CandidateAssetBundle draft-0.1) |
| `adr5_blender_poc/templates/` | Зарегистрированные шаблоны (data-only JSON) + `registry.json` с SHA-256 каждого файла |
| `adr5_blender_poc/fixtures/` | Source-фикстуры `.glb`/`.blend` + `FIXTURES_MANIFEST.json` (пиннинг) + `make_fixtures.py` (генерация через существующий worker.py) |
| `adr5_blender_poc/tests/` | 45 тестов: pure unit/negative + Blender smoke (честный SKIPPED без Blender) |
| `adr5_blender_poc/run_smoke.py` | Драйвер полного smoke: 6 позитивов + 7 негативов, пишет evidence |
| `adr5_blender_poc/evidence/` | Фактические результаты: bundle.json + variant.glb/blend + evidence_summary.json |
| `docs/adr5-streams/blender/` | SCOPE.md, TEST_EVIDENCE.md, CAPABILITY_MATRIX.md, INTEGRATION_NOTES.md |

## 2. Что НЕ изменено (границы соблюдены)

- `blender/worker.py` — **не изменён** (SHA-256 `9c7c24f3…96ee3`); PoC запускает его как есть,
  все варианты исполнены на существующих 18 операциях (`SAFE_WORKER_OPS`).
- `docs/specs/BLENDER_WORKER.md`, корневые `Cargo.toml`/`Cargo.lock`, crates/, CLI dispatcher,
  SQLite-схема — не тронуты. `git status` в коммите показывает только новые файлы своей зоны.
- Контрактный gate GLM-1 (`I:/prog/michelangelo-adr5-contracts`, коммит 4597c43) читан read-only:
  `schemas/adr5/candidate_bundle.schema.json`, `docs/adr5-streams/contracts/HANDOFF.md`,
  `docs/adr5-contracts/CONTRACT_GATE.md` §7.4, `NEGATIVE_TEST_MATRIX.md` (F9/F10/F13/F18).

## 3. Вход/выход PoC

- **Вход:** зарегистрированный template (data-only JSON, SHA-256 pinned) + типизированный запрос
  варианта (item_id, params: enum/number/boolean, output_stem) + пиннутые fixture-исходники.
- **Выход:** артефакты в private staging (variant.glb + variant.blend) с фактическими SHA-256,
  CandidateAssetBundle `michelangelo.candidate-asset-bundle/draft-0.1` со статусом `proposed`,
  отчёт фактической геометрии/UV/материалов из выходного GLB, телеметрия RAM/VRAM.
- **Ошибки:** адресные коды (`runner/errors.py`), никогда не маскируются (см. CAPABILITY_MATRIX).

## 4. Допущения

- Стенд: Windows 10, RTX 3050 8GB / 32GB RAM, Blender 5.2.3 LTS (`I:\Tools\Blender\blender.exe`,
  резолвится через `BLENDER_BIN` → PATH → типовые пути).
- Палитры/выборы крыш заданы таблицами зарегистрированного шаблона; свободный RGB/код через wire
  не проходит.
- GLB round-trip разбивает вершины куба по UV-швам (24 верт/куб) — учтено в плановых counts
  (`source_counts` = counts сцены варианта после импорта).
- Байтовая воспроизводимость `.blend` не заявляется; `.glb` на зафиксированной версии Blender
  детерминирован на практике (подтверждено: повторные прогоны дают те же SHA-256; вариант V1-none
  побайтово совпал с исходным walls_module.glb).

## 5. Блокеры/ограничения

- В 18 ops нет текстурной операции: шаблон, требующий текстуру, честно отказывается
  (`MISSING_TEXTURE`/`CAPABILITY_UNAVAILABLE`) до запуска; предложение op — в INTEGRATION_NOTES.
- Проверка сокетов V1 — статическая по данным шаблона + сверка плановых bounds/counts с
  фактическим `scene_summary`; это не симуляция стыков.
- `--factory-startup` не sandbox: грузятся только известные доверенные фикстуры; приём
  недоверенного `.blend` не поддерживается (блокер ADR §13.2 не снят).
