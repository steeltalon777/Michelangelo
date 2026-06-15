# Core Protocol Specification

## Transport

Single-threaded JSONL over stdin/stdout. One JSON object per line.
Stdout contains only protocol messages (requests, responses, events).
Stderr contains human-readable diagnostics.

## Message Format

### Request (stdin)

```json
{"id": "req-1", "method": "system.ping", "params": {}}
```

### Response (stdout)

```json
{"id": "req-1", "result": {"pong": true, "protocol_version": "0.1.0"}}
```

### Error Response (stdout)

```json
{"id": "req-1", "error": {"code": -32000, "message": "no project is currently open"}}
```

### Event (stdout, before response)

```json
{"event": "job.queued", "data": {"job_id": "job_...", "job_type": "blender_smoke_scene"}}
```

Events are fire-and-forget: they have no `id` field and do not expect a response.

## Methods

### Phase 1

| Method | Params | Returns |
|---|---|---|
| `system.ping` | `{}` | `{"pong": true, "protocol_version": "..."}` |
| `project.create` | `{"path": "...", "name": "..."}` | `ProjectDto` |
| `project.open` | `{"path": "..."}` | `ProjectDto` |
| `project.get_snapshot` | `{}` | `ProjectSnapshotDto` (includes `jobs: [...]`) |

### Phase 2 — Jobs

| Method | Params | Returns |
|---|---|---|
| `job.list` | `{}` | `{"jobs": [...]}` — raw storage records |
| `job.get` | `{"job_id": "..."}` | single job record or error |
| `job.cancel` | `{"job_id": "..."}` | `{"job_id": "...", "status": "cancelled"}` |

### Phase 2 — Blender

| Method | Params | Returns |
|---|---|---|
| `blender.get_capabilities` | `{}` | `{"blender_version": ..., "bpy_available": bool, "supported_operations": [...], "binary_found": bool}` |
| `blender.run_job` | `{"job_type": "...", "wait": true, "timeout_ms": ..., "spec": {"operations": [...]}}` | `{"job": ..., "status": "completed/failed"}` |

If `wait=true` (default), blocks until the job reaches a terminal state and emits lifecycle events before the response. If `wait=false`, returns immediately after queuing; the job is persisted but **no background execution occurs** in the current stdio process (queue-only behavior).

## Events

Required lifecycle events for Phase 2: `job.queued`, `job.started`, and one terminal event (`job.completed`, `job.failed`, or `job.cancelled`).

| Event | Data | Required |
|---|---|---|
| `job.queued` | `{"job_id": "...", "job_type": "..."}` | yes |
| `job.started` | `{"job_id": "...", "job_type": "..."}` | yes |
| `job.progress` | `{"job_id": "...", "job_type": "...", "progress_pct": N, "message": "..."}` | optional (Phase 2 batch jobs do not emit incremental progress) |
| `job.completed` | `{"job_id": "...", "job_type": "...", "status": "completed", "artifacts": [...]}` | yes (terminal) |
| `job.failed` | `{"job_id": "...", "job_type": "...", "status": "failed", "error": "..."}` | yes (terminal) |
| `job.cancelled` | `{"job_id": "..."}` | yes (terminal) |

## Lifecycle States

```
queued → running → completed
queued → running → failed
queued → cancelled
running → cancelled
```

Max concurrency: 1 for Phase 2.

## Job Persistence

Jobs are persisted in SQLite under `.michelangelo/metadata.db`. The `jobs` table records status, progress, timestamps, error messages, and output JSON. Jobs survive process restarts and are accessible through `job.list`, `job.get`, and `project.get_snapshot`.
