# The evaluation of a candidate (IT-8B)

Status: implemented (crate `michelangelo_evidence`, crate `michelangelo_eval`, the accept
gate of `michelangelo_edit`, the CLI `michelangelo evaluate`). The normative contract of the
state model is `docs/specs/VERIFICATION_STATE.md` (Stage 0) and `docs/tz/TZ_IT8_ACCEPTANCE_INTEGRITY.md`
§6 (IT-8B); this document describes the implementation.

The evaluation measures the **actual** geometry of a candidate (its snapshot) or of a published
scene revision against the pinned masks of the run. The report is immutable evidence: the Core
registry holds its digest, the report never self-certifies.

## The layout

```text
<run>/verification/evaluations/index.json       michelangelo.evaluation-registry/0
<run>/verification/evaluations/<eval-id>/report.json   michelangelo.evaluation-report/0
<run>/verification/evaluations/<eval-id>/work/   the staged geometry, the renders, the jobs
```

The registry is append-only: an entry is never overwritten. The entries are ordered by the
identifier of the evaluation, so the file is deterministic.

## The report

`michelangelo.evaluation-report/0`:

```json
{
  "schema": "michelangelo.evaluation-report/0",
  "evaluation_id": "ev-0001",
  "created_at": "2026-01-03T04:05:06Z",
  "run_id": "run-0001",
  "target": {
    "kind": "candidate",
    "id": "cand-0001",
    "aggregate_sha256": "<aggregate digest of the component set>",
    "base_scene_revision": "s0001"
  },
  "inputs": {
    "package_sha256": "<sha256 of package/package.json>",
    "views": [
      {
        "view_id": "front",
        "camera_digest": "<sha256 of the canonical camera JSON>",
        "mask_sha256": "<sha256 of the mask>",
        "resolution": [8, 8],
        "held_out": false
      }
    ],
    "evaluation_package_sha256": null
  },
  "thresholds": {
    "source_mean_iou": 0.9,
    "source_min_iou": 0.85,
    "evaluation_mean_iou": 0.8,
    "evaluation_min_iou": 0.7,
    "bbox_relative_error_max": 0.05,
    "per_view_regression_max": 0.01
  },
  "results": {
    "projection_conformance": "pass",
    "author_geometric_conditions": "pass",
    "independent_ground_truth": "unavailable",
    "author_hidden_decision": "pending"
  },
  "visible": {
    "views": [
      {
        "view_id": "front",
        "iou": 0.94,
        "bbox_error_px": 1.0,
        "base_iou": 0.93,
        "regression": -0.01
      }
    ],
    "mean_iou": 0.94,
    "min_iou": 0.94,
    "base_mean_iou": 0.93,
    "verdict": "pass",
    "held_out": null
  },
  "hidden": {
    "ground_truth": "absent",
    "claims": [
      {"id": "u_interior", "kind": "unknown", "status": "unverified",
       "evidence_refs": [], "requires_ground_truth": false}
    ],
    "verdict": "pending"
  },
  "checks": [
    {"id": "height", "kind": "author_geometric_condition", "required": true,
     "status": "pass", "detail": "the measured value 0.250000 m is inside the tolerance"}
  ],
  "acceptance_basis": ["projection_conformance=pass", "author_geometric_conditions=pass"],
  "tool_versions": {"michelangelo": "0.1.0", "solver": "voxel-carve/1"},
  "integrity": {
    "schema_digest": null,
    "note": "the report never self-certifies: the Core registry holds the digest of this file"
  }
}
```

### The four independent results

The results are never collapsed into one boolean (contract §6):

| Result | Values | Meaning |
|---|---|---|
| `projection_conformance` | `pass` / `fail` / `unavailable` | the silhouettes of the target against the pinned masks (and the held-out masks when there are any) |
| `author_geometric_conditions` | `pass` / `fail` / `unavailable` | the measurable conditions the author declared in the package |
| `independent_ground_truth` | `pass` / `fail` / `unavailable` | the comparison against an independent ground truth (never implemented in v0) |
| `author_hidden_decision` | `accepted` / `rejected` / `pending` | the decision of the author about a hidden shape (a decision, not a measurement) |

Missing data yields `unavailable` with a diagnostic; it is never a pass.

### The extensions of the schema

The schema of the contract (TZ §6) is implemented with three documented extensions that the
accept gate needs:

1. `target.base_scene_revision` — the revision the measured geometry was built on. The gate
   compares it with the base revision of the candidate (the contract §7 requires the base to
   match).
2. `visible.held_out` — the measured held-out views (the contract lists the held-out views as
   part of the projection conformance but does not give them a place in the document).
3. `checks[]` — the per-check detail (the kind, the requiredness and the status). The contract
   (§6, TZ §6) requires the gate to distinguish a **required** condition from an optional one.

## The digests

All the digests are SHA-256 over canonical bytes and are computed by the Core (contract §3):

* **file** — the SHA-256 of the bytes of the file.
* **geometry aggregate** — the SHA-256 over the lines `"<component>\u{0}<sha256>\n"`, sorted by
  the name of the component.
* **camera digest** — the SHA-256 of the canonical JSON of `{"view_id","location_m","look_at_m",
  "up_world","ortho_scale_m","shift_x","shift_y","clip_start_m","clip_end_m","width_px",
  "height_px"}`.
* **inputs digest** — the SHA-256 of the canonical JSON of `{"package_sha256","views":[{"view_id",
  "camera_digest","mask_sha256","resolution","held_out"}],"evaluation_package_sha256"}`, sorted
  by the identifier of the view.
* **evaluation digest** — the SHA-256 of the bytes of `report.json`, computed by the Core when
  the file is registered. The accept gate re-computes it.

## The pins of an evaluation

* The **package copy** of the run (`<run>/package/package.json`) is pinned by its digest. The
  masks of the views are pinned by their digests, the cameras by their camera digest and the
  resolution by the size of the image.
* The **held-out set** is optional (`--evaluation <pkg>`). A held-out view looking in the **same**
  direction as a source view (`dot(dir_i, dir_j) > 0.9`) adds no constraint, so the held-out
  result is `unavailable` with a diagnostic. Opposite orthographic views (`dot ≈ -1`) are allowed:
  they see the mirrored silhouette and are the held-out controls of the proof datasets.
* The **target** is the snapshot of the candidate (the full post-change component set) or the
  components of a revision. The **base** is the revision the target was built on. Both are
  rendered with identical inputs, cameras and settings; the per-view regression compares them.

## The accept gate

`michelangelo edit accept --evaluation <report>` runs the gate under the lock of the store,
after the evidence checks of IT-8A and before the publication:

1. the evaluation is registered in the registry of the run (`EVALUATION_UNREGISTERED`),
2. the re-computed digest of the report equals the registered digest and the digest that the
   decision of the candidate recorded (`EVALUATION_STALE`),
3. the target of the report is the geometry that is accepted: the kind, the identifier, the
   aggregate digest, the base revision and the run (`EVALUATION_STALE`),
4. the pinned inputs are the inputs of the run right now (`INPUT_CHANGED`),
5. the projection conformance passes (`VISIBLE_FAILED`),
6. every **required** author geometric condition passes (`AUTHOR_CONDITION_FAILED`),
7. no source view drops more than 0.01 IoU against the base revision (`REGRESSION`),
8. no claim that needs an independent ground truth is left unverified (`NO_GROUND_TRUTH`).

An acceptance without an evaluation is refused (`EVALUATION_MISSING`). Every refusal appends an
`accept-refused` record with its code to the history of the candidate; the latest mirror of the
candidate is not rewritten and no revision is published.

The hidden claims do **not** block the acceptance of a geometry: an unverified claim without a
ground truth is reported, never refused.

| Code | Exit code | Meaning |
|---|---|---|
| `EVALUATION_MISSING` | 1 | the acceptance carries no evaluation report |
| `EVALUATION_UNREGISTERED` | 1 | the report is not registered in the registry of the run |
| `EVALUATION_STALE` | 1 | the report does not match the digest or the pins of the run |
| `INPUT_CHANGED` | 1 | the package or the views of the run changed after the evaluation |
| `VISIBLE_FAILED` | 1 | the projection conformance does not pass |
| `AUTHOR_CONDITION_FAILED` | 1 | a required author geometric condition does not pass |
| `REGRESSION` | 1 | a source view regresses against the base revision |
| `NO_GROUND_TRUTH` | 1 | a claim needs a ground truth that is not available |

## The CLI

```text
michelangelo evaluate --run <run> (--candidate <id> | --revision sNNNN)
                      [--evaluation <pkg>] [--blender <bin>] [--json]
michelangelo edit accept --run <run> --candidate <id> --by <name>
                      [--evaluation <report>] [--json]
```

`michelangelo evaluate` exits with `0` when the verdict of the report is `pass`, with `1`
when the evaluation fails or the evidence is insufficient and with `2` for a usage error, an IO
error or a failed render or comparison. The report is registered **even when the verdict fails**:
the refusal needs its evidence.

`michelangelo edit accept` exits with `1` and the code of the gate when the evaluation is
missing or invalid; the option is not optional in the sense of the gate: without a registered
evaluation the acceptance is refused.

## Determinism

* The report is written with a fixed field order and a deterministic JSON encoder.
* The registry is ordered by the identifier of the evaluation.
* The measurements are the ones of the pinned comparator (`tools/solver/compare.py`) and of the
  pinned masks; the same inputs produce the same numbers.
* The evaluation renders the target and the base with the same job, the same cameras and the same
  settings; only the geometry differs.
* No absolute path is written into a report: the registry stores the path of the report relative
  to the run directory.

## The limits of v0

* **No independent ground truth.** The comparison against an independent ground truth is not
  implemented: the result is `unavailable` with the diagnostic "the comparison against an
  independent ground truth is not implemented in v0". The absence of a ground truth is never a
  pass.
* **The author conditions come from the accepted package data.** The only source of the required
  conditions is the `measurements` of the package copy of the run (the author-accepted data). A
  declared measurement that cannot be checked against the geometry (for example a distance
  between two points) yields `unavailable` and blocks the acceptance: a required condition is
  never treated as passed. The format of the input package stays v0.
* **The solver diagnostics are not recomputed.** The evaluation does not rerun the solver; the
  policy enforcement of IT-8C appears in the run report (`not_applied` supplements,
  `SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED`, `SUPPLEMENT_RULE_NOT_ALLOWED`, `STUB_FORBIDDEN`).
* **The hidden claims stay unverified.** The claims of the unknowns are reported as `unverified`
  and the verdict of the hidden part is `pending`; they never block the acceptance of a geometry.
  Only an explicit author resolution (`docs/specs/UNKNOWNS.md`) closes an unknown in a later
  bundle.
* **The evaluation of the bbox error needs Python.** The pinned comparator does not report the
  bounding box of a render; the error is measured with the PNG reader of the repository. When
  Python is not available the field is `null` and the report stays valid.
* **The automatic acceptance of the workspace policy is deferred.** An acceptance always needs a
  registered evaluation (contract §7), and the evaluation of a candidate can only run after its
  snapshot exists. The submission of a candidate that the policy would accept automatically
  therefore keeps the candidate `proposed` (with the diagnostic `EVALUATION_REQUIRED`) and the
  acceptance happens after `michelangelo evaluate`.

## The tests

* `crates/michelangelo_evidence/tests/evidence.rs` — the digests, the registry and every code of
  the gate (23 tests).
* `crates/michelangelo_eval/tests/evaluation.rs` — the assembly of the report, the pins, the
  regression, the hidden defaults, the author conditions and the held-out rule with a
  deterministic fake renderer and a deterministic fake comparator (12 tests).
* `crates/michelangelo_edit/tests/gate.rs` — every refusal of the accept gate and the happy
  path (10 tests).
* `crates/michelangelo_cli/tests/evaluate_cli.rs` — the usage contract of the CLI and the
  refusal of an acceptance without an evaluation (6 tests).

The real render is not proved by the test suite: the Lead reproduces it with a real Blender.

## The reproduction with a real Blender

The following sequence was executed on this machine with Blender 5.2.3 LTS and reproduces the
whole chain (the geometry is a 0.75 m box, the package is the committed fixture
`examples/input_packages/valid_full` without its measurements, the thresholds of the run are
`source_mean_iou = 0.8`):

```text
michelangelo edit init   --run <run>
michelangelo edit submit --run <run> --candidate <cand-dir>
michelangelo evaluate    --run <run> --candidate cand-0001
michelangelo edit accept --run <run> --candidate cand-0001 --by alice \
                         --evaluation <run>/verification/evaluations/ev-0001/report.json
```

Observed: the evaluation renders the snapshot and the base with the real worker, the mean IoU is
0.8889, the projection conformance is `pass`, the verdict is `pass` and the exit code is `0`; the
acceptance publishes `s0002`, the pointer moves and the bundle of the revision references the
evaluation with the digest of the report. With the measurement of the package (`height = 0.25 m`)
the author condition fails and the acceptance is refused with `AUTHOR_CONDITION_FAILED`.
