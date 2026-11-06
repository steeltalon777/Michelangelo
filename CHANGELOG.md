# Changelog

All notable changes to Michelangelo are documented here. The workspace version is `0.1.0`
(protocol version follows the crate version). Release dates are repository dates.

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
