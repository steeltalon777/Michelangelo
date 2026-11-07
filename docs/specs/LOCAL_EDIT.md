# Local Edit Boundary (v0)

Normative contract of the Michelangelo **local edit boundary** (Stage C): a
reconstruction run can be changed by an explicit, versioned *edit candidate*
that is validated by a deterministic validator and applied by an atomic
publisher. The previous accepted scene always survives a failed change.

Status: implemented in Stage C (crate `michelangelo_edit`, CLI
`michelangelo edit`). Schemas: `michelangelo.scene/0`,
`michelangelo.edit-candidate/0`, `michelangelo.edit-decision/0`,
`michelangelo.edit-candidate-index/0`, `michelangelo.edit-rollback/0`,
`michelangelo.edit-scene-view/0`, `michelangelo.edit-policy/0`.

## Purpose and ADR reference

ADR 0004 (§8–§9 and §13.3) requires a local-edit boundary before any
assistant: a candidate change is an explicit artifact, a candidate for a stale
base revision is rejected, protected geometry cannot be modified and a rollback
selects a previous accepted revision instead of inverting agent actions.

The boundary is Core-side (Rust library + CLI). It performs no network access,
never mutates the input package, never mutates the `source/` directory of a
run, never mutates a published scene revision and adds no Blender operation.
The geometry checks are OBJ-level: there is no re-render score in v0.

## Scene store

The store lives next to the artifacts of a reconstruction run:

```text
<workspace>/recon/<revision>/run-NNNN/
  scene/
    s0001/scene.json                      # the initial accepted scene
    s0001/components/<name>.obj           # one OBJ per component
    s0002/scene.json                      # created only by an accepted candidate
    s0002/components/<name>.obj
  candidates/
    cand-0001/candidate.json              # the submitted candidate
    cand-0001/decision.json               # the decision of the candidate
    cand-0001/geometry/<name>.obj         # the geometry of the candidate
    index.json                            # the ordered index of the candidates
  current.json                            # the authoritative pointer
```

* `s0001` is created by `michelangelo edit init --run <run_dir> [--protect <name,...>]`
  from the OBJs of the `source/` directory of the run. The component name is the stem of the
  OBJ file (`blockout`, `supplement_*`, `stub_*` are preserved), `part_id` is `null`;
  components listed by `--protect` are marked `protected: true` (an author decision taken at
  initialization time, not something a candidate may redefine) and unknown names are rejected.
  Because revisions are immutable, protection cannot be retrofitted into an existing `s0001`.
* The initialization is idempotent: an existing scene is never overwritten and
  an existing `current.json` is never reset.
* `current.json` and `candidates/index.json` are the only mutable files.

### The scene document

`scene.json`, schema `michelangelo.scene/0`:

```json
{
  "schema": "michelangelo.scene/0",
  "run_id": "run-0001",
  "scene_revision": "s0002",
  "base_scene_revision": "s0001",
  "parent_candidate_id": "cand-0001",
  "created_at": "2026-01-02T03:04:05Z",
  "components": [
    {
      "name": "lid",
      "part_id": null,
      "obj": "components/lid.obj",
      "sha256": "<64 hex characters>",
      "bbox_min": [-0.04, 0.05, -0.04],
      "bbox_max": [0.04, 0.07, 0.04],
      "protected": false
    }
  ],
  "source": {"package_id": "demo-lantern", "package_revision": 1},
  "notes": []
}
```

Every listed OBJ exists inside the revision directory and its SHA-256 matches
the bytes of the file. The document never carries an absolute path: the `obj`
field is relative to the revision directory.

### The pointer

`current.json`:

```json
{"scene_revision": "s0002", "candidate_id": "cand-0001", "accepted_at": "2026-01-02T03:04:05Z"}
```

`candidate_id` is `null` for the first revision and after a rollback.

### The candidate index

`candidates/index.json`, schema `michelangelo.edit-candidate-index/0`:

```json
{
  "schema": "michelangelo.edit-candidate-index/0",
  "candidates": [
    {
      "candidate_id": "cand-0001",
      "status": "accepted",
      "scene_revision": "s0002",
      "submitted_at": "2026-01-02T03:04:05Z",
      "decided_at": "2026-01-03T04:05:06Z",
      "decided_by": "alice",
      "reason": null
    }
  ],
  "rollbacks": [
    {
      "from_scene_revision": "s0002",
      "to_scene_revision": "s0001",
      "by": "alice",
      "at": "2026-01-04T05:06:07Z"
    }
  ]
}
```

The candidates are ordered by identifier, the rollbacks by time. The status of a
candidate is `proposed`, `accepted` or `rejected`.

## Immutability and atomicity

* A revision is written into `scene/<revision>.tmp` and renamed into place.
  A reader never observes a half-written revision and an existing revision is
  never overwritten.
* `current.json` and `candidates/index.json` are published with
  write-to-temp + rename.
* The order of an acceptance is: validate → publish the revision → publish the
  pointer → publish the index → write the decision report. A failure before the
  pointer is published leaves the previous `current.json` authoritative; the
  operation can simply be repeated (documented recovery).
* A component that a candidate does not change is copied into the new revision
  **byte-identically**; only the changed components carry new bytes. The base
  revision and the `source/` directory of the run are never modified.

## The edit candidate

Schema `michelangelo.edit-candidate/0`:

```json
{
  "schema": "michelangelo.edit-candidate/0",
  "candidate_id": "cand-0001",
  "base": {
    "package_id": "demo-lantern",
    "package_revision": 1,
    "scene_revision": "s0001",
    "run_id": "run-0001"
  },
  "task": {
    "task_id": "fix-lid",
    "allowed_components": ["lid"],
    "allowed_region": {"min": [-0.2, -0.2, -0.2], "max": [0.2, 0.2, 0.2]},
    "protected": ["base"],
    "allowed_intersections": [],
    "require_author_acceptance": true,
    "max_changed_vertices": 100000
  },
  "changes": [
    {"component": "lid", "action": "replace", "obj": "geometry/lid.obj"}
  ],
  "provenance": {"origin": "human", "detail": "a manual repair of the lid"},
  "budget": {"max_calls": 1, "max_seconds": 120, "max_cost_usd": 0.0}
}
```

Rules:

* The document is strict: an unknown field, a value of the wrong type, an
  unknown schema, an unknown origin, an unknown action, an invalid component
  name, an invalid region or a changed component that appears twice make the
  document *not a candidate*. Such a document is never applied; the decision is
  `error` with the diagnostic `INTERNAL_ERROR`.
* `candidate_id` and `component` match `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`.
* `base.scene_revision` is a revision name (`sNNNN`); `base.run_id` is the
  name of the run directory.
* `task.allowed_components` must not be empty and must be disjoint from
  `task.protected`.
* `task.allowed_region` is required; the minimum is below the maximum on
  every axis (package units, metres).
* `changes[].action` is `replace`, `add` or `delete`. In v0 `delete` is
  refused with `UNSUPPORTED_ACTION`; `replace` and `add` are supported.
* `changes[].obj` is a relative `.obj` path inside the candidate directory;
  it may not traverse upwards and may not be absolute.
* `provenance.origin` is `human`, `ai_author` or `test_backend`.
* `budget` is finite and non-negative and `max_calls` is at least 1.

## The checks and the diagnostic codes

The validator always runs the same checks in the same order, so two runs over
the same inputs produce byte-identical reports.

| Check | What it verifies | Codes |
|---|---|---|
| `candidate_schema` | the candidate follows `michelangelo.edit-candidate/0` | `INTERNAL_ERROR`, `BUDGET_INVALID` |
| `identity` | the package, the package revision and the run of the candidate base match the scene | `IDENTITY_MISMATCH` |
| `base_revision` | the candidate was built on the revision the store points to | `STALE_BASE` |
| `task_scope` | the allowed set is not empty and is disjoint from the protected set | `NOT_ALLOWED` |
| `components` | the action is supported, the component is allowed and exists (or does not exist for `add`) | `UNSUPPORTED_ACTION`, `NOT_ALLOWED`, `COMPONENT_UNKNOWN` |
| `protected` | no protected component is changed — protection is the union of the candidate task list and the scene-level `protected` flags established at `init` | `PROTECTED_MODIFIED` |
| `geometry` | the OBJ of every change is valid | `OBJ_PARSE`, `GEOMETRY_INVALID` |
| `region` | the bounding box of every change lies inside the allowed region | `REGION_VIOLATION` |
| `budget` | the change touches at most `max_changed_vertices` vertices | `BUDGET_INVALID` |
| `acceptance` | the acceptance of the candidate | `ACCEPTANCE_REQUIRED` |
| `policy` | the policy of the workspace | `POLICY_DENIED` |

A check with the status `fail` refuses the candidate (the decision is
`rejected`). The checks `acceptance` and `policy` never refuse a candidate:
they carry the status `skipped` and describe the state of the acceptance.

### The OBJ rules

The parser is strict and works on the OBJ level:

* `v x y z` — a vertex; an optional fourth coordinate is accepted and ignored
  (it must be finite and not zero),
* `f a b c ...` — a face with at least three vertices; a reference may carry
  texture and normal indices (`v`, `v/vt`, `v//vn`, `v/vt/vn`),
* `vn`, `vt`, `g`, `o`, `s`, `usemtl`, `mtllib`, comments and blank
  lines are ignored,
* every other keyword is refused (`OBJ_PARSE`),
* a non-finite coordinate, a vertex index that does not exist, a face that
  repeats a vertex and a face with a zero area are refused
  (`GEOMETRY_INVALID`).

### The diagnostics

```text
STALE_BASE          the candidate was built on an older revision
COMPONENT_UNKNOWN   the candidate changes a component the scene does not have
NOT_ALLOWED         the change is outside the allowed component set
PROTECTED_MODIFIED  the change touches a protected component
REGION_VIOLATION     the geometry leaves the allowed region
GEOMETRY_INVALID    the OBJ does not describe a usable geometry
OBJ_PARSE            the OBJ is not a well formed document
IDENTITY_MISMATCH    the base of the candidate belongs to another package or run
UNSUPPORTED_ACTION   the action of the change is not supported in v0
BUDGET_INVALID       the declared budget is invalid or exceeded
ACCEPTANCE_REQUIRED  the candidate waits for an explicit author acceptance
POLICY_DENIED        the edit policy refuses an automatic acceptance
IO_ERROR             a file or directory could not be read, written or renamed
INTERNAL_ERROR       the candidate does not follow its schema
```

The severities are `error` (the candidate is refused), `warning` (the author
is informed) and `info` (the state of the candidate).

## The decision report

`decision.json`, schema `michelangelo.edit-decision/0`:

```json
{
  "schema": "michelangelo.edit-decision/0",
  "candidate_id": "cand-0001",
  "decision": "proposed",
  "base_scene_revision": "s0001",
  "resulting_scene_revision": null,
  "checks": [
    {"check": "candidate_schema", "status": "pass", "detail": "the candidate follows 'michelangelo.edit-candidate/0'"}
  ],
  "diagnostics": [
    {
      "code": "ACCEPTANCE_REQUIRED",
      "severity": "info",
      "message": "the candidate waits for an explicit author acceptance",
      "address": null
    }
  ],
  "decided_at": "2026-01-02T03:04:05Z",
  "decided_by": null
}
```

* `decision` is `accepted`, `rejected`, `proposed` or `error`.
* `base_scene_revision` is the scene revision the decision was made against
  (the revision the store points to). It is `null` when the candidate could
  not be interpreted at all.
* `resulting_scene_revision` is the revision the decision produced (`null`
  when it produced none).
* `decided_by` is the author of an explicit acceptance or rejection (`null`
  for a submission and for a rollback).
* The report carries no absolute path.

An author rejection carries the reason of the rejection in the check
`author_decision` (the status `fail`).

## The policy of the workspace

The optional file `<workspace>/.michelangelo/edit_policy.json` describes when
a candidate may be applied without an explicit author acceptance:

```json
{
  "schema": "michelangelo.edit-policy/0",
  "auto_accept": true,
  "allowed_origins": ["human", "ai_author", "test_backend"]
}
```

* The file is optional. When it is absent, only an explicit author acceptance
  applies a candidate (the manual path of ADR 0004 §13.3).
* `auto_accept` — whether `submit` may apply a candidate that does not
  require an author acceptance.
* `allowed_origins` — the origins the policy accepts (all of them when the
  field is absent).
* A policy file that exists but cannot be used is an error (exit code 2); the
  boundary never guesses the intent of a broken policy.

## The CLI

```text
michelangelo edit init     --run <run_dir> [--protect <name,...>] [--json]
michelangelo edit submit   --run <run_dir> --candidate <dir> [--json]
michelangelo edit accept   --run <run_dir> --candidate <id> --by <name> [--json]
michelangelo edit reject   --run <run_dir> --candidate <id> --by <name> [--reason <text>] [--json]
michelangelo edit rollback --run <run_dir> --to <scene_revision> --by <name> [--json]
michelangelo edit show     --run <run_dir> [--candidate <id>] [--json]
```

| Exit code | Meaning |
|---|---|
| `0` | the operation succeeded (the candidate was accepted, proposed or rolled back) |
| `1` | the candidate was rejected by the validator |
| `2` | usage error, IO error or a candidate that cannot be interpreted |

* `submit` copies the candidate into the store (`candidates/<id>/`), validates
  it and decides: the decision is `proposed` (an explicit author acceptance is
  required) or `accepted` (the policy allowed an automatic acceptance).
* `accept` re-validates the candidate against the **current** revision and
  publishes a new revision when the candidate is still valid. A candidate that
  became stale in the meantime is rejected with `STALE_BASE`.
* `reject` records the refusal of an author; the scene is not modified.
* `rollback` points the store back to an existing revision; the revision
  directories themselves are never modified.
* `show` prints the state of the store (the pointer, the revisions, the
  candidates and the rollbacks) or one candidate.

The machine-readable output of `--json`:

| Command | Document |
|---|---|
| `init` | the scene (`michelangelo.scene/0`) |
| `submit`, `accept`, `reject` | the decision report (`michelangelo.edit-decision/0`) |
| `rollback` | the rollback report (`michelangelo.edit-rollback/0`) |
| `show` | the view of the store (`michelangelo.edit-scene-view/0`) |

## Failure and rollback semantics

| Situation | Result |
|---|---|
| the candidate is invalid | decision `rejected`, the scene is untouched, exit code 1 |
| the declared budget of the candidate is invalid | decision `rejected` with `BUDGET_INVALID`, exit code 1 |
| the candidate does not follow its schema | decision `error` with `INTERNAL_ERROR`, exit code 2 |
| the policy file is broken | error, exit code 2 |
| the publication fails (IO error) | the previous `current.json` stays authoritative, exit code 2 |
| the candidate is accepted | a new revision is published and the pointer moves, exit code 0 |
| the rollback target does not exist | error, exit code 2 |

The rollback is a pointer move, never an inversion: the revisions keep their
content and their lineage, and the rollback itself is recorded in the index.

## The limits of v0

* **Locality is component-level** (ADR 0004 §8.1): a change replaces or adds
  whole components. There is no partial-face edit and no stitching of
  continuous surfaces.
* **No deletion**: the action `delete` is refused with `UNSUPPORTED_ACTION`.
* **No re-render score**: the quality of a change is not measured in v0; the
  checks are OBJ-level (valid geometry, allowed region, allowed components).
* **No assistant**: the provenance of a candidate is declared by the candidate
  itself; no LLM, no network and no external 3D backend is involved.
* **Component-level budget**: the budget is recorded and validated; the
  counting of the calls of a future assistant stage is not part of v0.
* **One store per run**: the scene store is a lazily created directory tree
  inside the run directory; the layout constants of the workspace do not
  change and there is no database migration.

## The fixtures

`examples/edit/` carries the tiny, synthetic fixtures of the boundary: a
0.1 m box, a lid, a valid candidate, a candidate that leaves the allowed
region, a candidate that tampers with a protected component and a run
directory shaped like an IT-5 run. The run carries no dataset content.

```text
michelangelo edit init   --run examples/edit/run-0001
michelangelo edit submit --run examples/edit/run-0001 --candidate examples/edit/candidate-valid
michelangelo edit accept --run examples/edit/run-0001 --candidate cand-0001 --by alice
michelangelo edit show   --run examples/edit/run-0001 --json
```
