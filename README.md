# Michelangelo — AI-friendly 3D workbench headless core

Headless Rust core for the Michelangelo 3D workbench. It speaks JSONL over stdin/stdout:
stdout carries protocol messages only (responses and lifecycle events), stderr carries
diagnostics. Blender runs as a bounded external batch subprocess driven by the same protocol.

## Status

- Phase 1 (headless protocol/workspace/jobs) — done.
- Phase 2 (Blender integration, jobs, events, persistence) — accepted on
  **Blender 5.2.3 LTS** (Windows, headless `--background --factory-startup`).
- Phase 2.5 — reliability hardening and safe Blender scene operations (IT-1/IT-2 implementation,
  IT-3 acceptance automation/docs) — done: bounded subprocess execution with per-job logs, strict
  `result.json`/artifact validation, correct job identity and terminal-state ordering,
  deterministic empty scenes, 18 typed safe operations, calibrated orthographic silhouettes,
  Workbench-only rendering.
- Verification at the IT-2/IT-3 baseline: **271 Rust tests**, **30 Python worker validation
  checks**, and the cross-platform acceptance E2E (`scripts/smoke_blender_scene.py`) with a
  calibrated **16,384 px** silhouette (real Blender) plus a deterministic `--fake` mode.
- Next: Phase 3 (asset pipeline) — see [ROADMAP.md](ROADMAP.md). Visual Hull, Parametric
  Solver, LLM integration, Asset Graph and Unreal import are **not implemented**.

## Quick start

```bash
# Build the CLI
cargo build -p michelangelo_cli

# Health check
printf '{"id":"1","method":"system.ping","params":{}}\n' \
  | ./target/debug/michelangelo core stdio

# Create a project and read the snapshot (one process)
tmpdir=$(mktemp -d)
printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"demo"}}\n{"id":"s1","method":"project.get_snapshot","params":{}}\n' "$tmpdir" \
  | ./target/debug/michelangelo core stdio
```

On Windows use `target\debug\michelangelo.exe` and build JSON with a real serializer (see
[DEVSTAND_WINDOWS.md](docs/dev/DEVSTAND_WINDOWS.md)); `docs/dev/DEVSTAND_LINUX.md` covers Linux.

## Composite Blender scene (acceptance scenario)

The acceptance scenario creates a composite scene through `blender.run_job(wait=true)`:
components, a box/cylinder/sphere with a material, two orthographic cameras, a calibrated
silhouette render, a framed scene render, `get_scene_summary`, `save_blend` and `export_glb`.
The exact operations array is in
[examples/scene_ops/composite_scene_spec.json](examples/scene_ops/composite_scene_spec.json)
(see [examples/scene_ops/README.md](examples/scene_ops/README.md)).

```bash
# Canonical acceptance run against real Blender (prints SKIP and exits 0 when Blender is absent)
cargo build -p michelangelo_cli
python scripts/smoke_blender_scene.py

# Deterministic protocol/artifact run with a generated fake Blender (no Blender required)
python scripts/smoke_blender_scene.py --fake

# Strict mode: exit 2 instead of 0 when real Blender is unavailable
python scripts/smoke_blender_scene.py --require-blender
```

The driver runs the scenario through the CLI (`project.create` -> `blender.get_capabilities` ->
`blender.run_job` -> `job.list` -> `project.get_snapshot`), validates JSONL event order/identity,
job status and persistence, `.blend`/`.glb`/PNG headers, the calibrated silhouette alpha
(exactly 16,384 opaque pixels, bbox `x 128..383`, `y 224..287`), reopens the project in a second
process and exits non-zero on any failure. `--fake` checks protocol plumbing and the artifact
contract but skips pixel assertions.

Direct JSONL form of the same job (paths are workspace-relative):

```json
{"id":"j1","method":"blender.run_job","params":{"job_type":"blender_acceptance_scene","wait":true,"timeout_ms":300000,"spec":{"operations":[ ...examples/scene_ops/composite_scene_spec.json "operations"... ]}}}
```

Lifecycle events (`job.queued`, `job.started`, `job.completed`/`job.failed`/`job.cancelled`)
are emitted on stdout before the final response. `wait` defaults to `false` (queue-only); pass
`"wait": true` to execute synchronously. `job.progress` is optional and not emitted by the
current batch worker.

## Verification

```bash
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                     # 271 tests; real-Blender tests auto-skip without Blender

# Python worker (no bpy)
python -m py_compile blender/worker.py scripts/test_worker_validation.py scripts/smoke_blender_scene.py
python scripts/test_worker_validation.py   # 30/30

# Acceptance E2E (see above)
python scripts/smoke_blender_scene.py --fake
python scripts/smoke_blender_scene.py
```

`just` recipes wrap the same commands (`smoke-blender-scene`, `smoke-blender-scene-fake`,
`smoke-blender-scene-required`, `test-worker-python`, plus the existing `verify`/`smoke-*`).
The recipes are POSIX shell scripts, so they require `just` and `sh` (Git Bash/WSL on Windows);
`just` is not installed on the current Windows stand — run the direct commands instead.

## Architecture

| Crate | Responsibility |
|---|---|
| `michelangelo_protocol` | DTOs, numeric error codes, JSON-RPC-style envelopes, typed `BlenderOperation` whitelist |
| `michelangelo_core` | command router, service coordination, job scheduling, job/Blender handlers |
| `michelangelo_workspace` | local workspace layout, SQLite metadata (projects + jobs), `.michelangelo/metadata.db` |
| `michelangelo_jobs` | in-process job scheduler (max concurrency 1), event queue |
| `michelangelo_blender` | bounded Blender subprocess adapter, fake-executable support |
| `michelangelo_cli` | CLI entry point, JSONL stdio transport with event streaming |

Blender stays an external batch subprocess: the CLI writes `job.json`/`result.json` under
`<workspace>/jobs/<job_id>/`, redirects Blender stdout/stderr to per-job log files, and validates
the result before persisting the job state. See
[docs/specs/BLENDER_WORKER.md](docs/specs/BLENDER_WORKER.md) and
[docs/specs/CORE_PROTOCOL.md](docs/specs/CORE_PROTOCOL.md).

## Documentation

- Protocol: [CORE_PROTOCOL.md](docs/specs/CORE_PROTOCOL.md)
- Blender worker contract: [BLENDER_WORKER.md](docs/specs/BLENDER_WORKER.md)
- Product spec: [MVP_SPEC.md](docs/specs/MVP_SPEC.md)
- Current iteration TZ: [TZ_RELIABILITY_AND_BLENDER_SCENE_OPS_ITERATIVE.md](docs/tz/TZ_RELIABILITY_AND_BLENDER_SCENE_OPS_ITERATIVE.md)
- Roadmap / tasks / changelog: [ROADMAP.md](ROADMAP.md), [TASKS.md](TASKS.md), [CHANGELOG.md](CHANGELOG.md)
- Dev stands: [Windows](docs/dev/DEVSTAND_WINDOWS.md), [Linux](docs/dev/DEVSTAND_LINUX.md)
- Historical Phase 2 acceptance TZ (archived): [TZ_PHASE_2_ACCEPTANCE_HARDENING.md](docs/archive/TZ_PHASE_2_ACCEPTANCE_HARDENING.md)
- Example scene spec: [examples/scene_ops](examples/scene_ops/README.md)
