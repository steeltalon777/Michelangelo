# AI Entry Points — Michelangelo Headless Core

## Source Files

| File | Purpose |
|---|---|
| `crates/michelangelo_protocol/src/lib.rs` | Protocol crate root, re-exports all DTOs |
| `crates/michelangelo_protocol/src/error.rs` | `ErrorCode` (numeric) and `ProtocolError` |
| `crates/michelangelo_protocol/src/command.rs` | `CommandEnvelope` |
| `crates/michelangelo_protocol/src/response.rs` | `ResponseEnvelope` |
| `crates/michelangelo_protocol/src/event.rs` | `EventEnvelope`, event name constants |
| `crates/michelangelo_protocol/src/snapshot.rs` | `ProjectDto`, `ProjectSnapshotDto` |
| `crates/michelangelo_protocol/src/job.rs` | `JobDto`, `JobStatus`, `ArtifactDto` |
| `crates/michelangelo_protocol/src/blender.rs` | `BlenderJobSpec::validate()`, typed `BlenderOperation` (18 ops), `BlenderJobResult`, `BlenderCapabilitiesDto`, `SAFE_WORKER_OPS` |
| `crates/michelangelo_protocol/src/asset.rs` | `AssetDto`, `AssetSummaryDto`, `IndexStatus` |
| `crates/michelangelo_protocol/src/method.rs` | Method name constants (Phase 1 + Phase 2) |
| `crates/michelangelo_core/src/lib.rs` | Core crate root, re-exports `CoreService`, `Router` |
| `crates/michelangelo_core/src/service.rs` | `CoreService` (state + scheduler + blender adapter + storage + event channel) |
| `crates/michelangelo_core/src/router.rs` | `Router`, `Handler`, `default_router()`, all Phase 1 + Phase 2 handlers |
| `crates/michelangelo_jobs/src/lib.rs` | `Scheduler` (incl. `dequeue_job(job_id)`), `JobState`, `JobEvent`, `JobId` |
| `crates/michelangelo_blender/src/lib.rs` | `HeadlessBlenderAdapter` (bounded subprocess, result/artifact validation, Windows batch wrapper), `BlenderError`, `create_fake_blender()` |
| `crates/michelangelo_workspace/src/lib.rs` | Workspace crate root |
| `crates/michelangelo_workspace/src/service.rs` | `WorkspaceService`, `ProjectMeta` |
| `crates/michelangelo_workspace/src/layout.rs` | Directory layout, constants, validation |
| `crates/michelangelo_workspace/src/storage.rs` | SQLite `Storage` (projects + jobs tables) |
| `crates/michelangelo_workspace/src/error.rs` | `WorkspaceError` |
| `crates/michelangelo_cli/src/main.rs` | CLI entry point, clap definition |
| `crates/michelangelo_cli/src/stdio.rs` | JSONL stdin/stdout loop with event channel |
| `blender/worker.py` | Blender Python worker — 18 safe operations, deterministic scene init, validation, `--capabilities` |
| `blender/smoke_create_cube.py` | Standalone Blender smoke helper |
| `scripts/smoke_blender_scene.py` | Cross-platform acceptance E2E driver (real Blender + `--fake`; JSONL event ordering/identity, invalid-spec rejection, artifact headers, calibrated front silhouette and framed-scene silhouette alpha, reopen) |
| `scripts/test_worker_validation.py` | Python worker validation checks (no `bpy`) |
| `scripts/validate_blender_smoke.py` | JSONL smoke output structural validator (legacy recipes) |
| `scripts/create_fake_blender.sh` | POSIX fake Blender generator for the legacy smoke recipe |
| `examples/scene_ops/composite_scene_spec.json` | Composite acceptance scene used by the smoke driver and docs |
| `justfile` | Verification/smoke recipes (POSIX; see `smoke-blender-scene*`, `test-worker-python`) |
| `.github/workflows/ci.yml` | CI: Ubuntu + Windows Rust/Python checks and fake acceptance smoke |

## Adding a New Method

1. Add method constant in `crates/michelangelo_protocol/src/method.rs`
2. Add handler function in `crates/michelangelo_core/src/router.rs`
3. Register in `default_router()` in `router.rs`
4. Add tests

## Adding a New Blender Operation

1. Add the enum variant + validation in `crates/michelangelo_protocol/src/blender.rs` and update
   `SAFE_WORKER_OPS` (order is stable and mirrored by the worker).
2. Add the handler in `blender/worker.py`, register it in `OPERATIONS` and keep the registry parity
   guard/report list in sync.
3. Add Rust validation tests, Python `scripts/test_worker_validation.py` coverage, and (for real
   behavior) a real-Blender adapter test.
4. Update `docs/specs/BLENDER_WORKER.md` (params/defaults/ranges) and the acceptance spec only if
   the operation belongs to the acceptance scenario.
5. Run: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `python scripts/test_worker_validation.py`,
   `python scripts/smoke_blender_scene.py --fake` and (on a Blender stand)
   `python scripts/smoke_blender_scene.py`.

## Test Locations

- Protocol tests: `crates/michelangelo_protocol/src/*.rs` (unit + golden + Blender DTO/validation)
- Jobs tests: `crates/michelangelo_jobs/src/lib.rs`
- Blender tests: `crates/michelangelo_blender/src/lib.rs` (fake contract + real-Blender tests)
- Core tests: `crates/michelangelo_core/src/router.rs` and `service.rs`
- Workspace tests: `crates/michelangelo_workspace/src/*.rs`
- CLI tests: `crates/michelangelo_cli/src/stdio.rs` and `main.rs`
- Python worker tests: `scripts/test_worker_validation.py` (no `bpy`, 30 checks)
- Acceptance E2E: `scripts/smoke_blender_scene.py` (`--fake` and real; composite scene, 5
  artifacts, event ordering, invalid-spec rejection, calibrated + framed-scene silhouettes)
- Legacy smokes: `just smoke-jsonl`, `just smoke-reopen`, `just smoke-e2e`,
  `just smoke-blender-fake`, `just smoke-blender-real`, `just smoke-blender-real-glb`

## Current Limitations

- Real Blender 5.2.3 is available only on the local Windows stand; CI runners have no Blender, so
  real-Blender tests auto-skip and the real acceptance smoke stays local/manual.
- `just` is not installed on the current Windows stand: run the direct `cargo`/`python` commands
  from `docs/dev/DEVSTAND_WINDOWS.md`.
- Rendering is Workbench-only; EEVEE/Cycles determinism is out of scope.
- `create_reference_planes`, `apply_bevel`, curve/extrusion/boolean operations, Asset Graph, LLM,
  Visual Hull, Parametric Solver and Unreal import are not implemented.
