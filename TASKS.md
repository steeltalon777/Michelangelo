# Task Tracking

## Completed — Stage 1 (Foundation)

- [x] T-0001 Rust workspace skeleton (4 crates)
- [x] T-0002 Protocol DTOs (envelopes, errors, snapshot)
- [x] T-0003 CLI bootstrap (clap, --help, --version, core stdio)
- [x] T-0004 JSONL stdin/stdout loop (system.ping, error handling)

## Completed — Stage 2 (Workspace & Verification)

- [x] T-0005 Workspace create/open (directory layout, project.json)
- [x] T-0006 project.get_snapshot (state recovery DTO)
- [x] T-0007 justfile verification commands
- [x] T-0008 Minimal GitHub Actions CI

## Completed — Stage 3 (Hardening)

- [x] T-0009 Protocol error contract hardening (numeric error codes, golden tests)
- [x] T-0010 Core command router (registration-based dispatch, standalone handlers)
- [x] T-0011 Workspace filesystem invariants (validate_layout, Corrupted error)
- [x] T-0012 SQLite project metadata MVP (Storage, metadata.db, fallback to JSON)
- [x] T-0013 Project list/reopen smoke (process-boundary survival test)
- [x] T-0014 Headless job DTOs (JobStatus, JobDto)
- [x] T-0015 Asset graph placeholder DTOs (AssetDto, AssetSummaryDto, IndexStatus)
- [x] T-0016 Headless end-to-end scenario (smoke-e2e with Python validation)
- [x] T-0017 Documentation sync

## Completed — Phase 2 (Blender Integration)

- [x] T-0201 Protocol contract (methods, events, Blender DTOs, crate skeletons)
- [x] T-0202 SQLite job persistence (`jobs` table, CRUD APIs)
- [x] T-0203 Job scheduler and event queue (in-process, max concurrency 1, mpsc events)
- [x] T-0204 Blender worker contract (`blender/worker.py`, safe operations)
- [x] T-0205 Headless Blender adapter (`HeadlessBlenderAdapter`, fake executable support)
- [x] T-0206 Core command wiring (Phase 2 handlers in Router, CoreService integration)
- [x] T-0207 CLI JSONL event transport (event channel, stdout JSONL streaming)
- [x] T-0208 Snapshot includes jobs, process-boundary job persistence
- [x] T-0209 Failure/cancel/timeout hardening
- [x] T-0210 Verification recipes (`smoke-blender-fake`, `smoke-blender-real`)
- [x] T-0211 Real Blender stand smoke (originally validated with Blender 4.0.2; re-verified on 5.2.3 in Phase 2.5)
- [x] T-0212 Documentation sync
- [x] T-0213 Phase 2 acceptance hardening (smoke validation, contract sync, docs) — completed
  2026-06-12; archived TZ: `docs/archive/TZ_PHASE_2_ACCEPTANCE_HARDENING.md` (accepted with
  notes). JSONL smoke validation now rejects `status:"failed"`, terminal status is asserted and
  `job.progress`/`wait=false` semantics are documented/tested.

## Completed — Phase 2.5 (Reliability & Safe Blender Scene Ops)

TZ: `docs/tz/TZ_RELIABILITY_AND_BLENDER_SCENE_OPS_ITERATIVE.md` (branch `dev`; reviewers' verdicts
recorded in the TZ review log).

- [x] T-0301 Execution reliability hardening (IT-1, 2026-11-06; commits `95ebeed`, `8311ef7`) —
  Blender stdout/stderr redirected to per-job log files (no pipe deadlock), bounded
  version/capabilities/job timeouts with process-tree kill, strict `result.json`
  identity/status/artifact validation, failed/cancelled mapped to job failures, `wait=false`
  identity-safe queue (`dequeue_job` by id), terminal events emitted only after
  scheduler+storage transitions, `finished_at` lifecycle persistence. Evidence:
  `target/techlead/IT-1{,-fix}/` (gitignored) and `cargo test --workspace`.
- [x] T-0302 Safe Blender scene operations (IT-2, 2026-11-06; commits `59ca3c5`, `2cf4961`) —
  18 typed `BlenderOperation`s with Rust + Python validation, deterministic empty scene,
  primitives/transforms/materials/import (5 formats), orthographic cameras with calibrated
  projection, camera framing, Workbench render + RGBA silhouettes, `save_blend`/`export_glb`,
  containment-before-mutation for all output paths, `render_views` safe unique filenames. Evidence:
  271 Rust tests (12 real-Blender), 30/30 Python worker checks, Lead acceptance E2E with
  16,384-px calibrated silhouette (`target/techlead/lead-verify/IT-2/`).
- [x] T-0303 Contracts and documentation sync (IT-3, 2026-11-07; P2 rework applied) —
  `docs/specs/BLENDER_WORKER.md`
  (18-op contract, projection fixture, framed-scene acceptance note, reproducible worker
  example, Workbench restriction), `docs/specs/CORE_PROTOCOL.md`
  (capabilities pass-through vs `SAFE_WORKER_OPS` execution, raw persisted `job` record type,
  `blender.run_job` semantics, events, persistence), `docs/specs/MVP_SPEC.md`
  §10 aligned with the implemented safe set, README status/quick start, `docs/ai/*`,
  `docs/dev/DEVSTAND_{WINDOWS,LINUX}.md` (python3 default/overrides), historical note in
  `docs/archive/TZ_PHASE_2_ACCEPTANCE_HARDENING.md`, `examples/scene_ops/`.
- [x] T-0304 Cross-platform acceptance driver + CI (IT-3, 2026-11-07; P2 rework applied) —
  `scripts/smoke_blender_scene.py` (real Blender + `--fake`: 5 artifacts, JSONL event
  identity/ordering, invalid-spec `-32602` without a job record/events, artifact metadata
  cross-checks, calibrated front silhouette alpha, assertable framed-scene silhouette
  coverage/bbox/margin, reopen validation), `justfile` recipes (`smoke-blender-scene`,
  `smoke-blender-scene-fake`, `smoke-blender-scene-required`, `test-worker-python`; Python
  `python3` default with a Windows override), CI: Python worker checks + fake scene smoke on
  Ubuntu and a new `windows-latest` job (fmt/clippy/test/build + Python checks + fake scene
  smoke). Real Blender is not available on CI runners (documented blocker). Evidence:
  `target/techlead/IT-3/` (gitignored) — real run 119 checks / fake run 113 checks (3 skips),
  framed-vs-unframed counterfactual and a space-containing `TMP`/`TEMP` fake run.

## Completed — ADR 0004 Stage A (Validated Input Package v0)

TZ: `docs/tz/TZ_INPUT_PACKAGE_STAGE_A.md` (branch `dev`; review log in the TZ).

- [x] T-0401 Input package v0 contract + `michelangelo_input` validator (IT-4, 2026-11-07) —
  strict manifest (`michelangelo.input-package/0`, `deny_unknown_fields`), in-repo SHA-256,
  CRC-32 and PNG header parsing (no new crates), resource path containment, hash verification,
  validation levels 1–2 per ADR §6.1, addressable diagnostics, deterministic report.
- [x] T-0402 CLI `michelangelo input validate|describe` (IT-4) — report JSON on stdout, exit
  codes 0/1/2, `--require-complete`, `--no-hash-verify`, `--report`, `--quiet`; JSONL wire
  contract unchanged.
- [x] T-0403 Fixtures, spec and JSON Schema (IT-4) — `examples/input_packages/{valid_full,
  valid_partial}`, `docs/specs/INPUT_PACKAGE.md`,
  `docs/specs/schemas/input_package_v0.schema.json`; 105 crate tests, 10 CLI integration tests.
  Evidence: `target/techlead/IT-4/` (gitignored) — including a package derived from the local
  `harbor_lantern_v01` dataset (3 calibrated orthographic + 3 reference views, 1024², real
  hashes) validating as `valid_complete`.

## Completed — ADR 0004 Stage B (Vertical Reconstruction PoC)

TZ: `docs/tz/TZ_RECON_POC_STAGE_B.md` (branch `dev`; evidence in `target/techlead/IT-5/`).

- [x] T-0501 Deterministic voxel solver (IT-5, 2026-11-07) — `tools/solver/` (stdlib only):
  PNG decoder, view projection, silhouette carving from calibrated orthographic views, authored
  supplement rules (thickness/cavity/closure/connection/curve/symmetry), unknown stubs,
  Y-up OBJ meshing, deterministic `build_report.json`; comparator with per-view IoU/coverage
  (`quality_report.json`); 68 solver tests.
- [x] T-0502 Rust reconstruction pipeline (IT-5) — `michelangelo_recon` crate: camera
  basis→Euler-XYZ (verified against Blender 5.2.3), solver invocation, Blender job plan,
  run store (`recon/r%04d/run-%04d/`), report aggregation, `score` re-render; 60 crate tests.
- [x] T-0503 CLI `michelangelo reconstruct run|score|show` (IT-5) — exit codes 0/1/2, 6 CLI
  integration tests, spec `docs/specs/RECONSTRUCTION.md`; JSONL wire contract unchanged.
- [x] T-0504 Stage B acceptance (IT-5) — local PoC datasets (`harbor_lantern_v01`,
  `naval_cannon_v01`, `wagon_wheel_v01`) + synthetic cases (`blind_recess`, `alien_plant`,
  `composite_lamp`) generated by `tools/bench/make_synthetic_case.py`; hull-only and
  authored-supplement variants scored with held-out orthographic control views; blind-recess
  non-observability proven (top-mask IoU 1.0 vs a closed body).

## Completed — ADR 0004 Stage C (Local Edit Boundary)

TZ: `docs/tz/TZ_LOCAL_EDIT_STAGE_C.md` (branch `dev`; evidence in `target/techlead/IT-6/`).

- [x] T-0601 Scene revision store + validator/applier (IT-6, 2026-11-07) — `michelangelo_edit`
  crate: immutable `scene/sNNNN` revisions with atomic publish, explicit candidate schema,
  deterministic checks (stale base, identity, task scope, components, protected, geometry,
  region, budget, acceptance, policy), atomic apply with byte-identical untouched components,
  rollback, run-level protection established at `edit init --protect`; 122 crate tests.
- [x] T-0602 CLI `michelangelo edit init|submit|accept|reject|rollback|show` (IT-6) — 16 CLI
  integration tests, spec `docs/specs/LOCAL_EDIT.md`; JSONL wire contract unchanged.
- [x] T-0603 Stage C acceptance (IT-6) — end-to-end on a copy of a real Stage B run: accepted
  candidate publishes `s0002`, stale resubmit rejected, protected tamper rejected, region
  violation rejected, rollback to `s0001`; protected components byte-identical after accept.

## Not yet started

- Future Blender ops: `create_reference_planes`, `apply_bevel`, curve/extrusion/boolean ops
  (require a new TZ; not part of the current safe set).
- Phase 3 — asset pipeline: PNG ingest/decomposition, Asset Graph (SQLite FTS/RTree), thumbnails,
  masks, contours.
- Phase 4 — LLM orchestration (build plan, validation→`BlenderJobSpec`).
- Phase 5 — GUI/TUI shell.
- Network transport / MCP adapter.
