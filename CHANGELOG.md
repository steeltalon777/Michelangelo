# Changelog

All notable changes to Michelangelo are documented here. The workspace version is `0.1.0`
(protocol version follows the crate version). Release dates are repository dates.

## [Unreleased]

ADR 0004 Stage A (IT-4): validated input package v0. The JSONL wire contract is unchanged —
the new functionality is a library crate plus a CLI subcommand.

### Added

- **Input package v0 contract** (`michelangelo.input-package/0`): a versioned package directory
  (`<workspace>/input/<revision>/package.json` + `images/` + `masks/`) describing an object as
  calibrated orthographic views, masks, parts, author supplements of hidden shape, explicit
  unknowns, known measurements, uncertainty policy, provenance and acceptance entries.
  Normative spec: `docs/specs/INPUT_PACKAGE.md`; JSON Schema:
  `docs/specs/schemas/input_package_v0.schema.json`.
- **`michelangelo_input` crate**: strict manifest deserialization (`deny_unknown_fields`),
  in-repo SHA-256, CRC-32 and PNG header parsing (no new third-party crates), resource path
  containment (no traversal/absolute/drive/UNC/symlink escape), hash verification, validation
  level 1 (format/safety) and level 2 (sufficiency: calibrated-view completeness, distinct
  directions, acceptance, reference-projection rules), addressable diagnostics and a
  deterministic report (`michelangelo.input-package-report/0`).
- **CLI `michelangelo input validate|describe`**: report JSON on stdout, exit codes 0/1/2,
  `--require-complete`, `--no-hash-verify`, `--report`, `--quiet`.
- **Fixtures and tests**: `examples/input_packages/{valid_full,valid_partial}` (8×8 PNGs, real
  hashes), 105 crate tests, 10 CLI integration tests; evidence includes a package derived from
  the local `harbor_lantern_v01` PoC dataset (3 calibrated orthographic + 3 reference views,
  1024², hash-verified) validating as `valid_complete`.

ADR 0004 Stage B (IT-5): deterministic vertical reconstruction PoC. The JSONL wire contract is
unchanged — new library crate, Python solver tool, CLI subcommands and a bench harness.

### Added (Stage B)

- **`michelangelo_recon` crate**: deterministic pipeline from a Stage A package to an editable
  blockout — camera basis→Blender Euler-XYZ conversion (verified against Blender 5.2.3), solver
  subprocess invocation, `BlenderJobSpec` construction (existing 18 worker ops only), run store
  `<workspace>/recon/r%04d/run-%04d/`, workspace-relative artifacts and a
  `michelangelo.reconstruction-report/0` report; `score` re-renders the built blockout from a
  held-out evaluation package and writes `evaluation_quality_report.json`; 60 crate tests.
- **`tools/solver/`** (Python stdlib only): PNG decoder, orthographic view projection, voxel
  silhouette carving (AABB derivation, arbitrary ortho poses), authored supplement rules
  (thickness, cavity, closure, connection, curve, symmetry), explicit unknown stubs, Y-up OBJ
  blocky meshing, deterministic `build_report.json`, and `compare.py` per-view silhouette
  IoU/coverage with thresholds; 68 solver tests.
- **CLI `michelangelo reconstruct run|score|show`**: exit codes 0 pass / 1 quality fail /
  2 invalid input or setup; spec `docs/specs/RECONSTRUCTION.md`.
- **Bench harness `tools/bench/`**: dataset→package converter, Blender `bpy` synthetic-case
  generator (blind recess with a proof of non-observability, alien plant, composite lamp),
  authored supplement sets, single-case runner and the full matrix runner.
- **Stage B acceptance**: local PoC datasets (`harbor_lantern_v01`, `naval_cannon_v01`,
  `wagon_wheel_v01`) and the three synthetic cases, hull-only and with authored supplements;
  source silhouette IoU 0.94–0.97, held-out orthographic control IoU 0.94–0.95; blind-recess
  limit reported as unresolved, hidden-shape supplements applied and unknowns resolved.

### Changed (Stage B)

- ROADMAP Phase 4 Stage B marked done, `docs/ai/AI_CONTEXT.md` (crate, CLI, product direction,
  out of scope) and `TASKS.md` updated for IT-5.

ADR 0004 Stage C (IT-6): local edit boundary. The JSONL wire contract is unchanged — new library
crate, CLI subcommands and a spec.

### Added (Stage C)

- **`michelangelo_edit` crate**: immutable scene revisions (`scene/sNNNN`) per reconstruction run
  with atomic publish and an authoritative `current.json`; explicit
  `michelangelo.edit-candidate/0` (bounded task, allowed region, protected components, provenance,
  acceptance, budget); deterministic validator (`STALE_BASE`, `PROTECTED_MODIFIED`,
  `REGION_VIOLATION`, `COMPONENT_UNKNOWN`, `NOT_ALLOWED`, `UNSUPPORTED_ACTION`, `OBJ_PARSE`,
  `GEOMETRY_INVALID`, `IDENTITY_MISMATCH`, `BUDGET_INVALID`, `ACCEPTANCE_REQUIRED`,
  `POLICY_DENIED`); atomic applier that keeps untouched/protected components byte-identical;
  rollback; run-level protection established at `edit init --protect` and not redefinable by a
  candidate; 122 crate tests.
- **CLI `michelangelo edit init|submit|accept|reject|rollback|show`** with exit codes 0/1/2 and
  `--protect` at init; 16 CLI integration tests; spec `docs/specs/LOCAL_EDIT.md`; committed
  fixtures `examples/edit/`.
- **Stage C acceptance**: end-to-end on a copy of a real Stage B run — accepted candidate →
  `s0002`, stale resubmit rejected, protected tamper rejected, out-of-region candidate rejected,
  rollback to `s0001`; protected components byte-identical.

### Changed (Stage C)

- ROADMAP Phase 4 Stage C marked done, `docs/ai/AI_CONTEXT.md` and `TASKS.md` updated for IT-6.

ADR 0004 Stage D2 (IT-7): optional bounded assistant + MCP stdio adapter. No embedded model and
no network code; the JSONL core wire contract is unchanged.

### Added (Stage D2)

- **Bounded assistant task** (`michelangelo.assistant-task/0`): generated from a reconstruction
  run — base package/scene revision, allowed components and region, protected components,
  filtered references (package masks/images, run silhouette renders, build/quality reports only,
  never evaluation/ground-truth), acceptance criteria from the measured thresholds, budgets and
  an explicit data policy (`allow_network: false`, `allow_external_service: false`).
- **`michelangelo_assistant` crate**: deterministic task generation, strict workspace policy
  (`.michelangelo/assistant_policy.json`, deny-by-default; `allow_apply: true` is a startup
  error), per-run persisted budget counters (`assistant/budget.json`) and an append-only audit log
  (`assistant/audit.jsonl`), four MCP tools and a newline-delimited JSON-RPC 2.0 stdio server with
  the documented error codes and path confinement; 93 crate tests.
- **CLI `michelangelo assistant task|submit` and `michelangelo mcp serve`**; 12 CLI +
  4 stdio integration tests; spec `docs/specs/ASSISTANT_MCP.md`; fixtures `examples/assistant/`.
- **Stage D2 acceptance**: MCP session on a copy of the IT-6 run — task generated, submission
  `proposed` (never applied), budget exhaustion `-32003`, path escape `-32002`, malformed line
  `-32700` with the server still alive, policy denial `-32001`, invalid policy stops the server,
  audit complete with no absolute paths; explicit `michelangelo edit accept` applies the candidate.

### Changed (Stage D2)

- ROADMAP Phase 4 Stage D marked done, `docs/ai/AI_CONTEXT.md` and `TASKS.md` updated for IT-7;
  test count baseline 702 Rust tests.

### Changed

- README status, ROADMAP Phase 3 (Stage A done), `docs/ai/AI_CONTEXT.md` and `TASKS.md` updated
  for IT-4; test count baseline 387 Rust tests.

ADR 0004 hardening IT-8 (IT-8A acceptance integrity + IT-8B candidate evaluation). The input
package format stays v0 and the JSONL wire contract is unchanged; the new functionality is two
library crates plus CLI subcommands and an accept gate.

### Added (IT-8A/IT-8B)

- **Verification state contract** (`docs/specs/VERIFICATION_STATE.md`): one authoritative bundle
  pointer, immutable full-result snapshots, digest rules, verification taxonomy
  (`projection_conformance`, `author_geometric_conditions`, `independent_ground_truth`,
  `author_hidden_decision`), partial acceptance, resolution invalidation and policy precedence.
- **Acceptance integrity (IT-8A)**: `submit` snapshots the full post-change component set with
  per-file and aggregate SHA-256; publication reads only the snapshot; the store runs under an OS
  advisory lock (`std::fs::File::try_lock`, no TTL stealing) and accept re-reads the pointer and
  re-verifies evidence under it; duplicate accepts are idempotent (historical result, same
  revision, also after rollback), reject-after-accept is refused, crash-before-pointer recovers
  by reusing the published revision; every decision (including `accept-refused`) is append-only;
  each revision carries a hash-pinned unknown-state document and rollback restores it.
- **Candidate evaluation (IT-8B)**: `michelangelo_evidence` (report DTO, digests, append-only
  registry, pure accept gate) and `michelangelo_eval` (renders the actual candidate snapshot or
  revision and its base with pinned cameras/masks/package inputs; visible vs author conditions vs
  ground truth vs hidden decision; per-view 0.01 IoU regression; held-out views optional);
  `michelangelo evaluate --run (--candidate|--revision)` and a mandatory
  `edit accept --evaluation <report>` gate that requires a registered report digest, matching
  target aggregate/base/inputs, passing visible conformance and required author geometric
  conditions, and a pinned regression limit. Hidden claims stay `unverified`/`pending`; missing
  ground truth is `unavailable`, never a pass; `auto_accept` is deferred (submit stays `proposed`).
- Spec `docs/specs/EVALUATION.md`; 51 new tests (evidence 23, eval 12, edit gate 10, CLI 6).

### Added (IT-8C)

- **Unknown resolution**: versioned `unknown-bindings/0`, immutable author `confirmation/0`
  (claim + target geometry + evaluation + evidence), immutable `resolution/0` with seven checks,
  `unknown-state/0` (`open|stub_pending|resolved`, `reason:evidence_stale`), and closure only in
  a revision whose bundle references the resolution (`edit accept --resolve`). Partial acceptance
  accepts geometry while unknowns stay open; a later change of a bound geometry/binding/claim
  reopens `open` + `evidence_stale`; rollback restores the target bundle state; bindings without
  a claim let the confirmation carry the claim.
- **CLI**: `michelangelo unknown sync|list|show|bind|confirm|propose|resolve`; `edit accept
  --resolve`; `assistant task --unknown` and MCP `michelangelo_get_task` optional `"unknown"`
  argument (MCP still never applies). Spec `docs/specs/UNKNOWNS.md`.
- **Package policy enforcement in the solver**: unaccepted AI supplements are `not_applied` with
  `SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED` and do not close unknowns; `SUPPLEMENT_RULE_NOT_ALLOWED`
  for disallowed kinds; `STUB_FORBIDDEN`; accepted author decisions take precedence for the
  author's geometric intent; a candidate can only make the package policy stricter
  (`PACKAGE_POLICY_STRICTER`); no `uncertainty_policy` object keeps the Stage B behaviour.
  19 edit unknown tests + 17 solver policy tests.

### Changed (IT-8A/IT-8B)

- `docs/specs/LOCAL_EDIT.md`, `docs/specs/VERIFICATION_STATE.md`, `docs/specs/RECONSTRUCTION.md`,
  `docs/specs/EVALUATION.md`, TZ `TZ_IT8_ACCEPTANCE_INTEGRITY.md` and ROADMAP/TASKS/AI_CONTEXT
  updated for A/B/C; test count baseline 816 Rust tests + 86 solver tests.

## [0.1.0] — 2026-11-07

Phase 2 acceptance hardening, execution reliability, safe Blender scene operations and
cross-platform acceptance automation (IT-1/IT-2/IT-3). The JSONL wire format stays backward
compatible: changes are optional fields, new operation names, stricter validation and corrected
job-level behavior.

### Added

- **Safe Blender scene operations (18 typed ops).** New operations alongside the Phase 2 set:
  `create_cylinder`, `create_sphere`, `import_mesh` (`.glb`, `.gltf`, `.obj`, `.stl`, `.ply`),
  `set_transform`, `create_ortho_camera`, `set_camera_pose`, `set_camera_settings`,
  `frame_camera`, `set_render_settings`, `render_camera`, `create_component`. Implemented as a
  typed `BlenderOperation` whitelist with Rust + Python validation and exact wire JSON.
- **Deterministic scene per job.** Every job starts from an empty scene (no default
  Cube/Camera/Light), Workbench engine, 512x512 PNG RGBA defaults.
- **Calibrated orthographic cameras and silhouettes.** `look_at`, ortho scale, sensor fit,
  shift, clip planes; `render_camera` with `silhouette: true` produces a transparent-film RGBA
  PNG whose alpha channel is the silhouette mask. The documented fixture (2x1x0.5 box, front
  camera, ortho_scale 4, 512x512) yields exactly 16,384 opaque pixels, bbox `x 128..383`,
  `y 224..287`. Assertable camera framing: the acceptance scene creates the scene camera with
  `ortho_scale=100` and requires `frame_camera` to reframe it; a framed scene silhouette must
  reach ≥ 0.03 opaque coverage, a bbox span ≥ 40% of the frame and an ≥ 8 px border margin (a
  broken framing step fails the smoke).
- **Named components.** `create_component` creates `Component_<name>` collections; primitives and
  imports can join a component; `get_scene_summary` reports components and scene bounds.
- **Capabilities probing.** `blender.get_capabilities` now probes the configured binary and
  `blender/worker.py` (bounded 30 s) and reports the worker-reported operation list,
  Blender/Python version, glTF availability and render engines; a missing binary is a normal
  response with a static fallback and `error` instead of a hang. The adapter passes the
  worker-reported operation list through; execution remains restricted to the `SAFE_WORKER_OPS`
  whitelist.
- **Cross-platform acceptance driver** `scripts/smoke_blender_scene.py`: real-Blender and
  `--fake` modes, composite JSONL scenario from `examples/scene_ops/composite_scene_spec.json`
  (5 artifacts), event order/identity/ordering relative to the response, invalid-spec `-32602`
  with no job record/event, terminal-event vs response artifact metadata agreement, persisted job,
  artifact headers, calibrated front silhouette alpha, framed-scene silhouette coverage/margin,
  reopen across a process boundary. Exit codes 0/1/2 with an explicit SKIP when Blender is absent.
- **Recipes and CI.** `just` recipes `smoke-blender-scene`, `smoke-blender-scene-fake`,
  `smoke-blender-scene-required`, `test-worker-python` (Python defaults to `python3`, override
  `python_bin=python` on Windows); CI now runs Python worker checks and the fake acceptance smoke
  and has a `windows-latest` job (fmt/clippy/test/build + Python checks + fake smoke). Real
  Blender is not installed on CI runners (documented blocker).
- **Documentation.** Rewritten `docs/specs/BLENDER_WORKER.md` and `docs/specs/CORE_PROTOCOL.md`,
  aligned `docs/specs/MVP_SPEC.md` §10, updated README/ROADMAP/TASKS and dev-stand docs, added
  `examples/scene_ops/`, and a historical note for the archived Phase 2 TZ.

### Changed

- `blender.run_job` validates the complete typed spec **before** creating a job record; invalid
  operations/params/paths return `-32602` naming `operation[i]` and leave no orphan queued job.
- `wait` defaults to `false` (queue-only) and is documented explicitly. A `wait=false` job stays
  queued and can never be hijacked by a later `wait=true` request, which starts exactly its own
  job id.
- Terminal lifecycle events (`job.completed`/`job.failed`/`job.cancelled`) are emitted only after
  the scheduler and storage transitions succeed; inconsistent transitions return `-32603` without
  a contradictory terminal event.
- Job timeout precedence is explicit: request `timeout_ms` > `MICHELANGELO_BLENDER_TIMEOUT_MS` >
  300,000 ms; on timeout the process tree is killed and waited for.
- `render_views` derives unique, validated PNG names from camera names (strips `Cam_` only for
  canonical legacy views) and rejects unsafe names/collisions before writing anything.
- `set_camera_set` accepts the documented Phase 2 `side` view as a back-compat alias of `left`.

### Fixed

- Worker `result.json` with `status: "failed"` (or `cancelled`) is now surfaced as a failed
  (cancelled) job instead of a successful completion.
- `result.json`/artifact validation: job id must match, status must be known, every completed
  artifact must exist, be a regular non-empty file, resolve inside the workspace, and match its
  reported size and extension.
- Blender job stdout/stderr are redirected to per-job log files, so a chatty Blender run can no
  longer fill a pipe and deadlock the core; killed process trees are reaped.
- Job identity across `wait=false`/`wait=true` is preserved (no cross-job completion), and
  `finished_at`/`started_at` lifecycle timestamps are persisted.
- All user/asset-derived output paths (renders, `.blend`, `.glb`) are containment-checked inside
  the workspace before any filesystem mutation; crafted camera names can no longer write or
  delete files outside the workspace.
- `import_mesh` rejects missing/empty/outside files before launch and accepts a requested name
  that already equals the single imported object's own name (idempotent).
- Windows verbatim workspace roots (`\\?\...`) are normalized in the worker so the bundled glTF
  exporter can write `.glb` under the workspace; Windows drive/UNC path forms are rejected on
  every platform.

### Compatibility notes

- JSONL request/response/event shapes from Phase 1/Phase 2 remain valid; `job.progress` stays
  optional (batch workers do not emit it).
- Capabilities additions (`python_version`, `gltf_export_available`, `render_engines`, `error`)
  are optional fields; `blender_version` is `null` instead of an empty string when unavailable.
- Only **Blender 5.2.3 LTS** is verified (headless, `--background --factory-startup`); Blender 4.2
  is API-plausible but untested. Rendering is **Workbench-only** in this iteration.
- Real Blender acceptance smoke remains local/manual: GitHub-hosted CI runners have no Blender.

### Verification

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` — **271 tests** (12 real-Blender tests auto-skip without Blender).
- `python -m py_compile ...` and `python scripts/test_worker_validation.py` — **30/30**.
- `python scripts/smoke_blender_scene.py --fake` — PASS (113 checks, 3 checks skipped: the two
  silhouette pixel assertions + the real-Blender version string).
- `python scripts/smoke_blender_scene.py` on Blender 5.2.3 — PASS (119 checks incl. calibrated
  front silhouette, framed-scene silhouette and reopen).
