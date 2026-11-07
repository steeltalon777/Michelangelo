# TZ: Stage D2 — Optional Bounded Assistant Surface + MCP Adapter (ADR 0004 §13.4)

Status: in progress
Branch: `dev` (no push, no branch switch; commits only after verified checks and a GREEN review)
Owner: TechLead (orchestrator); implementation by worker subagents; independent review before commit.
Depends on: IT-4 (input package), IT-5 (reconstruction run), IT-6 (local-edit boundary).

## 0. Context and Baseline

- ADR 0004 §8 defines two modes of one core: standalone and "with assistant". §8.2 requires a
  bounded task and a candidate-change artifact; §8.3 requires that the assistant not touch the
  authoritative scene directly and that image-to-3D backends not be presented as local editors;
  §9.3 requires untrusted-input handling, explicit permission for sending references to external
  services, no secrets in tasks/prompts/reports, and bounded paid calls.
- ADR §13.4: connect a concrete API/MCP/3D-backend only **after** permissions, budgets, data
  policy and measurable acceptance criteria are defined. This iteration delivers the safe,
  provider-neutral part: the bounded task/candidate surface and an MCP adapter for local LLM
  clients over stdio. It does **not** embed any model/provider or network client.
- IT-6 provides the candidate validator/applier and the immutable scene store.

## 1. Goal

1. **Bounded assistant task** (`michelangelo.assistant-task/0`), generated deterministically from
   a reconstruction run: base package/scene revision, allowed components and region, protected
   components, relevant references only (masks, silhouette renders, reports), acceptance criteria
   and budgets. Never the whole project, never ground-truth/evaluation data.
2. **Bounded candidate intake**: `assistant submit` validates the task (budgets, permissions,
   reference kinds, candidate size), enforces persisted per-run budget counters, and routes the
   candidate through the IT-6 validator with `origin = ai_author`; the result is always
   `proposed` unless an explicit author/policy acceptance applies.
3. **MCP adapter** (`michelangelo mcp serve`): a stdio JSON-RPC 2.0 server exposing a small,
   allow-listed tool surface for local clients (get task, validate package, submit candidate,
   get report), with deny-by-default policy and no network/secret exposure.
4. **Policy, budgets, data policy and measurable acceptance criteria** documented and enforced,
   with an auditable audit log of every tool call (timestamp, tool, decision, candidate id,
   budget counters) inside the workspace store.

## 2. Out of Scope (hard limits)

- No embedded LLM/model provider, no HTTP/network client, no browser automation, no cloud 3D
  backend. The adapter is a local tool boundary; an external agent (LLM client) supplies
  candidates.
- No automatic application of AI candidates: default policy `allow_apply = false`; application
  requires explicit human `michelangelo edit accept` (or a future, separately reviewed policy).
- No changes to the JSONL Core wire contract; MCP is a separate transport/server command.
- No new Blender ops, no SQLite migration, no changes to `michelangelo_input`/recon/edit
  contracts other than adding public accessors needed by the adapter.
- No sending of images/masks outside the machine (nothing is sent anywhere by this code).

## 3. Contract (normative)

### 3.1 Assistant task

`schema michelangelo.assistant-task/0`:

```json
{
  "schema": "michelangelo.assistant-task/0",
  "task_id": "asst-0001",
  "created_at": "2026-11-07T12:00:00Z",
  "base": {"package_id": "harbor-lantern-poc", "package_revision": 1,
            "run_id": "run-0001", "scene_revision": "s0002"},
  "allowed_components": ["lid"],
  "allowed_region": {"min": [-0.2, -0.2, -0.1], "max": [0.2, 0.2, 0.1]},
  "protected": ["base", "bracket"],
  "unknowns": [{"id": "u_lid_inside", "description": "...", "policy": "author_required"}],
  "supplements": [{"supplement_id": "...", "kind": "thickness", "status": "applied"}],
  "references": {
    "views": [{"view_id": "front", "mask": "masks/front.png", "image": "images/front.png"}],
    "renders": [{"view_id": "front", "path": "renders/front.png"}],
    "quality_report": "quality_report.json",
    "build_report": "build_report.json"
  },
  "acceptance_criteria": {
    "min_source_iou": 0.95, "min_evaluation_iou": 0.80,
    "measurement_relative_error_max": 0.05,
    "reject_if_protected_changed": true
  },
  "budget": {"max_calls": 3, "max_seconds": 600, "max_cost_usd": 0.0,
             "max_candidate_bytes": 67108864},
  "data_policy": {"allow_network": false, "allow_external_service": false,
                  "allowed_reference_kinds": ["mask", "silhouette_render", "report"]},
  "provenance": {"origin": "core", "detail": "bounded task generated from run-0001"}
}
```

- Task generation is deterministic: sorted component/unknown lists, package-relative/run-relative
  paths only (no absolute host paths), `created_at` taken from the clock but excluded from
  determinism tests via an injectable clock in tests.
- `allowed_components` must be a subset of scene components, non-empty; `protected` = all scene
  components not in `allowed_components` unless the caller narrows it further; the two sets are
  disjoint. `allowed_region` required (`min < max`); it must intersect every allowed component's
  bbox (otherwise `TASK_EMPTY`).
- References are filtered: only calibrated-view masks/images, the run's own silhouette renders and
  the run's build/quality reports; never `evaluation/`, `ground_truth/`, package outside the run,
  or absolute paths.
- Budgets are finite, non-negative; `max_calls >= 1`; `max_candidate_bytes` in 1 KiB..=1 GiB.

### 3.2 Policy file

Optional `<workspace>/.michelangelo/assistant_policy.json`:
`{allow_apply: false, allow_submit: true, allowed_tools: ["get_task","validate_package",
"submit_candidate","get_report"], max_calls_per_run: 3, max_candidate_bytes: 67108864,
allow_external_service: false}`. Absent file = defaults above with `allow_apply=false`,
`allow_submit=true`; `allow_external_service` may only be set by the human (documented; no code
uses it to open a network connection — it is a declaration for future stages). Unknown fields and
invalid values reject startup with a clear error.
Policy precedence: the stricter of (policy file, task budget) wins.

### 3.3 Budgets and audit

- Per-run counters in `<run_dir>/assistant/budget.json`: `{calls_used, candidates_submitted,
  bytes_submitted, cost_used_usd, last_call_at, updated_at}`; updates are atomic
  (temp + rename), sequential per process; submission checks `calls_used < max_calls` and
  `bytes <= max_candidate_bytes`, incrementing only after a durable candidate record is written.
- Audit log `<run_dir>/assistant/audit.jsonl`: one JSON object per line
  `{ts, tool, decision, candidate_id|null, task_id|null, detail}`; append-only; no secrets, no
  absolute paths; a failed tool call is logged with `decision: "error"`.

### 3.4 MCP server

`michelangelo mcp serve [--workspace <WS>]` — newline-delimited JSON-RPC 2.0 over stdin/stdout
(stdio transport), no other I/O on stdout, diagnostics on stderr.

- Handshake: `initialize` → `{protocolVersion: "2025-06-18", capabilities: {tools: {}},
  serverInfo: {name: "michelangelo", version: <package version>}}`; `notifications/initialized`
  accepted silently; `ping` → `{}`; unknown methods → JSON-RPC `-32601`.
- `tools/list` returns exactly the policy-allowed subset of:
  1. `michelangelo_get_task` — args `{run_id}` → bounded task JSON (written under the run, path
     returned run-relative);
  2. `michelangelo_validate_package` — args `{package}` (workspace-relative path) → validation
     report summary;
  3. `michelangelo_submit_candidate` — args `{run_id, task_id, candidate_dir}` →
     `michelangelo.edit-decision/0` (always `proposed`; applied never by MCP);
  4. `michelangelo_get_report` — args `{run_id}` → reconstruction report summary.
- Every path argument must resolve inside the workspace (or the run store) after canonicalization;
  outside paths → tool error `-32002 PATH_NOT_ALLOWED`; no absolute host paths in responses.
- Tool errors use JSON-RPC error objects with codes `-32001 POLICY_DENIED`,
  `-32002 PATH_NOT_ALLOWED`, `-32003 BUDGET_EXHAUSTED`, `-32004 TASK_INVALID`, `-32005 NOT_FOUND`;
  malformed JSON → `-32700`; invalid request → `-32600`; invalid tool args → `-32602`.
- Sequential request handling is sufficient (no concurrency); a malformed line never kills the
  server; EOF terminates cleanly with exit 0.

### 3.5 CLI

```text
michelangelo assistant task   --run <run_dir> --allowed <a,b,...> [--region min=max]
                              [--out <file>] [--json]
michelangelo assistant submit --run <run_dir> --task <file> --candidate <dir> [--by <name>] [--json]
michelangelo mcp serve        [--workspace <WS>]
```
Exit codes: `0` ok/proposed, `1` rejected/policy denied/budget exhausted, `2` usage/IO.

## 4. Iteration Plan (IT-7)

Write scopes:

- `crates/michelangelo_assistant/**` (new crate: task generation, policy, budget/audit store,
  MCP server implementation, tests).
- `crates/michelangelo_cli/Cargo.toml`, `crates/michelangelo_cli/src/main.rs`,
  `crates/michelangelo_cli/src/assistant_cmd.rs`, `crates/michelangelo_cli/tests/assistant_cli.rs`,
  `crates/michelangelo_cli/tests/mcp_stdio.rs`.
- Root `Cargo.toml`/`Cargo.lock`.
- `docs/specs/ASSISTANT_MCP.md` (new spec: task schema, policy, budgets, data policy, MCP tool
  contract, error codes, audit), `docs/specs/MVP_SPEC.md` (one cross-reference paragraph),
  `ROADMAP.md`, `TASKS.md`, `docs/ai/AI_CONTEXT.md`, `CHANGELOG.md`, this TZ.
- `examples/assistant/` (small task/candidate fixtures for tests).

Task split:

- **D1 (worker)** — `michelangelo_assistant` crate: task generation from a run (using recon/edit
  public APIs), policy parsing/validation, budget/audit store, MCP server (initialize/list/call,
  tool dispatch, error mapping, path confinement, policy enforcement), tests (≥ 35) including
  stdio round-trip with a real child process? (in-process handler + a CLI-level stdio test in C2),
  policy denial, budget exhaustion, path escape, malformed JSON, no-network static check.
- **D2 (worker, after D1)** — CLI wiring (`assistant task|submit`, `mcp serve`) + integration
  tests over stdio pipes + docs spec + fixtures.
- **D3 (Lead)** — reconcile, ladder, end-to-end acceptance on a real IT-6 run (generate task,
  enforce budget across restart, submit valid + oversized candidates via MCP and CLI, policy
  denial, audit log integrity, no absolute paths), independent review, commit.

## 5. Acceptance Criteria

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` green; ≥ 35 new Rust tests (D1) + CLI/MCP integration tests (D2).
- MCP stdio evidence: `initialize` → `tools/list` (policy-filtered) → `tools/call
  michelangelo_get_task` → valid bounded task JSON → `tools/call michelangelo_submit_candidate`
  with a valid candidate → decision `proposed` → `michelangelo edit accept` via CLI applies it
  (new scene revision); a second submit beyond `max_calls` → `-32003 BUDGET_EXHAUSTED`; a path
  outside the workspace → `-32002`; `allow_apply` is false by default and no MCP tool applies.
- Task JSON contains no absolute paths, no evaluation/ground-truth references, and only
  allow-listed reference kinds; audit log has one line per call and no secrets.
- Static check: the new crate and CLI contain no HTTP/TCP/socket/process-spawn code (grep for
  `TcpStream|std::net|reqwest|ureq|hyper|socket` is empty; the only child processes allowed are
  the existing Blender/solver invocations, which this iteration does not add).
- `docs/specs/ASSISTANT_MCP.md` matches the implementation field-by-field and states the data
  policy and the ADR §8.2/§13.4 mapping.
- No wire-contract/worker changes; `git diff` only in §4 scopes.
- Commit on `dev`, explicit pathspecs, staged diff inspected, no push.

## 6. Evidence

Iteration executed 2026-11-07 on `dev` (base `5a0e4ea`, IT-6). Roles: D1 worker (child
`8cfa67c8`, route `minimax-cn/MiniMax-M3.1-Flash`, preset `worker`) implemented the
`michelangelo_assistant` crate, MCP server, CLI, spec and fixtures; the Lead ran the independent
MCP/policy acceptance, reviewed scope, wrote the repo docs and this evidence.

- Ladder: `cargo fmt --all -- --check` exit 0; `cargo clippy --workspace --all-targets -- -D
  warnings` exit 0; `cargo test --workspace` green — **702 Rust tests** (assistant 93, edit 122,
  recon 60, input 105, CLI 39+, existing suites); `cargo build -p michelangelo_cli` exit 0.
  Static: `Select-String` for `TcpStream|std::net|reqwest|ureq|hyper|socket` over
  `crates/michelangelo_assistant/src/*.rs` and `crates/michelangelo_cli/src/*.rs` → no matches.
- MCP acceptance (`target/techlead/IT-7/mcp_accept.py`, gitignored; workspace
  `target/techlead/IT-7/ws` = copy of the IT-6 run `real-run`):
  `michelangelo assistant task --run ... --allowed stub_u_flame_shape` produced a bounded task
  (`asst-0001`: 1 allowed component, 4 protected, region default, budget max_calls 3,
  max_candidate_bytes 67108864, `allow_network: false`). The stdio session then performed:
  `initialize` → protocolVersion `2025-06-18`, `tools: {}`; `notifications/initialized` (no
  response); `tools/list` → the four tools with JSON schemas; `get_task` → task as
  `structuredContent`; `get_report` → reconstruction summary (`status: pass`, source mean IoU
  0.9595); a malformed line → `-32700` and the server stayed alive (`ping` → `{}` after it);
  `get_task` with `run_id: "../../etc"` → `-32002`; three `submit_candidate` calls →
  decisions `proposed` (`cand-mcp-0001..0003`); a fourth call → `-32003 BUDGET_EXHAUSTED`;
  EOF → exit 0. `assistant/budget.json` after the session: `calls_used: 3`,
  `bytes_submitted: 4716`; `assistant/audit.jsonl` has one line per call (6 lines, including the
  error calls) and no absolute paths. The scene was **not** changed by MCP;
  `michelangelo edit accept --candidate cand-mcp-0001 --by owner` then published `s0003`
  (`current.json` = `s0003`, `candidate_id: cand-mcp-0001`). Decision file:
  `candidates/cand-mcp-0001/decision.json` = `proposed` before the accept.
- Policy checks (manual, same workspace): a policy with `allow_submit: false` still lists the
  tool in `tools/list` (filtered by `allowed_tools`) but a call → `-32001 POLICY_DENIED`; a policy
  with `allow_apply: true` stops the server at startup with a clear error (exit 2). Default policy
  (no file) admits submissions and never applies.
- Acceptance finding (documented, not a blocker): the edit `region` check is strict, so a
  candidate whose geometry lies exactly on the `allowed_region` boundary is rejected
  (`REGION_VIOLATION`); the acceptance candidate was inset by 1 cm and then passed. This is
  consistent with `LOCAL_EDIT.md` ("bbox lies inside the region") and is worth a future
  boundary-tolerance note in the candidate-authoring skill.
- Deviations recorded by the worker (documented in `ASSISTANT_MCP.md`): (1) the candidate is
  staged with `origin = ai_author`, `require_author_acceptance = true` and the effective
  (intersected) region; (2) region = intersection(candidate, task) — a wider candidate region is
  clamped, a non-intersecting one is `TASK_INVALID`; (3) `calls_used` counts admitted submissions,
  every call is audited, and `candidates_submitted` is incremented only for durably recorded
  candidates (observed 2 for 3 admitted submissions because one candidate id was re-submitted);
  (4) `allow_apply: true` is a startup error; (5) `tools/list` is filtered by `allowed_tools`,
  while `allow_submit` is enforced at call time; (6) run resolution accepts
  `<ws>/recon/<rev>/<run>` and `<ws>/<run>`; (7) `validate_package` without `run_id` is not
  audited (no run to attach the line to).
- Reviewer verdict: **GREEN** on attempt 2 (see review log). Attempt 1 returned RED for one MEDIUM finding: an MCP error response could carry an absolute host path when an IO error mentioned it (e.g. a missing candidate geometry file). Fixed in this iteration: `map_error(error, workspace)` now sanitizes every error message with `sanitize_paths` before building the `RpcError`, all call sites pass the workspace root, and a regression test (`an_io_error_over_the_transport_does_not_leak_an_absolute_path`) removes a candidate geometry file, submits over the transport and asserts `-32603` with no workspace path or drive prefix. Two LOW findings were also fixed: `last_call_at` is now recorded with `updated_at` on an admitted submission, and `is_forbidden_reference` also rejects `evaluation_renders` (case-insensitive). Attempt 2 confirmed the fixes and returned GREEN.

### Review log

| attempt | reviewer | route | verdict | findings |
|---|---|---|---|---|
| 1 | child `1b61a9b7` (read-only) | `xiaomi/mimo-v2.6-pro`, preset `reviewer` | RED | MEDIUM: MCP error responses could leak an absolute host path; LOW: `last_call_at` never populated; LOW: `evaluation_renders` missing from the forbidden-reference filter |
| 2 | child `07c2918f` (read-only) | `xiaomi/mimo-v2.6-pro`, preset `reviewer` | GREEN | F1 fixed with `sanitize_paths` in `map_error` (all call sites) + regression test; both LOW fixes confirmed; no new defects |
