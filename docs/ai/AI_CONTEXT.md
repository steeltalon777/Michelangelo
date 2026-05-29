# AI Context — Michelangelo Headless Core

## Architecture

```
CLI (michelangelo_cli)
  └─ JSONL stdin/stdout       ← transport boundary
       └─ CoreService (michelangelo_core)
            ├─ Router          ← method → handler dispatch
            └─ WorkspaceService (michelangelo_workspace)
                 ├─ Layout     ← directory structure
                 ├─ Storage    ← SQLite metadata
                 └─ project.json ← backward-compat metadata
```

## Crates

| Crate | Purpose | Key types |
|---|---|---|
| `michelangelo_protocol` | Stable DTOs for JSONL wire format | `CommandEnvelope`, `ResponseEnvelope`, `ProtocolError`, `ErrorCode`, `ProjectDto`, `ProjectSnapshotDto`, `JobDto`, `AssetSummaryDto` |
| `michelangelo_core` | Command routing, service coordination | `CoreService`, `Router`, `Handler` |
| `michelangelo_workspace` | Filesystem layout, SQLite storage | `WorkspaceService`, `Storage`, `ProjectMeta`, `WorkspaceError` |
| `michelangelo_cli` | CLI binary, JSONL transport | CLI entry point, `stdio::run_loop` |

## Protocol Numbers

- Error codes: numeric (JSON-RPC compatible); see `ErrorCode` in `michelangelo_protocol`
- Methods: `system.ping`, `project.create`, `project.open`, `project.get_snapshot`
- Transport: one JSON object per line on stdin/stdout; stderr for diagnostics

## Verification

```bash
just verify       # fmt + clippy + 108 tests
just smoke-jsonl  # CLI pipe smokes
just smoke-e2e    # validated end-to-end scenario
just smoke-reopen # process-boundary survival
```

## Out of Scope

Blender integration, GUI/TUI, LLM providers, MCP, Asset Graph ingest,
PNG decomposition, Qdrant/vector databases.
