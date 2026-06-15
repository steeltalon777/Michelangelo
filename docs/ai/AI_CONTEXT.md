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
            │    └─ subprocess: blender --background --python worker.py
            └─ WorkspaceService (michelangelo_workspace)
                 ├─ Layout     ← directory structure
                 ├─ Storage    ← SQLite metadata (projects + jobs)
                 └─ project.json ← backward-compat metadata
```

## Crates

| Crate | Purpose | Key types |
|---|---|---|
| `michelangelo_protocol` | Stable DTOs for JSONL wire format | `CommandEnvelope`, `ResponseEnvelope`, `EventEnvelope`, `ProtocolError`, `ErrorCode`, `ProjectDto`, `ProjectSnapshotDto`, `JobDto`, `JobStatus`, `BlenderJobSpec`, `BlenderJobResult`, `BlenderCapabilitiesDto`, `AssetSummaryDto` |
| `michelangelo_core` | Command routing, service coordination | `CoreService`, `Router`, `Handler` |
| `michelangelo_workspace` | Filesystem layout, SQLite storage (projects + jobs) | `WorkspaceService`, `Storage`, `ProjectMeta`, `WorkspaceError` |
| `michelangelo_jobs` | In-process job scheduler, event queue | `Scheduler`, `JobState`, `JobEvent`, `JobId` |
| `michelangelo_blender` | Blender subprocess adapter | `HeadlessBlenderAdapter`, `BlenderError`, `create_fake_blender()` |
| `michelangelo_cli` | CLI binary, JSONL transport with event streaming | CLI entry point, `stdio::run_loop` with event channel |

## Protocol Numbers

- Error codes: numeric (JSON-RPC compatible); see `ErrorCode` in `michelangelo_protocol`
- Methods: `system.ping`, `project.create`, `project.open`, `project.get_snapshot`, `job.list`, `job.get`, `job.cancel`, `blender.get_capabilities`, `blender.run_job`
- Events: `job.queued`, `job.started`, `job.progress` (optional), `job.completed`, `job.failed`, `job.cancelled`
- Transport: one JSON object per line on stdin/stdout; stderr for diagnostics; events before their associated response
- Worker ops: `create_box`, `set_material`, `set_camera_set`, `render_views`, `export_glb`, `save_blend`, `get_scene_summary`

## Verification

```bash
just verify                  # fmt + clippy + 181 tests
just smoke-jsonl             # CLI pipe smokes
just smoke-e2e               # validated end-to-end scenario
just smoke-reopen            # process-boundary survival
just smoke-blender-fake      # blender.run_job with fake Blender (always runs)
just smoke-blender-real      # required real Blender smoke: .blend artifact (skips if missing)
just smoke-blender-real-glb  # optional GLB export smoke (skips if glTF unavailable)
```

## Environment Variables

- `BLENDER_BIN` — path to Blender executable (default: `blender`)
- `MICHELANGELO_BLENDER_TIMEOUT_MS` — default timeout for Blender subprocess

## Out of Scope

GUI/TUI, LLM providers, MCP, Asset Graph ingest, PNG decomposition,
Qdrant/vector databases, network transport.
