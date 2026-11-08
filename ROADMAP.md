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

## 🔜 Phase 3 (In progress) — Validated Input Package & Asset Pipeline (Stage A of ADR 0004)

Direction fixed by accepted
[ADR 0004](docs/adr/0004-validated-visual-spec-and-hybrid-reconstruction.md) §13 (Stage A):
before reconstruction, define and validate the minimal input package — a versioned description
of the object (images + geometric supplements of hidden shape + explicit unknowns), see ADR §5.

- ✅ **Stage A done (IT-4, 2026-11-07):** input package v0 contract
  ([spec](docs/specs/INPUT_PACKAGE.md), [schema](docs/specs/schemas/input_package_v0.schema.json)),
  `michelangelo_input` crate (strict manifest, SHA-256/CRC-32/PNG header checks in-repo, path
  containment, levels 1–2 per ADR §6, addressable diagnostics, deterministic report) and
  `michelangelo input validate|describe`; wire contract unchanged. Evidence:
  [TZ](docs/tz/TZ_INPUT_PACKAGE_STAGE_A.md).
- Minimal input package ingest beyond validation (bbox, connected components, contours, mask
  generation) — next as part of the Stage B solver/pipeline
- Asset Graph indexing (SQLite FTS/RTree) — kept, serves as the indexed representation of the
  input package
- Thumbnail and mask generation — kept
- Future Blender reference-plane/blockout ops (`create_reference_planes`, `apply_bevel`) require
  their own TZ; they are not accepted by the current worker.

## 🔜 Phase 4 — Reconstruction PoC & Optional Assistant (Stages B–D of ADR 0004)

Stage B (ADR §13.2) — vertical PoC, no API required — ✅ **done (IT-5, 2026-11-07)**:

- ✅ Package → deterministic voxel-hull build (orthographic calibrated silhouette carving, authored
  supplements as explicit voxel rules, unknown stubs) → render through the unchanged worker
  (`import_mesh`, calibrated ortho cameras, `render_camera silhouette`) → per-view IoU quality
  report → editable `.blend`/`.glb` blockout with explicit unresolved list.
  `michelangelo reconstruct run|score|show`, `michelangelo_recon` crate,
  [spec](docs/specs/RECONSTRUCTION.md), [TZ](docs/tz/TZ_RECON_POC_STAGE_B.md).
- ✅ Validation cases: `harbor_lantern_v01` (composite lamp), `naval_cannon_v01` (blind recess /
  silhouette limit), `wagon_wheel_v01` (through-holes, thin spokes) from the local untracked PoC
  datasets, plus synthetic `blind_recess`, `alien_plant` and `composite_lamp` cases
  (`tools/bench/make_synthetic_case.py`), each with hull-only and authored-supplement variants.
  Measured: source-view silhouette IoU 0.94–0.97, held-out orthographic control IoU 0.94–0.95.

Stage C (ADR §13.3) — local edit boundary — ✅ **done (IT-6, 2026-11-07)**:

- ✅ Immutable scene revisions (`s0001`, `s0002`, …) per reconstruction run, atomic publish and
  authoritative `current.json`; explicit `michelangelo.edit-candidate/0` with a bounded task
  (allowed components, allowed region, protected components), provenance, acceptance and budget;
  deterministic validator (stale base, protected tamper — including run-level protection
  established at `edit init --protect`, region/geometry/identity checks, acceptance/policy) and
  atomic applier with rollback; CLI `michelangelo edit init|submit|accept|reject|rollback|show`.
  Spec: [LOCAL_EDIT.md](docs/specs/LOCAL_EDIT.md), TZ: [TZ](docs/tz/TZ_LOCAL_EDIT_STAGE_C.md).
  Proven end-to-end on a real Stage B run: accepted candidate → `s0002`, stale resubmit rejected,
  protected tamper rejected, out-of-region candidate rejected, rollback to `s0001`.

Stage D (ADR §13.4) — optional assistant — ✅ **done (IT-7, 2026-11-07)**:

- ✅ Bounded assistant task (`michelangelo.assistant-task/0`): base revision, allowed components,
  allowed region, protected components, filtered references (masks, silhouette renders, reports),
  acceptance criteria, budgets and an explicit data policy; workspace policy
  (`.michelangelo/assistant_policy.json`, deny-by-default); persisted per-run budget counters and
  an append-only audit log; MCP stdio adapter (`michelangelo mcp serve`) exposing
  `michelangelo_get_task`, `michelangelo_validate_package`, `michelangelo_submit_candidate`,
  `michelangelo_get_report`. A submission is always `proposed` — applying stays an explicit
  `michelangelo edit accept`. No embedded model, no network, no external service;
  `michelangelo assistant task|submit` CLI. Spec:
  [ASSISTANT_MCP.md](docs/specs/ASSISTANT_MCP.md), TZ:
  [TZ](docs/tz/TZ_ASSISTANT_STAGE_D.md).
- ✅ Verification/acceptance integrity (IT-8A/IT-8B): immutable full-result snapshot with
  per-file/aggregate digests, one authoritative bundle pointer, OS advisory lock, append-only
  decision history and idempotent accept; evaluation of the actual new geometry with pinned
  camera/mask/package inputs, visible vs author conditions vs ground truth vs hidden decision,
  per-view 0.01 IoU regression, and a mandatory accept gate with a Core-registered digest.
  Unknown resolution (IT-8C): versioned bindings, author confirmations, immutable resolutions
  materialised only by the accepted bundle, partial acceptance (geometry accepted while the
  unknown stays open), `evidence_stale` invalidation and rollback-restored state; solver-side
  package policy enforcement (unaccepted AI supplements not applied, forbidden stubs, allowed
  completion rules, accepted-author precedence). Remaining: the external-agent demo (replacing a
  stub via MCP, with a cube control that does not close the unknown).
- Natural language command interpretation — future (needs a provider under the same bounded-task /
  data-policy rules)
- Build plan validation → `BlenderJobSpec`; iterative refinement as bounded local-change
  candidates (ADR §8.2), not unrestricted scene ownership — the bounded candidate intake is
  implemented; a concrete LLM/build-plan provider remains future work

## 🔜 Phase 5 — GUI/TUI

- Desktop GUI shell
- Terminal UI for headless monitoring
