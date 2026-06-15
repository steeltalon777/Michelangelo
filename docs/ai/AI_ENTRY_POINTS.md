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
| `crates/michelangelo_protocol/src/blender.rs` | `BlenderJobSpec`, `BlenderOperation`, `BlenderJobResult`, `BlenderCapabilitiesDto`, `SAFE_WORKER_OPS` |
| `crates/michelangelo_protocol/src/asset.rs` | `AssetDto`, `AssetSummaryDto`, `IndexStatus` |
| `crates/michelangelo_protocol/src/method.rs` | Method name constants (Phase 1 + Phase 2) |
| `crates/michelangelo_core/src/lib.rs` | Core crate root, re-exports `CoreService`, `Router` |
| `crates/michelangelo_core/src/service.rs` | `CoreService` (state + scheduler + blender adapter + storage + event channel) |
| `crates/michelangelo_core/src/router.rs` | `Router`, `Handler`, `default_router()`, all Phase 1 + Phase 2 handlers |
| `crates/michelangelo_jobs/src/lib.rs` | `Scheduler`, `JobState`, `JobEvent`, `JobId` |
| `crates/michelangelo_blender/src/lib.rs` | `HeadlessBlenderAdapter`, `BlenderError`, `create_fake_blender()` |
| `crates/michelangelo_workspace/src/lib.rs` | Workspace crate root |
| `crates/michelangelo_workspace/src/service.rs` | `WorkspaceService`, `ProjectMeta` |
| `crates/michelangelo_workspace/src/layout.rs` | Directory layout, constants, validation |
| `crates/michelangelo_workspace/src/storage.rs` | SQLite `Storage` (projects + jobs tables) |
| `crates/michelangelo_workspace/src/error.rs` | `WorkspaceError` |
| `crates/michelangelo_cli/src/main.rs` | CLI entry point, clap definition |
| `crates/michelangelo_cli/src/stdio.rs` | JSONL stdin/stdout loop with event channel |
| `blender/worker.py` | Blender Python worker (7 safe operations) |
| `blender/smoke_create_cube.py` | Standalone Blender smoke helper |
| `scripts/validate_blender_smoke.py` | JSONL smoke output structural validator |
| `justfile` | Verification and smoke recipes (including `smoke-blender-fake`, `smoke-blender-real`, `smoke-blender-real-glb`) |

## Adding a New Method

1. Add method constant in `crates/michelangelo_protocol/src/method.rs`
2. Add handler function in `crates/michelangelo_core/src/router.rs`
3. Register in `default_router()` in `router.rs`
4. Add tests

## Test Locations

- Protocol tests: `crates/michelangelo_protocol/src/*.rs` (unit + golden)
- Jobs tests: `crates/michelangelo_jobs/src/lib.rs`
- Blender tests: `crates/michelangelo_blender/src/lib.rs` (includes fake + real Blender)
- Core tests: `crates/michelangelo_core/src/router.rs` and `service.rs`
- Workspace tests: `crates/michelangelo_workspace/src/*.rs`
- CLI tests: `crates/michelangelo_cli/src/stdio.rs` and `main.rs`
- Smoke tests: `just smoke-jsonl`, `just smoke-reopen`, `just smoke-e2e`, `just smoke-blender-fake`, `just smoke-blender-real`, `just smoke-blender-real-glb`
