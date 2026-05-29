# TZ: MVP Headless Rust Core Iterative

## Execution Checklist

- [x] 0. Context verified
- [x] 1. Architecture boundaries confirmed
- [x] 2. Implementation stage 1 complete: `T-0001`–`T-0004` foundation
- [x] 3. Implementation stage 2 complete: `T-0005`–`T-0008` workspace/snapshot/verification
- [ ] 4. Implementation stage 3 complete: `T-0009+` headless core hardening
- [x] 5. Unit/component tests complete
- [x] 6. Integration tests with real dependencies complete
- [x] 7. Stand smoke tests complete
- [ ] 8. UI automation tests complete — not applicable: no UI touched
- [ ] 9. User scenario tests complete
- [x] 10. Regression checks complete
- [x] 11. Documentation updated
- [ ] 12. Final acceptance review complete

## Check Rules

- Architect creates this checklist and acceptance criteria.
- Executor agents may check implementation/test items only after implementation and verification evidence exists.
- QA verifier may check final acceptance only after reviewing evidence for every checked item.
- Failed, skipped, or unavailable checks stay unchecked with a blocker note.
- No executor may replace real smoke/integration checks with unit tests only when runtime behavior is touched.

---

## Execution Strategy

- Sequential execution recommended.
- **Reason:** первые итерации создают общий Rust workspace, базовый protocol crate, CLI entry point и JSONL-transport. Эти задачи зависят друг от друга и затрагивают общие файлы (`Cargo.toml`, protocol DTOs, CLI bootstrap), поэтому параллельное исполнение на старте создаст лишние конфликты. После `T-0008` допускается точечная параллелизация только для независимых crates/areas, если orchestrator явно разделил ownership файлов и сделал integration checkpoint.

---

## 0. Context Authority

This TZ is based on the required project context files:

- `AGENTS.md`
- `README.md`
- `ROADMAP.md`
- `TASKS.md`
- `docs/ai/AI_CONTEXT.md`
- `docs/ai/generated/REPO_MAP.md`
- `docs/specs/MVP_SPEC.md`

Relevant current-state notes from context review:

- `docs/specs/MVP_SPEC.md` is the main active architecture/spec source.
- `docs/ai/generated/REPO_MAP.md` shows existing module directories under `crates/`, but no `Cargo.toml` files were detected at TZ creation time.
- Several active docs are currently empty placeholders; do not infer hidden requirements from them.
- `blender/worker.py` and `crates/michelangelo_blender/` exist in the repository map, but this TZ explicitly excludes Blender integration.

If later documents contradict this TZ, follow this precedence unless the user says otherwise:

1. explicit user instruction for the current run;
2. `AGENTS.md` and local workflow rules;
3. `docs/specs/MVP_SPEC.md`;
4. this TZ;
5. generated repo maps and historical notes.

---

## 1. Branch, Git, and Orchestrator Rules

This TZ is intended for autonomous OpenCode/GNHF-style orchestration inside the current repository policy.

Mandatory rules for every iteration:

- Agents work only on the `dev` branch.
- Agents must not work on `main`, `gnhf/*`, or any other branch unless the user first changes the local repository policy.
- Agents must not commit to `main`, `gnhf/*`, or any other non-`dev` branch.
- Agents must not push. The user performs all pushes manually.
- Before edits, run `git branch --show-current` and stop if the branch is not `dev`.
- If local `AGENTS.md` is stricter than this TZ, stop and report the stricter rule instead of bypassing it.
- Inspect `git status --short` before edits; do not overwrite unrelated user/agent changes.
- Stage only task-owned files with explicit pathspecs if a commit is explicitly requested by the user.
- Do not use broad `git add .` or `git add -A`.
- Do not commit unless the user/orchestrator policy explicitly requests it and relevant checks passed.
- Each iteration must be small, reversible, and independently verifiable.

- If an external orchestrator expects task branches, stop and ask the user to update `AGENTS.md` before creating or using them.

---

## 2. Goal

Prepare and then implement, through small autonomous iterations, the MVP slice of **headless Rust Core** for Michelangelo.

For this TZ, “MVP headless Rust Core” means:

- Rust workspace exists and builds.
- Core process can be exercised without GUI/TUI.
- Protocol DTOs and JSONL stdin/stdout smoke path exist.
- CLI can bootstrap the core process and run minimal commands.
- Workspace create/open and `project.get_snapshot` work in a deterministic local filesystem test workspace.
- Verification commands and minimal CI exist.
- Blender integration is not implemented in this TZ.

This is a smaller implementation slice than the full product MVP described in `docs/specs/MVP_SPEC.md`, which later includes PNG ingest, Asset Graph indexing, LLM orchestration, and Blender jobs.

---

## 3. Scope

### In Scope

- Rust workspace skeleton.
- Minimal crates required for the headless core foundation:
  - `crates/michelangelo_protocol/`
  - `crates/michelangelo_core/`
  - `crates/michelangelo_workspace/`
  - `crates/michelangelo_cli/`
- Protocol DTOs for commands, responses, errors, events, project metadata, and snapshots.
- JSONL over stdin/stdout transport smoke path.
- CLI bootstrap for local headless checks.
- Deterministic workspace create/open behavior.
- `project.get_snapshot` minimal state recovery command.
- `justfile` verification commands.
- Minimal CI for formatting, linting, tests, and smoke checks.
- Unit/component/integration/smoke tests needed by each task.

### Out of Scope for This TZ

- Blender worker changes.
- Any edits under `blender/`.
- Any functional implementation under `crates/michelangelo_blender/`.
- GUI work.
- TUI work.
- Blender subprocess execution.
- Blender job generation/execution.
- LLM provider integration.
- Network APIs or remote services.
- MCP adapter.
- Qdrant, embeddings, vector databases, or non-SQLite sidecars.
- PNG decomposition pipeline unless a later TZ explicitly authorizes it.
- Architecture redesign beyond `docs/specs/MVP_SPEC.md`.

### Dependency Policy

- Do not add dependencies speculatively.
- If a task genuinely requires a dependency, the executor must keep it minimal, justify it in the task report, and avoid Blender/GUI/TUI/network dependencies for this TZ.
- Heavy optional systems from the broader MVP (`Qdrant`, MCP, Blender daemon/addon, GUI frameworks) are forbidden in this TZ.

---

## 4. Architecture Boundaries to Preserve

- Rust Core is a separate headless process.
- UI shells communicate with Core through stable protocol messages; no UI is implemented here.
- MVP transport for this slice is JSONL over stdio.
- stdout must contain protocol JSONL messages only.
- stderr is for human-readable logs and diagnostics.
- Every request/response uses an `id`; events do not require an `id`.
- `project.get_snapshot` is the explicit state recovery boundary for future UI shells.
- Workspace filesystem is the local source of project files for this slice.
- SQLite-first remains the broader storage direction from `MVP_SPEC.md`; do not introduce another primary metadata backend.
- Blender is an external future executor, not part of this headless-core iteration.

---

## 5. Iteration Protocol for GNHF/OpenCode

For every `T-XXXX` task:

1. Re-read this TZ section for the selected task.
2. Confirm branch rule: only `dev`, never `main`, `gnhf/*`, or any other branch.
3. Inspect `git status --short` and protect unrelated changes.
4. Implement only the task scope.
5. Do not expand architecture or dependencies outside the task.
6. Run the task verification commands.
7. Run the smallest useful broader regression check available at that stage.
8. Update task evidence in the completion report.
9. Leave checklist items unchecked when verification fails or is unavailable.

Each task should be small enough to review as one iteration and should have a visible pass/fail signal.

Required completion report shape:

```markdown
## Evidence

| Check | Command / Tool | Result | Evidence |
|---|---|---|---|
| Branch | `git branch --show-current` | pass/fail | branch name |
| Static | `<command>` | pass/fail/skipped | short note/log path |
| Unit | `<command>` | pass/fail/skipped | short note/log path |
| Integration | `<command>` | pass/fail/skipped | temp workspace/DB note |
| Smoke | `<command>` | pass/fail/skipped | JSONL/CLI output note |
```

---

## 6. Test Strategy and Stand

### Static Checks

Applicable once Rust workspace exists:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

After `T-0007`, prefer:

- `just fmt-check`
- `just clippy`
- `just test`
- `just verify`

### Unit Tests

Use for:

- DTO serialization/deserialization.
- Error envelope mapping.
- Command routing.
- Workspace path validation.
- Snapshot construction.

### Component Tests

Use for:

- CLI help/version/bootstrap behavior.
- JSONL loop behavior with in-memory input/output.
- Core command handler behavior through public crate APIs.

### Integration Tests

Use real local dependencies where applicable:

- temporary filesystem workspace;
- SQLite file only after a SQLite task introduces it;
- no mocked filesystem for workspace create/open acceptance;
- no Blender, GUI, TUI, network, or LLM service.

### Real Stand Smoke Tests

The stand for this TZ is local and headless:

- database: none until SQLite task; then a temp SQLite file under a temp workspace;
- services to start: none;
- environment variables: none required; `RUST_LOG` may be used for diagnostics only;
- health checks: CLI `--help`, CLI `--version`, JSONL `system.ping`, and project command smoke;
- reset/cleanup: delete temporary workspace directory created by the test command;
- secrets: none.

Example smoke shape after `T-0004`:

```bash
printf '{"id":"1","method":"system.ping","params":{}}\n' \
  | cargo run -p michelangelo_cli -- core stdio
```

### UI Automation

Not applicable in this TZ because GUI/TUI work is explicitly out of scope. Leave the UI automation checklist item unchecked with note `not applicable: no UI touched` during QA review.

### User Scenarios

Applicable as headless CLI/protocol scenarios:

- create workspace;
- open workspace;
- request snapshot;
- verify JSONL response contract.

### Regression Pack

At minimum, after `T-0008`:

- `just verify` or equivalent raw cargo commands;
- JSONL ping smoke;
- workspace create/open/snapshot smoke.

---

## 7. Integration Checkpoints

### Checkpoint A — after `T-0004`

- Rust workspace builds.
- CLI can start.
- JSONL `system.ping` returns a valid response.
- stdout contains JSONL only.

### Checkpoint B — after `T-0008`

- Workspace create/open and snapshot path exists.
- `just verify` or raw cargo equivalent passes.
- Minimal CI exists and mirrors local verification.

### Checkpoint C — after `T-0012+`

- Storage-backed project metadata, if implemented, works in a temp workspace.
- Existing JSONL/CLI smoke remains compatible.
- No Blender/GUI/TUI files were modified.

---

## 8. Task Backlog

### T-0001 Rust workspace skeleton

**Goal**

Create the minimal Rust workspace foundation for the headless Core slice.

**Scope**

- Add root Rust workspace configuration.
- Add minimal crate manifests and placeholder source files only for crates needed by the first headless slice:
  - `michelangelo_protocol`
  - `michelangelo_core`
  - `michelangelo_workspace`
  - `michelangelo_cli`
- Keep all placeholder APIs minimal and compile-only.
- Do not include Blender crate work in this task.

**Acceptance Criteria**

- `cargo metadata --format-version 1` succeeds.
- `cargo test --workspace` succeeds.
- Workspace members are intentional and documented in the task report.
- No files under `blender/` are changed.
- No GUI/TUI crates or dependencies are added.

**Out of Scope**

- Protocol semantics.
- CLI command implementation beyond compile bootstrap.
- Workspace create/open behavior.
- SQLite schema.
- Blender, GUI, TUI, LLM, MCP.

**Verification**

- `git branch --show-current`
- `git status --short`
- `cargo metadata --format-version 1`
- `cargo fmt --all -- --check`
- `cargo test --workspace`

---

### T-0002 protocol DTOs

**Goal**

Define the first stable protocol DTOs needed by JSONL command/response flow and future UI state recovery.

**Scope**

- Implement DTOs in `crates/michelangelo_protocol/` for:
  - `CommandEnvelope`
  - `ResponseEnvelope`
  - `EventEnvelope`
  - protocol error body
  - `ProjectDto`
  - minimal `ProjectSnapshotDto`
- Define initial method names:
  - `system.ping`
  - `project.create`
  - `project.open`
  - `project.get_snapshot`
- Add serialization/deserialization tests and JSON examples/golden fixtures if practical.

**Acceptance Criteria**

- Request DTOs deserialize from one-line JSON.
- Response DTOs serialize to one-line JSON.
- Error responses preserve request `id` when available.
- DTOs do not reference Blender, GUI, TUI, or LLM implementation types.
- Tests cover success and error envelope examples.

**Out of Scope**

- Actual command execution.
- CLI stdio loop.
- Workspace filesystem behavior.
- Full Asset Graph DTO set.
- `BlenderJobSpec` implementation.

**Verification**

- `cargo fmt --all -- --check`
- `cargo test -p michelangelo_protocol`
- `cargo test --workspace`

---

### T-0003 CLI bootstrap

**Goal**

Provide a minimal CLI binary that can launch headless checks without GUI/TUI.

**Scope**

- Implement `michelangelo_cli` binary entry point.
- Support `--help` and `--version`.
- Add a stable subcommand placeholder for stdio mode, recommended shape: `core stdio`.
- Ensure CLI logs/diagnostics go to stderr, not protocol stdout.
- Keep CLI as a thin shell over core/protocol crates.

**Acceptance Criteria**

- `cargo run -p michelangelo_cli -- --help` exits successfully.
- `cargo run -p michelangelo_cli -- --version` exits successfully.
- CLI does not import or call Blender code.
- CLI does not implement GUI/TUI behavior.
- CLI bootstrap has component tests where practical.

**Out of Scope**

- Full command set.
- Project creation/opening.
- JSONL processing beyond a placeholder.
- Shell completions/installers/packages.

**Verification**

- `cargo fmt --all -- --check`
- `cargo test -p michelangelo_cli`
- `cargo run -p michelangelo_cli -- --help`
- `cargo run -p michelangelo_cli -- --version`

---

### T-0004 JSONL stdin/stdout smoke

**Goal**

Make the core process testable through JSONL over stdin/stdout with a minimal smoke command.

**Scope**

- Implement stdio JSONL loop for `core stdio`.
- Implement `system.ping` end-to-end through protocol/core/CLI.
- Read one JSON message per line from stdin until EOF.
- Write one JSON response per line to stdout.
- Write human-readable diagnostics only to stderr.
- Return structured error response for malformed JSON or unknown method.

**Acceptance Criteria**

- Valid ping request returns a valid JSONL response with the same `id`.
- Unknown method returns protocol error response.
- Malformed JSON does not panic and returns/prints a controlled error according to protocol rules.
- stdout contains only JSON protocol lines during stdio mode.
- Smoke command works from shell pipe.

**Out of Scope**

- Long-running event streaming.
- Async runtime selection unless already justified by earlier tasks.
- Project workspace commands.
- Network transport.

**Verification**

- `cargo test --workspace`
- `printf '{"id":"1","method":"system.ping","params":{}}\n' | cargo run -p michelangelo_cli -- core stdio`
- Unknown-method smoke, for example `method":"unknown.method"`.
- Malformed JSON smoke, with expected controlled failure behavior documented.

---

### T-0005 workspace create/open

**Goal**

Implement deterministic local workspace create/open behavior through the headless core boundary.

**Scope**

- Implement workspace service in `crates/michelangelo_workspace/`.
- Add core handlers for `project.create` and `project.open`.
- Expose commands through JSONL stdio.
- Create/validate the minimal local directory structure needed by the headless slice.
- Return `ProjectDto` with stable project id/name/root path fields.
- Use temporary directories in integration tests.

**Acceptance Criteria**

- `project.create` creates a project workspace at a requested path.
- `project.open` validates and opens an existing project workspace.
- Re-running create on an existing valid workspace is deterministic: either idempotent or returns a documented structured error.
- Invalid paths return structured errors, not panics.
- Tests use real temporary filesystem directories.
- No Blender worker is invoked or modified.

**Out of Scope**

- PNG import.
- Asset Graph building.
- SQLite schema unless explicitly introduced by this task with a minimal dependency justification.
- Blender output generation.
- GUI/TUI project explorer.

**Verification**

- `cargo test -p michelangelo_workspace`
- `cargo test --workspace`
- JSONL smoke for `project.create` using a temp directory.
- JSONL smoke for `project.open` using the same temp directory.

---

### T-0006 project.get_snapshot

**Goal**

Implement minimal explicit state recovery for future UI shells through `project.get_snapshot`.

**Scope**

- Add core handler for `project.get_snapshot`.
- Return minimal `ProjectSnapshotDto` containing:
  - current project metadata;
  - empty asset list or current known assets if already implemented;
  - empty/recent job list if jobs are not implemented yet;
  - workspace status;
  - protocol/core version metadata if available.
- Support snapshot after `project.create` and after `project.open`.
- Add tests for snapshot shape and error behavior when no project is open.

**Acceptance Criteria**

- Snapshot response is valid JSONL and uses the request `id`.
- Snapshot is deterministic for a newly created workspace.
- Snapshot does not require GUI/TUI state.
- Snapshot does not read Blender outputs or invoke Blender.
- Error response for no-open-project case is structured and tested.

**Out of Scope**

- Full Asset Graph contents.
- Render/model outputs.
- Live event replay.
- UI state hydration beyond protocol DTO shape.

**Verification**

- `cargo test --workspace`
- JSONL scenario: `project.create` → `project.get_snapshot`.
- JSONL scenario: `project.open` → `project.get_snapshot`.
- No-open-project error smoke.

---

### T-0007 justfile verification commands

**Goal**

Make local verification repeatable for autonomous agents and humans.

**Scope**

- Populate `justfile` with small non-interactive commands:
  - `fmt-check`
  - `fmt`
  - `clippy`
  - `test`
  - `verify`
  - `smoke-jsonl`
- `verify` should run the agreed static/test suite available at this stage.
- `smoke-jsonl` should run a minimal JSONL ping and, if available, project snapshot smoke.
- Keep commands Linux-friendly and non-interactive.

**Acceptance Criteria**

- `just --list` shows the verification commands.
- `just verify` passes in a clean local checkout with Rust toolchain installed.
- `just smoke-jsonl` exercises the CLI/protocol path.
- Commands do not start Blender, GUI, TUI, or network services.

**Out of Scope**

- Docker-based stand.
- Release packaging.
- Installer scripts.
- CI-only commands that cannot run locally.

**Verification**

- `just --list`
- `just fmt-check`
- `just clippy`
- `just test`
- `just verify`
- `just smoke-jsonl`

If `just` is unavailable on the runner, report the blocker and run the raw cargo/smoke equivalents without checking the `just` acceptance box.

---

### T-0008 minimal CI

**Goal**

Add a minimal CI workflow that mirrors local verification for the headless Rust Core slice.

**Scope**

- Add CI under `.github/workflows/` if the repository uses GitHub Actions.
- Run formatting check, clippy, tests, and JSONL smoke where feasible.
- Use stable Rust toolchain unless the repository later documents another toolchain.
- Do not add deployment, publishing, secrets, or release steps.
- Keep CI focused on the headless Rust workspace.

**Acceptance Criteria**

- CI workflow is non-interactive.
- CI does not require secrets.
- CI does not start Blender, GUI, TUI, or network services.
- Local command set and CI command set are visibly aligned.
- Workflow file is documented in the task report.

**Out of Scope**

- Publishing crates.
- Uploading release artifacts.
- Building Blender assets.
- Cross-platform release matrix beyond minimal verification.

**Verification**

- `just verify` or raw cargo equivalent.
- If available locally: `act` dry-run is optional, not required.
- Manual review that CI contains no secret usage and no push/deploy steps.

---

### T-0009 protocol error contract hardening

**Goal**

Make protocol errors stable enough for autonomous clients and future UI shells.

**Scope**

- Define stable error codes for:
  - malformed JSON;
  - invalid request envelope;
  - unknown method;
  - invalid params;
  - workspace not open;
  - internal error.
- Ensure every command path maps errors into `ResponseEnvelope`.
- Add golden examples for success and error responses.

**Acceptance Criteria**

- Error response shape is documented by tests/examples.
- Unknown method and invalid params never panic.
- Request `id` is preserved when parseable.
- Malformed lines produce deterministic controlled behavior.

**Out of Scope**

- Localization.
- Rich UI error presentation.
- Retry/backoff policies.
- Network error mapping.

**Verification**

- `cargo test -p michelangelo_protocol`
- `cargo test --workspace`
- JSONL malformed/unknown/invalid-param smoke cases.

---

### T-0010 core command router

**Goal**

Separate protocol transport from command routing and application services.

**Scope**

- Implement a minimal command router in `crates/michelangelo_core/`.
- Route only methods introduced by this TZ.
- Keep transport-specific stdin/stdout code in CLI or a thin transport layer.
- Keep workspace operations behind a service boundary.

**Acceptance Criteria**

- Router can be unit-tested without spawning the CLI binary.
- CLI stdio uses the same router as tests.
- Adding a new method has one obvious registration point.
- No Blender/GUI/TUI dependencies leak into core router.

**Out of Scope**

- Plugin system.
- Dynamic command loading.
- Network transports.
- Long-running job scheduler.

**Verification**

- `cargo test -p michelangelo_core`
- `cargo test --workspace`
- Existing JSONL smoke commands remain compatible.

---

### T-0011 workspace filesystem invariants

**Goal**

Make workspace layout validation explicit and testable.

**Scope**

- Define expected workspace directories/files for this headless slice.
- Validate missing/corrupt workspace structure with structured errors.
- Add tests for create/open/idempotency/corruption cases.
- Document any transitional files created before SQLite metadata lands.

**Acceptance Criteria**

- Workspace layout is deterministic.
- Opening a non-workspace path returns a structured error.
- Opening a partially corrupted workspace returns a structured error or repair behavior documented in tests.
- No Blender execution directories are used as active integration points.

**Out of Scope**

- Asset import.
- Cleanup/garbage collection.
- Migration framework unless needed by storage task.

**Verification**

- `cargo test -p michelangelo_workspace`
- `cargo test --workspace`
- Manual review of documented workspace invariants.

---

### T-0012 SQLite project metadata MVP

**Goal**

Introduce the first SQLite-backed metadata slice in line with `MVP_SPEC.md` without expanding beyond headless project metadata.

**Scope**

- Add minimal SQLite metadata support for project records if not already present.
- Create/open a temp SQLite database under the workspace.
- Store project id/name/root path/created timestamp or equivalent minimal metadata.
- Keep schema small and migration/init behavior deterministic.

**Acceptance Criteria**

- New workspace has an initialized metadata database.
- Existing workspace can be reopened and project metadata is read from storage.
- Integration tests use a real temp SQLite file.
- Dependency additions, if any, are minimal and justified in the task report.
- No alternate primary metadata backend is introduced.

**Out of Scope**

- Full Asset Graph tables.
- FTS/RTree.
- Job history persistence beyond project metadata.
- Remote databases.

**Verification**

- `cargo test -p michelangelo_workspace`
- `cargo test --workspace`
- Temp workspace create/open integration test with real SQLite file.
- `just verify` after `T-0007`.

---

### T-0013 project list/reopen smoke

**Goal**

Prove that project metadata survives process restart boundaries.

**Scope**

- Add command or test scenario for reopen after process boundary.
- If a `project.list` command is introduced, keep it limited to known local workspace roots and document scope.
- Verify `project.open` + `project.get_snapshot` after a fresh CLI invocation.

**Acceptance Criteria**

- A workspace created by one process can be opened by another process.
- Snapshot after reopen matches stored project metadata.
- Behavior is covered by integration or smoke test.
- No global user directory scanning is introduced unless explicitly approved.

**Out of Scope**

- Recent-projects UI.
- OS-specific app data integration.
- Cloud sync.

**Verification**

- `cargo test --workspace`
- Shell smoke using a temp workspace and two separate CLI invocations.
- `just smoke-jsonl` updated if appropriate.

---

### T-0014 headless job DTOs without execution

**Goal**

Prepare protocol/core state for future long-running jobs without implementing Blender or LLM execution.

**Scope**

- Add minimal job DTO/status enum if needed by snapshot:
  - `queued`
  - `running`
  - `completed`
  - `failed`
  - `cancelled`
- Snapshot may return an empty jobs list.
- Add serialization tests.

**Acceptance Criteria**

- Job DTOs are protocol-only or storage-only as appropriate.
- No executor starts Blender, LLM, network, or background workers.
- Snapshot shape remains backward-compatible with earlier tests.

**Out of Scope**

- Job scheduler.
- Blender job specs.
- Progress events beyond DTO shape.
- Async worker runtime.

**Verification**

- `cargo test -p michelangelo_protocol`
- `cargo test --workspace`
- Snapshot JSONL smoke still passes.

---

### T-0015 asset graph placeholder DTOs without ingest

**Goal**

Reserve the protocol shape for future Asset Graph snapshot fields without implementing image ingestion.

**Scope**

- Add minimal DTOs only if required for `ProjectSnapshotDto`:
  - asset summary;
  - top-level node summary;
  - index status.
- Return empty lists/status for fresh workspaces.
- Keep DTOs independent from PNG parser and storage details.

**Acceptance Criteria**

- Fresh snapshot clearly represents no imported assets.
- DTO tests cover empty/default snapshot.
- No PNG reading/parsing code is added.
- No FTS/RTree/indexing implementation is added.

**Out of Scope**

- PNG decomposition.
- Manifest import.
- Asset Graph persistence.
- Search/index commands.

**Verification**

- `cargo test -p michelangelo_protocol`
- `cargo test --workspace`
- JSONL snapshot smoke for fresh workspace.

---

### T-0016 headless end-to-end scenario

**Goal**

Create one deterministic end-to-end smoke scenario for the current headless MVP slice.

**Scope**

- Add a documented smoke script, test, or `just` recipe that performs:
  1. create temp workspace;
  2. call `project.create`;
  3. call `project.open` if needed;
  4. call `project.get_snapshot`;
  5. validate JSONL response basics;
  6. cleanup temp workspace.
- Keep it non-interactive and Linux-friendly.

**Acceptance Criteria**

- One command runs the complete headless scenario.
- Scenario uses a real temp filesystem workspace.
- Scenario does not depend on Blender, GUI/TUI, network, or secrets.
- Failure output is useful for autonomous agents.

**Out of Scope**

- Browser/desktop UI automation.
- Blender renders/exports.
- LLM chat.
- Long-running jobs.

**Verification**

- `just verify`
- `just smoke-jsonl`
- The new E2E smoke command documented in the task report.

---

### T-0017 documentation sync

**Goal**

Update active documentation to reflect the implemented headless core slice without changing architecture.

**Scope**

- Update only active docs that need current commands/entry points, for example:
  - `README.md`
  - `TASKS.md`
  - `ROADMAP.md`
  - `docs/ai/AI_CONTEXT.md`
  - `docs/ai/AI_ENTRY_POINTS.md`
- Document how to run verification and smoke commands.
- Document that Blender/GUI/TUI are intentionally not part of this slice.

**Acceptance Criteria**

- Docs list current Rust entry points and verification commands.
- Docs do not claim full product MVP is complete.
- Docs do not introduce new architecture beyond `MVP_SPEC.md`.
- Documentation matches actual commands from `justfile`/CI.

**Out of Scope**

- Rewriting the full MVP spec.
- Adding new ADRs unless a real architecture decision was made separately.
- Marketing/release notes.

**Verification**

- Manual doc review against actual commands.
- `just verify` or raw cargo equivalent.
- Confirm no Blender/GUI/TUI implementation files changed as part of doc sync.

---

## 9. Final Acceptance Criteria for This TZ

The headless Rust Core iterative MVP slice is acceptable when:

- All tasks through `T-0008` are implemented and verified.
- Any later tasks selected by the orchestrator have evidence and no unchecked silent failures.
- Branch rule evidence exists for each implementation iteration.
- No work was done on `main`, `gnhf/*`, or any other non-`dev` branch.
- No push was performed by agents.
- No Blender worker files were changed.
- No GUI/TUI implementation was added.
- Core runs as a headless process through CLI/stdin/stdout.
- JSONL protocol smoke passes.
- Workspace create/open works in a real temp filesystem workspace.
- `project.get_snapshot` returns deterministic minimal state.
- Local verification commands exist and pass.
- Minimal CI mirrors local checks.
- Documentation accurately describes what is and is not implemented.

---

## 10. Non-Applicable Checks

- **UI automation:** not applicable until a GUI/TUI TZ exists.
- **Blender stand smoke:** not applicable because Blender integration is explicitly out of scope.
- **Network/LLM integration:** not applicable because this TZ has no provider/API integration.
- **Real external database stand:** not applicable; SQLite, if introduced, must be tested as a local temp file.
