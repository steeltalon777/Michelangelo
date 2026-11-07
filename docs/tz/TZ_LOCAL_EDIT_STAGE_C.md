# TZ: Stage C — Local Edit Boundary (ADR 0004 §13.3)

Status: in progress
Branch: `dev` (no push, no branch switch; commits only after verified checks and a GREEN review)
Owner: TechLead (orchestrator); implementation by worker subagents; independent review before commit.
Depends on: IT-4 (input package) and IT-5 (reconstruction run store + report).

## 0. Context and Baseline

- ADR 0004 §8–§9 and §13.3 require a local-edit boundary before any assistant: a candidate change
  is an explicit, versioned artifact; the previous accepted scene survives a failed change; a
  candidate for a stale base revision is rejected; protected geometry cannot be modified; rollback
  selects a previous accepted revision instead of inverting arbitrary agent actions.
- IT-5 produces, per reconstruction run, an editable component set (voxel blockout, supplement
  objects, stub objects) and an immutable run store under `<workspace>/recon/<revision>/run-NNNN/`.
  Geometry artifacts are importable OBJs; quality is measured by silhouette IoU.
- This iteration adds a **candidate-based scene revision store and deterministic validator** with
  no Blender requirement and no external backend: the "manual candidate / test backend first"
  path from ADR §13.3.

## 1. Goal

1. Introduce an immutable, append-only **scene revision** store for a reconstruction run:
   `scene.json` + component OBJ files per accepted revision, hash-addressed.
2. Define a typed **edit candidate** (`michelangelo.edit-candidate/0`): base package + base scene
   revision, bounded task (allowed parts/components, allowed spatial region, protected parts),
   replacement geometry, provenance, acceptance status and budget facts.
3. Implement a deterministic validator + applier that enforces:
   - stale base revision → rejected (`STALE_BASE`);
   - protected components/parts unchanged (hash), unchanged components preserved byte-identically;
   - changes only inside the allowed component set and allowed region (bbox containment);
   - validated OBJ geometry (finite vertices, non-degenerate faces, component identity preserved);
   - author acceptance semantics (candidate accepted only explicitly or by a policy rule);
   - failed apply never modifies the previous accepted scene (atomic publish);
   - rollback to any previous accepted scene revision;
   - provenance/lineage recorded (candidate → accepted scene revision).
4. CLI surface (`michelangelo edit submit|accept|reject|rollback|show`) and machine-readable
   decision reports; no wire-method changes (CLI + library only, consistent with IT-4/IT-5).

## 2. Out of Scope (hard limits)

- No LLM, network, MCP or external 3D backend (that is Stage D); the "test backend" is a local
  generator in tests only.
- No new Blender ops and no Blender execution; geometry checks are OBJ-level (no re-render score).
- No partial-face/stitching edits of continuous surfaces; locality is component-level (ADR §8.1).
- No SQLite migration and no changes to workspace layout constants; the scene store is a lazily
  created directory tree under the run store.
- No JSONL wire methods/DTO/error/event changes.
- No changes to `michelangelo_input` contract, protocol/worker crates or docs of previous stages
  beyond cross-references.

## 3. Contract (normative)

### 3.1 Scene store

```text
<workspace>/recon/<revision>/run-NNNN/
  scene/
    s0001/scene.json                      # initial accepted scene (from the reconstruction run)
    s0001/components/<name>.obj
    s0002/scene.json                      # created only by an accepted candidate
    s0002/components/<name>.obj
  candidates/
    cand-0001/candidate.json
    cand-0001/decision.json               # written by submit/accept/reject
    cand-0001/geometry/<name>.obj
  current.json                            # {"scene_revision": "s0002", "candidate_id": "cand-0001",
                                          #  "accepted_at": "<RFC 3339 UTC>"} — single authoritative pointer
  candidates/index.json                   # ordered list of candidate ids and statuses
```

- `s0001` is created by `michelangelo edit init --run <run_dir>` from the run's `source/` OBJs
  (component name = OBJ file stem without extension; `stub_*`/`supplement_*`/`blockout` preserved).
  Initialization is idempotent; an existing scene is never overwritten.
- Revisions are immutable: after a revision directory is published (its `scene.json` and all OBJs
  written and fsynced), it is never modified. `current.json` and `candidates/index.json` are the
  only mutable files; both are published by write-to-temp + atomic rename, and a crash between the
  two leaves the previous pointer authoritative (documented recovery: rerun the operation).
- `scene.json`, schema `michelangelo.scene/0`:
  `{schema, run_id, scene_revision, base_scene_revision|null, parent_candidate_id|null,
  created_at, components: [{name, part_id|null, obj, sha256, bbox_min, bbox_max, protected}],
  source: {package_id, package_revision}, notes: [string]}`.
  Every listed OBJ exists, is a regular file inside the revision directory, and its SHA-256 matches.

### 3.2 Edit candidate

`schema michelangelo.edit-candidate/0`:

```json
{
  "schema": "michelangelo.edit-candidate/0",
  "candidate_id": "cand-0001",
  "base": {"package_id": "harbor-lantern-poc", "package_revision": 1,
            "scene_revision": "s0001", "run_id": "run-0001"},
  "task": {
    "task_id": "fix-glass-cavity",
    "allowed_components": ["lid"],
    "allowed_region": {"min": [-0.2, -0.2, -0.1], "max": [0.2, 0.2, 0.1]},
    "protected": ["base", "bracket"],
    "allowed_intersections": [],
    "require_author_acceptance": true,
    "max_changed_vertices": 250000
  },
  "changes": [{"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}],
  "provenance": {"origin": "human", "detail": "manual repair of a missing lid"},
  "budget": {"max_calls": 1, "max_seconds": 120, "max_cost_usd": 0.0}
}
```

- `changes[].action` ∈ `replace` \| `add` \| `delete` (v0: `replace`/`add` only; `delete` rejected
  as unsupported); `component` matches `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`; `obj` is a relative
  `.obj` path inside the candidate directory; each changed component appears at most once.
- `allowed_components`/`protected` name scene components; the sets must be disjoint and
  `allowed_components` non-empty; names not present in the base scene are rejected for `replace`
  (`COMPONENT_UNKNOWN`).
- `allowed_region` is required; `min < max` per axis (package units). A changed component's OBJ
  bbox must lie inside it (`REGION_VIOLATION`).
- `protected` components must stay byte-identical to the base scene revision
  (`PROTECTED_MODIFIED` on any hash change); all other untouched components are preserved
  byte-identically by the store.
- `require_author_acceptance`: when true, `submit` records `proposed`; only an explicit `accept`
  with `--by <name>` applies it. When false, submit may auto-accept only if the workspace policy
  file `<workspace>/.michelangelo/edit_policy.json` (optional) allows it; absent policy = manual.
- `budget` is recorded/validated (finite, non-negative, `max_calls >= 1`) and counted by future
  assistant stages; v0 enforces syntax and rejects a candidate whose declared `max_calls` is 0.

### 3.3 Validation decision report

`decision.json`, schema `michelangelo.edit-decision/0`:
`{schema, candidate_id, decision: "accepted"|"rejected"|"proposed"|"error", base_scene_revision,
resulting_scene_revision|null, checks: [{check, status: "pass"|"fail"|"skipped", detail}],
diagnostics: [{code, severity, message, address?}], decided_at, decided_by|null}`.
Codes: `STALE_BASE`, `COMPONENT_UNKNOWN`, `NOT_ALLOWED`, `PROTECTED_MODIFIED`,
`REGION_VIOLATION`, `GEOMETRY_INVALID`, `OBJ_PARSE`, `IDENTITY_MISMATCH`, `UNSUPPORTED_ACTION`,
`BUDGET_INVALID`, `ACCEPTANCE_REQUIRED`, `POLICY_DENIED`, `IO_ERROR`, `INTERNAL_ERROR`.

### 3.4 CLI

```text
michelangelo edit init   --run <run_dir> [--json]
michelangelo edit submit --run <run_dir> --candidate <dir> [--json]
michelangelo edit accept --run <run_dir> --candidate <id> --by <name> [--json]
michelangelo edit reject --run <run_dir> --candidate <id> --by <name> [--reason <text>] [--json]
michelangelo edit rollback --run <run_dir> --to <scene_revision> --by <name> [--json]
michelangelo edit show   --run <run_dir> [--candidate <id>] [--json]
```

Exit codes: `0` accepted/ok, `1` rejected/validation failure, `2` usage/IO. Applying a candidate
never mutates the base revision or the run's `source/`.

## 4. Iteration Plan (IT-6)

Write scopes:

- `crates/michelangelo_edit/**` (new crate: scene store, OBJ parser/validator, candidate model,
  validator/applier, reports, tests).
- `crates/michelangelo_cli/Cargo.toml`, `crates/michelangelo_cli/src/main.rs`,
  `crates/michelangelo_cli/src/edit_cmd.rs`, `crates/michelangelo_cli/tests/edit_cli.rs`.
- Root `Cargo.toml`/`Cargo.lock`.
- `docs/specs/LOCAL_EDIT.md` (new spec: scene store, candidate schema, checks, report, CLI,
  failure/rollback semantics).
- `examples/edit/` (tiny OBJ fixtures: base scene components, a valid candidate, an
  out-of-region candidate, a protected-tampering candidate) — synthetic, no dataset content.
- `ROADMAP.md` (Phase 4 Stage C note), `TASKS.md`, `docs/ai/AI_CONTEXT.md`, `CHANGELOG.md`,
  this TZ.

Task split:

- **C1 (worker)** — `michelangelo_edit` crate: scene store (init/publish/rollback, atomic
  `current.json`), OBJ parser (vertices/faces; reject non-finite, degenerate, unsupported
  constructs), candidate model + validator with all checks/codes, decision report, tests
  (≥ 40) covering every code, stale base, protected tamper, region violation, atomicity
  (failed publish leaves previous scene untouched), rollback, determinism, and a test that a
  protected component's bytes are untouched after accept.
- **C2 (worker, after C1)** — CLI `edit init|submit|accept|reject|rollback|show` + integration
  tests (`CARGO_BIN_EXE_michelangelo`), docs spec, fixtures; 6 scenarios end-to-end.
- **C3 (Lead)** — reconcile, ladder, end-to-end acceptance on a real IT-5 run (harbor lantern:
  init scene → submit a manual candidate replacing a component with a valid OBJ → accept →
  submit a stale candidate → rejected → rollback), independent review, commit.

## 5. Acceptance Criteria

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` green; ≥ 40 new Rust tests (C1) + CLI integration tests (C2).
- Scenario evidence (recorded in this TZ): init from a real run produces `s0001` with all
  components hashed; valid candidate accepted → `s0002` with exactly the changed component
  replaced and all protected/untouched components byte-identical; candidate for `s0001` submitted
  after acceptance is rejected `STALE_BASE`; candidate modifying a protected component is rejected
  `PROTECTED_MODIFIED`; candidate geometry outside `allowed_region` is rejected
  `REGION_VIOLATION`; a forced publish failure (simulated IO error / invalid OBJ) leaves
  `current.json` pointing at `s0002`; rollback to `s0001` succeeds and is recorded.
- `docs/specs/LOCAL_EDIT.md` matches the implemented schemas/exit codes field-by-field.
- No protocol/worker changes; no SQLite migration; `git diff` only in §4 scopes.
- Commit on `dev`, explicit pathspecs, staged diff inspected, no push.

## 6. Evidence

Iteration executed 2026-11-07 on `dev` (base `291edd1`, IT-5). Roles: C1 worker (child
`44830101`, route `minimax-cn/MiniMax-M3.1-Flash`, preset `worker`) implemented the
`michelangelo_edit` crate, CLI, fixtures and spec; the Lead reviewed, found and fixed a
protection-design flaw, ran the end-to-end acceptance, wrote this evidence and the repo docs.

- Ladder: `cargo fmt --all -- --check` exit 0; `cargo clippy --workspace --all-targets -- -D
  warnings` exit 0; `cargo test --workspace` green — **593 Rust tests** (edit crate 122, CLI 39,
  recon 60, input 105, existing suites unchanged); `cargo build -p michelangelo_cli` exit 0.
- End-to-end acceptance on a **copy of a real Stage B run**
  (`target/techlead/IT-5/bench/harbor_lantern_v01-supplements/.../run-0001`, copy under
  `target/techlead/IT-6/real-run`, original untouched):
  1. `edit init --protect blockout,supplement_*` → `s0001` with 5 components hashed, 4 protected.
  2. `edit submit` a generated candidate replacing `stub_u_flame_shape` (8 vertices inside the
     allowed region) → decision `proposed` (acceptance deferred to the author), exit 0.
  3. `edit accept --by owner` → decision `accepted`, resulting revision `s0002`, exit 0.
  4. stale resubmit (candidate id `cand-0005`, base `s0001`) → rejected `STALE_BASE`, exit 1.
  5. protected tamper (replacing the protected `blockout`) → rejected `PROTECTED_MODIFIED`,
     exit 1.
  6. out-of-region candidate → rejected `REGION_VIOLATION`, exit 1.
  7. `edit rollback --to s0001` → exit 0; `edit show` reports the current revision.
  Script: `target/techlead/IT-6/accept.py` (gitignored). `PROTECTED_MODIFIED` and
  `REGION_VIOLATION` decision reports are stored under the run's `candidates/` store.
- A second demo on the committed `examples/edit` fixtures (from the worker report): init → s0001,
  submit valid → proposed, accept → s0002, out-of-region rejected, protected rejected, rollback
  to s0001 — covered by `crates/michelangelo_cli/tests/edit_cli.rs`.
- Protection-design flaw found during acceptance and fixed in this iteration: the candidate
  document's own `task.protected` list was the only protection source, so a candidate could
  simply declare the component it wanted to change as allowed (the first acceptance run accepted
  a "protected tamper" as `proposed`). Protection is now established once at
  `edit init --protect <names>` and recorded as `protected: true` in the immutable `s0001`;
  the validator checks the union of the scene flags and the task list, so a candidate cannot
  redefine the author's protection. Added `init_scene_with_protection` + tests
  (`the_initialization_can_protect_components`,
  `the_initialization_rejects_an_unknown_protected_component`,
  `a_run_level_protected_component_cannot_be_changed`) and documented `--protect` in
  `docs/specs/LOCAL_EDIT.md`.
- Deviations recorded: (1) C1/C2 merged into one worker task (crate+CLI+fixtures+spec in one
  pass); (2) `current.json`/`candidates/index.json` keep the TZ layout plus an extra
  `rollbacks[]` list so a rollback is recorded; (3) `ACCEPTANCE_REQUIRED`/`POLICY_DENIED` are
  non-blocking diagnostics on a `proposed` decision; (4) a document-level parse failure is a
  `error` decision with exit 2, while an invalid budget is a `rejected` decision with exit 1;
  (5) locality is component-level bbox containment only (v0 limit, documented).
- Reviewer verdict: **GREEN** (see review log); no blocking finding.

### Review log

| attempt | reviewer | route | verdict | findings |
|---|---|---|---|---|
| 1 | child `f2c07f66` (read-only) | `xiaomi/mimo-v2.6-pro`, preset `reviewer` | GREEN | LOW: rollback accepts any existing revision (spec says "existing"; no ordering enforced — accepted as a documented nuance); command-level checks by Lead |
