# Verification State and Acceptance Contract (Stage 0 for IT-8)

Status: rev. 2 — owner revisions incorporated (2026-11-07). Frozen as Stage 0 basis after the
Lead check; normative for IT-8A/B/C until superseded.
Baseline: dev `36f1bac` (IT-7). Owner review: pending.

This document freezes the state model that IT-8A must not re-invent: one authoritative
pointer, immutable evidence, the lock protocol, the verification result taxonomy and the
acceptance rules. Implementation follows this contract; a deviation requires an explicit
owner gate.

## 1. Invariants

1. **One authoritative pointer.** `<run>/current.json` selects one scene revision. That
   revision (the *bundle*) references, by hash, its geometry, unknown state, bindings,
   evaluations, confirmations and resolutions. No second mutable pointer decides state.
2. **Publish exactly what was evaluated.** A published revision is materialised from the
   candidate snapshot, never from `candidates/<id>/geometry`. The evaluation report is
   registered by the Core with a computed digest; its self-reported hash is not trusted.
3. **Accepted means one consistent set.** The pointer switch is the only commit point. A
   crash before it leaves the previous bundle authoritative; a crash after it leaves the new
   bundle authoritative. There is no observable half-state.
4. **History is append-only.** Decisions, bindings, confirmations, resolutions and unknown
   states are immutable documents; only `current.json` and the latest-decision mirror are
   rewritten.
5. **No silent verification.** Missing data yields `unavailable` (or `pending`), never
   `verified`/`pass`. Old runs and candidates without new evidence keep working for reading
   but cannot be accepted without producing the new evidence.
6. **Verification is not author acceptance.** Measurements and author decisions are
   separate results (see §6). An author decision is a legitimate basis for accepting a
   hidden-shape choice; it does not turn an unmeasured dimension into a verified one.
7. **Partial progress is first-class.** Accepting a geometry change and closing a specific
   unknown are independent outcomes. A revision may be accepted with open unknowns.

## 2. Layout

```text
<run>/current.json                        {"scene_revision":"sNNNN", ...}
<run>/scene/sNNNN/scene.json              schema, components, verification bundle
<run>/scene/sNNNN/components/*.obj
<run>/verification/
  snapshots/<candidate_id>/manifest.json  michelangelo.geometry-snapshot/0
  snapshots/<candidate_id>/geometry/*.obj # FULL post-change component set
  evaluations/<eval-id>/report.json       michelangelo.evaluation-report/0
  evaluations/index.json                  append-only registry of evaluation digests
  bindings/<binding-id>.json              michelangelo.unknown-bindings/0 (versioned)
  confirmations/<conf-id>.json            michelangelo.confirmation/0
  resolutions/<resolution-id>.json        michelangelo.resolution/0
  unknowns/revisions/sNNNN.json           michelangelo.unknown-state/0 (immutable)
  unknowns/events.jsonl                   append-only audit trail
  run.lock                                OS advisory lock file
<run>/candidates/<id>/
  candidate.json                          michelangelo.edit-candidate/0
  geometry/*.obj                          submitted bytes (snapshot source only)
  decisions/NNNN-<kind>.json              append-only decision history
  decision.json                           latest-decision mirror
```

`scene.json` gains one object, `verification`:

```json
{
  "geometry_digest": "<sha256>",
  "bindings":   [{"version": "b0001", "path": "verification/bindings/b0001.json", "sha256": "..."}],
  "unknowns":   {"revision": "s0002", "path": "verification/unknowns/revisions/s0002.json", "sha256": "..."},
  "evaluations":[{"evaluation_id": "ev-0001", "path": "...", "sha256": "..."}],
  "confirmations":[{"confirmation_id": "cf-0001", "path": "...", "sha256": "..."}],
  "resolutions":[{"resolution_id": "rs-0001", "path": "...", "sha256": "..."}],
  "policies":   {"package_sha256": "...", "workspace_policy_sha256": "..."},
  "tool_versions": {"michelangelo": "0.1.0", "solver": "voxel-carve/1", "blender": "5.2.3 LTS"},
  "created_at": "<RFC 3339 UTC>"
}
```

`verification.bindings` is a list of binding-document references (one per active binding
version); it is empty until IT-8C creates the first binding. A revision only selects the
versions it refers to, so old revisions keep their binding set.

`current.json` is published by write-to-temp + rename. Publishing a revision means: write
`scene/sNNNN.tmp` completely, verify every referenced document hash, rename to `scene/sNNNN`,
then atomically replace `current.json`.

## 3. Digests

Digests are SHA-256 over canonical bytes. They are computed by the Core, never taken from
the document that declares them when the document is itself evidence.

- **file**: SHA-256 of the file bytes.
- **geometry aggregate**: `sha256` over lines `"<component>\u0000<sha256>\n"` sorted by
  component name, for the *full* component set of a revision/snapshot.
- **camera digest**: `sha256` of canonical JSON
  `{"view_id","location_m","look_at_m","up_world","ortho_scale_m","shift_x","shift_y",
    "clip_start_m","clip_end_m","width_px","height_px"}`.
- **package digest**: SHA-256 of `package.json` bytes.
- **inputs digest**: `sha256` of canonical JSON
  `{"package_sha256":..., "views":[{"view_id","camera_digest","mask_sha256","resolution",
  "held_out"}], "evaluation_package_sha256": ...}` sorted by `view_id`.
- **claim digest**: `sha256` of canonical JSON
  `{"unknown_id","claim","author_conditions":[...],"target_geometry_digest"}`.
- **evaluation digest**: SHA-256 of `report.json` bytes, computed by the Core when the file
  is registered in `evaluations/index.json`; accept re-computes it and compares both to the
  registry and to the value recorded by the candidate decision.

## 4. Lock and serialisation

- All state mutations take an exclusive OS advisory lock on `<run>/verification/run.lock`
  via `std::fs::File::try_lock()` (verified on Rust 1.96). The OS releases it when the
  owning process exits; there is no TTL-based stealing.
- Busy lock → error `STORE_BUSY`, CLI exit 2. No blocking wait in v0.
- Critical sections are short: snapshot registration, decision write, publish + pointer
  switch, rollback, binding/confirmation/resolution registration. Evaluation and rendering
  happen **outside** the lock.
- Immediately before the pointer switch, accept re-reads `current.json` and re-checks that
  the candidate base revision, package digest, bindings version and input digests still
  match; otherwise it fails without publishing (`STALE_BASE`/`INPUT_CHANGED`).
- All competing mutators (`accept`, `rollback`, `bind`, `confirm`, `resolve`) use the same
  lock. A crash while holding it is recovered by process death, not by manual file deletion.

## 5. Snapshots

`michelangelo.geometry-snapshot/0`:

```json
{
  "schema": "michelangelo.geometry-snapshot/0",
  "snapshot_id": "cand-0001",
  "created_at": "...",
  "base": {"package_id":"...","package_revision":1,"run_id":"...","scene_revision":"s0001",
            "geometry_digest": "..."},
  "candidate_sha256": "...",
  "task_sha256": "...",
  "package_sha256": "...",
  "bindings_sha256": "...",
  "package_policy_sha256": "...",
  "files": [{"component":"lid","path":"geometry/lid.obj","sha256":"...","source":"candidate"},
            {"component":"base","path":"geometry/base.obj","sha256":"...","source":"base"}],
  "aggregate_sha256": "...",
  "tool_versions": {"michelangelo":"0.1.0","solver":"voxel-carve/1"}
}
```

The snapshot always materialises the **full** post-change component set: unchanged
components are copied byte-identically from the base revision. Evaluation runs on the
snapshot; publication copies the snapshot geometry into the revision and verifies the
manifest hashes.

## 6. Verification results

An evaluation reports independent results; they are never collapsed into one boolean:

| Result | Values | Meaning |
|---|---|---|
| `projection_conformance` | `pass` / `fail` / `unavailable` | silhouette/measurement conformance against the pinned source and held-out projections |
| `author_geometric_conditions` | `pass` / `fail` / `unavailable` | measurable conditions declared by the author (e.g. a 2 mm wall), checkable from the mesh without any ground truth |
| `independent_ground_truth` | `pass` / `fail` / `unavailable` | comparison against an independent ground truth, when one is supplied |
| `author_hidden_decision` | `accepted` / `rejected` / `pending` | the author's decision about a hidden-shape choice; a decision, not a measurement |

Rules:
- Missing inputs produce `unavailable`; they are reported with a diagnostic, never silently
  treated as a pass.
- `independent_ground_truth` is optional. A fictional object may never have one; that does
  not block acceptance of the visible geometry.
- A confirmation by the author is recorded as an `author_hidden_decision` basis; it cannot
  convert an `unavailable` measurement into `pass`. The confirmation is stored separately
  (confirmation document + bundle reference) and never rewrites the immutable evaluation
  report.
- Supplements already accepted in the package do not require a repeated author confirmation.
  However `author_geometric_conditions = pass` requires verification of the actual geometry
  or a verifiable construction guarantee that preserves the condition. Executing a rule is
  not by itself proof of the required size: e.g. a requested 2 mm shell built on a 6 mm voxel
  grid can execute correctly and still not meet the declared tolerance. The measured value
  must satisfy the declared tolerance, otherwise the condition is `fail` or `unavailable`.
  A **required** condition with `fail`/`unavailable` cannot be treated as passed; an
  **optional** condition may remain explicitly unverified (`unavailable`).
- Held-out views must add an independent silhouette constraint: a view looking in the same
  direction as a source view (`dot(dir) > 0.9`) adds nothing and makes the held-out result
  `unavailable`. Opposite orthographic views (`dot ≈ -1`) are allowed — they see the mirrored
  silhouette and are the held-out controls of the proof datasets.

## 7. Acceptance rules

- **Geometry change acceptance** requires an evaluation whose registered digest, target
  aggregate and input digests match the candidate snapshot and the current state, with
  `projection_conformance = pass` and every **required** `author_geometric_conditions`
  entry `pass` (an optional entry may be `unavailable` only when it is explicitly marked
  optional and the gap is reported). Hidden decisions do not block geometry acceptance.
- **Unknown resolution** is separate and additionally requires: a versioned binding, an
  evaluation of exactly the resolving geometry, the required author confirmation for that
  claim/geometry, and the package `uncertainty_policy` satisfied. Resolving does not happen
  at proposal time: a `resolution` document is published into a revision bundle.
- **Partial acceptance** is allowed: a geometry revision may be accepted while unknowns
  remain open. Closing a specific unknown is recorded separately.
- **Invalidation of prior resolutions.** Every new bundle recomputes which prior resolutions
  are still valid. If a revision changes geometry, bindings or conditions that a resolution
  depended on, the resolution is not carried forward as current: the unknown appears `open`
  with `reason: evidence_stale` in the new bundle's state. Historical evidence and
  confirmations remain in the history but are not presented as current for the new variant.
  A resolution is carried forward only when all its dependency digests (target components,
  binding version, claim/conditions) are unchanged. This validity rule must be part of
  bundle formation from IT-8A onward, even though the full resolution transitions land in
  IT-8C.
- **Idempotent accept.** A repeated accept of an already accepted candidate returns the
  historical accepted result (same revision) and appends an `accept-idempotent` history
  record; it does not create a revision and does not rewrite the accepted decision. After a
  rollback, the same call still returns the historical result and does not switch the
  scene back.
- **Legacy data.** A candidate/run without snapshot evidence cannot be accepted:
  `SNAPSHOT_MISSING` (exit 1). Historical accepted revisions stay readable; no automatic
  `verified` is inferred for them.
- **Refused attempts are auditable.** A refused acceptance (missing snapshot, evidence
  mismatch, stale base) appends an `accept-refused` record with its diagnostic code to the
  candidate history; the latest-decision mirror is left untouched because the refusal is an
  attempt, not a new decision about the candidate.

## 8. Policy precedence and build-time enforcement

1. author decisions recorded and accepted inside the input package;
2. package `uncertainty_policy` (`require_author_acceptance`, `allowed_completion_rules`,
   `stub_policy`);
3. workspace policy (`edit_policy.json`, `assistant_policy.json`);
4. candidate/task flags.

A lower level can never weaken a higher one **for the author's geometric intent**
(supplements, unknown completion rules, acceptance of hidden choices). The hierarchy never
grants additional system access: security constraints — file/network access, budgets,
protected components/scope, sandbox/confinement, secrets policy — apply independently and
can be made stricter by any layer, never weaker by a package or candidate. A package
decision does not override budgets or protected scope.

The solver must not apply an unaccepted AI-authored supplement when the package requires
author acceptance; such a supplement is reported with status `not_applied` and diagnostic
`SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED`, and does not close the unknown. A supplement rejected by
`allowed_completion_rules` is `not_applied` with `SUPPLEMENT_RULE_NOT_ALLOWED`. An accepted
author decision overrides the package rule list for that element (the author's geometric
intent wins); a candidate can never weaken the package (`PACKAGE_POLICY_STRICTER`). A package
without an `uncertainty_policy` object keeps the Stage B behaviour (supplements applied, stubs
allowed); with the object, `require_author_acceptance` defaults to `true`.
`stub_policy: forbidden` is absolute: no stub mesh is built, the solver reports
`STUB_FORBIDDEN` and an accepted author decision cannot re-enable a stub.

## 9. Unknown state machine

States: `open`, `stub_pending`, `resolved`.
Events (append-only): `sync`, `bind`, `confirm`, `propose`, `resolve`, `rollback`.

- Initial state per revision is derived from the immutable solver report; stubs are
  `stub_pending` and remain in the unresolved list until resolved.
- `propose` writes an immutable proposal referencing candidate snapshot, evaluation and
  confirmation; it is a document, not a state, and does not change the current scene.
- `resolve` is only materialised by publishing a revision whose bundle references the
  resolution (typically `edit accept --resolve <id>`); a projection/condition/confirmation
  failure is refused with a typed code, and a proposal alone never closes the unknown.
- A part-id match alone is never a resolution basis; an explicit binding and evidence are
  required. A binding may omit the claim; the author confirmation then carries the claim
  (and the claim digest) for the resolution.
- **Invalidation (mandatory bundle rule):** if a later revision changes any dependency of a
  resolution (target component hashes, binding version, claim/conditions), that resolution
  is no longer current; the unknown becomes `open` with `reason: evidence_stale`. The old
  resolution/confirmation documents stay in history but are not reused. Unaffected
  resolutions are carried forward only when their dependency digests match.
- Rollback selects the target revision's immutable unknown-state document, including the
  resolutions that were current in that bundle.

## 10. Failure-test matrix

| Invariant | Test |
|---|---|
| snapshot immutability / substitution | tamper store geometry → publish snapshot (or `EVIDENCE_MISMATCH`); tamper snapshot → `EVIDENCE_MISMATCH` |
| one commit point / crash | crash between revision rename and pointer switch → old bundle authoritative, recovery succeeds |
| lock serialisation | two processes accept from one base → at most one succeeds, other `STORE_BUSY`/`STALE_BASE`; lock released on process death |
| base check at publish | injected current change between validate and publish → `STALE_BASE`, no revision |
| idempotent accept | second accept → `accept-idempotent`, same revision, history intact; after rollback → historical result, scene unchanged |
| reject-after-accept | `CANDIDATE_ALREADY_ACCEPTED`, no history rewrite |
| registered evaluation | missing/unregistered/stale/digest-mismatched reports rejected |
| visible vs author vs GT | each result independently `pass|fail|unavailable`; no GT → `unavailable`, not pass |
| partial acceptance | geometry accepted with open unknown; resolution separately refused without confirmation |
| unknown closure | part-id-only and unrelated-supplement closures refused; stub stays unresolved until resolution |
| policy precedence | package `require_author_acceptance=true` defeats candidate flag and workspace auto-accept; forbidden stubs; allowed rules |
| confirmation binding | confirmation for geometry A cannot resolve geometry B / older evaluation |
| rollback state | rollback restores geometry, bindings and unknown state of the target revision |
| legacy data | old candidate without snapshot → `SNAPSHOT_MISSING`; old accepted revision readable, not auto-verified |

## 11. Resolved decisions (owner, 2026-11-07)

1. **Projection regression.** Per-VIEW tolerance: every source view must not drop more than
   0.01 IoU versus the base revision, in addition to the absolute thresholds. The mean is a
   summary only. Base and candidate are compared with identical inputs, cameras and settings.
2. **Author geometric conditions.** Requirements may be derived from author-accepted rules in
   the package, but an `author_geometric_conditions = pass` is emitted only when the condition
   was actually verified on the resulting geometry (or by a verifiable construction guarantee
   that preserves it). Author confirmation and a fresh geometric verification are different
   things and are recorded separately.
3. **Retention.** IT-8 deletes nothing automatically. Accepted bundles and all their transitive
   dependencies are kept; garbage collection of unbound results is a separate future task.

