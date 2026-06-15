# Roadmap

## ✅ Phase 1 (Done) — Headless Core Foundation

Rust workspace with protocol DTOs, CLI entry point, JSONL stdio transport,
workspace create/open, snapshot recovery, verification commands, and CI.

**Test count:** 108 tests across 4 crates.

## 🔧 Phase 2 (In hardening) — Blender Integration

- Job scheduling and execution (`job.list`, `job.get`, `job.cancel`)
- Blender subprocess management (`blender.get_capabilities`, `blender.run_job`)
- Job lifecycle events (`job.queued`, `job.started`, `job.completed`, `job.failed`, `job.cancelled`; `job.progress` is optional for Phase 2)
- SQLite job persistence across process boundaries
- Fake Blender smoke for CI
- Real Blender smoke for local validation (required: `.blend`; optional: GLB)

Acceptance hardening: see `docs/tz/TZ_PHASE_2_ACCEPTANCE_HARDENING.md`.

## 🔜 Phase 3 — Asset Pipeline

- PNG decomposition and contour extraction
- Asset Graph indexing (SQLite FTS)
- Thumbnail and mask generation

## 🔜 Phase 4 — LLM Orchestration

- MCP adapter for LLM clients
- Natural language command interpretation
- Iterative refinement workflows

## 🔜 Phase 5 — GUI/TUI

- Desktop GUI shell
- Terminal UI for headless monitoring
