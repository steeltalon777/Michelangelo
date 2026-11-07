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
review GREEN for IT-1 attempt 2, IT-2 attempt 2 and IT-3 (P2 rework applied)).

- Bounded, non-deadlocking Blender subprocess execution with per-job log files and process-tree
  timeout; strict `result.json` identity/status/artifact validation; correct
  completed/failed/cancelled mapping; identity-safe `wait=false` queue.
- Deterministic empty scene per job; 18 typed, validated safe operations: primitives, transforms,
  materials, 5 import formats, named components, orthographic cameras (look-at, shift, clip),
  camera framing, calibrated Workbench silhouettes, renders, `.blend`/`.glb` export.
- Cross-platform acceptance E2E (`scripts/smoke_blender_scene.py`) with JSONL event/identity/
  ordering, invalid-spec rejection, artifact/reopen validation, a calibrated 16,384-px front
  silhouette and an assertable framed-scene silhouette (coverage/bbox/margin); deterministic
  `--fake` mode.
- Docs/contracts synced (`BLENDER_WORKER.md`, `CORE_PROTOCOL.md`, `MVP_SPEC.md` §10); CI now runs
  Rust + Python checks and the fake acceptance smoke on Ubuntu and Windows. Real Blender smoke is
  local/manual (GitHub runners lack Blender 5.2 — documented blocker).

**Verified on:** Blender **5.2.3 LTS** (Windows, headless `--background --factory-startup`),
Rust 1.96.0, Python 3.13.
**Test count:** **271 Rust tests** (12 real-Blender, auto-skip without Blender) + **30 Python**
worker validation checks; acceptance driver: 119 checks on real Blender / 113 on fake (3 checks
skipped in fake mode: real-version string and the two silhouette pixel assertions), including the
calibrated front silhouette, the framed-scene silhouette coverage/margin assertion and invalid-spec
rejection. Only
Blender 5.2.3 is verified; 4.2 is API-plausible but untested.

## 🔜 Phase 3 (Next) — Validated Input Package & Asset Pipeline (Stage A of ADR 0004)

Direction fixed by accepted
[ADR 0004](docs/adr/0004-validated-visual-spec-and-hybrid-reconstruction.md) §13 (Stage A):
before reconstruction, define and validate the minimal input package — a versioned description
of the object (images + geometric supplements of hidden shape + explicit unknowns), see ADR §5.

- Minimal input package: manifest/PNG ingest, alpha masks, bbox, connected components,
  contours (kept from the previous plan, now serving the ADR §5 package structure)
- Input validation levels per ADR §6: format/safety, sufficiency, geometric compatibility,
  draft build + back-projection, author acceptance; targeted diagnostics, no silent guessing
- Provenance and revision policy per ADR §5.3: origin/acceptance of supplements, conflict and
  revision handling
- Asset Graph indexing (SQLite FTS/RTree) — kept, serves as the indexed representation of the
  input package
- Thumbnail and mask generation — kept
- Future Blender reference-plane/blockout ops (`create_reference_planes`, `apply_bevel`) require
  their own TZ; they are not accepted by the current worker.

## 🔜 Phase 4 — Reconstruction PoC & Optional Assistant (Stages B–D of ADR 0004)

Stage B (ADR §13.2) — vertical PoC, no API required:

- Prepared package → limited deterministic build → render from source cameras → quality report
  → editable blockout with explicit unknowns (calibrated front-silhouette rendering from
  Phase 2.5 is the measurement base)
- Validation cases from ADR §13: control body with a blind recess, an alien plant with
  overlapping branches, a composite fantastic object

Stage C (ADR §13.3) — local edit boundary:

- Manual candidate / test backend first; prove protected-geometry enforcement, stale-revision
  rejection, failure keeps the previous version, rollback

Stage D (ADR §13.4) — optional assistant:

- MCP adapter for LLM clients — kept, but only after permissions, budgets, data policy and
  measurable acceptance criteria are defined (ADR §8)
- Natural language command interpretation
- Build plan validation → `BlenderJobSpec`; iterative refinement as bounded local-change
  candidates (ADR §8.2), not unrestricted scene ownership

## 🔜 Phase 5 — GUI/TUI

- Desktop GUI shell
- Terminal UI for headless monitoring
