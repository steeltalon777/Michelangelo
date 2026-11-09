# CAPABILITY_MATRIX — GLM-5 PoC (Blender V0–V1), фактическое состояние

- **Дата:** 2026-11-09 (fix-раунд; PoC — 2026-11-08). **База:** `b861547` (IT-8D).
  **Blender:** 5.2.3 LTS, headless, `--background --factory-startup`, Workbench-only Worker.
- Всё ниже — факты единственного контрольного прогона 2026-11-09
  (`adr5_blender_poc/evidence/run-20261109T115246Z/`, `generated_at=2026-11-09T11:53:00Z`;
  см. TEST_EVIDENCE.md), не обещания. Tracked `evidence/` — замороженный снимок
  предыдущей генерации и для чисел здесь не используется.

## 1. Уровни вариативности (нумерация ADR 0005 §4)

| Уровень | Возможность | Статус в PoC | Основание |
|---|---|---|---|
| V0 | Палитра/материалы (3 enum-палитры × 3 объекта) | **VERIFIED** | 3 headless-прогона: geometry profile идентичен source, материалы попарно различимы (baseColorFactor из GLB) |
| V0 | Сохранение геометрии при вариации материалов | **VERIFIED** | vertices/triangles/локальные bounds совпали между source и всеми 3 вариантами (`geometry_equal`) |
| V0 | Текстуры/decals | **NOT_SUPPORTED (gate отказывает)** | В 18 ops нет текстурной операции; зарегистрированный probe отклоняется (`MISSING_TEXTURE`/`CAPABILITY_UNAVAILABLE`) до запуска |
| V1 | Сборка из размеченных компонентов по сокету (стены + flat/gable/none) | **VERIFIED** | 3 headless-прогона; плановые bounds/counts совпали с фактическими (допуск 1e-3; max факт. дельта 9e-6) |
| V1 | Статическая проверка сокета (коннектор накрывает сокет, base_z) | **VERIFIED (static)** | `INVALID_SOCKET` на несовпадении; статическая проверка по данным шаблона + runtime bounds-check |
| V1 | Совместимость стыков как симуляция/физика | **NOT_SUPPORTED** | вне PoC; нужна отдельнаяcapability |
| V2 | Параметрическая геометрия (модификаторы/shape keys/GN) | **NOT_SUPPORTED** | вне PoC; требуется новый op/GN gate (ADR §4 V2) |
| V3 | Rule-based construction | **NOT_SUPPORTED** | вне PoC |
| V4 | Agent proposal | **NOT_SUPPORTED** | вне PoC |

## 2. Ресурсы и окружение

| Позиция | Статус | Метод |
|---|---|---|
| RAM одного прогона | **MEASURED**: peak 204.6–209.8 MiB (6 прогонов) | ctypes psapi `GetProcessMemoryInfo` PeakWorkingSetSize, polling 0.5 s |
| VRAM | **MEASURED (GPU total)**: 1573–1704 MiB суммарно по GPU во время прогонов | `nvidia-smi --query-gpu=memory.used`; НЕ только Blender — помечено в отчётах |
| Один тяжёлый Blender-процесс | **VERIFIED** | `tasklist /FI "IMAGENAME eq blender.exe"` перед каждым запуском (`RESOURCE_BUSY` иначе); паузы между прогонами |
| Headless/factory-startup | **VERIFIED** | все прогоны `--background --factory-startup` |
| Побитовая детерминированность GLB | **VERIFIED (на pinned 5.2.3)** | повторные прогоны дают одинаковые SHA-256; V1-none GLB == исходный walls_module.glb (`a2111677…`) |
| Побитовая детерминированность .blend | **NOT_CLAIMED** | внутренние поля Blender; не обещается |

## 3. Безопасность/границы доверия

| Механизм | Статус | Проверка |
|---|---|---|
| Только зарегистрированные шаблоны (data-only) | **VERIFIED** | `TEMPLATE_HASH_MISMATCH` на подменённом байте шаблона; реестр пиннит SHA-256 с `digest_rule: sha256-lf` (LF-байты = git blob, checkout-независимо) |
| Запрет кода в шаблоне (F10) | **VERIFIED** | `TEMPLATE_FORBIDDEN_KEY` на python/exec/shell/script/… ключи на любом уровне; параметры вне реестра слотов — `UNKNOWN_PARAMETER` |
| Свободный RGB не проходит | **VERIFIED** | цвета только из `palette_table` шаблона; параметр-цвет не существует в системе типов |
| Защищённые роли (F9, статический guard) | **VERIFIED** | слот, таргетящий protected object, отклоняется при загрузке (`SLOT_TARGETS_PROTECTED_ROLE`); в V1 защищены стены |
| Эффективный результат (F9-родственный) | **VERIFIED** | фактические bounds/counts сцены сверяются с плановыми (`BOUNDS_MISMATCH` иначе) |
| Path escape (F13) | **VERIFIED** | строковые инварианты (`INVALID_OUTPUT_PATH`) + фактический контейнмент resolve (`ARTIFACT_PATH_ESCAPE`); артефакты только в `outputs/` |
| Недоступная capability (F18) | **VERIFIED** | `CAPABILITY_UNAVAILABLE` до запуска, item блокируется, не «починяется» |
| Пиннинг исходников | **VERIFIED** | `SOURCE_HASH_MISMATCH` на подменённом байте fixture (до запуска) + rehash после прогона; `MISSING_MODULE` на отсутствующем файле |
| Честный nonzero на битом входе | **VERIFIED** | мусорный `.glb` проходит hash (свой фактический hash), Worker возвращает failed → `WORKER_FAILED`, без «починки» |
| `--factory-startup` = sandbox | **NOT_CLAIMED** | не sandbox; грузятся только доверенные фикстуры; недоверенный `.blend` не поддерживается |
| Сеть/API/LLM из Worker | **ABSENT** | код не содержит сетевых вызовов; worker.py не изменён |

## 4. Контракты (GLM-1, draft-0.1 — НЕ утверждены)

| Контракт | Статус |
|---|---|
| `candidate_bundle.schema.json` как форма выхода | **ORIENTED-ON**: 6 bundle'ов построены и прошли валидацию схемой (jsonschema draft 2020-12, 0 ошибок); статус `proposed` — константа |
| Порт `template.execute(bindings, capabilities, limits)` | **PoC-реализация**: `runner/template_executor.py::execute_variant` |
| Wire-протокол вариантов/шаблонов | **LOCAL DRAFT**: `michelangelo.adr5-poc.template/draft-0` — не утверждённый протокол, раунд 2 согласования |

## 5. Явно НЕ сделано (честный список)

- Рендеры/превью вариантов не снимались (appearance верифицирован по baseColorFactor GLB;
  EEVEE/Cycles Worker'ом не гарантируются — Workbench-only контракт).
- UV верифицированы как наличие `TEXCOORD_0` в выходном GLB; texel density/качество развёртки
  не измерялись.
- Никаких UE-ready/production-ready заявлений: импорт в движок не проверялся.
- Multi-source SourceSet, recipe lock, batch executor — чужие потоки; PoC не реализует их
  (bundle содержит base_ref/dependency_digests как заготовки provenance).
- F9 «родственный» runtime-случай (косвенное влияние разрешённой зависимости на защищённую часть
  через parent transform/material) в 18 ops недостижим частично: слоты не могут таргетить
  защищённые объекты, bounds-check ловит эффективный результат. Полная семантика эффективного
  diff'а — на интегратора/GLM-6 (см. INTEGRATION_NOTES).
