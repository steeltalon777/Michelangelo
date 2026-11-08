# TEST_EVIDENCE — GLM-5 PoC V0–V1, прогоны 2026-11-08

- **Стенд:** Windows 10.0.26100 x64, RTX 3050 8192 MiB, **Blender 5.2.3 LTS**
  (`I:\Tools\Blender\blender.exe`), headless `--background --factory-startup`.
- **База:** `dev@b861547` (IT-8D). **worker.py не изменён:**
  SHA-256 `9c7c24f3311aa0a5cdc168ff0b5d336067ce6c4200d6b2b3ac6f17e53cb96ee3`.
- **Полные машиночитаемые данные:** `adr5_blender_poc/evidence/evidence_summary.json`
  (+ по варианту: `bundle.json`, `variant.glb`, `variant.blend`).
- **Статус: EXECUTED** (не dry-run). Все прогоны — один Blender-процесс за раз,
  tasklist-guard перед каждым, паузы между запусками.

## 1. Команды

```bash
# 1) Pure unit/negative (без Blender), 44 теста
cd adr5_blender_poc/tests && python -m unittest \
  test_registry test_parameter_gate test_sockets_and_plan test_glb_report test_bundle_writer

# 2) Генерация/пиннинг фикстур (Blender, 2 прогона)
python adr5_blender_poc/fixtures/make_fixtures.py

# 3) Полный smoke: V0×3 + V1×3 + 7 негативов (Blender, 6 прогонов)
python adr5_blender_poc/run_smoke.py

# 4) Всё вместе (44 pure + Blender smoke, SKIPPED без Blender)
cd adr5_blender_poc/tests && python -m unittest discover -p "test_*.py"
```

Результат: (1) `OK, 44 tests`; (3) exit 0, `positives: 6, negatives ok: 7/7`;
(4) `OK` (45 тестов, smoke исполнен реально).

## 2. Пиннутые входы (SHA-256 по фактическим байтам)

| Вход | SHA-256 |
|---|---|
| fixture `house_master.glb` (walls+roof+door, 24v/36tri на стороне создания) | `eb985b8d1b2ca388ccc57dae66d342900616258e5b5751e608d058aa2f57d8a5` |
| fixture `house_master.blend` | `4f5251317b98d9a09465b5ef34aaa76a8b0c51da330b537aad26e29607f909c2` |
| fixture `walls_module.glb` (walls_base 4×3×2.4, top z=2.4) | `a2111677d1298c438db21206aaa29dc04bc8ebaef5a018693ea5150a7cd2200e` |
| fixture `walls_module.blend` | `26612f594b1684b81dd8f24327591d46674d028809afd98338d32f1f5909925d` |
| template `v0_palette_master` | `50e69342c30dae9a3ed6ad32f53257adbec7087645e9a427ae1ee27df96846eb` |
| template `v1_roof_assembly` | `a47aa9ea89c980ad9c72e9a4d840e46a8644c22bd46339911e5cee568492d731` |
| template `v0_textured_probe` (негативный) | `7791381f77b85e7713624e08f8120eaf7eab8d3a17500aa01c47cc81925e45f1` |
| `blender/worker.py` (не изменён) | `9c7c24f3311aa0a5cdc168ff0b5d336067ce6c4200d6b2b3ac6f17e53cb96ee3` |

## 3. Позитивные прогоны (факты из выходных GLB)

| Item | Параметр | Артефакты (sha256) | RAM peak / GPU total | Время |
|---|---|---|---|---|
| `item-v0-palette-master-brick-dark` | `palette_id=brick-dark` | glb `366dbcab…c504`, blend `a09e3e0f…9a0` | 206.4 MiB / 1397 MiB | 0.6 s |
| `item-v0-palette-master-plaster-warm` | `palette_id=plaster-warm` | glb `c644cb04…82d4`, blend `31629600…ba6` | 205.7 MiB / 1425 MiB | 0.6 s |
| `item-v0-palette-master-paint-teal` | `palette_id=paint-teal` | glb `a453ae38…6d1bd`, blend `0b0f7116…9641` | 206.8 MiB / 1442 MiB | 0.6 s |
| `item-v1-roof-assembly-flat` | `roof_variant=flat` | glb `ea58a42c…9dc8d79c`, blend `600f0d20…137db` | 205.7 MiB / 1442 MiB | 0.6 s |
| `item-v1-roof-assembly-gable` | `roof_variant=gable` | glb `ccbfdd8b…498b71`, blend `bd04bb58…b50` | 206.2 MiB / 1436 MiB | 0.6 s |
| `item-v1-roof-assembly-none` | `roof_variant=none` | glb `a2111677…2200e`, blend `60bae730…83b86` | 205.1 MiB / 1420 MiB | 0.6 s |

GPU — суммарно по GPU (все процессы), НЕ только Blender; RAM — peak working set процесса
blender.exe (ctypes psapi, polling 0.5 s).

Bundle'ы (draft-0.1, все `status=proposed`, валидация схемой GLM-1 — 0 ошибок):
sha256 в `evidence_summary.json`, например brick-dark bundle `7eb06c65…42f33`.

### 3.1. Фактическая геометрия/UV/материалы (из JSON-chunk выходных GLB)

**V0 (все 3 варианта):** nodes `door`, `roof`, `walls` — по 24 vertices / 12 triangles,
`TEXCOORD_0` присутствует у всех примитивов; локальные bounds идентичны source.
Материалы (pbrMetallicRoughness.baseColorFactor, фактические):

| Вариант | Mat_walls | Mat_roof | Mat_door |
|---|---|---|---|
| brick-dark | [0.3, 0.21, 0.17, 1] | [0.24, 0.18, 0.15, 1] | [0.14, 0.1, 0.08, 1] |
| plaster-warm | [0.87, 0.8, 0.68, 1] | [0.45, 0.3, 0.22, 1] | [0.35, 0.22, 0.14, 1] |
| paint-teal | [0.16, 0.45, 0.44, 1] | [0.9, 0.88, 0.84, 1] | [0.2, 0.25, 0.28, 1] |

Проверки: `geometry_preserved_vs_source: equal=true` (counts + bounds идентичны source-GLB) у всех
трёх; материалы попарно различимы (`materials_distinct: true`). Сцена:
3 меша, vertices 72, triangles 36 (round-trip: куб режется по UV-швам, 24 верт/куб), bounds
`[-2.2,-1.7,0]..[2.2,1.7,2.65]` — совпали с плановыми.

**V1 (3 сборки):** импорт стен + блок крыши.

| Сборка | Nodes | Scene vertices/triangles | Плановые bounds == факт |
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

`source_hashes_unchanged=true` у всех 6 вариантов (rehash fixture до/после прогона); копия в
staging проверялась по hash до запуска; оригиналы в `adr5_blender_poc/fixtures/` не писались
вариантами (выход только в private staging `outputs/`).

## 4. Негативы (7/7 — каждая ошибка адресная, без «починки»)

| Негатив | Ожидание | Факт | Механизм |
|---|---|---|---|
| F18: probe требует `texture_binding` | `CAPABILITY_UNAVAILABLE` | да | до запуска Blender |
| probe с выданной capability, файла текстуры нет | `MISSING_TEXTURE` | да | до запуска |
| подменён байт шаблона vs реестр | `TEMPLATE_HASH_MISMATCH` | да | реестр пиннит SHA-256 |
| подменён байт fixture vs пиннинг | `SOURCE_HASH_MISMATCH` | да | до запуска |
| fixture отсутствует | `MISSING_MODULE` | да | до запуска |
| V1: крыша 2.0×1.5 не накрывает сокет 4×3 | `INVALID_SOCKET` | да | статическая проверка |
| мусорный `.glb` (hash свой — проходит) | `WORKER_FAILED` (nonzero) | да | Worker вернул failed, import мусора не «чинится» |

Также покрыто pure-тестами (44): `TEMPLATE_FORBIDDEN_KEY` (F10: python/exec/shell/… на любом
уровне), `UNKNOWN_PARAMETER` (параметр вне реестра слотов, F10-семантика),
`SLOT_TARGETS_PROTECTED_ROLE` (F9 статический), `INVALID_OUTPUT_PATH` (F13: `..`, absolute,
backslash, drive, NUL), `ARTIFACT_PATH_ESCAPE`, `UNKNOWN_SOCKET`, вне-диапазона/NaN параметры,
`ASSET_DIGEST` по правилу 4 README схем GLM-1.

## 5. Валидация bundle'ов против схемы GLM-1

`validate_bundle_against_schema` (jsonschema Draft202012, схема
`I:/prog/michelangelo-adr5-contracts/schemas/adr5/candidate_bundle.schema.json`, draft-0.1,
read-only): все 6 bundle'ов — **0 ошибок**. Если contracts-worktree недоступен, проверка честно
пишет `SKIPPED`, не PASS.

## 6. Воспроизводимость

- .glb детерминированы на pinned Blender 5.2.3: повторные прогоны дают те же SHA-256
  (подтверждено между прогонами фикстур и вариантом V1-none).
- .blend байтово НЕ воспроизводим (не заявляется).
- Порядок запуска не влияет на содержание варианта; retry создаёт новую attempt-директорию
  (attempt id содержит время в мс + uuid-суффикс), существующие evidence не перезаписываются.
