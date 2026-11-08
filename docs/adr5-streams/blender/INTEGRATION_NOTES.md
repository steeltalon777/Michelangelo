# INTEGRATION_NOTES — GLM-5 → интегратор / DeepSeek / соседние потоки

- **Дата:** 2026-11-08. Статус: предложения, не реализованные интеграционные изменения.
- Все изменения общих файлов — вне полномочий этого потока; ниже — точки расширения и
  предложения. Существующий Worker API (18 ops) в PoC не расширялся.

## 1. Как PoC встаёт поверх текущего Worker'а

```
VariantRequest (typed params) ──► parameter_gate (enum/диапазон/F10/F13)
        │                                 │
        ▼                                 ▼
TemplateRegistry (sha256 pinning) ─► socket_check (V1, static)
        │
        ▼
plan_builder ─► job.json {operations: ≤18 ops} ─► blender --background --factory-startup
        │                                          --python blender/worker.py
        ▼
result.json ─► artifact verification + BOUNDS_MISMATCH check + GLB report
        ▼
CandidateAssetBundle draft-0.1 (status=proposed) ─► (будущий) canonical publisher
```

PoC-раннер — чистый Python (std lib), вне Rust-core. Для продакшена ожидаемая интеграция —
портирование логики gate'ов в `michelangelo_protocol`/новый crate, но контракты должны пройти
раунд 2 (см. §5).

## 2. Предложения новых Worker ops (только proposal, PoC их не содержит)

1. **`set_named_material` / `material_from_palette`** — сейчас `set_material` всегда создаёт
   `Mat_<object>`; для V0-семейств лучше именованный материал + палитра-параметр. Нужен
   contract gate (BLENDER_WORKER.md — общий файл).
2. **`get_object_report`** — per-object отчёт (vertices/triangles/UV-слои/материалы) на стороне
   Blender. PoC замещает это парсингом GLB (`runner/glb_report.py`), но для не-glb выходов
   (например, `.blend`) нужен op.
3. **`bind_texture`** — текстурный слот. Пока зарегистрированный шаблон с текстурой честно
   отказывается (`MISSING_TEXTURE`/`CAPABILITY_UNAVAILABLE`); реализация только после
   решения по доверию к файлам текстур (ADR §10 «Безопасность входов»).
4. **`check_socket_fit`** (runtime) — измерение фактического зазора/перекрытия стыков в сцене;
   сейчас сокет-проверка статическая (по данным шаблона) + bounds-check.

Каждое предложение требует: типизированные параметры в Rust-протоколе, валидацию, обновление
`SAFE_WORKER_OPS`/`BLENDER_WORKER.md`, negative tests. Никакого автозапуска .blend это не вводит.

## 3. Что должен учесть GLM-4 (Family) и GLM-3 (Batch)

- Форма `michelangelo.adr5-poc.template/draft-0` — локальный draft: поля `parameters`,
  `palette_table`, `component_blocks`, `sockets`, `component_choice_parameters`,
  `expected_bounds`/`expected_counts` — кандидат в `family_recipe`/`recipe_lock` (границы
  допустимого множества). Ограничение: V0/V1 kinds only — как в HANDOFF §3.
- `candidate_bundle` собирается раннером: `dependency_digests` уже включает worker.py/template/
  source hashes; `base_ref.kind=source_snapshot`; `expected_current_revision` не ставится
  (no-clobber create). Публикация — вне потока.
- Item-level isolation: отказ одного варианта не влияет на другие (проверено негативами в
  пределах одного процесса-раннера; межпроцессная очередь — зона GLM-3).
- Ресурсный guard PoC — `tasklist` + один процесс; для batch нужен общий admission
  (ADR §7.3) — вне зоны GLM-5.

## 4. Открытые вопросы для DeepSeek (приоритет проверки)

1. **F9 в V1:** достаточно ли static-guard'а (слот не может таргетить protected object) +
   bounds/counts-check, чтобы считать эффективное изменение защищённой роли пойманным в этих
   рамках? Контрпример приветствуется (например, влияние через общий материал — в 18 ops
   `set_material` создаёт новый материал на объект, shared material невозможен; но proposal
   §2.1 это меняет — тогда нужен эффективный diff).
2. **BOUNDS_MISMATCH как анти-маскинг:** плановые bounds лежат в зарегистрированном шаблоне —
   они не вычисляются из произвольного кода, но и не измеряются независимо. Приемлем ли такой
   уровень для V0/V1 PoC, или требуется независимый geometry digest (GLB POSITION hash) —
   сейчас сравнение делает раннер (заинтересованная сторона).
3. **Шаблоны как данные:** достаточен ли запрет ключей (F10) + whitelist 18 ops + enum/range
   параметров, чтобы считать template «не кодом»? Отсутствие `allow_arbitrary_code`
   зашито константой.
4. **`expected_current_revision`**: PoC всегда no-clobber (null). Для family-вариантов один
   asset_id порождает несколько candidate_id — CAS-семантика sibling'ов не проверялась
   (зона GLM-6/GLM-1).
5. **Round-trip counts:** плановые counts учитывают UV-split куба (24 верт/куб). Хрупко ли это
   при смене версии Blender/экспортера (рекомендация: пиннить версию Blender в recipe lock —
   поле `recipe_lock` GLM-1).
6. **GPU-телеметрия:** суммарная по GPU (nvidia-smi), не per-process. Достаточно ли для
   admission ADR §7.3 или нужен per-process VRAM (NVML)?

## 5. Согласование wire (раунд 2)

- Локальный `variant_request` PoC (item_id/template_id/params/output_stem) — НЕ заявлен как
  `michelangelo.variant-request/draft-0.1`; перед интеграцией — маппинг на схему GLM-4.
- Локальный `template registry` — НЕ схема GLM-1; предложение: registry принадлежит владельцу
  семейства (GLM-4/owner), а не blender-потоку; Blender-поток получает pinned template_ref+sha.
- Коды ошибок PoC (`runner/errors.py`) — кандидаты в общий реестр кодов CONTRACT_GATE §5;
  имена совпадать не обязаны.

## 6. Проверено негативами на этой итерации (сводка)

7/7 behavioral негативов + 44 pure-теста; details в TEST_EVIDENCE.md §4. Ничего из этого
не «чинится» молча: каждый отказ — адресный код + отказ исполнения.
