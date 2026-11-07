# AI Context — Michelangelo Headless Core

## Architecture

```
CLI (michelangelo_cli)
  └─ JSONL stdin/stdout       ← transport boundary (events + responses)
       └─ CoreService (michelangelo_core)
            ├─ Router          ← method → handler dispatch
            ├─ Scheduler (michelangelo_jobs)
            │    └─ mpsc events → event channel → stdout
            ├─ BlenderAdapter (michelangelo_blender)
            │    └─ subprocess: blender --background --factory-startup --python worker.py
            └─ WorkspaceService (michelangelo_workspace)
                 ├─ Layout     ← directory structure
                 ├─ Storage    ← SQLite metadata (projects + jobs)
                 └─ project.json ← backward-compat metadata
```

## Crates

| Crate | Purpose | Key types |
|---|---|---|
| `michelangelo_protocol` | Stable DTOs for JSONL wire format | `CommandEnvelope`, `ResponseEnvelope`, `EventEnvelope`, `ProtocolError`, `ErrorCode`, `ProjectDto`, `ProjectSnapshotDto`, `JobDto`, `JobStatus`, `BlenderJobSpec`, `BlenderOperation` (18 typed variants), `BlenderJobResult`, `BlenderCapabilitiesDto`, `SAFE_WORKER_OPS`, `AssetSummaryDto` |
| `michelangelo_core` | Command routing, service coordination | `CoreService`, `Router`, `Handler` |
| `michelangelo_workspace` | Filesystem layout, SQLite storage (projects + jobs) | `WorkspaceService`, `Storage`, `ProjectMeta`, `WorkspaceError` |
| `michelangelo_jobs` | In-process job scheduler, event queue | `Scheduler`, `JobState`, `JobEvent`, `JobId` |
| `michelangelo_blender` | Bounded Blender subprocess adapter | `HeadlessBlenderAdapter`, `BlenderError`, `create_fake_blender()` |
| `michelangelo_cli` | CLI binary, JSONL transport with event streaming | CLI entry point, `stdio::run_loop` with event channel |
| `michelangelo_input` | Stage A input package: strict manifest, resource safety, levels 1–2 validation | `InputPackage`, `validate_package`, `ValidateOptions`, `InputPackageReport`, `Diagnostic` |
| `michelangelo_recon` | Stage B reconstruction pipeline (solver → Blender job → IoU report) | `run`, `score`, `ReconstructionReport`, `camera_basis`/`rotation_deg`, run store, `ReconOptions` |
| `michelangelo_edit` | Stage C local edit boundary (scene revisions, candidates) | `SceneStore`, `init_scene_with_protection`, `submit_candidate`, `accept_candidate`, `rollback_scene`, `DecisionReport` |
| `michelangelo_assistant` | Stage D bounded assistant surface + MCP stdio adapter | `assistant-task/0` generation, policy, budget/audit store, four MCP tools, error mapping |

## Protocol Numbers

- Error codes: numeric (JSON-RPC compatible); see `ErrorCode` in `michelangelo_protocol`
- Methods: `system.ping`, `project.create`, `project.open`, `project.get_snapshot`, `job.list`,
  `job.get`, `job.cancel`, `blender.get_capabilities`, `blender.run_job`
- Events: `job.queued`, `job.started`, `job.progress` (optional, not emitted by batch jobs),
  `job.completed`, `job.failed`, `job.cancelled`
- Transport: one JSON object per line on stdin/stdout; stderr for diagnostics; events before their
  associated response; `wait` defaults to `false` (queue-only — pass `"wait": true` to execute)
- CLI-only commands (not JSONL methods, no wire contract impact): `michelangelo input
  validate|describe` (Stage A input package), `michelangelo reconstruct run|score|show` (Stage B
  reconstruction PoC), `michelangelo edit init|submit|accept|reject|rollback|show` (Stage C local
  edit boundary), `michelangelo assistant task|submit` and `michelangelo mcp serve` (Stage D
  bounded assistant/MCP); `michelangelo core stdio` is the JSONL transport
- Worker ops (18, stable order in `SAFE_WORKER_OPS`): `create_box`, `set_material`,
  `set_camera_set`, `render_views`, `export_glb`, `save_blend`, `get_scene_summary`,
  `create_cylinder`, `create_sphere`, `import_mesh`, `set_transform`, `create_ortho_camera`,
  `set_camera_pose`, `set_camera_settings`, `frame_camera`, `set_render_settings`,
  `render_camera`, `create_component`
- Deterministic scene per job; Workbench-only rendering; silhouettes are RGBA PNG with a
  transparent film; artifacts are `{path, type, size_bytes}` (blend/glb/png)
- `blender.run_job` failures are job-level (`result.status="failed"` + `job.failed` event);
  protocol errors are `-32000`/`-32602`/`-32603`

## Verification

```bash
# Rust (real-Blender tests auto-skip when Blender is absent) — 702 tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Python worker checks (no bpy) — 30/30
python -m py_compile blender/worker.py scripts/test_worker_validation.py scripts/smoke_blender_scene.py
python scripts/test_worker_validation.py

# Acceptance E2E: composite scene (5 artifacts), JSONL events/identity/ordering,
# invalid-spec rejection, calibrated front silhouette, framed-scene silhouette, reopen
python scripts/smoke_blender_scene.py --fake    # deterministic, no Blender required
python scripts/smoke_blender_scene.py           # real Blender (SKIP/exit 0 when absent)
python scripts/smoke_blender_scene.py --require-blender   # exit 2 when Blender is absent
```

`just` recipes wrap the same commands: `verify`, `test-worker-python`, `smoke-blender-scene`,
`smoke-blender-scene-fake`, `smoke-blender-scene-required`, plus the legacy POSIX smokes
(`smoke-jsonl`, `smoke-reopen`, `smoke-e2e`, `smoke-blender-fake`, `smoke-blender-real`,
`smoke-blender-real-glb`). Recipes need `just` + `sh` and default to `python3`; on Windows use
Git Bash/WSL with `just python_bin=python ...` or the direct commands (`just` is not installed on
the current Windows stand).

## Environment Variables

- `BLENDER_BIN` — path to the Blender executable (default: `blender` on `PATH`; `.cmd`/`.bat`
  programs are launched through `cmd /C call` on Windows)
- `MICHELANGELO_BLENDER_TIMEOUT_MS` — default Blender job timeout (explicit `timeout_ms` wins;
  built-in default `300_000` ms)
- `CARGO_TARGET_DIR` — optional Cargo target directory override for `cargo` commands. The
  acceptance driver does **not** read it: it resolves `<repo>/target/debug/michelangelo[.exe]` or
  accepts an explicit `--cli PATH`.

## Version Pins

- Rust 1.96.0 (workspace, edition 2021)
- Blender **5.2.3 LTS** verified (headless); bundled Python 3.13.13; Blender 4.2 API-plausible but
  untested. Workbench is the supported render engine for this iteration.
- System Python 3.13 for `blender/worker.py` validation + smoke scripts (stdlib only)

## Product Direction

- Accepted ADR (2026-11-07):
  [`docs/adr/0004-validated-visual-spec-and-hybrid-reconstruction.md`](../adr/0004-validated-visual-spec-and-hybrid-reconstruction.md)
  — validated visual spec + hybrid reconstruction: a versioned input package (images +
  geometric supplements of hidden shape + explicit unknowns), deterministic core, optional
  local AI-assisted completion. Staged plan (ADR §13): A) minimal input package + validation
  (ROADMAP Phase 3), B) vertical reconstruction PoC without API, C) local-edit boundary,
  D) optional assistant (ROADMAP Phase 4).
- Stage A is implemented (IT-4): [`docs/specs/INPUT_PACKAGE.md`](../specs/INPUT_PACKAGE.md),
  `michelangelo_input` crate and `michelangelo input validate|describe` (levels 1–2, hash-verified
  PNG resources, targeted diagnostics, deterministic report). No JSONL wire changes. Input
  packages live at `<workspace>/input/<revision>/package.json`; masks are PNG gray8/alpha with
  `>= 128` = object; calibrated views are orthographic with `ortho_scale` = full view width.
- Stage B is implemented (IT-5): [`docs/specs/RECONSTRUCTION.md`](../specs/RECONSTRUCTION.md),
  `michelangelo_recon` crate + `tools/solver/` (stdlib Python) + `michelangelo reconstruct
  run|score|show`. Deterministic voxel visual hull from calibrated orthographic silhouettes,
  authored supplements as explicit voxel rules, unknown stubs, Blender job rendering source and
  held-out cameras, per-view silhouette IoU/coverage report, editable `.blend`/`.glb` blockout in
  `<workspace>/recon/r%04d/run-%04d/`. Bench harness: `tools/bench/` (dataset conversion, synthetic
  cases, full matrix). No wire changes, no external API.
- Stage C is implemented (IT-6): [`docs/specs/LOCAL_EDIT.md`](../specs/LOCAL_EDIT.md),
  `michelangelo_edit` crate + `michelangelo edit init|submit|accept|reject|rollback|show`.
  Immutable scene revisions per run with atomic publish, explicit edit candidates (bounded task,
  allowed region, protected components, provenance, acceptance), deterministic validator
  (stale base, protected tamper, region/geometry/identity, acceptance/policy) and atomic applier
  with byte-identical untouched components and rollback. Run-level protection is established at
  `edit init --protect`; candidates cannot redefine it. No wire changes, no Blender, no network.
- Stage D2 is implemented (IT-7): [`docs/specs/ASSISTANT_MCP.md`](../specs/ASSISTANT_MCP.md),
  `michelangelo_assistant` crate + `michelangelo assistant task|submit` + `michelangelo mcp serve`.
  Bounded assistant task from a run (allowed components/region, protected parts, filtered
  references, acceptance criteria, budgets, data policy), workspace policy with deny-by-default
  apply, persisted per-run budget counters and an append-only audit log, and a local MCP stdio
  adapter exposing four tools. Submissions are always `proposed`; applying stays an explicit
  `michelangelo edit accept`. No embedded model, no network, no external service.

## Out of Scope

GUI/TUI, LLM providers, Asset Graph ingest, PNG decomposition, Qdrant/vector databases, network
transport, Unreal import, arbitrary Python execution, Blender addon/persistent daemon,
EEVEE/Cycles render reliability. Visual Hull is now implemented as a deterministic v0 blockout
solver (Stage B, voxel carving, no perspective/multi-view fitting, no hidden-void recovery);
Parametric Solver is still planned by accepted ADR 0004 (later stages) — deferred, not rejected.
A local MCP adapter exists (Stage D2, tool boundary only); LLM providers/API integration and
natural-language interpretation remain future work and are not required by the deterministic core.
