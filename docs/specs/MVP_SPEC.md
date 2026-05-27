# Спецификация MVP проекта «Микилянджело»

## 1. Назначение проекта

**Микилянджело** — локальная модульная 3D/AI-среда для рекурсивной декомпозиции визуальных ассетов, индексирования проектного контекста и поэтапного управления Blender через LLM-оркестратор.

Цель MVP — доказать полный вертикальный сценарий:

> Пользователь создаёт проект, импортирует PNG-слои/референсы, система разбирает их в индексируемый Asset Graph, LLM строит план действий, Rust Core формирует Blender job, Blender создаёт сцену/модель, результат возвращается в UI как рендеры, `.blend` и `.glb`.

Проект должен изначально проектироваться как локальный «завод» с заменяемыми оболочками, сменяемыми LLM-провайдерами, сменяемыми backend-хранилищами и расширяемым контуром взаимодействия с Blender.

---

## 2. Базовые архитектурные принципы

### 2.1 Core как отдельный процесс

Rust Core работает как отдельный headless-процесс:

```text
UI Shell
  ↕ JSONL / JSON-RPC over stdio
michelangelo-core
  ↔ SQLite + filesystem
  ↔ LLM APIs
  ↔ Blender batch worker
```

UI не линкуется напрямую с ядром и не имеет прямого доступа к Blender, SQLite, LLM API или индексам. Любая оболочка общается с Core только через стабильный командно-событийный протокол.

### 2.2 UI является заменяемой оболочкой

Первая оболочка может быть сделана на Slint, Tauri, WPF или другом стеке. Архитектура не должна зависеть от конкретного UI.

Поддерживаемый принцип:

```text
Slint / WPF / Swift / Kotlin / Web UI
        ↓
Core Protocol
        ↓
Rust Core
```

### 2.3 Blender не является центром системы

Blender — исполнитель 3D-команд, а не источник истины. Источник истины — проектный workspace, база метаданных, Asset Graph, jobs и версии артефактов.

### 2.4 LLM не работает с сырыми изображениями напрямую

LLM получает не PNG как «картинку», а структурированную карту проекта:

```text
PNG layer → bbox → components → contours → primitive hints → summaries → indexed nodes
```

LLM должна ориентироваться в визуальном проекте так же, как агент ориентируется в AI-friendly репозитории: через карту, узлы, краткие карточки, поиск и точечные запросы деталей.

### 2.5 Модульность по сменяемым границам

Модули проектируются вокруг мест, которые будут меняться:

- UI shell;
- LLM provider;
- storage backend;
- vector index;
- asset parser;
- Blender transport;
- agent interface;
- skills/recipes.

---

## 3. MVP Scope

### 3.1 Входит в MVP

MVP должен включать:

1. Headless Rust Core как отдельный процесс.
2. Простой UI shell или CLI-first режим.
3. Создание и открытие проекта.
4. Импорт папки PNG-слоёв и PNG-референсов.
5. Manifest-файл для описания ролей слоёв и видов.
6. Декомпозицию PNG:
   - размер изображения;
   - alpha/binary mask;
   - bbox непустой области;
   - connected components;
   - bbox компонентов;
   - контуры;
   - упрощённые контуры;
   - shape hints;
   - dominant colors.
7. Хранение метаданных в SQLite.
8. Хранение бинарных ассетов на filesystem.
9. Asset Graph с иерархией Project → SourceSet → View → Layer → Region → Component → Contour.
10. FTS-поиск по карточкам узлов.
11. Пространственный индекс bbox через SQLite RTree.
12. LLM chat с минимум одним OpenAI-compatible provider.
13. Генерацию build plan через LLM.
14. Валидацию build plan ядром.
15. Blender batch worker через `blender --background --python worker.py`.
16. Создание reference planes.
17. Генерацию простого hard-surface blockout.
18. Рендер front/side/top/perspective.
19. Экспорт `.blend` и `.glb`.
20. Job history, status, logs, errors.

### 3.2 Не входит в MVP

В MVP не входят:

- CAD/STEP/IGES/DXF;
- полноценный PSD parser;
- Figma/Penpot API;
- Qdrant как обязательная зависимость;
- embeddings как обязательная часть pipeline;
- MCP adapter;
- Blender addon;
- persistent Blender daemon;
- real-time scene editing;
- автоматическая VLM-оценка рендера;
- multi-user/cloud mode;
- полноценная генерация сложных органических моделей;
- персонажи, rigging, animation pipeline.

---

## 4. Целевой MVP-сценарий

### 4.1 Входные данные

Папка проекта:

```text
sensor_box/
  manifest.yaml
  refs/
    front.png
    side.png
    top.png
  layers/
    front__body.png
    front__screen.png
    front__buttons.png
    front__holes.png
    side__body.png
    top__body.png
```

Пример `manifest.yaml`:

```yaml
project:
  name: sensor_box
  object_type: hard_surface
  target: game_asset

scale:
  width_mm: 180
  height_mm: 120
  depth_mm: 70

views:
  front:
    reference: refs/front.png
    layers:
      - path: layers/front__body.png
        role: body
      - path: layers/front__screen.png
        role: screen
      - path: layers/front__buttons.png
        role: buttons
      - path: layers/front__holes.png
        role: mounting_holes
  side:
    reference: refs/side.png
    layers:
      - path: layers/side__body.png
        role: body
  top:
    reference: refs/top.png
    layers:
      - path: layers/top__body.png
        role: body

output:
  formats:
    - blend
    - glb
```

### 4.2 Основной workflow

1. Пользователь создаёт проект.
2. Пользователь импортирует `manifest.yaml`, PNG-референсы и PNG-слои.
3. Core создаёт workspace и SQLite DB.
4. Ingestor разбирает PNG-слои.
5. Asset Graph сохраняется в SQLite.
6. Indexer создаёт FTS/RTree индексы.
7. UI показывает карту проекта.
8. Пользователь пишет в чат: «собери простой корпус по слоям».
9. Orchestrator собирает релевантный контекст из Asset Graph.
10. LLM возвращает build plan.
11. Core валидирует build plan и превращает его в BlenderJobSpec.
12. BlenderAdapter создаёт `job.json`.
13. Core запускает Blender batch worker.
14. Blender worker создаёт сцену, reference planes, blockout, материалы, камеры.
15. Blender worker сохраняет `.blend`, `.glb`, рендеры и `result.json`.
16. Core сохраняет job output.
17. UI показывает результат.

---

## 5. Процессная архитектура

### 5.1 MVP topology

```text
[UI Shell]
  Slint / CLI / future WPF
        ↓ JSONL / JSON-RPC over stdio
[michelangelo-core]
  Rust headless process
        ↓
[SQLite + filesystem workspace]
        ↓
[Blender batch subprocess]
  blender --background --python worker.py
```

### 5.2 Future topology

```text
[Any UI Shell]
  ↕ stdio / HTTP / named pipe
[michelangelo-core]
  ↔ michelangelo-indexer
  ↔ michelangelo-blender-worker
  ↔ michelangelo-mcp
  ↔ Qdrant / pgvector / sqlite-vec
  ↔ Ollama / embedding server
  ↔ Blender persistent worker / addon
```

---

## 6. Core Modules

### 6.1 `michelangelo_protocol`

Назначение: общие DTO, команды, события и схемы.

Содержит:

- `CommandEnvelope`;
- `ResponseEnvelope`;
- `EventEnvelope`;
- `ProjectDto`;
- `AssetDto`;
- `AssetNodeDto`;
- `JobDto`;
- `ChatMessageDto`;
- `LlmProviderConfig`;
- `BlenderJobSpec`;
- `BlenderJobResult`.

Цель: все оболочки и адаптеры говорят на одном языке.

---

### 6.2 `michelangelo_core`

Назначение: главный application layer.

Отвечает за:

- маршрутизацию команд;
- координацию модулей;
- доступ к сервисам проекта;
- валидацию операций;
- публикацию событий.

Core не должен зависеть от конкретного UI.

---

### 6.3 `michelangelo_workspace`

Назначение: управление workspace и проектами.

Функции:

- `create_project`;
- `open_project`;
- `scan_project`;
- `load_manifest`;
- `resolve_project_paths`;
- `validate_workspace_structure`.

Структура проекта:

```text
project/
  michelangelo.db
  manifest.yaml
  assets/
  refs/
  layers/
  thumbnails/
  masks/
  contours/
  jobs/
  blender/
    scenes/
    renders/
    exports/
```

---

### 6.4 `michelangelo_storage`

Назначение: абстракция хранения.

Traits:

```rust
trait MetadataStore {}
trait AssetStore {}
trait TextIndex {}
trait SpatialIndex {}
trait VectorIndex {}
```

MVP implementations:

```text
MetadataStore = SQLite
AssetStore = local filesystem
TextIndex = SQLite FTS5
SpatialIndex = SQLite RTree
VectorIndex = Noop
```

Future implementations:

```text
MetadataStore = Postgres
VectorIndex = Qdrant / sqlite-vec / pgvector
TextIndex = Tantivy / Postgres FTS
```

---

### 6.5 `michelangelo_ingest`

Назначение: импорт и декомпозиция графических ассетов.

MVP input:

- PNG;
- папка PNG-слоёв;
- manifest.yaml.

Future input:

- SVG;
- PSD;
- Figma export;
- Penpot export.

PNG decomposition pipeline:

```text
load image
  → detect alpha/non-empty mask
  → compute layer bbox
  → connected components
  → component bbox
  → contours
  → simplified contours
  → shape hints
  → dominant colors
  → thumbnails
  → Asset Graph nodes
```

MVP должен учитывать, что автоматическая декомпозиция PNG-слоёв может ошибаться из-за сглаживания, теней, полупрозрачности, шума и плохо подготовленных слоёв. Поэтому в pipeline должен быть предусмотрен fallback-режим ручной или полуавтоматической коррекции:

- пользователь может изменить роль слоя;
- пользователь может подтвердить или отклонить найденные компоненты;
- пользователь может вручную задать bbox/region;
- пользователь может пометить компонент как `ignore`, `body`, `button`, `hole`, `screen`, `detail`;
- исправления сохраняются в Asset Graph как пользовательские overrides.

В MVP ручная коррекция может быть реализована сначала через manifest/CLI, а полноценный UI-редактор регионов вынесен в более поздний этап.

---

### 6.6 `michelangelo_asset_graph`

Назначение: AI-friendly Visual Asset Graph.

Иерархия:

```text
Project
  SourceSet
    View
      Layer
        Region
          Component
            Contour
              PrimitiveHint
```

Каждый node хранит:

- `id`;
- `project_id`;
- `asset_id`;
- `parent_id`;
- `node_type`;
- `name`;
- `role`;
- `view_name`;
- `summary`;
- `bbox`;
- `metrics`;
- `tags`;
- `confidence`;
- ссылки на файлы масок, контуров, thumbnails.

LLM взаимодействует с графом через безопасные tools/commands:

- `get_project_map`;
- `search_nodes`;
- `get_node_card`;
- `get_node_children`;
- `get_node_geometry`;
- `get_relevant_context`.

---

### 6.7 `michelangelo_indexer`

Назначение: построение поисковых и навигационных индексов.

MVP:

- FTS5 по `name`, `role`, `summary`, `tags`;
- RTree по bbox;
- incremental reindex по hash файла;
- index status.

Future:

- embeddings;
- Qdrant sidecar;
- BGE/nomic/qwen embedding models;
- hybrid search;
- visual captions через VLM.

Принцип:

```text
PNG → decomposition → node cards → text/vector index
```

Не индексировать сырые пиксели напрямую как основной источник смысла.

---

### 6.8 `michelangelo_llm`

Назначение: взаимодействие с LLM.

MVP provider:

- OpenAI-compatible API.

Target providers:

- DeepSeek;
- Qwen;
- OpenAI-compatible local gateways;
- Ollama later;
- Gemini/Kimi later if needed.

Функции:

- chat;
- streaming chat;
- model selection;
- API key management;
- prompt building;
- token/context budget;
- structured output parsing;
- retry/error handling.

API keys должны храниться через платформенный secret store:

- Windows: DPAPI / Credential Manager;
- macOS: Keychain;
- Linux: secret-service / encrypted local fallback.

---

### 6.9 `michelangelo_orchestrator`

Назначение: превращение цели пользователя в план и jobs.

Функции:

- сбор релевантного контекста из Asset Graph;
- формирование prompt для LLM;
- получение build plan;
- валидация build plan;
- преобразование build plan в jobs;
- управление итерациями.

MVP pipeline:

```text
User prompt
  → Project map
  → Relevant asset context
  → LLM build plan
  → Core validation
  → BlenderJobSpec
```

На MVP `get_relevant_context` не должен притворяться магическим семантическим поиском. До появления embeddings он работает через простые и проверяемые механизмы:

- фильтры по `role`, `view_name`, `node_type`, `tags`;
- FTS5-поиск по `name`, `summary`, `tags`;
- правила выбора типовых узлов для hard-surface pipeline;
- ограничение глубины Asset Graph;
- ручной выбор/фиксация relevant nodes пользователем.

LLM получает сначала компактную карту проекта, а затем точечно запрашивает детали узлов. Полные контуры и тяжёлые данные не добавляются в prompt без явной необходимости.

---

### 6.10 `michelangelo_blender`

Назначение: управление Blender.

Trait:

```rust
trait BlenderAdapter {
    async fn execute_job(&self, job: BlenderJobSpec) -> Result<BlenderJobResult>;
    async fn get_capabilities(&self) -> Result<BlenderCapabilities>;
}
```

MVP adapter:

```text
HeadlessBlenderAdapter
```

Механика:

```text
Core creates job.json
Core runs blender --background --python worker.py -- job.json result.json
Worker executes bpy operations
Worker writes result.json
Core reads outputs and logs
```

Future adapters:

- PersistentBlenderAdapter;
- BlenderAddonAdapter;
- LocalHttpBlenderAdapter;
- NamedPipeBlenderAdapter;
- MCPBlenderAdapter if useful.

---

### 6.11 `michelangelo_jobs`

Назначение: управление долгими операциями.

Job types:

- `AssetImport`;
- `AssetIndexing`;
- `LlmChat`;
- `BuildPlan`;
- `BlenderBuildBlockout`;
- `BlenderRenderViews`;
- `BlenderExportModel`.

Statuses:

- `queued`;
- `running`;
- `completed`;
- `failed`;
- `cancelled`.

Каждый job хранит:

- `id`;
- `project_id`;
- `job_type`;
- `status`;
- `input_json`;
- `output_json`;
- `logs`;
- timestamps;
- артефакты.

---

### 6.12 `michelangelo_events`

Назначение: event bus для UI и внутренних сервисов.

Events:

- `ProjectOpened`;
- `AssetImported`;
- `AssetIndexed`;
- `AssetGraphUpdated`;
- `JobStarted`;
- `JobProgress`;
- `JobFinished`;
- `JobFailed`;
- `ChatTokenDelta`;
- `RenderCreated`;
- `ModelExported`.

UI слушает события и обновляет состояние без прямого доступа к внутренним модулям.

---

### 6.13 `michelangelo_cli`

Назначение: тестирование core без UI.

MVP commands:

```bash
michelangelo project create ./demo
michelangelo project open ./demo
michelangelo assets import-layers ./demo ./layers
michelangelo index run ./demo
michelangelo graph map ./demo
michelangelo blender build-blockout ./demo
michelangelo blender render-views ./demo
michelangelo blender export-glb ./demo
```

CLI обязателен до полноценного UI.

---

### 6.14 `michelangelo_mcp` later

Назначение: MCP adapter для внешних агентов.

Не входит в MVP.

Будущие tools:

- `get_project_map`;
- `search_asset_nodes`;
- `get_node_geometry`;
- `run_blender_job`;
- `render_views`;
- `export_model`.

MCP не должен быть ядром. Он должен быть одним из интерфейсов к Core.

---

## 7. UI Shell Requirements

### 7.1 Роль UI

UI отвечает только за:

- отображение проектов;
- drag-and-drop файлов;
- чат;
- превью ассетов;
- отображение Asset Graph;
- запуск команд;
- мониторинг jobs;
- просмотр рендеров;
- настройки.

UI не отвечает за:

- хранение данных;
- запуск Blender напрямую;
- работу с LLM API напрямую;
- парсинг PNG;
- построение индексов;
- генерацию Blender jobs.

Порядок развития интерфейсов:

```text
v0.1–v0.4: headless Core + CLI
v0.5: Rust TUI для удобной локальной работы в терминале
v0.6+: полноценная GUI-оболочка
```

TUI рассматривается как промежуточная рабочая оболочка: дешевле GUI, удобнее CLI, хорошо подходит для отладки jobs, чата, логов, карты проекта и состояния индексации. Полноценные GUI-оболочки должны появляться только после стабилизации Core Protocol и основных pipeline.

### 7.2 MVP panels

Минимальные панели:

```text
Project Explorer
Asset/Layers Panel
Chat Workspace
Job Monitor
Render Gallery
Settings
```

### 7.3 Settings MVP

Настройки:

- workspace root;
- path to Blender executable;
- LLM provider;
- model name;
- API key alias;
- request timeout;
- indexing enabled/disabled;
- vector index disabled in MVP.

---

## 8. Core Protocol MVP

### 8.1 Transport

MVP transport:

```text
JSONL over stdio
```

Rules:

- stdout содержит только JSONL protocol messages;
- stderr содержит human-readable logs;
- каждое сообщение — одна строка JSON;
- request/response используют `id`;
- events не имеют `id`.

### 8.2 Command example

Request:

```json
{"id":"1","method":"project.create","params":{"path":"D:/Michelangelo/demo","name":"demo"}}
```

Response:

```json
{"id":"1","result":{"project_id":"demo","db_path":"D:/Michelangelo/demo/michelangelo.db"}}
```

Event:

```json
{"event":"job.progress","data":{"job_id":"job_42","progress":65,"message":"Rendering front view"}}
```

### 8.3 Future transports

Future transports may include:

- HTTP on `127.0.0.1`;
- named pipes;
- Unix domain sockets;
- MCP stdio adapter;
- remote HTTP mode with auth.

Transport must not change business command semantics.

### 8.4 State synchronization

UI не должен восстанавливать состояние только через историю событий. При старте, переподключении или открытии проекта должен существовать явный snapshot-метод:

```text
project.get_snapshot
```

Snapshot должен возвращать минимально достаточное состояние:

- текущий проект;
- список ассетов;
- верхний уровень Asset Graph;
- активные/последние jobs;
- настройки проекта;
- последние render/model outputs;
- состояние индексации.

События используются для live-обновлений после snapshot, а не как единственный источник восстановления UI-состояния.

---

## 9. Asset Graph data model MVP

### 9.1 SQLite tables draft

```sql
projects(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  root_path TEXT NOT NULL,
  created_at TEXT NOT NULL
);

assets(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  path TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  width INTEGER,
  height INTEGER,
  format TEXT,
  created_at TEXT NOT NULL
);

nodes(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  asset_id TEXT,
  parent_id TEXT,
  node_type TEXT NOT NULL,
  name TEXT,
  role TEXT,
  view_name TEXT,
  summary TEXT,
  confidence REAL
);

node_bbox(
  node_id TEXT PRIMARY KEY,
  x REAL NOT NULL,
  y REAL NOT NULL,
  width REAL NOT NULL,
  height REAL NOT NULL
);

node_metrics(
  node_id TEXT PRIMARY KEY,
  area REAL,
  aspect_ratio REAL,
  circularity REAL,
  dominant_colors_json TEXT
);

node_tags(
  node_id TEXT NOT NULL,
  tag TEXT NOT NULL
);

node_contours(
  id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL,
  detail_level TEXT NOT NULL,
  path TEXT NOT NULL,
  points_count INTEGER
);

jobs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  job_type TEXT NOT NULL,
  status TEXT NOT NULL,
  input_json TEXT,
  output_json TEXT,
  created_at TEXT NOT NULL,
  finished_at TEXT
);
```

### 9.2 FTS draft

```sql
CREATE VIRTUAL TABLE node_fts USING fts5(
  node_id UNINDEXED,
  name,
  role,
  view_name,
  summary,
  tags
);
```

### 9.3 RTree draft

```sql
CREATE VIRTUAL TABLE node_bbox_rtree USING rtree(
  node_rowid,
  min_x,
  max_x,
  min_y,
  max_y
);
```

---

## 10. Blender Worker MVP

### 10.1 Worker contract

Input:

```text
job.json
```

Output:

```text
result.json
.blend
.glb
renders/*.png
logs
```

`worker.py` должен быть тонким исполнителем над `bpy`, а не вторым ядром системы. Он не принимает архитектурных решений, не общается с LLM, не строит Asset Graph и не содержит бизнес-логики проекта.

Каждая операция из `BlenderJobSpec` должна иметь строго типизированный handler внутри worker layer:

```text
create_reference_planes → handle_create_reference_planes
create_box              → handle_create_box
create_cylinder         → handle_create_cylinder
apply_bevel             → handle_apply_bevel
render_views            → handle_render_views
export_glb              → handle_export_glb
```

Если операция неизвестна или параметры не проходят валидацию, worker возвращает ошибку в `result.json`, а Core помечает job как `failed`.

### 10.2 Example BlenderJobSpec

```json
{
  "job_id": "job_001",
  "job_type": "build_blockout",
  "project_dir": "D:/Michelangelo/demo",
  "input_blend": null,
  "output_blend": "blender/scenes/model_v001.blend",
  "operations": [
    {
      "op": "create_reference_planes",
      "views": ["front", "side", "top"]
    },
    {
      "op": "create_box",
      "name": "body",
      "size": [3.0, 1.6, 0.5],
      "location": [0, 0, 0]
    },
    {
      "op": "apply_bevel",
      "object": "body",
      "width": 0.05,
      "segments": 3
    },
    {
      "op": "render_views",
      "views": ["front", "side", "top", "perspective"],
      "output_dir": "blender/renders/job_001"
    },
    {
      "op": "export_glb",
      "path": "blender/exports/model_v001.glb"
    }
  ]
}
```

### 10.3 Supported MVP operations

- `create_reference_planes`;
- `create_box`;
- `create_cylinder`;
- `apply_bevel`;
- `set_material`;
- `set_camera_set`;
- `render_views`;
- `export_glb`;
- `save_blend`;
- `get_scene_summary`.

`execute_arbitrary_python` is not part of safe MVP. It may exist only in dev/debug mode behind explicit flag.

---

## 11. LLM Orchestration MVP

### 11.1 Context strategy

LLM получает только релевантный контекст:

```text
project summary
asset graph map
selected node cards
available operations
current job state
latest result summary
```

LLM не получает весь проект, все контуры и все изображения сразу.

### 11.2 Build plan output

LLM должна возвращать структурированный план:

```json
{
  "goal": "build simple hard-surface blockout",
  "selected_nodes": ["layer.front.body", "layer.front.buttons", "layer.front.holes"],
  "steps": [
    {
      "action": "create_body",
      "source_node": "layer.front.body",
      "method": "box_from_bbox"
    },
    {
      "action": "place_buttons",
      "source_node": "layer.front.buttons",
      "method": "cylinders_from_components"
    },
    {
      "action": "cut_mounting_holes",
      "source_node": "layer.front.holes",
      "method": "boolean_cylinders"
    }
  ]
}
```

Core валидирует план и преобразует в BlenderJobSpec.

---

## 12. Development Roadmap

### v0.1 — CLI + Workspace + Storage

- создать repo structure;
- реализовать `michelangelo_protocol`;
- реализовать `michelangelo_workspace`;
- реализовать SQLite schema;
- реализовать CLI `project create/open`.

### v0.2 — PNG Ingestion + Asset Graph

- импорт PNG;
- alpha bbox;
- connected components;
- contours;
- node creation;
- FTS/RTree;
- CLI `assets import-layers`, `graph map`;
- CLI/manifest fallback для ручной коррекции ролей, bbox и игнорируемых компонентов.

### v0.3 — Blender Batch Worker

- `worker.py`;
- `HeadlessBlenderAdapter`;
- build simple scene;
- render views;
- export `.blend` and `.glb`.

### v0.4 — LLM Chat + Build Plan

- OpenAI-compatible provider;
- DeepSeek/Qwen config;
- streaming chat;
- structured build plan;
- context builder from Asset Graph.

### v0.5 — Rust TUI Shell

- терминальная оболочка на Rust;
- project explorer;
- asset/node list;
- chat panel;
- job monitor;
- log viewer;
- render/model output list;
- settings view;
- работа через тот же Core Protocol, что и будущие GUI shells.

TUI не должен иметь прямого доступа к storage, Blender или LLM. Он является клиентом Core, как и будущие Slint/WPF/Swift/Kotlin оболочки.

### v0.6 — First GUI Shell

- Slint/Tauri/WPF/other shell;
- project explorer;
- chat;
- asset panel;
- job monitor;
- render gallery;
- settings;
- использование уже стабилизированного Core Protocol.

### v0.7 — Safer Orchestrator

- validate plan;
- map plan to allowed operations;
- better job progress;
- error recovery;
- iteration history.

### v0.7 — Optional Index Enhancements

- vector index interface;
- optional Qdrant sidecar;
- embedding provider settings;
- semantic search over node cards.

### v0.8 — SVG / PSD / MCP candidates

- SVG import;
- basic PSD adapter;
- MCP adapter prototype;
- persistent Blender worker research.

---

## 13. MVP Acceptance Criteria

MVP считается успешным, если выполнен сценарий:

1. Core запускается как отдельный процесс.
2. CLI или UI создаёт новый проект.
3. Пользователь импортирует PNG-слои.
4. Система строит Asset Graph.
5. Система показывает карту проекта.
6. LLM получает краткий релевантный контекст, а не весь проект целиком.
7. LLM предлагает build plan.
8. Core валидирует build plan.
9. Core запускает Blender batch job.
10. Blender создаёт сцену с reference planes и простым blockout.
11. Система генерирует рендеры front/side/top/perspective.
12. Система экспортирует `.blend` и `.glb`.
13. UI/CLI показывает paths, logs и итоговый report.

---

## 14. Основные риски

### 14.1 Слишком раннее усложнение

Риск: начать с Qdrant, MCP, PSD, Figma, Blender addon и GUI одновременно.

Митигация: CLI-first, PNG-first, SQLite-first, batch Blender-first.

### 14.2 LLM hallucination в build plan

Риск: модель предлагает несуществующие операции или неверные аргументы.

Митигация: строгий allowed operation set, schema validation, plan validation, no arbitrary Python in safe mode.

### 14.3 Blender batch latency

Риск: `blender --background` медленный для мелких операций.

Митигация: MVP принимает latency; позже persistent Blender worker.

### 14.4 Сложность image decomposition

Риск: PNG-слои могут быть грязными, плохо именованными, с шумом или сложной прозрачностью.

Митигация: manifest.yaml, thumbnails, manual role correction in UI, deterministic heuristics before ML.

### 14.5 UI может начать протекать в core

Риск: первая оболочка начнёт напрямую управлять storage/Blender/LLM.

Митигация: Core Protocol boundary, separate process, no direct module access from UI.

---

## 15. Короткое определение проекта

**Микилянджело** — локальный AI-friendly 3D workbench с Rust Core, заменяемыми UI-оболочками, индексируемым графом визуальных ассетов и безопасным управлением Blender через jobs.
