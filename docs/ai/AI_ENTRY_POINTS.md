# AI Entry Points — Michelangelo Headless Core

## Source Files

| File | Purpose |
|---|---|
| `crates/michelangelo_protocol/src/lib.rs` | Protocol crate root, re-exports all DTOs |
| `crates/michelangelo_protocol/src/error.rs` | `ErrorCode` (numeric) and `ProtocolError` |
| `crates/michelangelo_protocol/src/command.rs` | `CommandEnvelope` |
| `crates/michelangelo_protocol/src/response.rs` | `ResponseEnvelope` |
| `crates/michelangelo_protocol/src/event.rs` | `EventEnvelope` |
| `crates/michelangelo_protocol/src/snapshot.rs` | `ProjectDto`, `ProjectSnapshotDto` |
| `crates/michelangelo_protocol/src/job.rs` | `JobDto`, `JobStatus` |
| `crates/michelangelo_protocol/src/asset.rs` | `AssetDto`, `AssetSummaryDto`, `IndexStatus` |
| `crates/michelangelo_protocol/src/method.rs` | Method name constants |
| `crates/michelangelo_core/src/lib.rs` | Core crate root, re-exports `CoreService`, `Router` |
| `crates/michelangelo_core/src/service.rs` | `CoreService` (state management) |
| `crates/michelangelo_core/src/router.rs` | `Router`, `Handler`, `default_router()` |
| `crates/michelangelo_workspace/src/lib.rs` | Workspace crate root |
| `crates/michelangelo_workspace/src/service.rs` | `WorkspaceService`, `ProjectMeta` |
| `crates/michelangelo_workspace/src/layout.rs` | Directory layout, constants, validation |
| `crates/michelangelo_workspace/src/storage.rs` | SQLite `Storage` |
| `crates/michelangelo_workspace/src/error.rs` | `WorkspaceError` |
| `crates/michelangelo_cli/src/main.rs` | CLI entry point, clap definition |
| `crates/michelangelo_cli/src/stdio.rs` | JSONL stdin/stdout loop |

## Adding a New Method

1. Add method constant in `crates/michelangelo_protocol/src/method.rs`
2. Add handler function in `crates/michelangelo_core/src/router.rs`
3. Register in `default_router()` in `router.rs`
4. Add tests

## Test Locations

- Protocol tests: `crates/michelangelo_protocol/src/*.rs` (unit + golden)
- Core tests: `crates/michelangelo_core/src/router.rs` and `service.rs`
- Workspace tests: `crates/michelangelo_workspace/src/*.rs`
- CLI tests: `crates/michelangelo_cli/src/stdio.rs` and `main.rs`
