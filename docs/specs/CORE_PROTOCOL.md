# Core Protocol Specification

Revision: Phase 2.5 (reliability hardening + safe Blender scene operations). The wire format is
backward compatible with Phase 1/Phase 2: additions are optional fields or new operation names.

## Transport

- Single-threaded JSONL over stdin/stdout; one JSON object per line (UTF-8).
- stdout carries **only** protocol messages: responses and lifecycle events.
- stderr carries human-readable diagnostics.
- Events are drained and written **before** the response of the request that produced them.
- Requests are processed sequentially until stdin EOF; there is no background runner in the
  stdio process.

## Message formats

### Request (stdin)

```json
{"id":"req-1","method":"system.ping","params":{}}
```

`id` is echoed in the response. Missing/invalid `id` yields a response with `id: null`.

### Response (stdout)

```json
{"id":"req-1","result":{"pong":true,"protocol_version":"0.1.0"}}
```

### Protocol error response (stdout)

```json
{"id":"req-2","error":{"code":-32602,"message":"operation[0] ('save_blend'): ..."}}
```

### Event (stdout, before its associated response)

```json
{"event":"job.queued","data":{"job_id":"job_...","job_type":"blender_acceptance_scene"}}
```

Events never have an `id` and do not expect a response.

### Error codes

| Code | Meaning |
|---|---|
| `-32700` | Parse error (invalid JSON) |
| `-32600` | Invalid request envelope |
| `-32601` | Method not found |
| `-32602` | Invalid params (including invalid Blender specs) |
| `-32603` | Internal error (including scheduler/storage transition failure) |
| `-32000` | No project is currently open |
| `-32001` | Workspace already exists |

## Methods

### Phase 1

| Method | Params | Returns |
|---|---|---|
| `system.ping` | `{}` | `{"pong": true, "protocol_version": "0.1.0"}` |
| `project.create` | `{"path": "...", "name": "..."}` | `ProjectDto` `{id, name, root_path}` |
| `project.open` | `{"path": "..."}` | `ProjectDto` |
| `project.get_snapshot` | `{}` | `ProjectSnapshotDto` (includes `jobs: [...]`, assets summary, status/versions) |

`project.create` creates the workspace layout and `.michelangelo/metadata.db`. `root_path` is the
canonicalized workspace path (on Windows this may carry the `\\?\` verbatim prefix); `project.open`
accepts either the canonical form or the caller's spelling.

### Phase 2 — jobs

| Method | Params | Returns |
|---|---|---|
| `job.list` | `{}` | `{"jobs": [<storage record>, ...]}` |
| `job.get` | `{"job_id": "..."}` | one storage record or `-32602` (`job not found`) |
| `job.cancel` | `{"job_id": "..."}` | `{"job_id": "...", "status": "cancelled"}` |

Storage job records are raw persisted rows with the fields `id`, `project_id`, `job_type`,
`status`, `progress_pct`, `message`, `input_json`, `output_json`, `error`, `logs`, `created_at`,
`started_at`, `finished_at`.

### Phase 2 — Blender

| Method | Params | Returns |
|---|---|---|
| `blender.get_capabilities` | `{}` | capabilities descriptor (below) |
| `blender.run_job` | `BlenderJobSpec` (below) | job-level result (below) |

## `blender.get_capabilities`

Request: `blender.get_capabilities {}`. The probe is bounded and never hangs the request.

Response fields (new fields are optional; Phase 2 field names are unchanged):

```json
{
  "blender_version": "Blender 5.2.3 LTS",
  "bpy_available": true,
  "supported_operations": ["create_box", "...18 safe ops..."],
  "binary_found": true,
  "python_version": "3.13.13",
  "gltf_export_available": true,
  "render_engines": ["BLENDER_WORKBENCH", "BLENDER_EEVEE", "CYCLES"]
}
```

`python_version`, `gltf_export_available`, `render_engines` and `error` are optional fields and
are omitted when unknown; `error` is present only when probing failed (the payload above is the
successful-probe shape).

- `supported_operations` is the operation list actually reported by `blender/worker.py`
  (`--capabilities`), in stable order. The static whitelist is used as a fallback when probing
  fails; unknown/extra operations are never advertised as available.
- Missing binary: normal response with `binary_found=false`, `bpy_available=false`,
  `blender_version=null`, static whitelist in `supported_operations`, `error` set.
- Binary present but the worker probe failed: `binary_found=true`, `bpy_available=false`, `error`
  set, static fallback operations.
- `render_engines` reports engines installed in the Blender build; the **supported render engine
  for jobs is Workbench only** (see `BLENDER_WORKER.md`).

## `blender.run_job`

Params are the typed `BlenderJobSpec`:

```json
{
  "job_type": "blender_acceptance_scene",
  "wait": true,
  "timeout_ms": 300000,
  "spec": {
    "operations": [
      {"op": "create_box", "name": "body", "size": [2.0, 1.0, 0.5]},
      {"op": "save_blend", "path": "blender/scenes/smoke.blend"}
    ]
  }
}
```

| Field | Required | Notes |
|---|---|---|
| `job_type` | yes | trimmed non-empty, ≤ 128 chars, no control characters |
| `wait` | no (default `false`) | `true` = execute synchronously; `false` = queue-only |
| `timeout_ms` | no | explicit Blender timeout override (see timeout precedence) |
| `spec.operations` | yes | ≤ 256 typed operations; see `BLENDER_WORKER.md` |

Validation happens **before** any job record, scheduler entry or event is created. Invalid
operations/params, unsafe paths, unsupported view/import formats and unknown `op` values return
`-32602` with a message naming `operation[i] ('<op>')`; no orphan queued job is left behind.

### `wait=false` (queue-only)

The job is persisted and enqueued, only `job.queued` is emitted in this process, and the response
is:

```json
{"id":"job_...","job_type":"blender_acceptance_scene","status":"queued"}
```

There is no background execution loop in the stdio process: a queued job stays `queued` until a
later request in the same process runs it. The queue is identity-safe: a later `wait=true` request
starts **its own** job (`Scheduler::dequeue_job` by id), never the FIFO head, so a `wait=false` job
can never be hijacked or complete under another id.

### `wait=true` (synchronous execution)

Flow: create persisted record → enqueue → `job.queued` → start exactly this job →
`job.started` → persist `running` → run the bounded Blender subprocess → map the worker result to
the terminal state → emit exactly one terminal event → return the response.

Responses (all are successful JSON-RPC envelopes; failures are **job-level**, not protocol-level):

| Terminal outcome | Response `result` |
|---|---|
| completed | `{"job": <JobDto or null>, "status": "completed"}` |
| failed | `{"job": <JobDto or null>, "status": "failed", "error": "<message>"}` |
| cancelled | `{"job": <JobDto or null>, "status": "cancelled"}` |

The returned `job` is re-read from storage after the transition, so `status`, `started_at`,
`finished_at`, `error` and `output_json` reflect persisted state; every event for the job carries
the same job id.

### Job-level failures vs protocol errors

- **Job-level** (`result.status == "failed"`, plus one `job.failed` event): worker returned
  `status:"failed"`, the adapter rejected the result/artifacts, the binary was missing, the
  subprocess crashed or timed out. The job is persisted as `failed` and `error` carries the
  diagnostic (adapter tails are capped).
- **Protocol-level** (`error` envelope): no project open (`-32000`), invalid request/params or job
  spec (`-32602`), unknown method (`-32601`), or an internal scheduler/storage transition failure
  (`-32603`). An invalid spec never creates a job.

### Terminal event ordering guarantees

- `job.completed` / `job.failed` / `job.cancelled` are emitted **only after** both the storage and
  scheduler terminal transitions succeeded.
- If a transition fails, the job is recovered to `failed` where legal, no contradictory terminal
  event is emitted, and the request returns `-32603` naming the inconsistency.
- A job that fails after its record exists but before execution (enqueue/start/storage failure) is
  terminal-failed consistently: scheduler, storage and the single emitted `job.failed` event all
  report the same job id and reason. The legal transition is `queued → failed`.
- No terminal event is emitted twice; `wait=false` jobs emit only `job.queued` in the current
  process.

### Timeout precedence

`blender.run_job` uses the first positive value of:

1. the request's `timeout_ms` argument;
2. the `MICHELANGELO_BLENDER_TIMEOUT_MS` environment variable;
3. the built-in default `300_000` ms.

Values of `0` and unparsable environment values are ignored (a bounded positive timeout always
results). The adapter's probes are independently bounded: `blender --version` (10 s) and
`--capabilities` (30 s). On timeout the whole process tree is killed and waited for; the job fails
with a message including the timeout and a stderr tail.

### Artifact verification

A `completed` worker result is accepted only when:

- every listed artifact path is workspace-relative, traversal-free and has no Windows drive/UNC
  form on any platform;
- artifact `type` is one of `blend`/`glb`/`png` and matches the file extension;
- the file exists, is a regular non-empty file, resolves inside the workspace after symlink
  resolution, and `size_bytes` (when present) equals the real file size.

Violations fail the job (`MissingArtifact`/`InvalidArtifact` mapped to a `failed` job and
`job.failed` event). `failed`/`cancelled` worker results keep only safe partial artifacts and
never mask the original error.

## Events

| Event | Data | Required |
|---|---|---|
| `job.queued` | `{"job_id","job_type"}` | yes, once per started job |
| `job.started` | `{"job_id","job_type"}` | yes for `wait=true` |
| `job.progress` | `{"job_id","job_type","progress_pct","message"}` | optional; current batch workers do not emit it |
| `job.completed` | `{"job_id","job_type","status":"completed","artifacts":[...]}` | yes (terminal) |
| `job.failed` | `{"job_id","job_type","status":"failed","error":"..."}` | yes (terminal) |
| `job.cancelled` | `{"job_id", "job_type"?}` | yes (terminal; `job_type` included when cancelled during execution) |

For a `wait=true` job the required sequence is exactly
`job.queued → job.started → one of job.completed/job.failed/job.cancelled`, all with the same
`job_id`; events are written before the final response.

## Lifecycle states

```text
queued → running → completed
queued → running → failed
queued → running → cancelled
queued → cancelled
queued → failed            (internal failure path before execution; consistent identity)
```

Max concurrency is 1. `job.cancel` moves a cancellable job to `cancelled`, persists the status and
emits `job.cancelled`.

## Job persistence

Jobs are persisted in SQLite at `<workspace>/.michelangelo/metadata.db` (table `jobs`) and survive
process restarts. Persisted fields include `status`, `progress_pct`, `message`, timestamps
(`created_at`, `started_at`, `finished_at`), `error`, `logs`, and `output_json` (the validated
worker result, including artifact metadata and `scene_summary`).

- `job.list` returns raw records (with `started_at`/`finished_at` and `output_json`);
- `job.get` returns one raw record;
- `project.get_snapshot` maps records to `JobDto`: `completed_at` is the snapshot alias of
  `finished_at`, and `artifacts` is derived from `output_json.artifacts`.

A completed job is readable after reopening the project in a new process with `status=completed`,
non-null `started_at`/`finished_at` and its artifact list intact.
