# TEST_EVIDENCE — GLM-5 PoC V0–V1, контрольный прогон 2026-11-09

- **Стенд:** Windows 10.0.26100 x64, RTX 3050 8192 MiB, **Blender 5.2.3 LTS**
  (`I:\Tools\Blender\blender.EXE`), headless `--background --factory-startup`.
- **База:** `dev@b861547` (IT-8D). **worker.py не изменён:** SHA-256
  `28dfc5c91e8a09163178de565e095ff2ad2f9556482436dbb20419550acd7035`
  (`digest_rule: sha256-lf` — LF-нормализованные байты = git-blob;
  CRLF-байты этого worktree дают `9c7c24f3…96ee3` — тот же файл;
  `git diff b861547 -- blender/worker.py` пуст).
- **Контрольный прогон — единственный источник §2–§3:**
  `adr5_blender_poc/evidence/run-20261109T115246Z/evidence_summary.json`,
  `generated_at = 2026-11-09T11:53:00Z` (UTC). Каталог `run-<ts>/` —
  disposable (игнорируется git); tracked `adr5_blender_poc/evidence/` —
  замороженный снимок предыдущей генерации (`e89704f`) и в §2–§3 не
  используется (MEDIUM-3: числа не смешиваются между прогонами).
- **Статус: EXECUTED** (не dry-run). Все прогоны — один Blender-процесс за раз,
  tasklist-guard перед каждым, паузы между запусками.

## 0. Evidence-политика (HIGH-1, исправлено)

- Каждый запуск smoke создаёт **новый no-clobber** каталог
  `adr5_blender_poc/evidence/run-<UTC timestamp>/` (`run_smoke.create_run_dir`,
  `mkdir(exist_ok=False)`; при коллизии имени добавляется суффикс `-2`, `-3`, …).
- **Никакого `rmtree`**: tracked `adr5_blender_poc/evidence/` — замороженный
  снимок; smoke его не очищает и не перезаписывает. Все новые артефакты —
  только в `run-<ts>/`.
- `evidence/run-*/` игнорируются git (`adr5_blender_poc/.gitignore`), поэтому
  штатный `python -m unittest discover` не пачкает tracked-дерево: после
  полного прогона `git status --short` пуст (проверено на коммите fix-раунда).
- Регрессия закреплена тестами: `tests/test_evidence_runs.py` (no-clobber;
  существующий каталог и frozen-файл не очищаются) и `tests/test_smoke_v0_v1.py`
  (SHA-256-снимок tracked evidence до/после полного smoke обязан совпасть).

## 1. Команды

```bash
# 1) Pure unit/negative (без Blender), 49 тестов
cd adr5_blender_poc/tests && python -m unittest \
  test_registry test_parameter_gate test_sockets_and_plan test_glb_report \
  test_bundle_writer test_evidence_runs

# 2) Генерация/пиннинг фикстур (Blender, 2 прогона; только при пересоздании фикстур)
python adr5_blender_poc/fixtures/make_fixtures.py

# 3) Полный smoke: V0×3 + V1×3 + 7 негативов (Blender, 6 прогонов)
python adr5_blender_poc/run_smoke.py

# 4) Всё вместе (49 pure + Blender smoke, SKIPPED без Blender)
cd adr5_blender_poc/tests && python -m unittest discover -p "test_*.py"
```

Результат: (1) `OK, 49 tests`; (3) exit 0, `positives: 6, negatives ok: 7/7`,
evidence — `evidence/run-20261109T115246Z/`; (4) `Ran 50 tests … OK` (smoke
исполнен реально, Blender найден через `BLENDER_BIN`/PATH/типовые пути).

## 2. Пиннутые входы и правило digest (MEDIUM-2)

**Правило:** текстовые входы (шаблоны, `blender/worker.py`) пиннятся с
`digest_rule: sha256-lf` — SHA-256 по LF-нормализованным байтам (CRLF → LF).
Это в точности digest содержимого git-blob, поэтому пин не зависит от
`core.autocrlf`/checkout: CRLF-worktree и LF-checkout дают один и тот же sha.
Бинарные входы (`.glb`/`.blend`) пиннятся с `digest_rule: sha256-bytes` —
сырые байты (= git blob); LF-нормализация к ним не применяется.

| Вход | digest_rule | SHA-256 (пин) |
|---|---|---|
| fixture `house_master.glb` (walls+roof+door, 24v/36tri на стороне создания) | `sha256-bytes` | `eb985b8d1b2ca388ccc57dae66d342900616258e5b5751e608d058aa2f57d8a5` |
| fixture `house_master.blend` | `sha256-bytes` | `4f5251317b98d9a09465b5ef34aaa76a8b0c51da330b537aad26e29607f909c2` |
| fixture `walls_module.glb` (walls_base 4×3×2.4, top z=2.4) | `sha256-bytes` | `a2111677d1298c438db21206aaa29dc04bc8ebaef5a018693ea5150a7cd2200e` |
| fixture `walls_module.blend` | `sha256-bytes` | `26612f594b1684b81dd8f24327591d46674d028809afd98338d32f1f5909925d` |
| template `v0_palette_master` | `sha256-lf` | `6d080a13d1676bee29dbaf45578ea5189abff604441672afd45b34a63248ddef` |
| template `v1_roof_assembly` | `sha256-lf` | `4a1c21c2ceafd86686ee35777e2d5810bdb693c0a4b0dd2862f52fdad5c9084b` |
| template `v0_textured_probe` (негативный) | `sha256-lf` | `7791381f77b85e7713624e08f8120eaf7eab8d3a17500aa01c47cc81925e45f1` |
| `blender/worker.py` (не изменён) | `sha256-lf` | `28dfc5c91e8a09163178de565e095ff2ad2f9556482436dbb20419550acd7035` |

LF-пины сверены с содержимым git-blob'ов:
`git cat-file -p HEAD:adr5_blender_poc/templates/v0_palette_master.json | sha256sum`
= `6d080a13…48ddef`, `…/v1_roof_assembly.json` = `4a1c21c2…c9084b`,
`HEAD:blender/worker.py` = `28dfc5c9…d7035`.

Для прозрачности: сырые CRLF-байты этого worktree дают
`50e69342…6846eb` (v0_palette_master), `a47aa9ea…492d731` (v1_roof_assembly),
`9c7c24f3…96ee3` (worker.py) — **эти значения больше не являются пинами**;
именно они раньше давали ложный `TEMPLATE_HASH_MISMATCH` на LF-checkout.
Реестр (`templates/registry.json`) требует явный `digest_rule: "sha256-lf"`
для каждой записи и отвергает иное правило (`TEMPLATE_INVALID`).

## 3. Позитивные прогоны (факты контрольного прогона)

| Item | Параметр | `variant.glb` SHA-256 | `variant.blend` SHA-256 | `bundle.json` SHA-256 | RAM peak / GPU total | Время |
|---|---|---|---|---|---|---|
| `item-v0-palette-master-brick-dark` | `palette_id=brick-dark` | `366dbcab3ace35f352fc380978c93a15c5981eb950a21570bc3c9312e39dc504` | `75167d0ae4f1034e45b4ca8a1c6d4625a76bd16b05decd162b943e133a3b0590` | `b6be54bc04d043a4471a968e405bd7adab8e4d47e1e3ef1a8c750f13303528c7` | 209.8 MiB / 1573 MiB | 1.6 s |
| `item-v0-palette-master-plaster-warm` | `palette_id=plaster-warm` | `c644cb0443849042a9f65381bcc8b667572c55cc0868485d1f850c3dc698f2d4` | `e20f5474f641351709403c003ce36b119696ff843e76cbb8c60b5814561a14fc` | `40c398d2d18fb26fa2da02491160144ab136745bbcc241077a7fa27cedaeb209` | 207.1 MiB / 1573 MiB | 0.6 s |
| `item-v0-palette-master-paint-teal` | `palette_id=paint-teal` | `a453ae3854303af23245d2602842779cee333a62b589f09693aae8246d4cd1bd` | `589041f6038422edcd139b7b3c529f7127f08286973a24afc9b1f8bb9e718ab6` | `ed8b7ffdbfee5d778e5835b9af16df41c23206db6cf1ade7e1473df6521c112b` | 207.9 MiB / 1600 MiB | 0.6 s |
| `item-v1-roof-assembly-flat` | `roof_variant=flat` | `ea58a42c1eb276d0aa63e75eb7385b4a7dad01ea05ac4a67c910985f9dc8d79c` | `39364ff83771a94771b97d16db72459dd61b44ab78bd9dacdf571286f79dfa4e` | `582ba479d00b8a4ae6b3f879b2f3e6d99896ef74c85deef1512c2a4c4a319cbf` | 206.3 MiB / 1599 MiB | 0.6 s |
| `item-v1-roof-assembly-gable` | `roof_variant=gable` | `ccbfdd8b8315601619fde5f689c2c0e8afa1505ab89f1ebd91edfb949b498b71` | `83dbe498633becb73ff08b12f7cda68c254e5a434d2891000fb4a6ea889a2fbc` | `0082a3ec043000df8d1d531f1df4673987983239c4e9747731e249a5d6b83b1e` | 206.5 MiB / 1600 MiB | 0.6 s |
| `item-v1-roof-assembly-none` | `roof_variant=none` | `a2111677d1298c438db21206aaa29dc04bc8ebaef5a018693ea5150a7cd2200e` | `0626496dd053d6ff8bd3ffbfa5bea9f3e628039b5f4b611c8001ffe0a9bfc7ab` | `18832516160dc7d578341a323a042f12d9b3a5215905f75c5564cb291b79f14c` | 204.6 MiB / 1704 MiB | 0.6 s |

GPU — суммарно по GPU (все процессы), НЕ только Blender; RAM — peak working set процесса
`blender.exe` (ctypes psapi, polling 0.5 s). Первый прогон V0 — 1.6 s (холодный старт),
остальные 0.6 s.

Bundle'ы (draft-0.1, все `status=proposed`, `schema_validation: []` — 0 ошибок
против схемы GLM-1): sha256 в таблице выше и в
`evidence/run-20261109T115246Z/evidence_summary.json`.

### 3.1. Фактическая геометрия/UV/материалы (из JSON-chunk выходных GLB)

**V0 (все 3 варианта):** nodes `door`, `roof`, `walls` — по 24 vertices / 12 triangles,
`TEXCOORD_0` присутствует у всех примитивов; локальные bounds идентичны source.
Сцена: 3 меша, vertices 72, triangles 36, bounds `[-2.2,-1.7,0]..[2.2,1.7,2.65]`
(совпали с плановыми). Материалы (pbrMetallicRoughness.baseColorFactor, фактические):

| Вариант | Mat_walls | Mat_roof | Mat_door |
|---|---|---|---|
| brick-dark | [0.3, 0.21, 0.17, 1] | [0.24, 0.18, 0.15, 1] | [0.14, 0.1, 0.08, 1] |
| plaster-warm | [0.87, 0.8, 0.68, 1] | [0.45, 0.3, 0.22, 1] | [0.35, 0.22, 0.14, 1] |
| paint-teal | [0.16, 0.45, 0.44, 1] | [0.9, 0.88, 0.84, 1] | [0.2, 0.25, 0.28, 1] |

Проверки: `geometry_preserved_vs_source: equal=true` (counts + bounds идентичны
source-GLB) у всех трёх; материалы попарно различимы
(`materials_distinct_vs_prev: true` у plaster-warm и paint-teal; brick-dark — первый
в серии, сравнение с source не заявляется).

**V1 (3 сборки):** импорт стен + блок крыши.

| Сборка | Nodes (GLB) | Scene vertices/triangles | Плановые bounds == факт |
|---|---|---|---|
| flat | `walls_base`, `roof_slab` | 32 / 24 | да: `[-2.2,-1.7,0]..[2.2,1.7,2.65]` |
| gable | `walls_base`, `roof_panel_south`, `roof_panel_north` | 40 / 36 | да, max дельта 9e-6 ≤ допуска 1e-3: `[-2.3,-1.760009,0]..[2.3,1.760009,3.485423]` |
| none | `walls_base` | 24 / 12 | да: `[-2.0,-1.5,0]..[2.0,1.5,2.4]` |

Материалы в V1 не применяются (стены — protected role; слотов на неё нет).
Детерминизм round-trip: GLB варианта `none` побайтово равен исходному
`walls_module.glb` (`a2111677…2200e`).
Gable-геометрия: панели 4.6×1.963011×0.12, rotation X ±30°, центры (0,∓0.88,2.942709) —
выведена из угла 30° и полупролёта 1.7; фактические bounds подтвердили расчёт.

### 3.2. Сохранность исходников

`source_hashes_unchanged=true` у всех 6 вариантов (rehash fixture до/после прогона);
копия в staging проверялась по hash до запуска; оригиналы в
`adr5_blender_poc/fixtures/` не писались вариантами (выход только в private staging
`outputs/`).

## 4. Негативы (7/7 — каждая ошибка адресная, без «починки»)

| Негатив | Ожидание | Факт | Механизм |
|---|---|---|---|
| F18: probe требует `texture_binding` | `CAPABILITY_UNAVAILABLE` | да | до запуска Blender |
| probe с выданной capability, файла текстуры нет | `MISSING_TEXTURE` | да | до запуска |
| подменён байт шаблона vs реестр | `TEMPLATE_HASH_MISMATCH` | да | реестр пиннит SHA-256 (`sha256-lf`) |
| подменён байт fixture vs пиннинг | `SOURCE_HASH_MISMATCH` | да | до запуска |
| fixture отсутствует | `MISSING_MODULE` | да | до запуска |
| V1: крыша 2.0×1.5 не накрывает сокет 4×3 | `INVALID_SOCKET` | да | статическая проверка |
| мусорный `.glb` (hash свой — проходит) | `WORKER_FAILED` (nonzero) | да | Worker вернул failed, import мусора не «чинится» |

Также покрыто pure-тестами (49): `TEMPLATE_FORBIDDEN_KEY` (F10: python/exec/shell/… на
любом уровне), `UNKNOWN_PARAMETER` (параметр вне реестра слотов, F10-семантика),
`SLOT_TARGETS_PROTECTED_ROLE` (F9 статический), `INVALID_OUTPUT_PATH` (F13: `..`,
absolute, backslash, drive, NUL), `ARTIFACT_PATH_ESCAPE`, `UNKNOWN_SOCKET`,
вне-диапазона/NaN параметры, `ASSET_DIGEST` по правилу 4 README схем GLM-1,
LF-нормализация digest-пинов (CRLF-байты не дают `TEMPLATE_HASH_MISMATCH`),
no-clobber каталогов `evidence/run-<ts>/`.

## 5. Валидация bundle'ов против схемы GLM-1

`validate_bundle_against_schema` (jsonschema Draft202012, схема
`I:/prog/michelangelo-adr5-contracts/schemas/adr5/candidate_bundle.schema.json`,
draft-0.1, read-only; путь переопределяется `GLM1_CONTRACTS_DIR`): все 6 bundle'ов
контрольного прогона — **0 ошибок** (`schema_validation: []`). Если
contracts-worktree/jsonschema недоступны, проверка честно пишет `SKIPPED`, не PASS.

## 6. Воспроизводимость и целостность evidence

- `.glb` детерминированы на pinned Blender 5.2.3: SHA-256 выходных GLB совпали
  с предыдущим замороженным снимком (V0: `366dbcab…`, `c644cb04…`, `a453ae38…`;
  V1 flat/gable: `ea58a42c…`, `ccbfdd8b…`), а V1-none побайтово равен исходному
  `walls_module.glb` (`a2111677…`).
- `.blend` байтово НЕ воспроизводим (не заявляется): hashes `variant.blend`
  отличаются между прогонами.
- Каждый запуск smoke создаёт новый `evidence/run-<ts>/`; внутри прогона retry
  создаёт новую attempt-директорию (attempt id содержит время в мс + uuid-суффикс).
  Существующие evidence (tracked снимок и прошлые `run-*`) не перезаписываются
  и не удаляются (HIGH-1).
