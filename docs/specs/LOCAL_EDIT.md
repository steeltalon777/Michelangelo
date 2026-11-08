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
    s0001/scene.json                      # the initial accepted scene (+ verification bundle)
    s0001/components/<name>.obj           # one OBJ per component
    s0002/scene.json                      # created only by an accepted candidate
    s0002/components/<name>.obj
  verification/
    snapshots/<candidate_id>/manifest.json  # full post-change result at submit (immutable)
    snapshots/<candidate_id>/geometry/*.obj # full component set (base + change)
    unknowns/revisions/sNNNN.json          # unknown state referenced by the revision (immutable)
    run.lock                               # OS advisory lock of the store
  candidates/
    cand-0001/candidate.json              # the submitted candidate
    cand-0001/geometry/<name>.obj         # the submitted bytes (snapshot source only)
    cand-0001/decisions/NNNN-<kind>.json  # append-only decision history
    cand-0001/decision.json               # latest-decision mirror
    index.json                            # the ordered index of the candidates
  current.json                            # the authoritative pointer (one bundle)
```

* `s0001` is created by `michelangelo edit init --run <run_dir> [--protect <name,...>]`
  from the OBJs of the `source/` directory of the run. The component name is the stem of the
  OBJ file (`blockout`, `supplement_*`, `stub_*` are preserved), `part_id` is `null`;
  components listed by `--protect` are marked `protected: true` (an author decision taken at
  initialization time, not something a candidate may redefine) and unknown names are rejected.
  Because revisions are immutable, protection cannot be retrofitted into an existing `s0001`.
  Init also writes the first unknown-state document from the immutable `report.json` (every
  unknown `open`, stubs flagged; no unknown is closed by a part-id match in this stage).
* `submit` copies the candidate into `candidates/<id>` **and** creates an immutable full-result
  snapshot under `verification/snapshots/<id>` with per-file SHA-256, the base geometry digest,
  candidate/task/package/policy digests and the aggregate digest (§`VERIFICATION_STATE.md`).
  Publication always uses the snapshot; the candidate geometry folder is never the source of a
  revision.
* `current.json`, `candidates/index.json` and the latest mirror are the only mutable files;
  every decision (including refusals) is appended to `candidates/<id>/decisions/`.
* The initialization is idempotent: an existing scene is never overwritten and
  an existing `current.json` is never reset.
* Every store mutation takes the exclusive OS advisory lock `verification/run.lock`
  (`std::fs::File::try_lock`): a busy lock is `STORE_BUSY` (exit 2), the boundary never waits,
  and the OS releases the lock when the owning process dies (no TTL stealing). Accept
  re-reads `current.json` under the lock and requires the candidate base to still be current.
* A refused acceptance (missing snapshot, evidence mismatch, stale base) appends an
  `accept-refused` record with its diagnostic code; the latest mirror is not rewritten.

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
  "verification": {
    "geometry_digest": "<aggregate sha256 of the component set>",
    "bindings": [],
    "unknowns": {"revision": "s0002",
                 "path": "verification/unknowns/revisions/s0002.json",
                 "sha256": "<64 hex characters>"},
    "evaluations": [], "confirmations": [], "resolutions": [],
    "policies": {"package_sha256": null, "workspace_policy_sha256": null},
    "tool_versions": {"michelangelo": "0.1.0", "solver": "voxel-carve/1"},
    "created_at": "2026-01-02T03:04:05Z"
  },
  "notes": []
}
```

Every listed OBJ exists inside the revision directory and its SHA-256 matches
the bytes of the file. The document never carries an absolute path: the `obj`
field is relative to the revision directory. The `verification` block is the
bundle of hash-pinned evidence; every referenced document must exist and match
its digest before the revision can be published. `bindings` is an empty list
until IT-8C creates the first binding version; `evaluations`, `confirmations`
and `resolutions` are filled by IT-8B/IT-8C.

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

* At `submit` the full post-change result is snapshotted under
  `verification/snapshots/<candidate_id>` (temp dir + rename): unchanged components are
  copied byte-identically from the base revision, changed/added components come from the
  candidate; the manifest pins every file, the base digest and the aggregate digest.
* A revision is written into `scene/<revision>.tmp` and renamed into place.
  A reader never observes a half-written revision and an existing revision is
  never overwritten. Publication copies the geometry **from the snapshot**, never from
  `candidates/<id>/geometry`.
* `current.json` and `candidates/index.json` are published with
  write-to-temp + rename. All mutations run under the exclusive OS lock
  `verification/run.lock`; a busy lock is `STORE_BUSY` (exit 2) and the boundary never waits.
* The order of an acceptance is: validate → lock → re-read the pointer and require the
  candidate base to still be current → re-verify the snapshot → publish the revision →
  publish the pointer → publish the index → write the decision history. A failure before the
  pointer is published leaves the previous `current.json` authoritative; the operation can
  be repeated and reuses the already published revision of the same candidate.
* A component that a candidate does not change is copied into the new revision
  **byte-identically**; only the changed components carry new bytes. The base
  revision and the `source/` directory of the run are never modified.
* A repeated acceptance of an accepted candidate is idempotent: it returns the historical
  accepted result (same `resulting_scene_revision`, exit 0), appends an `accept-idempotent`
  history record and never rewrites the accepted decision or the candidate status. After a
  rollback the same call still returns the historical result and does **not** switch the
  scene back.
* Rejecting an accepted candidate is refused (`CANDIDATE_ALREADY_ACCEPTED`, exit 1) without
  rewriting the history; a repeated rejection is idempotent.
* A refused acceptance (missing snapshot, evidence mismatch, stale base) appends an
  `accept-refused` record with the code; the latest mirror is left untouched.

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
* `auto_accept` — declared for future use only. Since IT-8B an accepted candidate needs a
  registered evaluation, and an evaluation can only run after the snapshot exists, `submit`
  always keeps the candidate `proposed` (`EVALUATION_REQUIRED` diagnostic); the policy flag
  cannot bypass the evaluation gate.
* `allowed_origins` — the origins the policy accepts (all of them when the
  field is absent).
* A policy file that exists but cannot be used is an error (exit code 2); the
  boundary never guesses the intent of a broken policy.

## The CLI

```text
michelangelo edit init     --run <run_dir> [--protect <name,...>] [--json]
michelangelo edit submit   --run <run_dir> --candidate <dir> [--json]
michelangelo edit accept   --run <run_dir> --candidate <id> --by <name> [--evaluation <report>] [--resolve <unknown>] [--json]
michelangelo edit reject   --run <run_dir> --candidate <id> --by <name> [--reason <text>] [--json]
michelangelo edit rollback --run <run_dir> --to <scene_revision> --by <name> [--json]
michelangelo edit show     --run <run_dir> [--candidate <id>] [--json]

michelangelo unknown sync    --run <run_dir> [--json]
michelangelo unknown list    --run <run_dir> [--json]
michelangelo unknown show    --run <run_dir> [--revision <sNNNN>] [--json]
michelangelo unknown bind    --run <run_dir> --unknown <id> --components <a,b> --region <min=max> --by <name> [--claim <text>] [--json]
michelangelo unknown confirm --run <run_dir> --unknown <id> --by <name> --evaluation <eval-id> [--evidence <file>] [--claim <text>] [--json]
michelangelo unknown propose --run <run_dir> --unknown <id> [--candidate <id>] [--evaluation <eval-id>] [--json]
michelangelo unknown resolve --run <run_dir> --unknown <id> [--candidate <id>] [--evaluation <eval-id>] --by <name> [--json]
```

| Exit code | Meaning |
|---|---|
| `0` | the operation succeeded (accepted/proposed/rolled back/resolved, or an idempotent repeat) |
| `1` | the candidate was rejected; the evaluation gate refused it (`EVALUATION_MISSING`, `EVALUATION_UNREGISTERED`, `EVALUATION_STALE`, `EVALUATION_ALREADY_REGISTERED`, `INPUT_CHANGED`, `VISIBLE_FAILED`, `AUTHOR_CONDITION_FAILED`, `REGRESSION`, `NO_GROUND_TRUTH`); the unknown state machine refused it (`BINDING_MISSING`, `BINDING_MISMATCH`, `CONFIRMATION_MISSING`, `CONFIRMATION_STALE`, `AGENT_CONFIRMATION_REFUSED`); or the store refused the operation (`STALE_BASE`, `EVIDENCE_MISMATCH`, `SNAPSHOT_MISSING`, `SNAPSHOT_MISMATCH`, `CANDIDATE_ALREADY_ACCEPTED`) |
| `2` | usage error, IO error, a candidate that cannot be interpreted, or a busy store (`STORE_BUSY`) |

* `submit` copies the candidate into the store (`candidates/<id>/`), validates
  it, creates the full-result snapshot and keeps the decision `proposed`: an
  acceptance always needs a registered evaluation of that snapshot.
* `accept` re-validates the candidate against the **current** revision, then runs
  the evaluation gate on the registered `--evaluation` report (see
  `docs/specs/EVALUATION.md`): target aggregate, base revision, pinned inputs, visible
  conformance, required author geometric conditions and per-view regression. Before the
  evaluation can pass, the snapshot must be the one pinned by the immutable submit decision
  (manifest digest, candidate/task/package/base digests) and the full diff against the base is
  re-checked: the manifest file set equals base ∪ declared changes, unchanged components are
  byte-identical to the base, changed components equal the declared candidate bytes and
  protected components are unchanged (`SNAPSHOT_MISMATCH` otherwise; a coordinated tamper of
  geometry **and** manifest is refused). Immediately before the pointer switch every document
  the new bundle references is loaded by path and its SHA-256 is re-checked; accept and reject
  take the store lock before reading the transition state, so a stale reject cannot overwrite a
  concurrent acceptance. A candidate that became stale is refused with `STALE_BASE`; a
  missing/invalid evaluation is refused with the gate code and an `accept-refused` history
  record. `--resolve <unknown>` additionally closes that unknown in the new bundle (see
  `docs/specs/UNKNOWNS.md`); without it the geometry is accepted and every unknown stays open
  (partial acceptance).
* `unknown` manages the explicit bindings, author confirmations and resolutions of the run.
  Closure is never automatic and never based on a `part_id` match; it is materialised only by a
  revision whose bundle references the resolution. A later change of the bound geometry,
  binding or claim invalidates the closure (`open` + `evidence_stale`), and a rollback restores
  the target bundle's state.
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
| the candidate has no snapshot (legacy) | `SNAPSHOT_MISSING`, no revision, `accept-refused` history, exit code 1 |
| the candidate geometry or the snapshot was tampered after submit | `EVIDENCE_MISMATCH`, no revision, `accept-refused` history, exit code 1 |
| the candidate base is not current at publication time | `STALE_BASE`, no revision, `accept-refused` history, exit code 1 |
| the store lock is held by another process | `STORE_BUSY`, no change, exit code 2 |
| the candidate was already accepted | repeated accept returns the historical result (exit 0); rejecting it is `CANDIDATE_ALREADY_ACCEPTED` (exit 1) |
| no registered evaluation is supplied for accept | `EVALUATION_MISSING`, no revision, `accept-refused` history, exit code 1 |
| the evaluation is unregistered, its digest is stale or the inputs changed | `EVALUATION_UNREGISTERED` / `EVALUATION_STALE` / `INPUT_CHANGED`, no revision, exit code 1 |
| the evaluation's visible conformance fails, a required author condition fails or a per-view regression exceeds 0.01 IoU | `VISIBLE_FAILED` / `AUTHOR_CONDITION_FAILED` / `REGRESSION`, no revision, exit code 1 |
| a required claim has no ground truth | `NO_GROUND_TRUTH`, no revision, exit code 1 |
| a crash happens between publishing the revision and the pointer | the previous bundle stays authoritative; a retry reuses the published revision |

The rollback is a pointer move, never an inversion: the revisions keep their
content and their lineage (including the unknown-state document each revision
references), and the rollback itself is recorded in the index.

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
# evaluation requires a run with the pinned package copy (a real IT-5 run):
michelangelo evaluate    --run <run> --candidate cand-0001 [--evaluation <eval-package>]
michelangelo edit accept --run <run> --candidate cand-0001 --by alice \
                         --evaluation <run>/verification/evaluations/ev-0001/report.json
michelangelo edit show   --run examples/edit/run-0001 --json
```

The `examples/edit` run has no package copy, so its unit tests register a synthetic
evaluation through `michelangelo_evidence::testing`; the real evaluation path is exercised on
a run produced by `michelangelo reconstruct` (see `docs/specs/EVALUATION.md`).
