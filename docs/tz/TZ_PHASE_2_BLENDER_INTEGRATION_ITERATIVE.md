# TZ: Phase 2 — Blender Integration Iterative

## Execution Strategy

- [ ] 🟢 Parallel execution recommended
- **Reason:** Phase 2 naturally splits into independent ownership areas after a small sequential contract stage: protocol DTOs/events, SQLite job persistence, job scheduler/event queue, Blender subprocess adapter/worker, CLI JSONL transport, and documentation. Parallelism is useful only after the protocol/crate skeleton is agreed. Shared files (`Cargo.toml`, `Cargo.lock`, `justfile`, `.github/workflows/ci.yml`, `crates/michelangelo_core/src/*`, `crates/michelangelo_cli/src/*`) must be owned by the orchestrator or by a single integration unit to avoid merge conflicts.

## Execution Checklist

- [x] 0. Context verified
- [x] 1. Architecture boundaries confirmed
- [x] 2. Implementation stage 1 complete — protocol contract and crate skeletons
- [x] 3. Implementation stage 2 complete — accepted 2026-05-29: all three reviewer findings resolved; `cargo test --workspace` passes; real Blender worker smoke passes
- [ ] 4. Implementation stage 3 complete — core/router/CLI integration
- [ ] 5. Implementation stage 4 complete — hardening, smoke recipes, docs sync
- [ ] 6. Unit/component tests complete
- [ ] 7. Integration tests with real dependencies complete
- [ ] 8. Stand smoke tests complete
- [ ] 9. UI automation tests complete
- [ ] 10. User scenario tests complete
- [ ] 11. Regression checks complete
- [ ] 12. Documentation updated
- [ ] 13. Final acceptance review complete

## Check Rules

- Architect creates this checklist and acceptance criteria.
- Executor agents may check implementation/test items only after implementation and verification evidence exists.
- QA verifier may check final acceptance only after reviewing evidence for every checked item.
- Failed, skipped, or unavailable checks stay unchecked with a blocker note.
- Runtime behavior cannot be accepted with unit tests only: Phase 2 requires static checks, unit/component tests, temp SQLite integration tests, subprocess tests with a fake Blender executable, and real Blender smoke when Blender is available.
- UI automation is expected to remain **not applicable** for this TZ because no GUI/TUI is in scope. It may be checked only by QA with the note `not applicable: no UI touched` after confirming no UI/TUI files were changed.

---

## 0. Context Authority

This TZ is based on the current repository state on `dev` reviewed on 2026-05-29:

- `ROADMAP.md`: Phase 2 scope is **Blender Integration**:
  - Job scheduling and execution;
  - Blender subprocess management;
  - Job progress events.
- `TASKS.md`: Phase 1 tasks `T-0001`–`T-0017` are complete; Blender job execution is not started.
- `README.md`: current architecture is headless Rust core over JSONL stdin/stdout.
- `docs/specs/MVP_SPEC.md`: authoritative product MVP spec for Blender worker, jobs, events, process topology, and safe operation set.
- `docs/ai/AI_CONTEXT.md` and `docs/ai/AI_ENTRY_POINTS.md`: current crate/file entry points.
- `docs/tz/TZ_MVP_HEADLESS_CORE_ITERATIVE.md`: completed Phase 1 implementation contract.
- `AGENTS.md`: agents work only on `dev`, must not push, must stage only explicit pathspecs if committing is later requested.

Current implemented baseline:

- Rust workspace members:
  - `crates/michelangelo_protocol`
  - `crates/michelangelo_core`
  - `crates/michelangelo_workspace`
  - `crates/michelangelo_cli`
- Protocol methods currently implemented:
  - `system.ping`
  - `project.create`
  - `project.open`
  - `project.get_snapshot`
- Current `ProjectSnapshotDto.jobs` is always empty.
- Current `JobDto` is protocol-only and minimal: `id`, `name`, `status`, `created_at`, `completed_at`.
- Current `EventEnvelope` exists, but no event bus or stdio event streaming exists.
- Workspace layout already creates:
  - `jobs/`
  - `blender/scenes/`
  - `blender/renders/`
  - `blender/exports/`
- SQLite storage currently has only `projects` table.
- `blender/worker.py` and `blender/smoke_create_cube.py` exist but are empty.
- `docs/specs/BLENDER_WORKER.md`, `docs/specs/CORE_PROTOCOL.md`, and ADR placeholders are empty.

Precedence if documents conflict:

1. explicit user instruction for the current run;
2. `AGENTS.md` and local workflow rules;
3. `docs/specs/MVP_SPEC.md`;
4. this TZ;
5. generated/historical notes.

---

## 1. Goal

Implement Phase 2 as a small, deterministic, headless Blender integration slice that can be driven by autonomous agents and verified without GUI, LLM, PNG ingest, or network services.

Phase 2 is acceptable when:

- Core can schedule and execute Blender jobs from the JSONL protocol.
- Job state is visible through commands and `project.get_snapshot`.
- Job history survives process boundaries through the workspace SQLite DB.
- Blender is launched as a bounded subprocess, not embedded into the Rust process.
- Blender worker accepts a strict JSON job spec and writes a strict JSON result.
- Progress/status events are emitted as protocol JSONL events and never mixed with human logs.
- A deterministic real Blender smoke job creates at least a `.blend` artifact and, where supported by the local Blender build, a `.glb` and/or simple render artifact.
- Tests cover fake-subprocess behavior, temp SQLite persistence, CLI JSONL event behavior, and real Blender smoke when Blender is available.

Recommended Phase 2 MVP vertical scenario:

```text
project.create
  → blender.run_job(wait=true, job_type="blender_smoke_scene")
  → stdout JSONL event stream: job.queued/job.started/job.progress/job.completed
  → response for blender.run_job contains terminal JobDto + artifact paths
  → project.get_snapshot shows recent job
  → process restart: project.open + job.list returns persisted job history
```

---

## 2. Scope

### In Scope

- Protocol expansion for Phase 2 jobs, Blender specs/results, and job events.
- New or extended job lifecycle fields:
  - `job_type`
  - `status`
  - `progress_pct`
  - `message`
  - `input_json`
  - `output_json`
  - `logs`
  - `error`
  - `artifacts`
  - timestamps.
- SQLite-backed job persistence under the existing workspace DB.
- Minimal in-process job scheduler for local headless execution.
- Bounded Blender subprocess adapter using `blender --background --python <worker.py> -- <job.json> <result.json>`.
- Thin Blender Python worker over `bpy`.
- Strict safe operation set for the Phase 2 smoke path:
  - `create_box`
  - `set_material`
  - `set_camera_set`
  - `render_views` if feasible in headless environment
  - `export_glb` if feasible
  - `save_blend`
  - `get_scene_summary`.
- Protocol methods for job visibility and Blender execution:
  - `job.list`
  - `job.get`
  - `job.cancel`
  - `blender.get_capabilities`
  - `blender.run_job`.
- JSONL event streaming from `core stdio` while preserving stdout as protocol-only output.
- `just` recipes for fake and real Blender smoke tests.
- CI/local verification updates that do not require secrets.
- Documentation updates for active entry points and Blender worker contract.

### Out of Scope

- PNG import/decomposition and Asset Graph indexing. These remain Phase 3.
- LLM provider integration, build-plan generation, prompt building, or orchestration. These remain Phase 4.
- GUI/TUI, Playwright/FlaUI, Slint/Tauri/WPF, or render gallery UI.
- MCP adapter, HTTP server, named pipes, or network transport.
- Persistent Blender daemon, Blender addon, live scene editing, or socket server.
- Arbitrary Python execution in safe mode.
- CAD/STEP/IGES/DXF, PSD/Figma/Penpot adapters.
- Qdrant/vector DB/embeddings.
- Cloud, multi-user, auth, or secrets handling.

### Dependency Policy

- Prefer standard library threads/channels/process APIs for Phase 2 scheduling and subprocess management unless a dependency is clearly justified.
- Do not add an async runtime (`tokio`, `async-std`) only to run one Blender subprocess. Add it only if an ADR or task report proves it is needed.
- Python worker must use only Blender's bundled Python and standard library plus `bpy`; no `pip install` dependencies for the smoke path.
- Blender binary path must be configurable by environment variable name `BLENDER_BIN`; default executable name may be `blender`.
- No task may hardcode local absolute paths, user directories, secrets, or machine-specific Blender install paths.

---

## 3. Architecture Boundaries to Preserve

- Rust Core remains a separate headless process.
- JSONL over stdin/stdout remains the MVP transport.
- stdout contains only protocol JSON objects, one line per message.
- stderr contains human-readable diagnostics/logs only.
- Request/response messages use `id`; events do not use `id`.
- UI shells, future LLM orchestration, and future MCP adapters interact through the same Core Protocol only.
- Blender is an external batch executor. It is not the source of truth.
- Workspace filesystem + SQLite metadata remain the local source of truth for projects, job history, and artifact paths.
- Worker scripts must be thin execution layers over `bpy`; business decisions stay in Rust/Core.
- Job paths must resolve under the active project workspace. Reject or sanitize absolute/parent-traversal paths in job specs/results.
- No arbitrary Python operation is allowed in the safe MVP worker contract.

---

## 4. Orchestrator Operating Model

Mandatory rules for every iteration:

1. Run `git branch --show-current`; stop unless the branch is `dev`.
2. Run `git status --short`; protect unrelated changes.
3. Re-read the task scope and file ownership before edits.
4. Implement only the selected task.
5. Run the smallest useful verification first, then broader regression.
6. Record evidence in the completion report.
7. Do not push.
8. If a commit is explicitly requested later, stage only task-owned files with explicit pathspecs; never use `git add .` or `git add -A`.
9. Leave TZ checkboxes unchecked when a check fails or a required stand is unavailable.

Required executor report shape:

```markdown
## Evidence

| Check | Command / Tool | Result | Evidence |
|---|---|---|---|
| Branch | `git branch --show-current` | pass/fail | branch name |
| Status | `git status --short` | pass/fail | unrelated changes note |
| Static | `<command>` | pass/fail/skipped | short note/log path |
| Unit/component | `<command>` | pass/fail/skipped | short note/log path |
| SQLite integration | `<command>` | pass/fail/skipped | temp DB/workspace note |
| Fake Blender subprocess | `<command>` | pass/fail/skipped | fake executable/log note |
| Real Blender smoke | `<command>` | pass/fail/skipped | artifact paths/log note |
| Regression | `<command>` | pass/fail/skipped | short note |
```

---

## 5. Parallel Work Plan and File Ownership

### Stage 0 — Sequential Preflight

Owner: orchestrator.

- `T-0200` context, branch, and environment preflight.
- No source edits except possibly this TZ checklist/evidence notes after implementation begins.

### Stage 1 — Sequential Contract and Skeleton

Owner: orchestrator or a single contract agent.

- `T-0201` protocol contract, method/event constants, DTO shapes, and new crate skeleton registration.
- Owns shared files:
  - `Cargo.toml`
  - `Cargo.lock`
  - `crates/michelangelo_protocol/src/*`
  - optional new crate skeleton manifests under `crates/michelangelo_jobs/` and `crates/michelangelo_blender/`

Integration checkpoint after Stage 1:

- `cargo metadata --format-version 1`
- `cargo test -p michelangelo_protocol`
- `cargo test --workspace`

### Stage 2 — Parallel Foundations

Run up to three implementation subagents in parallel. Do not let them edit the same files.

| Unit | Task(s) | Writable ownership | Required inputs | Output |
|---|---|---|---|---|
| A — Job persistence | `T-0202` | `crates/michelangelo_workspace/src/*` only | Stage 1 DTO contract | SQLite `jobs` table and storage APIs |
| B — Scheduler/events | `T-0203` | `crates/michelangelo_jobs/src/*` only | Stage 1 DTO/event contract | In-process queue, lifecycle state machine, event emission API |
| C — Blender worker/adapter | `T-0204`, `T-0205` | `crates/michelangelo_blender/src/*`, `blender/worker.py`, `blender/smoke_create_cube.py` | Stage 1 `BlenderJobSpec`/`BlenderJobResult` contract | Fake-testable subprocess adapter and strict Python worker |

Stage 2 agents must not edit `crates/michelangelo_core/`, `crates/michelangelo_cli/`, `justfile`, CI, root `Cargo.toml`, or root `Cargo.lock` unless the orchestrator explicitly serializes those edits.

Integration checkpoint after Stage 2:

- Orchestrator resolves crate wiring and lockfile changes.
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### Stage 3 — Sequential Core/CLI Integration

Owner: one integration agent.

- `T-0206` core service/router wiring.
- `T-0207` CLI JSONL response/event writer.
- `T-0208` snapshot/job polling integration.

Writable ownership:

- `crates/michelangelo_core/src/*`
- `crates/michelangelo_cli/src/*`
- integration tests that exercise core/CLI boundaries

Integration checkpoint after Stage 3:

- `just verify`
- existing `just smoke-jsonl`
- existing `just smoke-reopen`
- existing `just smoke-e2e`
- new fake Blender smoke if already added

### Stage 4 — Parallel Hardening, Recipes, Docs

Run at most three independent units in parallel.

| Unit | Task(s) | Writable ownership | Output |
|---|---|---|---|
| D — Failure/cancel/timeout hardening | `T-0209` | `crates/michelangelo_jobs/src/*`, `crates/michelangelo_blender/src/*`, targeted tests | timeout/cancel/failure semantics |
| E — Verification recipes/CI | `T-0210`, `T-0211` | `justfile`, `.github/workflows/ci.yml`, smoke scripts/tests | fake and real Blender smoke commands |
| F — Docs sync | `T-0212` | `README.md`, `TASKS.md`, `ROADMAP.md`, `docs/ai/*`, `docs/specs/*` | current entry points and worker/protocol docs |

Final checkpoint after Stage 4:

- `just verify`
- `just smoke-jsonl`
- `just smoke-reopen`
- `just smoke-e2e`
- `just smoke-blender-fake`
- `just smoke-blender-real` when Blender is installed and health check passes
- manual docs review

---

## 6. Protocol Contract Requirements

### 6.1 New Methods

Add method constants in `crates/michelangelo_protocol/src/method.rs`:

| Method | Purpose | Blocking behavior |
|---|---|---|
| `job.list` | Return recent jobs for the active project | immediate |
| `job.get` | Return one job by `job_id` | immediate |
| `job.cancel` | Request cancellation for a queued/running job | immediate; terminal status may arrive as event |
| `blender.get_capabilities` | Check configured Blender executable and supported worker operations | immediate or bounded timeout |
| `blender.run_job` | Schedule a safe Blender job from `BlenderJobSpec` | returns queued job unless `wait=true`; may emit events |

Recommended `blender.run_job` params:

```json
{
  "job_type": "blender_smoke_scene",
  "wait": true,
  "timeout_ms": 180000,
  "spec": {
    "operations": [
      {"op": "create_box", "name": "body", "size": [2.0, 1.0, 0.5], "location": [0.0, 0.0, 0.0]},
      {"op": "set_material", "object": "body", "color": [0.2, 0.4, 0.8, 1.0]},
      {"op": "set_camera_set", "views": ["front", "side", "top", "perspective"]},
      {"op": "save_blend", "path": "blender/scenes/smoke_v001.blend"},
      {"op": "export_glb", "path": "blender/exports/smoke_v001.glb"}
    ]
  }
}
```

Rules:

- `wait=false` or missing `wait` returns after queueing.
- `wait=true` blocks only up to the effective timeout and returns a terminal `JobDto` or structured error.
- Events may be emitted before the final response.
- Clients must match responses by `id` and events by `event` name.

### 6.2 Events

Add event constants and/or typed event payload DTOs for:

- `job.queued`
- `job.started`
- `job.progress`
- `job.completed`
- `job.failed`
- `job.cancelled`

Event payload minimum:

```json
{
  "job_id": "job_...",
  "job_type": "blender_smoke_scene",
  "status": "running",
  "progress_pct": 35,
  "message": "Saving blend file",
  "artifact_paths": []
}
```

Rules:

- `progress_pct` is integer `0..100` when known.
- Terminal events include artifact paths or error summary.
- Events must never contain raw Blender stdout spam; logs are stored in job logs/artifacts and summarized.

### 6.3 Job DTO

Extend `JobDto` without breaking existing snapshot tests unnecessarily. Existing fields may remain, but Phase 2 snapshot must expose enough runtime state for a UI/job monitor.

Minimum fields:

- `id: String`
- `name: String`
- `job_type: String`
- `status: JobStatus`
- `progress_pct: Option<u8>`
- `message: Option<String>`
- `created_at: Option<String>`
- `started_at: Option<String>`
- `completed_at` or `finished_at: Option<String>`
- `error: Option<String>`
- `artifacts: Vec<ArtifactDto>` or equivalent serializable list.

### 6.4 Blender DTOs

Add strict DTOs for:

- `BlenderJobSpec`
- `BlenderOperation`
- `BlenderJobResult`
- `BlenderArtifactDto`
- `BlenderCapabilitiesDto`

Acceptance rules:

- Unknown operations are rejected by Rust validation or by worker result with structured error.
- `execute_arbitrary_python` is not part of the safe DTO set.
- Relative paths in specs/results are resolved under the active workspace only.
- DTO tests include JSON serialization/deserialization examples and invalid-operation cases.

---

## 7. Test Strategy and Real Stand

### 7.1 Static Checks

Required after every implementation stage:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

After `justfile` is updated, prefer:

```bash
just fmt-check
just clippy
just test
just verify
```

### 7.2 Unit Tests

Required for:

- Protocol DTO serialization/deserialization.
- Method/event constants.
- Job status transitions.
- Job queue ordering and terminal state rules.
- Path validation for artifacts/spec paths.
- Blender result parsing.
- Worker JSON spec validation where testable outside Blender.

### 7.3 Component Tests

Required for:

- `CoreService` handlers for `job.*` and `blender.*` methods.
- Router registration.
- CLI stdio serialization of interleaved `ResponseEnvelope` and `EventEnvelope`.
- Ensuring stdout contains protocol JSONL only.
- Ensuring stderr receives human diagnostics.

### 7.4 Integration Tests with Real Internal Dependencies

Required:

- Temporary filesystem workspace.
- Real SQLite file under `.michelangelo/metadata.db`.
- Job persistence across process/CoreService boundaries.
- Fake Blender executable/script that simulates:
  - success;
  - non-zero exit;
  - malformed `result.json`;
  - timeout;
  - delayed progress if supported.

Fake Blender tests are mandatory because they are deterministic and must run even when real Blender is not installed.

### 7.5 Real Stand Smoke Tests

The Phase 2 stand is local/headless and has no long-running service.

| Item | Requirement |
|---|---|
| Database | SQLite file in a temporary Michelangelo workspace |
| DB lifecycle | Created by `project.create`; removed with temp workspace cleanup |
| Seed data | Fresh project only; no PNG/Asset Graph seed required |
| Services | None |
| External executable | Blender CLI subprocess |
| Environment variables | `BLENDER_BIN`, `MICHELANGELO_BLENDER_TIMEOUT_MS`, `RUST_LOG`, `CARGO_TARGET_DIR` |
| Health checks | `${BLENDER_BIN:-blender} --version`; `${BLENDER_BIN:-blender} --background --python-expr "import bpy; print('MICHELANGELO_BPY_OK')"` |
| Smoke output | `.blend` required; `.glb` and render PNGs required if the operation is enabled in the smoke spec |
| Cleanup | Remove temp workspace after smoke; do not delete user workspaces |

Required smoke recipes to add:

```bash
just smoke-blender-fake
just smoke-blender-real
```

`just smoke-blender-real` must:

- check Blender availability first;
- skip with a clear message if Blender is missing, without pretending success;
- create a temp workspace;
- run `project.create` through `michelangelo_cli core stdio`;
- run `blender.run_job` with `wait=true`;
- validate JSONL events and final response;
- validate expected artifact files exist;
- clean up temp workspace.

### 7.6 UI Automation

Not applicable in this TZ because GUI/TUI is out of scope. If a future executor touches UI/TUI files, they must stop and ask for a separate TZ.

### 7.7 User Scenarios

Required headless scenario:

1. Create project.
2. Check Blender capabilities.
3. Run smoke Blender job with `wait=true`.
4. Observe job progress events.
5. Verify terminal response and artifact paths.
6. Reopen project in a new process.
7. Verify `job.list` and `project.get_snapshot` show persisted job history.

### 7.8 Regression Pack

Existing Phase 1 regressions must remain green:

```bash
just verify
just smoke-jsonl
just smoke-reopen
just smoke-e2e
```

Phase 2 adds:

```bash
just smoke-blender-fake
just smoke-blender-real
```

If real Blender is unavailable, keep real stand checkbox unchecked and report blocker: `Blender CLI unavailable on this machine`.

---

## 8. Task Backlog

### T-0200 Context, Branch, and Environment Preflight

**Goal**

Verify the current project state before Phase 2 edits.

**Scope**

- Confirm branch is `dev`.
- Inspect working tree.
- Re-read:
  - `AGENTS.md`
  - `ROADMAP.md`
  - `TASKS.md`
  - `README.md`
  - `docs/specs/MVP_SPEC.md` sections 6.10, 6.11, 6.12, 8, 10.
- Probe local tooling:
  - Rust/cargo;
  - `just`;
  - Python 3;
  - Blender if available.

**Acceptance Criteria**

- Branch evidence is recorded.
- Unrelated local changes are identified and protected.
- Executor knows whether real Blender smoke can run on this machine.
- No implementation files are changed.

**Verification**

```bash
git branch --show-current
git status --short
just --list
cargo test --workspace
${BLENDER_BIN:-blender} --version
```

If Blender is missing, record blocker for real stand smoke but continue with fake subprocess tasks.

---

### T-0201 Protocol Contract and Crate Skeletons

**Goal**

Define the Phase 2 command/event/job/Blender wire contract and prepare crate boundaries.

**Scope**

- Extend `crates/michelangelo_protocol/src/method.rs` with Phase 2 method constants.
- Extend/add protocol DTO modules for jobs, events, Blender specs/results/capabilities/artifacts.
- Keep serialization backward-compatible where practical.
- Add typed tests/golden JSON examples for new DTOs.
- Create new crate skeletons if chosen by the orchestrator:
  - `crates/michelangelo_jobs/`
  - `crates/michelangelo_blender/`
- Update root workspace membership only in this sequential task or by orchestrator integration.

**Acceptance Criteria**

- New methods and events are documented by tests.
- `JobDto` can represent queued/running/completed/failed/cancelled jobs with progress and artifacts.
- `BlenderJobSpec` rejects/does not model arbitrary Python execution.
- `BlenderJobResult` can represent success and structured failure.
- `cargo test -p michelangelo_protocol` passes.
- `cargo test --workspace` passes.

**Out of Scope**

- Actual scheduling.
- SQLite job table.
- Blender subprocess launch.
- CLI event streaming.

**Verification**

```bash
cargo fmt --all -- --check
cargo test -p michelangelo_protocol
cargo test --workspace
```

---

### T-0202 SQLite Job Persistence

**Goal**

Persist job history in the workspace SQLite DB.

**Scope**

- Extend `crates/michelangelo_workspace/src/storage.rs` initialization with `jobs` storage.
- Add storage APIs for:
  - create job;
  - update status/progress/message;
  - save output/error/artifacts/log summary;
  - list jobs for current project;
  - load job by id.
- Keep schema deterministic and migration-free unless a migration mechanism is explicitly introduced.
- Store JSON blobs as text where appropriate for `input_json`/`output_json`/artifacts.

Recommended table shape aligned with `MVP_SPEC.md`:

```sql
jobs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  job_type TEXT NOT NULL,
  status TEXT NOT NULL,
  progress_pct INTEGER,
  message TEXT,
  input_json TEXT,
  output_json TEXT,
  error TEXT,
  logs TEXT,
  created_at TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT
)
```

Artifacts may be stored either as `output_json` or a small normalized table if justified.

**Acceptance Criteria**

- Fresh workspace initializes the `jobs` table.
- Job records survive process/CoreService boundaries.
- Listing jobs for a project is deterministic and ordered newest-first or oldest-first, documented by tests.
- Failed/cancelled/completed terminal states are persisted.
- Tests use a real temp SQLite file.

**Out of Scope**

- Asset Graph tables.
- FTS/RTree.
- Global recent-project scanning.

**Verification**

```bash
cargo test -p michelangelo_workspace
cargo test --workspace
```

---

### T-0203 Job Scheduler and Event Queue

**Goal**

Provide a minimal in-process job runner that can queue, run, cancel, and emit events.

**Scope**

- Implement in `crates/michelangelo_jobs/` or an orchestrator-approved equivalent module.
- Support max concurrency `1` for Blender jobs in Phase 2.
- Define lifecycle:
  - `queued` → `running` → `completed`
  - `queued`/`running` → `cancelled`
  - `queued`/`running` → `failed`
- Provide an event sink/channel abstraction that emits `EventEnvelope` or typed events.
- Provide polling/read APIs for current job state.
- Support `wait=true` semantics through a bounded wait/timeout mechanism.

**Acceptance Criteria**

- Scheduler unit tests cover lifecycle transitions.
- Events are emitted in expected order for success and failure.
- Cancel request affects queued jobs immediately and running jobs best-effort through adapter cancellation.
- No UI, network, LLM, or Blender-specific business logic leaks into generic scheduler code.

**Out of Scope**

- Multi-process scheduler.
- Persistent background daemon.
- Priority queues beyond FIFO.
- Distributed workers.

**Verification**

```bash
cargo test -p michelangelo_jobs
cargo test --workspace
```

---

### T-0204 Blender Worker Contract and Python Scripts

**Goal**

Implement a thin safe Blender Python worker that executes a strict JSON job spec.

**Scope**

- Fill `blender/worker.py`.
- Fill or replace `blender/smoke_create_cube.py` as a simple standalone smoke helper if still useful.
- Worker invocation contract:

```bash
blender --background --python blender/worker.py -- <job.json> <result.json>
```

- Worker reads `job.json`, validates operation names/parameters, executes safe handlers, writes `result.json`.
- Worker logs progress to stderr or an explicit log file, not stdout protocol.
- Worker result includes:
  - `job_id`;
  - `status`;
  - `message`;
  - `artifacts`;
  - `scene_summary`;
  - `error` if failed.

**Acceptance Criteria**

- Unknown op returns structured failure in `result.json`.
- Safe smoke spec creates a `.blend` file when run by real Blender.
- `.glb` export is implemented if available in the target Blender version; otherwise failure/skip is explicit and tested.
- Worker does not import project Rust code.
- Worker does not execute arbitrary Python from job input.

**Out of Scope**

- Advanced modeling.
- Asset Graph/reference image placement from PNGs.
- Long-lived Blender process.
- Blender addon.

**Verification**

```bash
python3 -m py_compile blender/worker.py blender/smoke_create_cube.py
${BLENDER_BIN:-blender} --background --python blender/worker.py -- <temp-job.json> <temp-result.json>
```

The real Blender command is allowed to be skipped only if Blender is not installed; record blocker.

---

### T-0205 Headless Blender Adapter

**Goal**

Launch Blender as a bounded subprocess from Rust and parse worker results.

**Scope**

- Implement in `crates/michelangelo_blender/`.
- Provide a `HeadlessBlenderAdapter` or equivalent.
- Resolve Blender executable from `BLENDER_BIN` or default `blender`.
- Write `job.json` under `jobs/<job_id>/`.
- Execute worker as:

```text
blender --background --python <repo-or-installed-worker.py> -- <job.json> <result.json>
```

- Capture stdout/stderr/logs.
- Enforce timeout and best-effort kill.
- Parse `result.json` into `BlenderJobResult`.
- Return structured errors for:
  - Blender binary not found;
  - non-zero exit;
  - timeout;
  - missing result file;
  - malformed result JSON.
- Provide fake executable support for deterministic integration tests.

**Acceptance Criteria**

- Fake success test passes without real Blender.
- Fake non-zero exit maps to failed job.
- Fake timeout maps to failed or cancelled job with clear error.
- Logs do not pollute protocol stdout.
- Artifact paths are workspace-relative and validated.

**Out of Scope**

- Persistent worker.
- Socket/HTTP/pipe transport to Blender.
- Real-time progress from Blender stdout unless implemented safely and tested.

**Verification**

```bash
cargo test -p michelangelo_blender
cargo test --workspace
```

---

### T-0206 Core Command Wiring

**Goal**

Expose job and Blender behavior through `CoreService` and the command router.

**Scope**

- Extend `CoreService` with job storage/scheduler/Blender adapter integration.
- Add handlers in `crates/michelangelo_core/src/router.rs` for:
  - `job.list`
  - `job.get`
  - `job.cancel`
  - `blender.get_capabilities`
  - `blender.run_job`
- Enforce current-project requirement for job operations.
- Update `project.get_snapshot` to include recent persisted jobs.
- Map workspace/job/blender errors into stable `ProtocolError` codes.

**Acceptance Criteria**

- Router tests prove all new methods are registered.
- `blender.run_job` without open project returns structured error.
- `job.list` returns persisted jobs after create/run/reopen.
- Snapshot includes recent jobs.
- Existing Phase 1 methods remain backward-compatible.

**Out of Scope**

- UI state management.
- LLM build-plan conversion.
- Asset Graph query context.

**Verification**

```bash
cargo test -p michelangelo_core
cargo test --workspace
```

---

### T-0207 CLI JSONL Event Transport

**Goal**

Make `michelangelo_cli core stdio` output responses and events as valid JSONL protocol messages.

**Scope**

- Update `crates/michelangelo_cli/src/stdio.rs` or supporting modules.
- Use one stdout writer path for both responses and events to avoid interleaved partial lines.
- Keep stderr for diagnostics only.
- Ensure clients can receive events before the final response for `wait=true` jobs.
- Preserve existing behavior for simple request/response commands.

**Acceptance Criteria**

- Existing `system.ping` smoke still returns one response line.
- Existing project smokes still pass.
- A job smoke can produce multiple event lines plus one response line.
- Every stdout line parses as JSON.
- Event lines have top-level `event`; response lines have top-level `id`.
- Tests cover malformed input and event serialization.

**Out of Scope**

- Network transport.
- MCP stdio adapter.
- UI subscriptions.

**Verification**

```bash
cargo test -p michelangelo_cli
cargo test --workspace
just smoke-jsonl
```

---

### T-0208 Polling, Snapshot, and Process-Boundary Job Scenario

**Goal**

Prove job state can be queried and recovered after process restart.

**Scope**

- Add or update smoke/integration scenario:
  1. process 1 creates project;
  2. process 1 runs fake or real Blender job;
  3. process 2 opens the same project;
  4. process 2 calls `job.list` and `project.get_snapshot`;
  5. output proves completed/failed job history persisted.
- Ensure `job.get` returns structured not-found error for unknown job id.

**Acceptance Criteria**

- Reopen scenario is automated.
- Job history survives process boundary through SQLite.
- Snapshot and `job.list` agree on recent job state.
- Existing `just smoke-reopen` still passes.

**Verification**

```bash
cargo test --workspace
just smoke-reopen
just smoke-e2e
```

---

### T-0209 Failure, Cancellation, and Timeout Hardening

**Goal**

Make failed Blender jobs diagnosable and bounded.

**Scope**

- Timeout handling for subprocess execution.
- Best-effort process kill on timeout/cancel.
- Structured failure status and error messages.
- Job log capture with reasonable size limits.
- Tests for:
  - missing Blender binary;
  - non-zero subprocess exit;
  - worker writes malformed result;
  - timeout;
  - cancellation of queued job;
  - cancellation of running job if supported.

**Acceptance Criteria**

- No hanging tests.
- Failed jobs reach terminal state and persist.
- `job.failed` or `job.cancelled` event is emitted.
- Error messages are useful but do not leak secrets or full environment dumps.

**Verification**

```bash
cargo test -p michelangelo_jobs
cargo test -p michelangelo_blender
cargo test --workspace
```

---

### T-0210 Verification Recipes and CI Alignment

**Goal**

Make Phase 2 verification repeatable locally and in CI.

**Scope**

- Update `justfile` with:
  - `smoke-blender-fake`
  - `smoke-blender-real`
  - optional `smoke-blender-health`.
- Keep existing recipes working.
- Update GitHub Actions only if appropriate:
  - mandatory checks remain Rust fmt/clippy/tests/build;
  - fake Blender smoke may run in CI;
  - real Blender smoke may be optional/manual/matrix-gated because installing Blender can be slow.
- No secrets in CI.

**Acceptance Criteria**

- `just --list` shows Phase 2 recipes.
- `just verify` passes.
- `just smoke-blender-fake` passes without real Blender.
- `just smoke-blender-real` either passes with artifacts or skips with explicit missing-Blender blocker.
- CI remains non-interactive.

**Verification**

```bash
just --list
just verify
just smoke-jsonl
just smoke-reopen
just smoke-e2e
just smoke-blender-fake
just smoke-blender-real
```

---

### T-0211 Real Blender Stand Smoke

**Goal**

Validate the actual Blender subprocess path on a local headless stand.

**Scope**

- Probe Blender health.
- Run `blender.run_job` through `michelangelo_cli core stdio`, not by calling Rust internals directly.
- Validate JSONL event/response stream.
- Validate output artifacts under temp workspace.
- Capture evidence:
  - Blender version;
  - command run;
  - artifact paths;
  - short event sequence;
  - cleanup result.

**Acceptance Criteria**

- Real Blender starts in background mode.
- Worker imports `bpy` and writes `result.json`.
- At least `.blend` artifact exists.
- `.glb` and renders either exist or are explicitly marked unsupported/skipped by the smoke spec.
- No protocol logs are printed to stdout except JSONL messages.

**Verification**

```bash
${BLENDER_BIN:-blender} --version
${BLENDER_BIN:-blender} --background --python-expr "import bpy; print('MICHELANGELO_BPY_OK')"
just smoke-blender-real
```

---

### T-0212 Documentation Sync

**Goal**

Update active docs to reflect Phase 2 implementation without claiming Phase 3/4 features.

**Scope**

- Update as applicable:
  - `README.md`
  - `TASKS.md`
  - `ROADMAP.md`
  - `docs/ai/AI_CONTEXT.md`
  - `docs/ai/AI_ENTRY_POINTS.md`
  - `docs/specs/BLENDER_WORKER.md`
  - `docs/specs/CORE_PROTOCOL.md`
- Document:
  - new crates/modules;
  - new protocol methods/events;
  - worker invocation contract;
  - `BLENDER_BIN` configuration;
  - smoke commands;
  - explicit out-of-scope items.

**Acceptance Criteria**

- Docs match actual commands and implemented method names.
- Docs do not claim PNG ingest, Asset Graph, LLM, UI, or MCP are complete.
- `TASKS.md` reflects completed Phase 2 tasks only after they are actually done.
- `ROADMAP.md` marks Phase 2 done only after final acceptance.

**Verification**

```bash
just verify
just smoke-blender-fake
```

Manual doc review against actual file paths and commands is required.

---

### T-0213 Final Acceptance Review

**Goal**

Verify Phase 2 evidence and close the TZ.

**Scope**

- Review all checked checklist items.
- Verify evidence exists for every checked implementation/test/doc item.
- Verify skipped checks have explicit reasons.
- Verify real Blender smoke was either passed or left unchecked with blocker.
- Verify no out-of-scope files/features were introduced.
- Verify no secrets or machine-specific paths were committed.

**Acceptance Criteria**

- `just verify` passes.
- Fake Blender smoke passes.
- Real Blender smoke passes on a machine with Blender installed, or remains unchecked with a clear blocker.
- Existing Phase 1 smokes still pass.
- Job history persists across process boundary.
- Progress events are visible as protocol JSONL.
- Documentation is current.
- Final acceptance checklist is updated by QA/verifier, not prematurely by implementers.

**Verification**

```bash
git branch --show-current
git status --short
just verify
just smoke-jsonl
just smoke-reopen
just smoke-e2e
just smoke-blender-fake
just smoke-blender-real
```

---

## 9. Acceptance Criteria by Feature

### Job Scheduling and Execution

- Jobs can be queued from protocol command.
- Jobs transition through valid lifecycle states.
- Job state is stored in SQLite and visible after reopen.
- `job.list` and `job.get` work for the active project.
- `job.cancel` is implemented for queued jobs and best-effort for running jobs.
- Queue is deterministic with max concurrency `1` for Phase 2.

### Blender Subprocess Management

- Blender executable path can be configured with `BLENDER_BIN`.
- Blender is launched only as background subprocess.
- Worker contract uses `job.json` and `result.json`.
- Timeout and non-zero exit are handled.
- Logs are captured without corrupting protocol stdout.
- Safe worker op set is enforced.
- Real smoke creates workspace artifacts.

### Job Progress Events

- `EventEnvelope` lines are emitted on stdout as JSONL protocol messages.
- Event names use `job.*` constants.
- Events can appear before the final command response.
- Response lines still preserve request `id`.
- Event tests cover success and failure paths.

### Regression Compatibility

- Existing methods keep their names and basic response contracts.
- Existing Phase 1 tests and smoke recipes remain green.
- Snapshot remains the recovery boundary for UI clients and now includes recent jobs.

---

## 10. Non-Applicable Checks

- **UI automation:** not applicable because Phase 2 has no GUI/TUI. Do not add UI code in this TZ.
- **Network/service stand:** not applicable because Phase 2 uses local CLI + SQLite + Blender subprocess only.
- **LLM/user prompt quality checks:** not applicable; LLM orchestration is Phase 4.
- **PNG/Asset Graph scenarios:** not applicable; asset pipeline is Phase 3.

---

## 11. System Prerequisites for Executors

Before putting this TZ into implementation, the machine should have:

- Rust stable toolchain with `cargo`, `rustfmt`, and `clippy`.
- `just` command runner.
- Python 3 for smoke validation scripts and worker syntax checks.
- A native build toolchain for Rust crates that compile C dependencies:
  - Linux: `build-essential` or equivalent (`gcc`, linker, libc headers);
  - `pkg-config` recommended.
- Blender CLI installed and available as `blender` in `PATH`, or path provided through `BLENDER_BIN`.
- Headless Blender runtime dependencies for Linux rendering/background mode:
  - OpenGL/Mesa runtime libraries;
  - X11/GL compatibility libraries commonly required by Blender packages.
- Standard shell/coreutils tools used by current smoke recipes: `bash`, `printf`, `mktemp`.

No API keys, network services, Docker services, databases, or LLM providers are required for Phase 2.
