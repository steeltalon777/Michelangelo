# TZ: Reliability Hardening and Blender Scene Operations (Iterative)

Status: in progress
Branch: `dev` (no push, no merge; commits only after verified checks)
Owner: TechLead (orchestrator); implementation by worker subagents; independent review required.

## 0. Context and Baseline

Baseline at start (2026-11-06, `dev` = `ca670a1`, one local commit ahead of `origin/dev`,
untracked `docs/research/` left untouched):

- Rust workspace: `michelangelo_protocol`, `michelangelo_workspace`, `michelangelo_jobs`,
  `michelangelo_blender`, `michelangelo_core`, `michelangelo_cli`.
- Phase 1 (JSONL core) and Phase 2 (Blender subprocess/jobs) are implemented.
- `cargo test --workspace` is green on the baseline.
- Local stand: Windows, Rust 1.96.0, Blender 5.2.3 LTS on `PATH` (resolved via `BLENDER_BIN`
  or the default executable name), Python 3.13/3.14, Git Bash available. `just` is NOT installed
  in this environment: recipe
  bodies are executed via equivalent direct commands until IT-3 adds cross-platform drivers.
  WSL exists only as a docker-desktop distro; no Linux Blender stand is available locally.

Known technical debt confirmed by the audit (must be fixed in this TZ):

1. Blender subprocess stdout is piped but never drained; Blender stderr is drained only after
   exit. A chatty Blender run can fill the pipe and deadlock the core (no timeout by default).
2. `wait_with_timeout` kills only the direct child and does not wait for a killed process tree.
3. `HeadlessBlenderAdapter` does not verify the `job_id` inside `result.json`, does not validate
   `status`, and does not verify that reported artifacts exist / are non-empty / stay inside the
   workspace.
4. `blender.run_job` marks every adapter `Ok(...)` as completed, even when the worker returned
   `status: "failed"`.
5. `wait=false` leaves the job queued in the scheduler; a later `wait=true` request calls
   `dequeue()` and starts *the earlier job* instead of its own. Job identity and FIFO/queue
   bookkeeping are corrupted; scheduler transition errors are ignored via `let _ =`.
6. `blender.get_capabilities` hardcodes `bpy_available: true` whenever `blender --version`
   succeeds and advertises a static operation list, not the list actually implemented by
   `blender/worker.py`.
7. Blender worker runs inside Blender's default startup scene (default Cube/Camera/Light), so
   scene content and silhouettes are not deterministic. Material API uses deprecated
   `use_nodes`; scene triangle count is computed without `calc_loop_triangles()`.
8. No safe operations for cylinders, spheres, mesh import, exact transforms, orthographic
   cameras, camera pose/ortho scale/resolution/framing, selected-camera silhouette renders,
   named components.

## 1. Goal

Bring existing Rust Core / Blender Adapter / Blender Worker execution to a stable,
cross-platform state and add the minimal safe Blender scene capabilities required by the future
reconstruction pipeline.

Acceptance criterion (user):

> Michelangelo must be able, through the existing protocol, to create a simple composite scene,
> set calibrated orthographic cameras, obtain silhouette images with known projection geometry,
> save `.blend`/GLB and correctly report the result or an error.

## 2. Out of Scope

- Visual Hull, Parametric Solver, LLM integration, Asset Graph, Unreal Import.
- Arbitrary Python execution through the public protocol.
- New transport (HTTP/socket/named pipes), persistent Blender daemon/addon, async runtime.
- Core redesign not justified by a concrete confirmed defect.
- Push, merge, branch switching, commits to `main`.
- EEVEE/Cycles rendering reliability (headless CI/determinism): Workbench is the supported
  engine for this TZ; the contract documents this restriction.

## 3. Preserved Architecture Boundaries

- JSONL over stdin/stdout remains the transport; stdout is protocol-only, stderr is diagnostics.
- Blender stays an external bounded batch subprocess; workspace files + SQLite stay the source
  of truth.
- Job spec/result contract uses `job.json` / `result.json` under `<workspace>/jobs/<job_id>/`.
- Operation whitelist only; no `execute_arbitrary_python` or equivalent.
- All spec/result file paths are workspace-relative; absolute paths and `..` are rejected on
  both the Rust and Python sides.
- `wait=false` remains documented queue-only behavior (no background execution in the stdio
  process) but must not corrupt the queue or job identity.
- Existing Phase 2 request/response/event shapes remain backward compatible; additions are
  optional fields or new operation names.

## 4. Iteration Plan

Each iteration: worker implements -> Lead runs the verification ladder -> Lead commits on `dev`
with explicit pathspecs -> independent reviewer gets the commit diff + TZ + evidence -> green
verdict required before the next iteration is considered closed. Red verdict -> fix commit and
re-review (max 3 attempts per iteration).

### IT-1 — Execution Reliability (Priority 1)

Write scopes:

- `crates/michelangelo_blender/src/lib.rs`
- `crates/michelangelo_blender/Cargo.toml`, root `Cargo.lock` (unix-only `libc` if needed)
- `crates/michelangelo_protocol/src/blender.rs` (capabilities DTO extension only)
- `crates/michelangelo_jobs/src/lib.rs` (targeted dequeue/identity API + tests)
- `crates/michelangelo_core/src/router.rs` (job/Blender handlers + tests)
- `blender/worker.py` (capability-probe entry mode only; handlers unchanged)
- `scripts/create_fake_blender.sh` (approved in rework: fake must echo the job id and create
  the artifact it reports so the strict adapter validation can be exercised end-to-end)
- `scripts/validate_blender_smoke.py` only if the smoke contract requires it

Acceptance:

- Subprocess stdout/stderr are redirected to per-job log files; no pipe deadlock is possible.
  A fake Blender that writes megabytes to both streams completes without hanging.
- Effective timeout is always bounded: explicit `timeout_ms` > `MICHELANGELO_BLENDER_TIMEOUT_MS`
  > built-in default. On timeout the process tree is terminated and waited for; a clear
  `Timeout` error is returned.
- Adapter validates `result.json`: valid JSON, `job_id` equals requested id, `status` is
  `completed|failed|cancelled`, known artifact types, workspace-relative artifact paths,
  artifacts exist/non-empty, provided `size_bytes` matches. Failed worker results are surfaced
  as job failures, not successes.
- `blender.run_job` maps worker `failed`/`cancelled` statuses to the correct scheduler/storage
  state and terminal event; no `let _ =` swallowing of scheduler/storage transitions.
- `wait=false` job stays queued and is never hijacked by a later `wait=true` request; the
  later job runs and completes under its own id. Scheduler exposes dequeue-by-id; queue order
  and terminal-state identity are unit-tested.
- `blender.get_capabilities` probes the configured binary and `blender/worker.py` (bounded
  timeout) and reports the actual worker operation list, `bpy_available`, Blender/Python
  version, glTF availability; falls back to the static whitelist with `bpy_available=false`
  and an error when probing fails. Unavailable binary is a normal capabilities response, not
  a hanging request.
- Existing fake/real smoke behavior stays green; fake executables create the artifacts they
  report.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` green.

### IT-2 — Safe Blender Scene Operations (Priority 2)

Depends on IT-1. Write scopes:

- `blender/worker.py` (full handler set + input validation + deterministic scene init)
- `crates/michelangelo_protocol/src/blender.rs` (typed `BlenderOperation` whitelist DTO,
  validation, tests; existing wire JSON stays valid)
- `crates/michelangelo_blender/src/lib.rs` (pre-spawn validation incl. workspace artifact
  existence for `import_mesh`, tests)
- `crates/michelangelo_core/src/router.rs` (spec validation error mapping only)
- `scripts/`/tests needed for operation coverage (no docs yet)

Operation set (exact wire contract in section 5.4): existing ops `create_box`,
`set_material`, `set_camera_set`, `render_views`, `export_glb`, `save_blend`,
`get_scene_summary` plus `create_cylinder`, `create_sphere`, `import_mesh`, `set_transform`,
`create_ortho_camera`, `set_camera_pose`, `set_camera_settings`, `frame_camera`,
`set_render_settings`, `render_camera`, `create_component`.

Acceptance:

- Every job starts from an empty deterministic scene (no default Cube/Camera/Light).
- Primitives are created with exact dimensions, named objects, exact location/rotation/scale;
  duplicate object names are rejected with clear errors.
- Named components are Blender collections (`Component_<name>`); primitives/imports can join a
  component; scene summary lists components and scene bounds.
- Orthographic cameras support location, rotation, look-at, ortho scale, shift, clip planes.
- `set_transform` applies exact location / rotation (degrees) / scale; optional scale apply.
- `set_render_settings` supports resolution and percentage; Workbench is the supported engine.
- `render_camera` renders only the selected camera to a workspace-relative PNG; `silhouette:
  true` produces RGBA with transparent film and a flat white silhouette; alpha channel is the
  silhouette mask. Projection contract in section 5.6 holds exactly.
- `import_mesh` imports one workspace artifact (`.glb`, `.gltf`, `.obj`, `.stl`, `.ply`) that
  exists, is non-empty and inside the workspace; imported objects can join a component; `name`
  is accepted only when exactly one object is imported.
- `save_blend` / `export_glb` remain whitelisted and produce verifiable artifacts.
- Rust-side spec validation rejects unknown ops, malformed/out-of-range params, unsafe paths
  and unsupported import formats with `-32602`; the worker re-validates defensively.
- Existing Phase 2 JSON still parses; legacy `req/response` shapes unchanged.
- Real Blender 5.2.3 smoke covers primitives, transforms, ortho camera, calibrated silhouette,
  components, `.blend` + `.glb`. Full ladder green.

### IT-3 — Contracts, Tests, Docs, CI, Acceptance (Priority 3)

Depends on IT-1 and IT-2. Write scopes:

- `scripts/smoke_blender_scene.py` (new cross-platform E2E driver + PNG alpha validation)
- `justfile`, `.github/workflows/ci.yml`
- `README.md`, `TASKS.md`, `ROADMAP.md`, `CHANGELOG.md`
- `docs/specs/BLENDER_WORKER.md`, `docs/specs/CORE_PROTOCOL.md`
- `docs/tz/TZ_RELIABILITY_AND_BLENDER_SCENE_OPS_ITERATIVE.md` (checklist/evidence)
- `docs/ai/*`, `docs/dev/*` only for stale entry points/version pins

Acceptance:

- `just smoke-blender-scene` runs the acceptance JSONL scenario against real Blender, validates
  JSONL event order/identity, job status, `.blend`/`.glb` headers, calibrated PNG silhouette
  alpha coverage and reopens the project to verify persisted job history. Missing Blender =
  explicit skip (non-zero only if a real run was requested but failed).
- `just smoke-blender-fake` remains deterministic without Blender.
- CI verifies fmt/clippy/tests, Python worker syntax/unit checks, CLI smokes, fake Blender
  smoke. Real Blender smoke stays local/manual (documented blocker: CI runner lacks Blender 5.2).
- Docs match implemented behavior; compatibility matrix records tested Blender 5.2.3 LTS and
  the Workbench-only render restriction; stale references fixed.
- Full local ladder green on Windows; Linux status documented honestly (no local Blender stand;
  CI Rust checks only).
- Independent reviewer green verdict on the final diff.

## 5. Detailed Contracts

### 5.1 Adapter execution contract

- `run_job`/`run_job_with_fake` share one implementation.
- Job directory `<workspace>/jobs/<job_id>/`; `job_id` restricted to `[A-Za-z0-9_-]{1,128}`.
- Stale `result.json`, `stdout.log`, `stderr.log` are removed before launch.
- `job.json` contains `job_id`, `spec`, absolute `workspace_root` (UTF-8).
- Blender command: `<blender> --background --factory-startup --python <worker.py> -- <job.json>
  <result.json>`; stdout -> `stdout.log`, stderr -> `stderr.log` in the job dir.
  `--factory-startup` keeps user add-ons/startup files out of the deterministic batch run.
- Timeout resolution: explicit arg > env `MICHELANGELO_BLENDER_TIMEOUT_MS` > 300_000 ms.
  Both the job run and the `blender --version` / worker-capabilities probes are bounded; no
  subprocess wait may block the single-threaded core indefinitely.
- On timeout: platform kill-tree (Windows `taskkill /T /F`, Unix process group SIGKILL), then
  `wait()`; error text includes timeout ms and stderr tail.
- Error variants/taxonomy: `BinaryNotFound`, `NonZeroExit` (with stderr tail), `Timeout`,
  `MissingResultFile`, `MalformedResult`, `JobIdMismatch`, `InvalidStatus`,
  `MissingArtifact`, `InvalidArtifact`, `InvalidJobId`, `Io`.
- Worker `result.json` handling:
  - readable + parseable: validate identity/status; `completed` with non-zero exit is an error;
    `failed`/`cancelled` is returned as a job-level result (partial artifacts preserved);
  - unreadable/absent + non-zero exit -> `NonZeroExit`; absent + zero exit ->
    `MissingResultFile`; malformed + zero exit -> `MalformedResult`.
- Diagnostic tails are capped (stderr <= 4 KiB, stdout <= 2 KiB) and must not contain raw
  environment dumps.

### 5.2 Job identity and queue contract

- `Scheduler::dequeue_job(job_id)` starts exactly that queued job; fails if the id is not
  queued or another job is running.
- `wait=true` starts its own job via `dequeue_job`; it must not call the FIFO `dequeue()`.
- `wait=false` keeps the job in the queued state and returns `{"id","job_type","status":
  "queued"}`; queued jobs are visible to `job.list`/`job.get` and cancellable.
- Terminal transitions use the same job id for scheduler, storage and events.
- Terminal protocol events (`job.completed` / `job.failed` / `job.cancelled`) are emitted only
  after the corresponding scheduler and storage transitions succeeded. If an internal
  transition fails, the job is marked failed where legal, no contradictory terminal event is
  emitted, and the request returns `InternalError` describing the inconsistency.
- A job that fails internally after its record exists but before execution
  (enqueue/start/storage transition failure) is terminal-failed consistently as
  `queued→failed`: scheduler terminal status, storage status and the emitted terminal event all
  report the same job id and failure reason.
- Job statuses persisted: `queued > running > completed|failed|cancelled`, plus the internal
  `queued→failed` path above.

### 5.3 Capabilities contract

Request: `blender.get_capabilities {}`.

Response fields (wire-compatible; new ones optional):

```json
{
  "blender_version": "Blender 5.2.3 LTS" | null,
  "bpy_available": true,
  "supported_operations": ["create_box", "..."],
  "binary_found": true,
  "python_version": "3.13.13",              // optional
  "gltf_export_available": true,            // optional
  "render_engines": ["BLENDER_WORKBENCH"],  // optional
  "error": "..."                            // optional, only on probe failure
}
```

- Probe runs `blender --background --factory-startup --python worker.py -- --capabilities`
  with a bounded timeout and parses a single `MICHELANGELO_CAPABILITIES:<json>` marker line
  from stdout.
- Missing binary: normal response with `binary_found=false`, `bpy_available=false`,
  `blender_version=null`, static whitelist in `supported_operations`, `error` set.
- Binary present but probe fails: `binary_found=true`, `bpy_available=false`, `error` set;
  operations fall back to the static whitelist.

### 5.4 Operation wire contract

All operations are `{"op": "<name>", ...params}`. Ops and parameters:

| op | params | notes |
|---|---|---|
| `create_box` | `name`, `size` [x,y,z] (default 2,2,2), `location` (0,0,0), `rotation_deg`?, `component`? | exact dimensions; object name must be unique |
| `create_cylinder` | `name`, `radius` (0.5), `depth` (1.0), `location`, `rotation_deg`?, `component`? | `vertices` optional 3..1024, default 32 |
| `create_sphere` | `name`, `radius` (0.5), `location`, `rotation_deg`?, `segments` (32), `ring_count` (16), `component`? | UV sphere |
| `import_mesh` | `path`, `name`?, `component`? | workspace artifact `.glb/.gltf/.obj/.stl/.ply`; `name` only for single-object artifacts |
| `set_transform` | `object`, `location`?, `rotation_deg`?, `scale`?, `apply_scale`? (false) | at least one of location/rotation/scale |
| `set_material` | `object`, `color` [r,g,b,a] | 3 or 4 components, 0..1; Principled base color + viewport color |
| `set_camera_set` | `views` | legacy named views: front/back/left/right/top/bottom/perspective |
| `create_ortho_camera` | `name`, `location`, `rotation_deg`? XOR `look_at`?, `ortho_scale` (4.0), `shift_x`?, `shift_y`?, `clip_start` (0.1), `clip_end` (1000), `component`? | orthographic, sensor fit HORIZONTAL |
| `set_camera_pose` | `camera`, `location`?, `rotation_deg`? XOR `look_at`? | at least one |
| `set_camera_settings` | `camera`, `ortho_scale`?, `shift_x`?, `shift_y`?, `clip_start`?, `clip_end`? | ortho cameras only for `ortho_scale` |
| `frame_camera` | `camera`, `objects`?/`component`?, `margin` (1.1) | ortho only; centers + fits scene bounds in camera plane |
| `set_render_settings` | `resolution` [w,h]?, `resolution_percentage`? (100) | Workbench engine fixed |
| `render_camera` | `camera`, `path`, `silhouette` (false) | selected camera only; PNG; silhouette => flat white + transparent film RGBA |
| `render_views` | `path`? (default `blender/renders`) | legacy: renders every camera |
| `create_component` | `name` | creates collection `Component_<name>` (idempotent) |
| `save_blend` | `path` | `.blend` extension |
| `export_glb` | `path` | `.glb` extension; glTF addon must be available |
| `get_scene_summary` | _(none)_ | returns deterministic summary, updates result `scene_summary` |

- Unknown ops are rejected by Rust deserialization/validation (`-32602`) and defensively by the
  worker with a structured failed result.
- Max 256 operations per job; duplicate object/camera names within one job are rejected.
- Names: non-empty, <= 64 chars, no control chars/`/`/`\`.
- Paths: non-empty, relative, no `..`, no NUL, <= 1024 chars, workspace-relative.
- Numeric params: finite, bounded (`|location|,|rotation| <= 1e6`, size/radius/depth > 0,
  scale components != 0, color 0..=1, resolution 1..16384, percentage 1..100).

### 5.5 Deterministic scene initialization

Each job starts from a clean scene: all objects, collections (except the master collection),
meshes, cameras, lights, materials, images are removed, orphans purged. The worker never writes
to stdout except in `--capabilities` mode; all progress goes to stderr with `[michelangelo]`.

Component collections are named `Component_<name>`; objects without `component` are linked to
the scene collection. `scene_summary` keeps legacy keys (`objects`, `meshes`, `materials`,
`cameras`, `triangles`, `vertices`) and adds `components: [{name, objects}]` and
`bounds: {min:[x,y,z], max:[x,y,z]}` when geometry exists. Triangle count uses
`mesh.calc_loop_triangles()`.

### 5.6 Camera projection contract

Orthographic camera, `sensor_fit = HORIZONTAL`, no lens shift by default:

- Camera looks along local `-Z`, local `+Y` is up. `look_at` computes
  `(target - location).to_track_quat('-Z','Y')`.
- `ortho_scale` = visible world width (horizontal extent) in Blender units.
- Visible height = `ortho_scale * resolution_y / resolution_x`.
- Visible x range at target depth = `[center_x - ortho_scale/2, center_x + ortho_scale/2]`.
- Pixel mapping for an object whose projected camera-space center is `(cx, cy)` and size
  `(w, h)`: `width_px = w * resolution_x / ortho_scale`,
  `height_px = h * resolution_y / (ortho_scale * resolution_y / resolution_x)` =
  `h * resolution_x / ortho_scale`; `x_px = (0.5 + cx / ortho_scale) * resolution_x`,
  `y_px = (0.5 + cy / (ortho_scale * res_y / res_x)) * resolution_y` (bottom-left origin).
- Acceptance fixture: 2x1x0.5 box at origin, front camera `(0,-10,0)` looking at origin,
  `ortho_scale=4`, 512x512, silhouette -> exactly 256x64 = 16384 pixels with alpha > 127/255,
  bbox x 128..383, y 224..287 (verified with Blender 5.2.3).
- `frame_camera` uses the world bounding box of selected objects/components, projects corners
  into camera space, computes `ortho_scale = max(width, height*aspect) * margin` and recenters
  the camera location on the bbox center in its local XY plane (ortho cameras only).

### 5.7 Artifact contract

- Result artifacts: `{path, type, size_bytes}` with `type` in `blend|glb|png`;
  extension must match type (`.blend`, `.glb`, `.png`).
- `completed` jobs: every listed artifact must exist, be a regular file, non-empty, resolve
  inside `<workspace>`, and `size_bytes` (when present) must equal the file size.
- `failed`/`cancelled`: paths are still sanitized; listed partial artifacts are reported but
  their absence does not mask the original error.

### 5.8 Validation and error reporting

- Rust rejects invalid specs before creating a job record (no orphan queued jobs on invalid
  params); error code `-32602` with a message naming the offending operation/index.
- Worker validates independently and returns `status:"failed"` + `error` + partial artifacts.
- Adapter errors are mapped to failed job state, `job.failed` event and response
  `{job, status:"failed", error}`; protocol-level errors (no project/invalid params/internal)
  remain JSON-RPC error envelopes.
- No raw environment dumps, no full log spam in protocol payloads.

## 6. Verification Matrix

| Layer | Command / check | IT-1 | IT-2 | IT-3 |
|---|---|---|---|---|
| Static | `cargo fmt --all -- --check` | required | required | required |
| Static | `cargo clippy --workspace --all-targets -- -D warnings` | required | required | required |
| Unit/component | `cargo test --workspace` | required | required | required |
| Python | `python -m py_compile blender/worker.py` | required | required | required |
| Python | worker validation unit checks (no bpy) | optional | required | required |
| Fake subprocess | `just smoke-blender-fake` | required | required | required |
| Real Blender | `cargo test -p michelangelo_blender` (real smoke tests auto-skip if missing) | required | required | required |
| Real Blender | `just smoke-blender-real` | required | required | required |
| Acceptance E2E | `just smoke-blender-scene` (JSONL + calibrated silhouette + reopen) | - | - | required |
| Regression | `just smoke-jsonl`, `just smoke-reopen`, `just smoke-e2e` | required | required | required |
| Cross-platform | Windows local real Blender; Linux CI Rust checks; no local Linux Blender stand | required | required | required |

Evidence for each iteration: `target/techlead/<iteration>/` (gitignored) with `report.md`,
`logs/` and `evidence.md`; Lead copies a concise verified summary into this TZ. The independent
reviewer receives the iteration commit diff (written to `target/techlead/<iteration>/diff.txt`)
and the TZ, and returns a verdict; verdict is recorded in section 8.

## 7. Compatibility and Blender Versions

- Tested target: Blender 5.2.3 LTS (Windows, headless `--background --factory-startup`),
  bundled Python 3.13.
- Code avoids Blender-6.0-removed APIs: `Material.use_nodes` is only set on Blender < 5.0;
  material base color is applied to the Principled BSDF node when present.
- Minimum claimed version: Blender 4.2 LTS by API surface, but only 5.2.3 is verified in this
  TZ; docs must not claim untested versions as verified.
- Windows/Linux: paths passed as process arguments (no shell interpolation), job/result JSON
  is UTF-8 with escaped separators, fake executables are platform-specific (`.sh` / `.cmd`),
  logs are files (no pipe deadlocks), and timeout kills the process tree on both platforms.

## 8. Evidence and Review Log

| Iteration | Commit | Reviewer route/verdict | Notes |
|---|---|---|---|
| IT-1 attempt 1 | `95ebeed` | Separate read-only context (same model family — Windows reviewer route broken: missing `mcp__*_warehouse__` shields/strict `bash` allow-list): **YELLOW** | P1-1 unbounded `blender --version` in capabilities probe; P2: terminal event emitted before transition success, queued failure scheduler/storage split, adapter spec-path rule inconsistent, TZ hygiene. Rework ordered. |
| IT-1 attempt 2 | `8311ef7` | Same separate read-only context: **GREEN** | P1-1 fixed (bounded `--version` with kill-tree); terminal events after successful transitions; queued→failed identity; spec-path rules aligned; missing tests added; `finished_at` lifecycle persistence added (Lead-found). Residual P2 (version parsed from output tail; `taskkill` unbounded in the timeout path) accepted as non-blocking and recorded in the review. |
| IT-2 attempt 1 | `59ca3c5` | Same separate read-only context: **YELLOW** | P1: `render_views` derived filenames from imported camera names could unlink/write PNGs outside the workspace before artifact rejection (Blender imports glTF node names verbatim; reproduced by the Lead with a crafted glTF). P2: FS mutation before containment, import name idempotency, missing outside-import/render_views tests. Rework ordered. |
| IT-2 attempt 2 | `2cf4961` | Same separate read-only context: **GREEN** | Containment-before-mutation for all output paths; safe derived render filenames/collision rejection; import name idempotency; crafted-glTF sentinel regression test (Lead-independent repro: sentinel SHA/size/mtime unchanged); positive render_views (4 unique PNGs), set_transform bbox, cross-platform Windows-path rejection, legacy JSON compat. 271 tests, Lead acceptance E2E 28/28 with calibrated 16384-px silhouette. Non-blocking P2 notes (case-insensitive/Windows reserved names, concurrent symlink TOCTOU) accepted and documented for IT-3. |
| IT-3 attempt 1 | `9749b60` | Same separate read-only context: **GREEN** with six non-blocking P2 doc/assertion items | Cross-platform fake/real acceptance driver, examples, docs, CI. Reviewer P2s: tautological opaque-alpha framing check, wrong `job` response type in CORE_PROTOCOL, over-strong capabilities guarantee, non-reproducible worker example, missing event/response ordering + artifact-metadata checks, python3/`CARGO_TARGET_DIR` doc inconsistency. Rework ordered. |
| IT-3 attempt 2 | `cbb146f` | Same separate read-only context: **GREEN** | All six P2s materially fixed: discriminating framing counterfactual (framed 26,142 px/0.0997 vs unframed 86 px/0.00033), raw storage `job` record documented, capabilities pass-through wording, reproducible example, event-before-response ordering + artifact metadata equality + invalid-spec `-32602` with no record, python3 default and `--cli`/`CARGO_TARGET_DIR` docs. |
| IT-3 final accuracy | `e2da7fb` | Follow-up to reviewer's remaining P2 notes (final confirmation requested) | Corrected examples-README counterfactual (margin passes; coverage+span fail, ~91x) and documented thresholds (0.03..0.95, span ≥205 px, 8 px margin); gated vacuous invalid-request check; removed tautological opaque-alpha check. Final smoke counts: fake 113 checks/3 skips, real 119 checks/0 skips; 271 Rust tests unchanged. |

Reviewer-route note: on this Windows profile the `worker` agent preset shield allows the
unavailable `bash` tool, and the `researcher`/`reviewer` shields reference MCP prefixes for
servers that are not registered. These are harness-configuration issues, not repository code;
until they are fixed the independent review runs in a separate read-only context on the same
available model family, and this limitation is reported to the user.

## 9. Execution Checklist

- [x] IT-1 implementation complete
- [x] IT-1 verification ladder green (Windows) — Lead-run, see review evidence
- [x] IT-1 independent review green — attempt 2 (`8311ef7`) GREEN
- [x] IT-2 implementation complete
- [x] IT-2 verification ladder green + real Blender op smoke — 271 tests, Lead acceptance E2E 28/28
- [x] IT-2 independent review green — attempt 2 (`2cf4961`) GREEN
- [x] IT-3 E2E acceptance scenario + docs/CI complete
- [x] IT-3 full regression + real Blender smoke green — 271 Rust tests, fake 113 checks, real 119 checks (Blender 5.2.3)
- [x] IT-3 independent review green — attempt 1 GREEN with P2s, attempt 2 GREEN after fixes; final accuracy pass `e2da7fb`
- [ ] Final report (Russian): changed files, tests, limitations, JSONL example — produced in the chat after the final review confirmation

## 10. Final Result

Acceptance criterion is met on the verified stand (Windows, Blender 5.2.3 LTS):

- composite scene with named components, box/cylinder/sphere, material and exact transforms;
- calibrated orthographic cameras (`look_at`, ortho scale, pose/settings/framing, resolution);
- silhouette/alpha render of a selected camera: exactly **16,384** pixels α>127, bbox
  **(128,383,224,287)** for the documented fixture; framed scene silhouette coverage
  0.0997 with a meaningful bbox (counterfactual without `frame_camera`: 86 px);
- `.blend` (zstd) and `.glb` (glTF v2) saved and validated;
- job state/artifacts/events persisted across a process boundary with `started_at`/`finished_at`;
- invalid specs/worker failures/silhouette timeouts reported as structured job/protocol errors.

Final commits on `dev` (not pushed): `95ebeed`, `8311ef7`, `7ffb232`, `59ca3c5`, `2cf4961`,
`ff230ef`, `9749b60`, `cbb146f`, `e2da7fb` (+ this TZ record commit).

Residual limitations: Linux real-Blender stand unavailable locally (CI runs Rust/Python/fake
only); Workbench-only rendering; LLM/Visual Hull/Parametric Solver/Asset Graph/Unreal out of
scope; same-model-family reviewer context used because the Windows reviewer preset is
misconfigured; `just` is not installed on this stand (Python driver and direct commands used).
