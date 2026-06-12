# TZ: Phase 2 — Acceptance Hardening

## Execution Strategy

- [x] 🟢 Parallel execution recommended
- **Reason:** работа делится на независимые области: hardening smoke-команд и CI, уточнение runtime-контракта jobs/events, синхронизация документации и финальная проверка на реальном стенде. Параллелизм допустим после короткого Stage 0, где исполнитель фиксирует ветку, грязные изменения и текущий контракт. Один и тот же файл не должен редактироваться двумя исполнителями в одной стадии; общие файлы (`justfile`, `.github/workflows/ci.yml`, `docs/specs/*`, `ROADMAP.md`, `TASKS.md`, `README.md`) назначаются только одному work unit или интегратору.

## Execution Checklist

- [x] 0. Context verified
- [x] 1. Architecture boundaries confirmed
- [x] 2. Implementation stage 1 complete — smoke validation and real Blender stand contract hardened
- [x] 3. Implementation stage 2 complete — job/event/wait semantics made explicit and tested
- [x] 4. Implementation stage 3 complete — docs, roadmap, tasks, and previous Phase 2 TZ synchronized
- [x] 5. Unit/component tests complete
- [x] 6. Integration tests with real dependencies complete
- [x] 7. Stand smoke tests complete
- [x] 8. UI automation tests — not applicable: no UI/TUI files were touched
- [x] 9. User scenario tests complete
- [x] 10. Regression checks complete
- [x] 11. Documentation updated
- [x] 12. Final acceptance review complete — accepted with notes by QA on 2026-06-12

## Check Rules

- Architect creates this checklist and acceptance criteria.
- Executor agents may check implementation and test items only after implementation and verification evidence exists.
- QA verifier may check final acceptance only after reviewing evidence for every checked item.
- Failed, skipped, or unavailable checks stay unchecked with a blocker note.
- A skipped real Blender stand is not equivalent to a passed real Blender stand. It may be acceptable for local developer convenience, but final Phase 2 acceptance requires either a real pass or an explicit user-approved exception.
- UI automation is expected to remain **not applicable** because this TZ does not touch GUI/TUI. It may be checked only by QA with the note `not applicable: no UI touched` after confirming no UI/TUI files were changed.
- If a commit is later requested, agents must follow `AGENTS.md`: work only on `dev`, stage only explicit task-owned pathspecs, inspect staged diff, never push.

---

## 0. Context Authority

This TZ hardens the current Phase 2 Blender Integration before starting Phase 3 Asset Graph work.

Authoritative local rules:

- `/home/makc/AI_sandbox/Michelangelo/michelangelo/AGENTS.md`
  - agents work only on `dev`;
  - do not switch branches unless explicitly asked;
  - do not push;
  - if committing is explicitly requested later, stage only explicit task-owned pathspecs.
- Existing Phase 2 TZ:
  - `/home/makc/AI_sandbox/Michelangelo/michelangelo/docs/tz/TZ_PHASE_2_BLENDER_INTEGRATION_ITERATIVE.md`
- Active specs:
  - `/home/makc/AI_sandbox/Michelangelo/michelangelo/docs/specs/CORE_PROTOCOL.md`
  - `/home/makc/AI_sandbox/Michelangelo/michelangelo/docs/specs/BLENDER_WORKER.md`
  - `/home/makc/AI_sandbox/Michelangelo/michelangelo/docs/specs/MVP_SPEC.md`

Current known baseline from project inspection:

- Actual repo path: `/home/makc/AI_sandbox/Michelangelo/michelangelo`.
- Branch must be `dev`.
- Phase 1 is complete.
- Phase 2 is implemented enough for local checks, but final acceptance is not safe yet.
- Phase 3 Asset Graph is not started in code and is out of scope for this TZ.
- Existing local checks observed before this TZ:
  - `just verify` — passed;
  - `just smoke-e2e` — passed;
  - `just smoke-reopen` — passed;
  - `just smoke-blender-fake` — passed;
  - `just smoke-blender-real` printed `PASSED`, but the JSONL job actually ended with `status:"failed"` because `export_glb` failed in Blender Python with missing `numpy`.
- Known acceptance defect: `just smoke-blender-real` currently validates only the presence of events/response, not terminal success.
- Known contract ambiguity:
  - docs mention `job.progress`, but current execution path does not reliably emit progress during Blender work;
  - `wait=false` returns a queued job, but there is no clearly verified background execution loop;
  - `BLENDER_WORKER.md` says `render_views` uses `path`, while the worker code uses `base_path`.

Precedence if documents conflict:

1. explicit user instruction for the current run;
2. `AGENTS.md` and global agent workflow rules;
3. this hardening TZ;
4. existing Phase 2 TZ;
5. active specs;
6. generated or historical notes.

---

## 1. Goal

Close Phase 2 acceptance honestly before Phase 3 work begins.

Phase 2 hardening is acceptable when:

- smoke tests cannot produce false positives for failed Blender jobs;
- required real Blender smoke verifies a successful terminal job status, not only the presence of JSONL output;
- the required real smoke path does not depend on optional GLB export if the local Blender Python cannot run the glTF exporter dependency chain;
- optional GLB/export checks are separated, health-gated, and documented;
- `job.queued`, `job.started`, terminal events, `job.progress`, and `wait=false` semantics are documented and tested consistently with actual behavior;
- `ROADMAP.md`, `TASKS.md`, `README.md`, `CORE_PROTOCOL.md`, `BLENDER_WORKER.md`, and the previous Phase 2 TZ do not overstate acceptance;
- a QA/verifier can decide final acceptance from command evidence without reading implementation intent.

Non-goal: implement Asset Graph, PNG ingest, LLM orchestration, GUI/TUI, MCP, or network transport.

---

## 2. Scope

### In Scope

- Harden `just smoke-blender-real` validation.
- Harden `just smoke-blender-fake` validation if it has the same weak checks.
- Add or update JSONL validation so smoke recipes parse protocol output structurally, not by loose grep-only checks.
- Ensure smoke recipes fail when a supposed successful job ends with:
  - `job.failed`;
  - response `error`;
  - response `result.status != "completed"`;
  - missing expected artifact;
  - invalid JSONL line.
- Separate required `.blend` smoke from optional `.glb` export smoke.
- Decide and document Phase 2 semantics for:
  - required events;
  - optional or required `job.progress`;
  - `wait=true`;
  - `wait=false`.
- Align tests and docs with that decision.
- Fix or explicitly document `render_views` parameter naming mismatch (`path` vs `base_path`).
- Add deterministic fake Blender smoke to CI if it is intended to be part of the regression pack.
- Update documentation and tracking files so they match the verified state.

### Out of Scope

- PNG import/decomposition, masks, contours, Asset Graph tables, FTS/RTree.
- LLM provider integration, prompt compaction, BuildPlan generation.
- Shape-oriented Blender ops such as curve-from-contour, extrusion, holes, displacement.
- GUI/TUI and UI automation implementation.
- MCP adapter, HTTP server, named pipe server, or network transport.
- Persistent Blender daemon or Blender addon.
- Installing local packages into Blender Python from the task itself.
- Hardcoding machine-specific Blender paths or secrets.

---

## 3. Architecture Boundaries

- Rust Core remains a headless process driven through JSONL stdin/stdout.
- stdout remains protocol-only JSONL; human diagnostics go to stderr.
- Blender remains an external bounded batch executor launched through a configured binary.
- Workspace SQLite DB remains under `.michelangelo/metadata.db`.
- Smoke tests must use temporary workspaces and clean them up.
- Artifact paths in protocol results must be workspace-relative or explicitly validated as under the temporary workspace.
- No arbitrary Python execution is allowed through Blender worker specs.
- Phase 2 final acceptance must not rely on Phase 3 functionality.

---

## 4. Execution Plan

### Stage 0 — Sequential context gate

Owner: orchestrator / first executor.

Required actions before edits:

1. Run:
   - `git branch --show-current`
   - `git status --short`
2. Stop if branch is not `dev`.
3. Record dirty files and distinguish:
   - pre-existing Phase 2 changes;
   - files changed by this hardening task.
4. Re-read:
   - `AGENTS.md`;
   - this TZ;
   - `docs/tz/TZ_PHASE_2_BLENDER_INTEGRATION_ITERATIVE.md`;
   - `justfile`;
   - `docs/specs/CORE_PROTOCOL.md`;
   - `docs/specs/BLENDER_WORKER.md`.
5. Do not start Phase 3 work.

Exit criteria:

- branch is confirmed as `dev`;
- task-owned files are listed;
- unrelated dirty changes are protected;
- executor report can distinguish baseline changes from new changes.

### Stage 1 — Parallel work units

Run these units in parallel only if each has exclusive file ownership.

#### Unit A — Smoke validation and stand hardening

Primary ownership:

- `justfile`
- optional helper under a task-owned path if the executor chooses to avoid large inline shell/Python snippets, for example `scripts/validate_blender_smoke.py`
- `.github/workflows/ci.yml` only if adding deterministic fake Blender smoke to CI is assigned to this unit

Required changes:

1. Make `just smoke-blender-real` structurally validate JSONL output.
2. Required real smoke path must verify at minimum:
   - project creation response is successful;
   - `blender.run_job(wait=true)` response exists;
   - terminal response has `result.status == "completed"`;
   - no terminal `job.failed` event is present for the success path;
   - expected `.blend` artifact exists and has size > 0;
   - protocol output lines are valid JSON.
3. Remove `export_glb` from the required real smoke path **or** guard it so GLB dependency failures cannot cause a false `PASSED` result.
4. If GLB remains useful, add a separate optional check, e.g. `smoke-blender-real-glb`, with explicit behavior:
   - skip with clear diagnostic if Blender/glTF prerequisites are unavailable;
   - fail nonzero if the optional command is run in strict mode and job fails;
   - never print `PASSED` when response status is `failed`.
5. Harden `just smoke-blender-fake` similarly:
   - require `job.queued`, `job.started`, terminal success, and completed response;
   - validate fake artifact metadata returned by the fake worker.
6. Avoid fragile validation based only on `grep`/`tail` presence checks. A small `python3` JSON parser is acceptable for smoke validation.

Acceptance criteria:

- A failed Blender job cannot make `just smoke-blender-real` print `PASSED`.
- Current known `export_glb`/missing-`numpy` failure is either:
  - outside the required smoke path; or
  - reported as an optional GLB failure/skip; or
  - fails the command honestly without false pass.
- If Blender binary is present and `.blend` save succeeds, required real smoke passes without requiring GLB.
- If Blender binary is absent, command may skip for developer convenience, but the executor must report stand smoke as skipped, not passed.

#### Unit B — Job/event/wait semantics hardening

Primary ownership examples:

- `crates/michelangelo_core/src/router.rs`
- `crates/michelangelo_jobs/src/lib.rs`
- `crates/michelangelo_protocol/src/job.rs`
- `crates/michelangelo_protocol/src/event.rs`
- related Rust tests under the owning crates

Required decision:

Make one explicit Phase 2 decision and align code/tests/docs with it.

Decision B1 — `job.progress`:

- Preferred minimal acceptance: required events are `job.queued`, `job.started`, and one terminal event (`job.completed`, `job.failed`, or `job.cancelled`). `job.progress` is optional in Phase 2 because current batch worker does not stream incremental progress.
- Alternative acceptable implementation: emit deterministic progress events for `wait=true` jobs and test them.
- Not acceptable: docs/tests claim `job.progress` is required while implementation does not provide it.

Decision B2 — `wait=false`:

- Preferred minimal acceptance: document and test `wait=false` as queue-only for the current stdio/core process unless a background runner is actually implemented. It may return a persisted queued job, but must not be described as completed background execution.
- Alternative acceptable implementation: implement and test real background execution while the core process remains alive.
- Not acceptable: protocol docs promise background execution while jobs stay queued forever in the actual Phase 2 path.

Required validation:

- Add or update tests for the chosen event contract.
- Add or update tests for the chosen `wait=false` contract.
- Ensure `job.list` / `job.get` behavior matches persisted job status after the tested scenario.

Acceptance criteria:

- There is no mismatch between implementation, tests, `CORE_PROTOCOL.md`, and the existing Phase 2 TZ.
- `wait=true` successful path ends with persisted `completed` status and a completed response.
- Failed jobs persist `failed` and emit/report failure honestly.
- `wait=false` behavior is either implemented as background execution or explicitly documented/tested as queue-only.

#### Unit C — Documentation and tracking sync

Primary ownership:

- `README.md`
- `ROADMAP.md`
- `TASKS.md`
- `docs/specs/CORE_PROTOCOL.md`
- `docs/specs/BLENDER_WORKER.md`
- `docs/tz/TZ_PHASE_2_BLENDER_INTEGRATION_ITERATIVE.md`
- `docs/ai/AI_CONTEXT.md`
- `docs/ai/AI_ENTRY_POINTS.md`

Required changes:

1. Document required vs optional Phase 2 smoke checks:
   - required fake Blender smoke;
   - required real `.blend` smoke when Blender is available;
   - optional GLB/export smoke if applicable.
2. Document real stand prerequisites by environment variable names only:
   - `BLENDER_BIN`;
   - `CARGO_TARGET_DIR` if relevant;
   - no secret values.
3. Align `CORE_PROTOCOL.md` with the chosen `job.progress` and `wait=false` behavior.
4. Align `BLENDER_WORKER.md` with actual worker parameters, especially `render_views` `path` vs `base_path`.
5. Update `ROADMAP.md` and `TASKS.md` so Phase 2 is not marked final-accepted until this hardening evidence exists.
6. Update the previous Phase 2 TZ checklist/status only after evidence exists; do not check final acceptance prematurely.
7. If deterministic fake smoke is added to CI, update docs to say so.

Acceptance criteria:

- Docs no longer imply that Phase 2 real Blender smoke passed if the actual terminal job status was `failed`.
- GLB dependency behavior is explicit and not hidden behind a passing smoke command.
- Phase 3 remains clearly blocked on Phase 2 acceptance, not on optional GLB perfection.

### Stage 2 — Integration and verification

Owner: orchestrator / integration executor.

Required actions:

1. Resolve conflicts between Units A/B/C.
2. Run static and regression checks from this TZ.
3. Run fake Blender smoke.
4. Run real Blender smoke if Blender is available.
5. Produce evidence table.
6. Update checkboxes only for checks that actually passed or were QA-confirmed as not applicable.

---

## 5. Required Test Strategy

### Static checks

Required:

- `git branch --show-current`
- `git status --short`
- `just fmt-check`
- `just clippy`

Acceptance:

- branch is `dev`;
- unrelated changes are identified and protected;
- formatting and clippy pass.

### Unit tests

Required:

- `cargo test --workspace`

Recommended targeted tests before full workspace test if implementation touches a single crate:

- `cargo test -p michelangelo_core`
- `cargo test -p michelangelo_jobs`
- `cargo test -p michelangelo_protocol`
- `cargo test -p michelangelo_blender`
- `cargo test -p michelangelo_cli`

Acceptance:

- touched-crate tests pass;
- full workspace tests pass before final acceptance.

### Component tests

Required where code changes touch router/stdio/protocol:

- CLI/core JSONL tests must validate event ordering and terminal response semantics.
- Tests must cover successful `wait=true` path and selected `wait=false` behavior.

Acceptance:

- JSONL output remains machine-parseable;
- stdout protocol is not polluted with human logs;
- terminal job status in response and persisted job status agree.

### Integration tests with real dependencies

Required:

- temp SQLite workspace under `.michelangelo/metadata.db`;
- fake Blender subprocess smoke:
  - `just smoke-blender-fake`.

Acceptance:

- fake smoke validates terminal success structurally;
- persisted job history can be read after the run if the scenario covers reopen/list;
- no false positive is possible for failed response status.

### Real stand smoke tests

Required when Blender is available:

- `just smoke-blender-real`

Optional if implemented:

- `just smoke-blender-real-glb`

Acceptance:

- required real smoke verifies `.blend` creation and terminal `completed` status;
- if Blender is absent, report skipped with blocker note;
- if Blender is present and the job fails, command exits nonzero and the TZ check remains unchecked;
- optional GLB failure must not invalidate required `.blend` smoke unless the optional GLB command was explicitly selected as required by the user.

### UI automation

Not applicable by default.

Reason:

- this TZ touches headless core, subprocess smoke, and docs only;
- no GUI/TUI exists in Phase 2 scope.

If UI/TUI files are touched unexpectedly:

- stop and ask for scope confirmation;
- define a platform-specific UI automation check before accepting the change.

### User scenario tests

Required scenario:

```text
project.create
  → blender.run_job(wait=true, create_box + save_blend)
  → JSONL events include job.queued and job.started
  → terminal event is job.completed
  → response result.status == "completed"
  → .blend artifact exists under workspace and size > 0
  → job.get/job.list or snapshot/reopen shows persisted completed job where covered by current commands
```

Acceptance:

- scenario passes with fake Blender;
- scenario passes with real Blender when Blender is available;
- no scenario check passes on `status:"failed"`.

### Regression pack

Required:

- `just verify`
- `just smoke-e2e`
- `just smoke-reopen`
- `just smoke-blender-fake`
- `just smoke-blender-real` when Blender is available

Optional / environment-dependent:

- optional GLB smoke command if added.

Acceptance:

- all required available checks pass;
- skipped checks are explicitly reported as skipped and remain unchecked unless QA/user grants an exception.

### Acceptance review

Required evidence table in executor report:

```markdown
## Evidence

| Check | Command / Tool | Result | Evidence |
|---|---|---|---|
| Branch | `git branch --show-current` | pass/fail | branch name |
| Status | `git status --short` | pass/fail | unrelated changes note |
| Static | `just fmt-check` / `just clippy` | pass/fail/skipped | short note/log path |
| Unit tests | `cargo test --workspace` | pass/fail/skipped | short note/log path |
| E2E smoke | `just smoke-e2e` | pass/fail/skipped | short note/log path |
| Reopen smoke | `just smoke-reopen` | pass/fail/skipped | short note/log path |
| Fake Blender | `just smoke-blender-fake` | pass/fail/skipped | terminal status/artifact note |
| Real Blender | `just smoke-blender-real` | pass/fail/skipped | `BLENDER_BIN` availability, terminal status, artifact note |
| Optional GLB | `<command if added>` | pass/fail/skipped/not applicable | diagnostic note |
| Docs sync | file review | pass/fail | changed docs list |
```

---

## 6. Real Test Stand Definition

### Database

- Type: SQLite.
- Location: `.michelangelo/metadata.db` inside a temporary workspace created by smoke commands.
- Lifecycle:
  - create temp workspace;
  - run `project.create` through core JSONL;
  - run Blender job;
  - inspect protocol response and artifacts;
  - cleanup temp workspace with the recipe trap.

### Seed data

- No pre-existing seed DB required.
- Smoke command creates a project and one Blender smoke job.
- Required job operations for real smoke:
  - `create_box`;
  - `save_blend`.
- Optional operations only in optional checks:
  - `export_glb`;
  - render/export checks that require additional local Blender dependencies.

### Services and processes

- Rust CLI/core process:
  - `cargo run -p michelangelo_cli -- core stdio`, or prebuilt debug binary from `cargo build -p michelangelo_cli`.
- Blender process:
  - `${BLENDER_BIN:-blender} --background --python blender/worker.py -- <job.json> <result.json>` through the Rust adapter.
- No external DB daemon.
- No network service.
- No UI service.

### Environment variables

Names only; never document secret values:

- `BLENDER_BIN` — optional path/name of Blender executable.
- `CARGO_TARGET_DIR` — optional Cargo target directory override.
- `RUST_LOG` — optional diagnostics if already supported by local tooling.

### Health checks

Required before real stand pass can be claimed:

- Blender binary exists or command reports skip:
  - `${BLENDER_BIN:-blender} --version`
- CLI builds:
  - `cargo build -p michelangelo_cli`
- Required real smoke parses JSONL and terminal response:
  - `result.status == "completed"`
- Required `.blend` artifact exists and has nonzero size.

### Reset / cleanup

- Smoke commands must use `mktemp -d` or equivalent temporary workspace.
- Temporary workspace must be removed after successful or failed run unless the executor intentionally preserves it for failure investigation and reports the path.
- Do not run destructive Docker/database cleanup commands for this TZ.

---

## 7. File Ownership Guide

Use this guide to avoid parallel edit conflicts.

### Smoke/CI unit

- `justfile`
- `.github/workflows/ci.yml`
- optional task-owned helper script if introduced

### Core semantics unit

- `crates/michelangelo_core/src/router.rs`
- `crates/michelangelo_core/src/service.rs`
- `crates/michelangelo_jobs/src/lib.rs`
- `crates/michelangelo_protocol/src/*.rs`
- related crate tests

### Blender adapter/worker unit, only if needed

- `crates/michelangelo_blender/src/lib.rs`
- `blender/worker.py`
- related tests

Do not modify worker behavior unless smoke hardening or documented contract mismatch requires it.

### Documentation unit

- `README.md`
- `ROADMAP.md`
- `TASKS.md`
- `docs/specs/CORE_PROTOCOL.md`
- `docs/specs/BLENDER_WORKER.md`
- `docs/tz/TZ_PHASE_2_BLENDER_INTEGRATION_ITERATIVE.md`
- `docs/ai/AI_CONTEXT.md`
- `docs/ai/AI_ENTRY_POINTS.md`

---

## 8. Required Acceptance Decisions

Executor must record final decisions in the report and docs.

### Decision 1 — Required real smoke contents

Default decision for this TZ:

- Required: `create_box` + `save_blend` + completed terminal status + nonzero `.blend` artifact.
- Optional: `export_glb`.

Rationale:

- Current local Blender can create/save `.blend` but `export_glb` may fail because Blender's glTF exporter imports `numpy` and local Blender Python may not provide it.
- Phase 2 should verify Blender subprocess execution without making optional export dependencies a false acceptance blocker.

### Decision 2 — GLB check behavior

Default decision for this TZ:

- GLB is a separate optional smoke/health check.
- Optional GLB command must never print `PASSED` on failed job status.
- Docs must say GLB support depends on local Blender/glTF exporter prerequisites.

### Decision 3 — `job.progress`

Default decision for this TZ:

- `job.progress` is optional for Phase 2 batch jobs unless implemented and tested.
- Required lifecycle events for smoke acceptance are `job.queued`, `job.started`, and one terminal event.

### Decision 4 — `wait=false`

Default decision for this TZ:

- `wait=true` is the accepted execution path for Phase 2 smoke.
- `wait=false` must be documented/tested according to actual behavior:
  - either queue-only and not guaranteed to run in background;
  - or true background execution if implemented in this hardening task.

---

## 9. Definition of Done

This TZ is done when all are true:

- `just smoke-blender-real` cannot pass on `status:"failed"`.
- Required real smoke no longer requires GLB export unless the environment explicitly supports it and the command validates it honestly.
- Fake Blender smoke validates terminal success structurally.
- `job.progress` and `wait=false` are documented and tested according to actual behavior.
- `render_views` parameter naming is either fixed or documented consistently.
- `ROADMAP.md`, `TASKS.md`, existing Phase 2 TZ, and active specs are synchronized.
- Regression pack evidence is recorded.
- UI automation is explicitly marked not applicable or re-scoped if UI was touched.
- Final acceptance remains unchecked until QA/verifier reviews evidence.

---

## 10. Handoff to Phase 3

Do not start Phase 3 Asset Graph implementation until this hardening is accepted or explicitly waived by the user.

After acceptance, the next architecture task should be a separate TZ for:

- `docs/specs/ASSET_GRAPH.md` contract;
- Phase 3 SQLite schema for `assets`, `analysis_runs`, `shapes`, `shape_relations`, `sidecars`;
- first PNG MVP slice: `asset.import_png`, sha256, dimensions, alpha mask, bbox, one foreground shape, cropped mask sidecar, snapshot asset summary.
