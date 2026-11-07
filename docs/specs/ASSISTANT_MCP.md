# Bounded Assistant Surface and MCP Adapter (v0)

Normative contract of the Michelangelo **assistant surface** (Stage D2): a
reconstruction run can be handed to an external assistant as a *bounded task*,
and the assistant can hand a *candidate change* back to the local edit
boundary. The surface is deny-by-default, budgeted, audited and local.

Status: implemented in Stage D2 (crate `michelangelo_assistant`, CLI
`michelangelo assistant` and `michelangelo mcp`). Schemas:
`michelangelo.assistant-task/0`, `michelangelo.assistant-policy/0`,
`michelangelo.assistant-budget/0`.

## Purpose and ADR reference

ADR 0004 §8.2 requires that a future finishing task names the base revision,
the allowed components and the spatial region, the protected parts, the
available references and contours, the budget of time, calls and cost and the
verification criteria — and that the result is a separate candidate that is
never applied automatically on a stale base. ADR 0004 §13.4 permits connecting
a concrete API, MCP adapter or 3D backend **only after** the permissions, the
budgets, the data policy and the measurable acceptance criteria are defined.

This iteration delivers that provider-neutral part:

* a deterministic **bounded task** generated from a reconstruction run,
* a **policy** that decides which tools exist and which budgets apply,
* a **budget** with persisted counters and an append-only **audit log**,
* an **MCP adapter** (newline-delimited JSON-RPC 2.0 over stdio) that exposes
  four allow-listed tools to a local LLM client.

The surface is Core-side (Rust library + CLI). It never applies a candidate:
the decision of a submission is always `proposed` and an explicit author
acceptance (`michelangelo edit accept`) publishes the new scene revision.

### No network, no embedded model

The crate and the CLI of this stage contain **no HTTP client, no TCP or socket
code, no embedded model and no provider integration**. Nothing is ever sent
anywhere: the task, the candidate and the reports stay on the machine. The
static check of the acceptance criteria greps the new crate and the CLI for
`TcpStream`, `std::net`, `reqwest`, `ureq`, `hyper` and `socket` and
finds nothing. The only child processes of the repository are the existing
Blender and solver invocations, which this iteration does not add.

The external agent (an LLM client) is a *client of the MCP adapter*: it
supplies the candidate. The adapter never calls a model itself.

## The bounded assistant task

Schema `michelangelo.assistant-task/0`:

```json
{
  "schema": "michelangelo.assistant-task/0",
  "task_id": "asst-0001",
  "created_at": "2026-11-07T12:00:00Z",
  "base": {
    "package_id": "harbor_lantern_v01",
    "package_revision": 1,
    "run_id": "real-run",
    "scene_revision": "s0001"
  },
  "allowed_components": ["stub_u_flame_shape"],
  "allowed_region": {"min": [-0.015, -0.029, -0.178], "max": [0.014, 0.042, -0.117]},
  "protected": ["blockout", "supplement_bracket_arm_connection"],
  "unknowns": [
    {"id": "u_flame_shape", "description": "flame silhouette is not silhouette-observable", "policy": "stub_allowed"}
  ],
  "supplements": [
    {"supplement_id": "chamber_void", "kind": "cavity", "status": "applied"}
  ],
  "references": {
    "views": [{"view_id": "front", "mask": "package/masks/front.png", "image": null}],
    "renders": [{"view_id": "front", "path": "renders/front.png"}],
    "quality_report": "quality_report.json",
    "build_report": "build_report.json"
  },
  "acceptance_criteria": {
    "min_source_iou": 0.85,
    "min_evaluation_iou": 0.7,
    "measurement_relative_error_max": 0.05,
    "reject_if_protected_changed": true
  },
  "budget": {
    "max_calls": 3,
    "max_seconds": 600,
    "max_cost_usd": 0.0,
    "max_candidate_bytes": 67108864
  },
  "data_policy": {
    "allow_network": false,
    "allow_external_service": false,
    "allowed_reference_kinds": ["mask", "silhouette_render", "report"]
  },
  "provenance": {"origin": "core", "detail": "bounded task generated from run 'real-run'"}
}
```

Rules:

* The document is strict: an unknown field makes the task *not a task*. The
  document never carries an absolute path; every path is relative to the run
  directory.
* `base` names the package, the package revision, the run and the **current**
  scene revision of the run (the revision the store points to).
* `allowed_components` is a non-empty subset of the components of the scene.
  A component that the run protects (`michelangelo edit init --protect`) can
  never be allowed.
* `protected` is every other component of the scene, so the two sets are
  always disjoint.
* `allowed_region` is required and `min < max` on every axis. The default is
  the bounding box of the allowed components with a margin of 5 % on every
  axis. A region that does not intersect an allowed component is refused
  (`TASK_EMPTY`).
* `unknowns` and `supplements` are summarised from the manifest of the
  package and from the report of the run and are sorted by identifier. The
  policy of an unknown is `author_required`, `rule_allowed` or
  `stub_allowed`.
* `references` are filtered: only the **calibrated** views of the package
  (their masks and images), the silhouette renders of the run
  (`renders/`) and the build and quality reports of the run. The
  `evaluation/`, `evaluation_renders/` and `ground_truth/` data are never
  referenced, and a reference is only listed when the file exists inside the
  run.
* `acceptance_criteria` comes from the thresholds of the report
  (`source_min_iou`, `evaluation_min_iou`, `bbox_relative_error_max`).
  A change of a protected component always refuses the result.
* `budget` is finite and non-negative; `max_calls >= 1` and
  `max_candidate_bytes` is in 1 KiB..=1 GiB. The budget of the task is the
  budget of the policy at the moment of the generation; the stricter of the
  two applies at the submission.
* The generation is deterministic: the same run, the same request and the same
  policy produce the same task. Only `created_at` changes, and it comes from
  an injected clock (a fixed clock in the tests).

The task is written into the run directory:

```text
<run_dir>/assistant/tasks/asst-0001.json
```

## The policy of the workspace

The optional file `<workspace>/.michelangelo/assistant_policy.json`:

```json
{
  "allow_apply": false,
  "allow_submit": true,
  "allowed_tools": ["get_task", "validate_package", "submit_candidate", "get_report"],
  "max_calls_per_run": 3,
  "max_candidate_bytes": 67108864,
  "allow_external_service": false
}
```

* The file is optional: an absent file means the values above. The file is
  strict — an unknown field, an unknown tool, a duplicated tool, a
  `max_calls_per_run` below 1, a size limit outside 1 KiB..=1 GiB or an
  `allow_apply` of `true` is a startup error (exit code 2). The boundary
  never guesses the intent of a broken policy.
* The search for the file walks up the directory tree, like the edit policy of
  Stage C.
* `allow_apply` must stay `false`: the acceptance is always an explicit
  author decision. A policy that sets it is refused.
* `allow_external_service` is a declaration of the human for a future stage.
  This stage never opens a network connection, so the flag changes nothing; the
  data policy of every task denies the network and the external services.
* **Precedence:** the stricter of the policy and the budget of the task wins
  (`min` of `max_calls_per_run` and `max_calls`, `min` of
  `max_candidate_bytes` and `max_candidate_bytes`).

## The budgets and the audit log

The counters of a run live in `<run_dir>/assistant/budget.json`:

```json
{
  "schema": "michelangelo.assistant-budget/0",
  "calls_used": 1,
  "candidates_submitted": 1,
  "bytes_submitted": 1234,
  "cost_used_usd": 0.0,
  "last_call_at": "2026-11-07T12:00:00Z",
  "updated_at": "2026-11-07T12:00:00Z"
}
```

* The counters are published with a write-to-temp + rename, so a reader never
  observes a half-written file and the counters survive separate invocations of
  the CLI (a new store instance reads the same run directory).
* `calls_used` counts the admitted submissions of the run (the calls that
  consume the bounded budget), `candidates_submitted` the candidates that
  entered the store and `bytes_submitted` the bytes they carry. The counters
  are incremented **after** the durable candidate record was written.
* A submission is admitted only when `calls_used < max_calls` and the total
  number of submitted bytes stays below `max_candidate_bytes`; otherwise the
  call is refused with `BUDGET_EXHAUSTED`.

The audit log of a run lives in `<run_dir>/assistant/audit.jsonl`: one JSON
object per line, append-only:

```text
{"ts":"2026-11-07T12:00:00Z","tool":"submit_candidate","decision":"proposed","candidate_id":"cand-0001","task_id":"asst-0001","detail":"the candidate 'cand-0001' is 'proposed'"}
```

* The decisions are `ok`, `proposed`, `rejected` and `error`. A failed
  call (a policy denial, an exhausted budget, a path outside the workspace, an
  invalid task) is written with the decision `error`.
* The line never carries an absolute path and never carries a secret: the
  messages are sanitized before they are written.

## The data policy

| Rule | Value |
|---|---|
| network access | never (`allow_network: false`) |
| external services | never (`allow_external_service: false`) |
| reference kinds | `mask`, `silhouette_render`, `report` |
| evaluation data | never referenced |
| ground truth | never referenced |
| absolute paths | never written into a task, an audit line or a response |
| secrets | never read, never written |

## The MCP adapter

`michelangelo mcp serve [--workspace <WS>]` serves the surface over the
newline-delimited JSON-RPC 2.0 stdio transport: one JSON object per line on
stdin, one JSON object per line on stdout, the diagnostics on stderr. The
requests are handled sequentially; a malformed line never kills the server and
the end of the input terminates the server with the exit code 0.

### The handshake

```text
-> {"jsonrpc":"2.0","id":1,"method":"initialize"}
<- {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"michelangelo","version":"0.1.0"}}}
-> {"jsonrpc":"2.0","method":"notifications/initialized"}
```

The notification is accepted silently. `ping` answers `{}`.

### The tools

`tools/list` returns the policy-filtered subset of the four tools:

| Tool | Arguments | Result |
|---|---|---|
| `michelangelo_get_task` | `{run_id}` | the bounded task and its run-relative path |
| `michelangelo_validate_package` | `{package, run_id?}` | the summary of the validation report |
| `michelangelo_submit_candidate` | `{run_id, task_id, candidate_dir}` | the decision report (`michelangelo.edit-decision/0`) |
| `michelangelo_get_report` | `{run_id}` | the summary of the reconstruction report |

* `run_id` is the name of the run directory (`run-0001`) or the
  workspace-relative path of the run.
* `package` and `candidate_dir` are workspace-relative paths (a path
  relative to the run is accepted for the candidate).
* Every path argument is canonicalized and must resolve **inside** the
  workspace (or inside the run store). An absolute path, a drive prefix or a
  `..` traversal is refused with `PATH_NOT_ALLOWED`.
* A response never carries an absolute host path.

### The submission

`michelangelo_submit_candidate` runs the full validation:

1. the policy must allow the submission (`allow_submit`);
2. the budget of the run must admit the call (`max_calls`, the size of the
   candidate);
3. the candidate must follow `michelangelo.edit-candidate/0`;
4. the candidate must stay inside the scope of the task: the same package,
   the same run, the same scene revision, an allowed set that is a subset of
   the task, a protection that contains the protection of the task and only
   allowed components. The region of the task is the authority: the effective
   region of the candidate is the intersection of the region of the candidate
   and of the region of the task; a candidate whose region does not intersect
   the region of the task is refused;
5. the candidate is staged with the origin `ai_author`, with
   `require_author_acceptance: true` and with the effective region, and is
   routed through `michelangelo_edit::apply::submit_candidate`.

The decision of the submission is therefore always `proposed`: the MCP
adapter never applies a candidate and never publishes a scene revision. An
explicit `michelangelo edit accept --candidate <id> --by <name>` publishes
the new revision.

### The error codes

| Code | Meaning |
|---|---|
| `-32700` | the line is not valid JSON |
| `-32600` | the request is not a valid JSON-RPC 2.0 request |
| `-32601` | the method is not supported |
| `-32602` | the arguments of the tool are invalid (or the tool is unknown) |
| `-32603` | an unexpected internal failure |
| `-32001` | the policy of the workspace denies the call (`POLICY_DENIED`) |
| `-32002` | a path resolves outside the workspace (`PATH_NOT_ALLOWED`) |
| `-32003` | the budget of the run is exhausted (`BUDGET_EXHAUSTED`) |
| `-32004` | the task or the candidate is not usable (`TASK_INVALID`) |
| `-32005` | the run, the task or the candidate does not exist (`NOT_FOUND`) |

## The CLI

```text
michelangelo assistant task   --run <run_dir> --allowed <a,b,...> [--region min=max]
                              [--out <file>] [--json]
michelangelo assistant submit --run <run_dir> --task <file> --candidate <dir> [--by <name>] [--json]
michelangelo mcp serve        [--workspace <WS>]
```

| Exit code | Meaning |
|---|---|
| `0` | the task was written or the candidate was proposed |
| `1` | the candidate was rejected, the policy denied the call or the budget is exhausted |
| `2` | usage error, IO error or a document that cannot be interpreted |

* `--region` takes `min=max`, three comma separated coordinates on each
  side, for example `--region -0.2,-0.2,-0.1=0.2,0.2,0.1`. Without it the
  region is the bounding box of the allowed components with a margin of 5 %.
* `--out` writes the task into a file instead of
  `<run_dir>/assistant/tasks/<task_id>.json`.
* `assistant submit` writes the same audit line as the MCP tool and never
  applies the candidate.

## The acceptance criteria

| Criterion | Measure | ADR |
|---|---|---|
| the task is bounded | the task names the base revision, the allowed components, the region, the protected components and the budget; the two component sets are disjoint | §8.2 |
| the references are filtered | no path of the task points into `evaluation/`, `evaluation_renders/` or `ground_truth/`; only the three allow-listed kinds occur | §8.2, §9.3 |
| no absolute path | no task, audit line or MCP response contains an absolute host path | §9.3 |
| the candidate is not applied | the decision of a submission is `proposed`; the scene revision only changes after `michelangelo edit accept` | §8.2 |
| the budget is bounded | the counters persist across separate CLI invocations; the call beyond `max_calls` is refused with `-32003` | §8.2, §9.3 |
| the policy is deny-by-default | `allow_apply` is false; an invalid policy file stops the server with a clear error | §13.4 |
| the data policy is explicit | `allow_network` and `allow_external_service` are false; nothing leaves the machine | §9.3, §13.4 |
| the audit is complete | one line per call, including the failed calls; no secret and no absolute path | §13.4 |
| the adapter is local | the static check finds no HTTP, TCP or socket code and no embedded model | §13.4 |

The measurable version of the criteria is the test suite: 93 tests of the
crate `michelangelo_assistant`, 12 integration tests of
`michelangelo assistant` and 4 integration tests of the stdio transport.


## The transcript of an acceptance run

A manual run against the run of IT-6 (`--workspace target/techlead/IT-6`,
the run `real-run`). The long responses are truncated here; the full
transcript is the output of the stdio integration test.

```text
-> {"jsonrpc":"2.0","id":1,"method":"initialize"}
<- {"jsonrpc":"2.0","id":1,"result":{"capabilities":{"tools":{}},"protocolVersion":"2025-06-18","serverInfo":{"name":"michelangelo","version":"0.1.0"}}}
-> {"jsonrpc":"2.0","method":"notifications/initialized"}
-> {"jsonrpc":"2.0","id":2,"method":"tools/list"}
<- {"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"michelangelo_get_task",...},{"name":"michelangelo_validate_package",...},{"name":"michelangelo_submit_candidate",...},{"name":"michelangelo_get_report",...}]}}
-> {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"michelangelo_get_task","arguments":{"run_id":"real-run"}}}
<- {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\n  \"path\": \"assistant/tasks/asst-0001.json\", ..."}],"isError":false,"structuredContent":{"path":"assistant/tasks/asst-0001.json","schema":"michelangelo.assistant-task/0","task":{...}}}}
-> {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"michelangelo_submit_candidate","arguments":{"run_id":"real-run","task_id":"asst-0001","candidate_dir":"mcp-demo/candidate-0001"}}}
<- {"jsonrpc":"2.0","id":4,"result":{"content":[...],"isError":false,"structuredContent":{"candidate_id":"cand-mcp-0001","decision":"proposed","base_scene_revision":"s0001","resulting_scene_revision":null,...}}}
-> {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"michelangelo_get_report","arguments":{"run_id":"real-run"}}}
<- {"jsonrpc":"2.0","id":5,"result":{"content":[...],"isError":false,"structuredContent":{"package_id":"harbor_lantern_v01","status":"pass","source_mean_iou":0.9595,...}}}
```

The decision of the submission is `proposed`; the scene revision only changes
after `michelangelo edit accept`. The stdio integration test adds the
budget exhaustion (`-32003`), the path escape (`-32002`), the malformed line
(`-32700`) and the end of the input (exit code 0).

## The limits of v0

* **No model, no network.** The adapter is a tool boundary; an external agent
  supplies the candidate.
* **No automatic application.** The decision of a submission is always
  `proposed`.
* **Component-level scope.** The allowed region is a box; there is no
  per-face or per-intersection budget.
* **No re-score.** The acceptance criteria are measured by the human (or by a
  later stage); the surface does not re-render the scene.
* **One policy, one workspace.** The policy file is found by walking up the
  directory tree; there is no per-run policy.
* **The audit is per run.** A call that names no run (a package validation
  without a `run_id`) is not written into an audit log.

## The fixtures

`examples/assistant/` carries a small, synthetic example of the surface: a
task document and a candidate directory. It carries no dataset content.
