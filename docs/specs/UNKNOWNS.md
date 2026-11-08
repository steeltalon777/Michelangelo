# Unknowns: bindings, confirmations and resolutions (v0)

Normative contract of the **unknown state machine** of Michelangelo (Stage C / IT-8C):
the hidden aspects of a reconstruction run that the input package declares as
unknown. An unknown is closed by an explicit, documented chain of evidence —
never by a guess, never by a part-id match and never by an agent.

Status: implemented in IT-8C (crate `michelangelo_edit` module `unknowns`,
CLI `michelangelo unknown`). Normative basis:
`docs/specs/VERIFICATION_STATE.md` sections 2, 7, 8 and 9.

## Purpose

A reconstruction run knows what it does not know: the package declares
unknowns, the solver reports them (as a stub or as unresolved), and the
author decides how they are closed. The state machine makes that decision
auditable:

* every step is an immutable document (a binding, a confirmation, a
  resolution) — nothing is rewritten,
* the closure happens **only** in a scene revision whose bundle references the
  resolution,
* a resolution whose evidence went stale is not carried forward: the unknown
  becomes `open` with the reason `evidence_stale`,
* the historical documents stay in the history but are not presented as
  current for a new variant.

## The documents

```text
<run>/verification/bindings/bNNNN.json         michelangelo.unknown-bindings/0
<run>/verification/confirmations/cf-NNNN.json  michelangelo.confirmation/0
<run>/verification/resolutions/r-NNNN.json     michelangelo.resolution/0
<run>/verification/unknowns/revisions/sNNNN.json michelangelo.unknown-state/0
<run>/verification/unknowns/events.jsonl       append-only audit trail
```

### The binding (`michelangelo.unknown-bindings/0`)

A binding says *which* geometry an unknown is about. It is versioned: every
`bind` writes the next version `bNNNN` with the whole set of the bindings,
so a revision that refers to a version sees exactly the bindings of that
moment.

```json
{
  "schema": "michelangelo.unknown-bindings/0",
  "version": "b0001",
  "created_at": "2026-01-02T03:04:05Z",
  "bindings": [
    {
      "unknown_id": "u_flame_shape",
      "components": ["lid"],
      "region": {"min": [-0.2, -0.2, -0.2], "max": [0.2, 0.2, 0.2]},
      "by": "alice",
      "claim": "the flame is a tapering, symmetric profile"
    }
  ]
}
```

A part-id match is **never** a binding: the binding names the components and
the region explicitly.

### The author confirmation (`michelangelo.confirmation/0`)

A confirmation is the decision of the author about a claim, for one geometry
and for one evaluation. It is bound to

* the claim digest (`sha256` over `{unknown_id, claim, author_conditions,
  target_geometry_digest}`),
* the aggregate digest of the geometry the evaluation measured,
* the identifier **and** the digest of the evaluation,
* the digest of the evidence file of the author (optional).

A confirmation can never be reused for another geometry, another claim or
another evaluation. An agent may not confirm its own work: the reserved
identities (`agent`, `assistant`, `ai`, `ai_author`, `autonomous`,
`bot`, `dsh`, `llm`, `michelangelo`, `model`) are refused with
`AGENT_CONFIRMATION_REFUSED`.

### The resolution (`michelangelo.resolution/0`)

A resolution is the checked decision that closes an unknown. It references
the binding version, the candidate, the evaluation, the confirmation and the
claim digest, and it carries the checks it is based on:

| check | meaning |
|---|---|
| `binding` | a binding exists and the change touches its components and its region |
| `geometry` | the snapshot of the candidate is the geometry the evaluation measured |
| `projection_conformance` | the evaluation passes the projection conformance |
| `author_conditions` | every **required** author geometric condition passes (an optional one may stay `unavailable` with an explicit note) |
| `confirmation` | an accepted author confirmation exists for this claim, this geometry and this evaluation |
| `package_policy` | the `uncertainty_policy` of the package is satisfied |
| `binding_intersection` | the new geometry intersects the region of the binding |

A proposal (`unknown propose`) writes the document but does not change the
scene: the unknown stays open until a revision that references the resolution
is published.

### The state of a revision (`michelangelo.unknown-state/0`)

Every revision carries an immutable state document with one entry per
unknown:

```json
{
  "unknown_id": "u_flame_shape",
  "status": "stub_pending",
  "solver_status": "stub",
  "stub": true,
  "reason": null,
  "detail": "stub box of 10% of the hull bbox",
  "binding_version": null,
  "resolution_id": null
}
```

## The state machine

States: `open`, `stub_pending`, `resolved`. Events (append-only in
`unknowns/events.jsonl`): `sync`, `bind`, `confirm`, `propose`, `resolve`,
`rollback`. A proposal is a document, not a state.

Rules:

1. **The initial state** is derived from the immutable report of the solver:
   every unknown is `open`, a stub is `stub_pending` and stays in the
   unresolved list until a resolution closes it. The report is never
   rewritten.
2. **A binding is required.** Without a binding an unknown cannot be
   closed (`BINDING_MISSING`); a change that does not touch the components
   and the region of the binding is refused (`BINDING_MISMATCH`).
3. **A confirmation is required.** The confirmation must match the claim, the
   target geometry digest, the evaluation and its digest; otherwise the
   resolution is refused (`CONFIRMATION_MISSING`). An unregistered or a
   tampered evaluation is refused (`EVALUATION_UNREGISTERED`,
   `EVALUATION_STALE`).
4. **A resolution before an evaluation is impossible.** The resolution needs
   the evaluation of exactly the resolving geometry.
5. **Partial acceptance is first-class.** `edit accept` accepts the geometry
   and leaves every unknown open when `--resolve` is absent. The closure of
   an unknown is an independent outcome.
6. **The closure happens only in the bundle.** The unknown becomes
   `resolved` only in the revision whose bundle references the resolution
   (the bundle lists the bindings, the confirmation and the resolution with
   their digests). A resolution that survives a later revision keeps its
   confirmation, resolution and binding references in that later bundle;
   `verification.bindings` lists every binding version those resolutions use.
7. **Invalidation is mandatory.** Every new bundle recomputes the validity of
   the carried resolutions. A resolution is carried forward only when all of
   its dependencies still hold: the bound components, the binding version and
   the claim. Otherwise the unknown becomes `open` with the reason
   `evidence_stale`; the old resolution and confirmation stay in the history
   but are not current.
8. **A rollback restores the state of the target bundle.** The revisions keep
   their own immutable state document, including the resolutions that were
   current in that bundle.
9. **`unknown show --revision` reads the bindings of that revision's bundle.**
   It never substitutes the globally latest binding; after a rebind and a
   rollback the old claim/scope is shown. The current view follows the current
   bundle.

## The CLI

```text
michelangelo unknown sync    --run <run_dir> [--json]
michelangelo unknown list    --run <run_dir> [--revision sNNNN] [--json]
michelangelo unknown show    --run <run_dir> [--revision sNNNN] [--json]
michelangelo unknown bind    --run <run_dir> --unknown <id> --components <a,b,...>
                             --region min=max --by <name> [--claim <text>] [--json]
michelangelo unknown confirm --run <run_dir> --unknown <id> --by <name>
                             --evaluation <eval-id> [--evidence <file>] [--claim <text>] [--json]
michelangelo unknown propose --run <run_dir> --unknown <id> --candidate <id>
                             --evaluation <eval-id> --by <name> [--json]
michelangelo unknown resolve --run <run_dir> --unknown <id> [--candidate <id>]
                             [--evaluation <eval-id>] --by <name> [--json]
michelangelo edit accept     --run <run_dir> --candidate <id> --by <name>
                             --evaluation <report> [--resolve <unknown-id>] [--json]
michelangelo assistant task  --run <run_dir> [--unknown <id>] [--allowed <a,b,...>]
                             [--region min=max] [--out <file>] [--json]
```

| Exit code | Meaning |
|---|---|
| `0` | the operation succeeded |
| `1` | the operation was refused (a typed code is printed: `UNKNOWN_NOT_FOUND`, `BINDING_MISSING`, `BINDING_MISMATCH`, `CONFIRMATION_MISSING`, `RESOLUTION_REFUSED`, `UNKNOWN_POLICY_VIOLATION`, `STUB_FORBIDDEN`, `AGENT_CONFIRMATION_REFUSED`, `EVALUATION_UNREGISTERED`, `EVALUATION_STALE`, `EVALUATION_MISSING`) |
| `2` | usage error, IO error or a busy store (`STORE_BUSY`) |

`unknown sync` imports the unknowns of the immutable report of the run into
the state of the current revision. It is idempotent: the state document of a
revision is immutable, so a repeated synchronisation never rewrites it; the
report shows what the state does not know yet.

`unknown resolve` accepts the candidate and publishes the revision that
references the resolution. Without `--candidate` and `--evaluation` it uses
the latest proposal of the unknown.

## The assistant task of an unknown

`michelangelo assistant task --run <run> --unknown <id>` narrows the bounded
task to the binding of the unknown:

* `allowed_components` are the components of the binding, `allowed_region`
  is the region of the binding; a caller-supplied `--allowed`/`--region` may only
  **narrow** them (subset/inside), never widen them. A request outside the binding
  is refused (`SCOPE_DENIED`, task invalid),
* the task carries `unknown_id` and `unknown_criteria` (the components, the
  region, the claim, the binding version, the requirement of a confirmation
  and of a registered evaluation, the flag of the package and the conditions
  the result must satisfy),
* the references are filtered: only the unknown of the binding is listed.

The MCP adapter returns the same document through `get_task`; the assistant
surface never applies a change and never confirms an unknown.

## The policy of the package and the precedence

The uncertainty policy of the package (`uncertainty_policy` of
`package.json`) is the second level of the precedence of the contract
`docs/specs/VERIFICATION_STATE.md` section 8:

1. the author decisions that are accepted inside the package,
2. the `uncertainty_policy` of the package (`require_author_acceptance`,
   `allowed_completion_rules`, `stub_policy`),
3. the policy of the workspace (`edit_policy.json`, `assistant_policy.json`),
4. the flags of the candidate or of the task.

A lower level never weakens a higher one **for the author geometric intent**
(the supplements, the completion rules of an unknown, the acceptance of a
hidden choice). The security constraints — file and network access, the
budgets, the protected scope, the sandbox and the secrets — are independent:
no level of the hierarchy may weaken them.

The enforcement in the build (the solver, `tools/solver`):

* an AI-authored supplement is applied **only** with an accepted author
  decision for that element (`supplements:<id>`); otherwise it is not
  applied, the diagnostic `SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED` is reported
  and the unknown it would close stays unresolved,
* `allowed_completion_rules` limits the applied kinds of the supplements; a
  kind that is not listed is not applied and the diagnostic
  `SUPPLEMENT_RULE_NOT_ALLOWED` is reported,
* `stub_policy: forbidden` forbids the stubs: the unknown stays unresolved,
  no stub geometry is produced and the diagnostic `STUB_FORBIDDEN` (an
  error) is reported,
* an accepted author decision of the package has the highest precedence: it
  satisfies the author acceptance and it allows a completion rule the package
  does not list.

The enforcement in the edit boundary: the validation of a candidate carries a
`package_policy` check, the flag `require_author_acceptance` of a candidate
cannot weaken the policy of the package (the diagnostic
`PACKAGE_POLICY_STRICTER`) and a resolution always needs the accepted
confirmation of the author.

## The diagnostics

```text
UNKNOWN_NOT_FOUND        the run does not know the unknown
BINDING_MISSING         the unknown carries no binding
BINDING_MISMATCH        the change does not touch the binding of the unknown
CONFIRMATION_MISSING    no accepted author confirmation for the claim and the geometry
RESOLUTION_REFUSED      the unknown is already resolved / the resolution is refused
UNKNOWN_POLICY_VIOLATION the policy of the package forbids the resolution
STUB_FORBIDDEN          the package forbids the stubs
AGENT_CONFIRMATION_REFUSED an agent tried to confirm its own work
PACKAGE_POLICY_STRICTER the flag of the candidate cannot weaken the package policy
SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED  an AI supplement was not applied and the unknown stays open
SUPPLEMENT_RULE_NOT_ALLOWED         a completion rule is not allowed by the package
PACKAGE_POLICY          the policy of the package that the build applied (info)
```

## The limits of v0

* The state machine is Core-side: no network, no LLM, no Blender.
* A resolution closes one unknown. A revision may carry several resolutions,
  but the CLI accepts one `--resolve` per acceptance.
* The evidence of an author is a file whose digest is pinned; the content
  itself is not interpreted.
* The state document of a revision is immutable: a change of the state is a
  new revision (or a rollback to an older one).
