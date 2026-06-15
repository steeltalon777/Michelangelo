# Michelangelo — AI-friendly 3D workbench headless core

Headless Rust core for the Michelangelo 3D workbench. Communicates via JSONL over stdin/stdout.

## Quick start

```bash
# Build
cargo build -p michelangelo_cli

# Health check
printf '{"id":"1","method":"system.ping","params":{}}\n' \
  | cargo run -p michelangelo_cli -- core stdio

# Create project and get snapshot
tmpdir=$(mktemp -d)
printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"demo"}}\n{"id":"s1","method":"project.get_snapshot","params":{}}\n' "$tmpdir" \
  | cargo run -p michelangelo_cli -- core stdio
```

## Verification

```bash
just verify              # fmt + clippy + tests
just smoke-e2e           # end-to-end protocol scenario
just smoke-blender-fake  # blender.run_job with fake Blender (always runs)
just smoke-blender-real  # blender.run_job with real Blender (required path: .blend; skips if missing)
```

Optional smoke commands:

```bash
just smoke-blender-real-glb   # GLB export smoke (skips if glTF prerequisites unavailable)
```

## Status

Phase 2 (Blender Integration) is implemented. Acceptance hardening is in progress — see `docs/tz/TZ_PHASE_2_ACCEPTANCE_HARDENING.md`.

## Architecture

- `michelangelo_protocol` — DTOs, error codes, JSON-RPC–style envelopes, methods/events
- `michelangelo_core` — command router, service coordination, job scheduling
- `michelangelo_workspace` — local filesystem layout, SQLite metadata storage (projects + jobs)
- `michelangelo_jobs` — in-process job scheduler, event queue
- `michelangelo_blender` — Blender subprocess adapter, fake-executable testing
- `michelangelo_cli` — CLI entry point, JSONL stdio transport with event streaming

## Phase 2 — Blender Integration

Blender jobs can be scheduled and executed through the JSONL protocol:

```bash
printf '{"id":"c1","method":"project.create","params":{"path":"/tmp/test","name":"demo"}}\n{"id":"j1","method":"blender.run_job","params":{"job_type":"blender_smoke_scene","wait":true,"spec":{"operations":[{"op":"create_box","name":"box","size":[2.0,1.0,0.5]},{"op":"save_blend","path":"blender/scenes/test.blend"}]}}}\n' \
  | cargo run -p michelangelo_cli -- core stdio
```

Protocol events (`job.queued`, `job.started`, `job.completed`, `job.failed`, `job.cancelled`) are emitted on stdout before the final response. `job.progress` is optional in Phase 2 (not emitted by current batch workers).
