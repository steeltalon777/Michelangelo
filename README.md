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
just verify     # fmt + clippy + tests
just smoke-e2e  # end-to-end protocol scenario
```

## Architecture

- `michelangelo_protocol` — DTOs, error codes, JSON-RPC–style envelopes
- `michelangelo_core` — command router, service coordination
- `michelangelo_workspace` — local filesystem layout, SQLite metadata storage
- `michelangelo_cli` — CLI entry point, JSONL stdio transport

## Status

MVP headless core (iterations 1–3). Blender integration, GUI/TUI, LLM, and
Asset Graph ingest are not yet implemented.
