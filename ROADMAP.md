# Roadmap

## ✅ Phase 1 (Done) — Headless Core Foundation

Rust workspace with protocol DTOs, CLI entry point, JSONL stdio transport,
workspace create/open, snapshot recovery, verification commands, and CI.

**Test count at completion:** 108 tests across 4 crates.

## ✅ Phase 2 (Done) — Blender Integration

- Job scheduling and execution (`job.list`, `job.get`, `job.cancel`)
- Blender subprocess management (`blender.get_capabilities`, `blender.run_job`)
- Job lifecycle events (`job.queued`, `job.started`, `job.completed`, `job.failed`,
  `job.cancelled`; `job.progress` remains optional and is not emitted by batch jobs)
- SQLite job persistence across process boundaries
- Fake Blender smoke for CI
- Real Blender smoke for local validation (required: `.blend`; optional: GLB)
- Acceptance hardening closed (T-0213, archive:
  [`docs/archive/TZ_PHASE_2_ACCEPTANCE_HARDENING.md`](docs/archive/TZ_PHASE_2_ACCEPTANCE_HARDENING.md))

## ✅ Phase 2.5 (Done) — Reliability & Safe Blender Scene Ops

Iteration TZ:
[`docs/tz/TZ_RELIABILITY_AND_BLENDER_SCENE_OPS_ITERATIVE.md`](docs/tz/TZ_RELIABILITY_AND_BLENDER_SCENE_OPS_ITERATIVE.md)
(IT-1 reliability, IT-2 safe scene operations, IT-3 contracts/docs/CI/acceptance; independent
review GREEN for IT-1 attempt 2 and IT-2 attempt 2, IT-3 under final review).

- Bounded, non-deadlocking Blender subprocess execution with per-job log files and process-tree
  timeout; strict `result.json` identity/status/artifact validation; correct
  completed/failed/cancelled mapping; identity-safe `wait=false` queue.
- Deterministic empty scene per job; 18 typed, validated safe operations: primitives, transforms,
  materials, 5 import formats, named components, orthographic cameras (look-at, shift, clip),
  camera framing, calibrated Workbench silhouettes, renders, `.blend`/`.glb` export.
- Cross-platform acceptance E2E (`scripts/smoke_blender_scene.py`) with JSONL event/identity/
  artifact/reopen validation and a calibrated 16,384-px silhouette; deterministic `--fake` mode.
- Docs/contracts synced (`BLENDER_WORKER.md`, `CORE_PROTOCOL.md`, `MVP_SPEC.md` §10); CI now runs
  Rust + Python checks and the fake acceptance smoke on Ubuntu and Windows. Real Blender smoke is
  local/manual (GitHub runners lack Blender 5.2 — documented blocker).

**Verified on:** Blender **5.2.3 LTS** (Windows, headless `--background --factory-startup`),
Rust 1.96.0, Python 3.13.
**Test count:** **271 Rust tests** (12 real-Blender, auto-skip without Blender) + **30 Python**
worker validation checks; acceptance driver: 104 checks on real Blender / 100 on fake. Only
Blender 5.2.3 is verified; 4.2 is API-plausible but untested.

## 🔜 Phase 3 (Next) — Asset Pipeline

- Manifest/PNG ingest, alpha masks, bbox, connected components, contours
- Asset Graph indexing (SQLite FTS/RTree)
- Thumbnail and mask generation
- Future Blender reference-plane/blockout ops (`create_reference_planes`, `apply_bevel`) require
  their own TZ; they are not accepted by the current worker.

## 🔜 Phase 4 — LLM Orchestration

- MCP adapter for LLM clients
- Natural language command interpretation
- Build plan validation → `BlenderJobSpec`
- Iterative refinement workflows

## 🔜 Phase 5 — GUI/TUI

- Desktop GUI shell
- Terminal UI for headless monitoring
