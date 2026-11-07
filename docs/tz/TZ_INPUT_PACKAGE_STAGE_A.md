# TZ: Stage A — Minimal Input Package v0 (ADR 0004 §13.1)

Status: in progress
Branch: `dev` (no push, no branch switch; commits only after verified checks and a GREEN review)
Owner: TechLead (orchestrator); implementation by worker subagents; independent review before commit.

## 0. Context and Baseline

Baseline at start (2026-11-07, `dev` = `78429b4`):

- ADR 0004 is accepted; §13.1 (Stage A) requires defining and validating the minimal input package
  before any reconstruction work: mandatory information for the supported orthographic mode,
  partial supplements, view registration, conflict and revision policy.
- ROADMAP Phase 3 lists the minimal input package (manifest/PNG ingest, alpha masks, bbox,
  connected components, contours) and input validation levels per ADR §6.
- Handoff 2026-11-07 §4 fixes the Stage A implementation scope: JSON manifest + JSON Schema +
  Rust validator (validation levels 1–2 of ADR §6.1), no new Blender ops, no change to the JSONL
  wire contract (`CORE_PROTOCOL`/`BLENDER_WORKER`), no SQLite migration.
- Owner decisions (2026-11-07, this session):
  - package lives at `workspace/input/<revision>/package.json`, revision directories are
    immutable snapshots named `r0001`, `r0002`, …;
  - images: PNG only, RGB8 or RGBA8; masks: PNG, 8-bit grayscale (`0` = background,
    `>= 128` = object) or RGBA alpha (`alpha >= 128` = object); JPEG/other formats rejected;
  - one calibrated view is a valid partial package; a complete package needs at least two
    calibrated views with distinct orthographic directions; `--require-complete` turns partial
    into a failure.
- Local untracked sample datasets exist at `I:\prog\Michelangelo_PoC\` (`solver_input/` prepared
  views + `evaluation/` held-out views + `ground_truth/`). They are evidence/acceptance material
  for Stage B and are **not** committed to Git; Stage A must not depend on them, but its field
  names must be able to represent their calibration facts (orthographic camera location,
  look-at, up, `ortho_scale` = full view width, 1024x1024, 1 px = ortho_scale/1024, mask
  grayscale 0/255, units metres, known overall dimensions with tolerance).

## 1. Goal

Ship contract v0 of the validated input package and its Rust validator (levels 1–2 of ADR §6.1),
usable from the CLI without touching the JSONL wire contract:

- a strict, versioned JSON manifest (`michelangelo.input-package/0`) for one package revision;
- a JSON Schema document describing that manifest (documentation + external tooling);
- a Rust library crate and CLI command that validate format/safety (level 1) and sufficiency
  (level 2), verify resource hashes, resolve paths safely inside the package directory, and emit
  targeted, addressable diagnostics;
- partial packages are first-class: independent valid parts stay usable, the report states
  completeness and exactly which information blocks metric reconstruction;
- fixtures, unit/integration tests and a spec page that documents the contract.

## 2. Out of Scope (hard limits for this iteration)

- No JSONL methods, DTOs, protocol error codes, event names, `CORE_PROTOCOL` or `BLENDER_WORKER`
  changes. The new functionality is reachable only through a CLI subcommand.
- No new Blender operations, no Blender execution, no real-Blender acceptance in this iteration.
- No SQLite schema/migrations, no Asset Graph indexing, no thumbnails.
- No pixel-level image analysis (bbox/connected components/contours/IoU) — that is Stage B
  (solver); Stage A validates container format, dimensions and hashes only.
- No reconstruction, solver, supplements semantics evaluation, provenance database, or package
  import from the PoC datasets (Stage B).
- No new third-party crates: implement SHA-256, CRC-32 and PNG header parsing in-repo; use only
  `serde`, `serde_json`, `clap` and existing workspace deps.
- Do not commit `docs/research/`, `docs/handoff/`, `I:\prog\Michelangelo_PoC\` or anything under
  `target/`.

## 3. Contract v0 (normative for this iteration)

### 3.1 Package layout

```text
<workspace>/input/r0001/package.json
<workspace>/input/r0001/images/<view_id>.png      # RGB8 or RGBA8
<workspace>/input/r0001/masks/<view_id>.png       # gray8 or RGBA8
```

- The package directory name, when it matches `^r[0-9]{4}$`, must equal `revision`
  (`r0001` ↔ `revision: 1`); otherwise report a level-2 warning `REVISION_DIR_MISMATCH`.
- All resource paths are relative to the package directory, use `/`, are ≤ 1024 chars, contain no
  `..`, no leading `/`, no backslash, no drive/UNC form, no NUL; they must resolve (after symlink
  resolution) inside the canonicalized package directory and point at a regular file.
- Revision directories are immutable by convention: validation never writes into the package and
  reports a level-2 warning `PACKAGE_DIR_WRITABLE` only if `package.json` is newer than all
  resources and the directory name does not match the revision pattern (documented convention,
  not enforced by filesystem ACLs).

### 3.2 Manifest

Top-level object, unknown fields rejected (`deny_unknown_fields`):

| Field | Type | Rules |
|---|---|---|
| `schema` | string | exactly `michelangelo.input-package/0` |
| `package_id` | string | `^[a-z0-9][a-z0-9._-]{0,63}$` |
| `object` | object | `name`: 1..200 chars; optional `notes`: ≤ 2000 chars |
| `revision` | integer | 1..1_000_000 |
| `units` | string | `m` \| `cm` \| `mm` (all lengths in the manifest use these units) |
| `coordinate_system` | object | `handedness`: `right`; `up`, `right`, `forward`: signed axis strings (`+X`,`-X`,`+Y`,`-Y`,`+Z`,`-Z`), pairwise distinct |
| `views` | array | 1..64 entries, `view_id` unique |
| `parts` | array | 0..256 entries, `part_id` unique |
| `supplements` | array | 0..256 entries, `supplement_id` unique |
| `unknowns` | array | 0..256 entries, `id` unique |
| `measurements` | array | 0..64 entries, `id` unique (optional; known/declared dimensions) |
| `uncertainty_policy` | object | optional; see §3.2.6 |
| `provenance` | object | required; `origin` ∈ `human` \| `ai_author` \| `mixed`; optional `detail` (≤ 2000) |
| `acceptance` | array | 0..512 entries |

#### 3.2.1 View

```json
{
  "view_id": "front",
  "kind": "calibrated",
  "image": {"path": "images/front.png", "sha256": "<64 lowercase hex>", "width_px": 1024, "height_px": 1024},
  "mask":  {"path": "masks/front.png", "sha256": "<64 lowercase hex>", "format": "grayscale8", "foreground": "white"},
  "projection": {
    "type": "orthographic",
    "location_m": [0.0, -4.0, 0.0],
    "look_at_m": [0.0, 0.0, 0.0],
    "up_world": [0.0, 0.0, 1.0],
    "ortho_scale_m": 1.0,
    "shift_x": 0.0,
    "shift_y": 0.0,
    "clip_start_m": 0.02,
    "clip_end_m": 100.0
  },
  "provenance": {"origin": "ai_author", "detail": "calibrated orthographic render"}
}
```

- `view_id`: `^[a-z][a-z0-9_]{0,31}$`.
- `kind`: `calibrated` (usable as a metric constraint) or `reference` (visual reference only).
- `image` and `mask`: both required. `sha256` required (64 lowercase hex), verified against the
  file always for `calibrated` views and for `reference` views too (level 1; `--no-hash-verify`
  downgrades mismatches to warnings). `width_px`/`height_px`: 1..16384 and must equal the PNG
  IHDR dimensions. Image must be PNG with bit depth 8 and color type 2 (RGB) or 6 (RGBA);
  mask must be PNG bit depth 8 and color type 0 (grayscale, `foreground: "white"`) or 6
  (RGBA, `foreground: "alpha"`). Mask dimensions must equal image dimensions.
- `projection`: required for `calibrated`; optional for `reference` (ignored with level-2 warning
  `REFERENCE_PROJECTION_IGNORED` when present). `type` exactly `orthographic`.
  `location_m`, `look_at_m`, `up_world`: finite, components abs ≤ 1e6; `up_world` length in
  `[0.99, 1.01]`; `(look_at_m - location_m)` nonzero and not parallel to `up_world`
  (`|dot(unit(dir), unit(up))| < 0.999`). `ortho_scale_m` > 0 and ≤ 1e6, equals the visible
  world width of the image; visible height = `ortho_scale_m * height_px / width_px`.
  `shift_x`/`shift_y` in `[-10, 10]` (default `0`); `clip_start_m` > 0 (default `0.1`);
  `clip_end_m` > `clip_start_m` (default `1000`).
- Pixel convention (fixed, not expressible differently in v0): image origin top-left, `u` right,
  `v` down, camera looks at `look_at_m`, `u_axis = normalize(cross(view_dir, up_world))`,
  `v_axis = up_world`; `u_px = (0.5 + u_cam / ortho_scale_m) * width_px`,
  `v_px = (0.5 - v_cam / ortho_scale_m) * width_px` where `u_cam`, `v_cam` are camera-space
  coordinates in package units (the same mapping as `BLENDER_WORKER.md` §Camera projection).
- `provenance` (optional): `origin` ∈ `human` \| `ai_author` \| `mixed`; optional `detail` ≤ 1000.

#### 3.2.2 Part

```json
{"part_id": "lid", "label": "lid", "parent": null, "protected": false}
```

`part_id`/`label`: `^[A-Za-z0-9][A-Za-z0-9 _.-]{0,63}$`; `parent` optional reference to an existing
`part_id`; parent graph must be acyclic; `protected` optional bool (default `false`; used by
Stage C, validated here only for shape).

#### 3.2.3 Supplement (authored completion of hidden shape)

```json
{
  "supplement_id": "lid_thickness",
  "part_id": "lid",
  "kind": "thickness",
  "geometry": {"value_m": 0.012},
  "provenance": {"origin": "human", "detail": "author decision"}
}
```

- `kind` ∈ `thickness` \| `curve` \| `section` \| `profile` \| `closure` \| `cavity` \|
  `connection` \| `symmetry`.
- `part_id` must reference an existing part (level 1 error `SUPPLEMENT_PART_UNKNOWN`).
- `geometry` must be a non-empty JSON object; all numeric values finite with abs ≤ 1e6; depth of
  nesting ≤ 8; serialized size ≤ 64 KiB.
  Per-kind required numbers (level 1 error `SUPPLEMENT_GEOMETRY_INVALID` when missing/invalid):
  - `thickness`: `value_m` > 0;
  - `curve`: `points_m` array of 2..256 points, each `[x,y,z]` finite;
  - `section` / `profile`: `points_m` (2..256 2D or 3D points) plus optional `plane`
    (`xy` \| `yz` \| `xz`);
  - `cavity`: `bbox_m` = `[x,y,z]` sizes, all > 0, or `points_m` ≥ 4;
  - `closure` / `connection`: `from` and `to` each `[x,y,z]` finite;
  - `symmetry`: `axis` ∈ `x` \| `y` \| `z` and optional `plane_offset_m` finite.
- `provenance` required (`human` \| `ai_author` \| `computed`).

#### 3.2.4 Unknown

```json
{"id": "u_lid_inside", "description": "hidden lid interior", "affected_parts": ["lid"], "policy": "stub_allowed"}
```

`id` unique `^[a-z][a-z0-9_]{0,63}$`; `description` 1..500; `affected_parts` 0..64 existing part
ids; `policy` ∈ `author_required` \| `rule_allowed` \| `stub_allowed`.

#### 3.2.5 Measurement (known dimension)

```json
{"id": "height", "label": "overall height", "kind": "height", "value_m": 0.645986, "tolerance_m": 0.002, "origin": "human"}
```

`kind` ∈ `height` \| `width` \| `depth` \| `distance` \| `bbox`; `value_m` > 0 and finite;
`tolerance_m` ≥ 0 and finite, default `0`; `origin` ∈ `human` \| `ai_author` \| `computed`.

#### 3.2.6 Uncertainty policy

```json
{
  "allowed_completion_rules": ["thickness", "closure", "symmetry", "primitive_fill"],
  "stub_policy": "allowed",
  "require_author_acceptance": true
}
```

`allowed_completion_rules`: 0..8 entries from the supplement `kind` set plus `primitive_fill`;
`stub_policy` ∈ `allowed` \| `forbidden`; `require_author_acceptance`: bool (default `true`).

#### 3.2.7 Acceptance

```json
{"target": "supplements:lid_thickness", "status": "accepted", "by": "owner", "at": "2026-11-07T12:00:00Z"}
```

`target` = `<views|parts|supplements|unknowns|measurements>:<id>` and must reference an existing
element (level 1 error `ACCEPTANCE_TARGET_UNKNOWN`); `status` ∈ `proposed` \| `accepted` \|
`needs_revision`; `by` non-empty ≤ 128; `at` RFC 3339. Duplicate `(target)` pairs: only the last
entry is authoritative; an earlier `accepted` followed by anything else is a level-2 warning
`ACCEPTANCE_SUPERSEDED`. If `uncertainty_policy.require_author_acceptance` is true, every
supplement with `provenance.origin = ai_author` must have an `accepted` acceptance entry
(level 2 warning `SUPPLEMENT_NOT_ACCEPTED`).

### 3.3 Validation levels and report

Level 1 — format/safety: schema/version, types, enums, bounds, unique ids, reference integrity,
resource existence/regular-file/containment/PNG container format/dimensions/hash, manifest size
≤ 1 MiB, at most 64 views, at most 512 acceptance entries, total referenced bytes ≤ 512 MiB,
no path traversal, no non-finite numbers, no unknown fields.

Level 2 — sufficiency: per-view usability and package completeness for metric reconstruction:
`calibrated` views with calibrated projection + mask form the measurable set; a package is
`complete` when at least 2 calibrated views have pairwise distinct directions
(`|dot(unit(d1), unit(d2))| <= 0.9`, i.e. ≥ ~26° apart) and those directions span at least two
coordinate axes; otherwise `partial` with explicit missing reasons. Also checks acceptance and
reference-projection rules above. Duplicate calibrated view directions (same |dot| > 0.999) are
a level-2 warning `DUPLICATE_VIEW_DIRECTION` (they are not independent silhouette constraints,
ADR §9.1).

Report JSON (printed by the CLI, stable shape):

```json
{
  "schema": "michelangelo.input-package-report/0",
  "package_id": "…",
  "revision": 1,
  "status": "valid_complete" | "valid_partial" | "invalid",
  "levels": {
    "format_safety": {"ok": true, "errors": 0, "warnings": 1},
    "sufficiency":   {"ok": true, "errors": 0, "warnings": 0}
  },
  "completeness": {
    "status": "complete" | "partial",
    "calibrated_views": 2,
    "distinct_directions": 2,
    "axes_covered": ["x", "z"],
    "missing": ["INSUFFICIENT_VIEW_DIRECTIONS"]
  },
  "diagnostics": [
    {"level": 1, "code": "HASH_MISMATCH", "severity": "error", "address": "views[0].mask.sha256", "message": "…"}
  ],
  "resources": [
    {"address": "views[0].image", "path": "images/front.png", "sha256": "…", "bytes": 1234, "verified": true}
  ]
}
```

- `status` is `invalid` if any level-1 error exists; `valid_complete` / `valid_partial` otherwise.
  Level-2 errors also make the package `invalid` only when `--require-complete` is passed;
  otherwise they downgrade to `missing` reasons and level-2 warnings (ADR §6.1: an incomplete
  package does not block independent parts). A level-1 error always wins.
- Diagnostics are deterministic and sorted by `(level, address, code)`; addresses use JSON-path
  like `views[2].mask.path`, `supplements[0].geometry.value_m`.
- CLI exit codes: `0` valid (complete or partial, warnings allowed); `1` invalid or absent
  `--require-complete` partial; `2` usage/IO error (missing package dir, unreadable manifest).

### 3.4 CLI

```text
michelangelo input validate --package <DIR> [--require-complete] [--no-hash-verify]
                            [--report <FILE>] [--quiet]
michelangelo input describe --package <DIR>
```

- `validate` prints the report JSON to stdout (compact? pretty, documented) and exits per §3.3;
  `--report` also writes it to a file inside the package's parent directory (path must not be
  inside the package revision directory); `--quiet` prints only a one-line human summary to
  stderr and nothing to stdout.
- `describe` prints a short summary (ids, revision, view ids/kinds, counts, completeness) for a
  package that passes level 1.
- Both commands work without an open project; `--workspace <WS>` is not needed (package dir is
  given directly). The CLI never writes into the package directory.

## 4. Iteration Plan (IT-4)

Write scopes:

- `crates/michelangelo_input/**` (new crate: `Cargo.toml`, `src/lib.rs`, `src/sha256.rs`,
  `src/png.rs`, `src/manifest.rs`, `src/validate.rs`, `src/report.rs`, `src/error.rs`, tests).
- `Cargo.toml` (workspace members: add `crates/michelangelo_input`), `Cargo.lock`.
- `crates/michelangelo_cli/Cargo.toml` (+ `michelangelo_input` dep), `crates/michelangelo_cli/src/main.rs`
  (new `input` subcommand; may add `src/input_cmd.rs`).
- `docs/specs/INPUT_PACKAGE.md` (new spec page, normative for v0).
- `docs/specs/schemas/input_package_v0.schema.json` (new JSON Schema, mirrors §3.2).
- `examples/input_packages/valid_full/**`, `examples/input_packages/valid_partial/**`
  (small committed fixtures: 2 and 1 calibrated views, 8x8 PNGs, real hashes).
- `ROADMAP.md` (Phase 3 Stage A progress note only), `TASKS.md` (T-0401..T-0403 completed
  entries), `docs/ai/AI_CONTEXT.md` (one short paragraph on the input package + validator),
  `docs/tz/TZ_INPUT_PACKAGE_STAGE_A.md` (this TZ: evidence + review log).
- No other files.

Task split (sequential; one writer at a time):

- **T-A1 (worker)** — `michelangelo_input` crate: SHA-256 (NIST test vectors: empty string,
  `"abc"`, one 1,000,000×`a`; known digests), CRC-32 (IEEE) for PNG, PNG header parser
  (signature, IHDR, color type/bit depth validation, IEND presence, unknown critical chunks
  rejected), manifest structs with strict deserialization, validator levels 1–2 with the exact
  diagnostic codes below, report types, unit tests. Fixtures for the crate live in
  `crates/michelangelo_input/tests/fixtures/**` plus committed `examples/input_packages/**`.
  Must not touch CLI, docs or the workspace manifest.
- **T-A2 (worker, after T-A1)** — CLI `input validate` / `input describe` wiring and integration
  tests invoking the built binary (`CARGO_BIN_EXE_michelangelo`), exit codes, `--report`,
  `--quiet`; plus `docs/specs/INPUT_PACKAGE.md` and
  `docs/specs/schemas/input_package_v0.schema.json` consistent with §3 and the Rust types.
- **T-A3 (Lead)** — reconcile, run the full ladder (`cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  `cargo build -p michelangelo_cli`, CLI acceptance on fixtures + an ad-hoc package built from
  the local PoC dataset's calibration facts), independent review, commit.

Diagnostic codes (level, code):
1: `SCHEMA_UNKNOWN_FIELD`, `SCHEMA_TYPE`, `SCHEMA_ENUM`, `SCHEMA_RANGE`, `ID_INVALID`,
   `ID_DUPLICATE`, `PATH_INVALID`, `PATH_OUTSIDE_PACKAGE`, `RESOURCE_MISSING`,
   `RESOURCE_NOT_REGULAR`, `RESOURCE_TOO_LARGE`, `PNG_SIGNATURE`, `PNG_IHDR_INVALID`,
   `PNG_COLOR_TYPE`, `PNG_DIMENSION_MISMATCH`, `PNG_TRUNCATED`, `HASH_MISMATCH`,
   `HASH_INVALID`, `REFERENCE_UNKNOWN`, `PARENT_CYCLE`, `GEOMETRY_INVALID`,
   `SUPPLEMENT_PART_UNKNOWN`, `SUPPLEMENT_GEOMETRY_INVALID`, `MEASUREMENT_INVALID`,
   `ACCEPTANCE_TARGET_UNKNOWN`, `MANIFEST_TOO_LARGE`, `TOTAL_BYTES_EXCEEDED`;
2: `INSUFFICIENT_CALIBRATION`, `INSUFFICIENT_VIEW_DIRECTIONS`, `DUPLICATE_VIEW_DIRECTION`,
   `REFERENCE_PROJECTION_IGNORED`, `SUPPLEMENT_NOT_ACCEPTED`, `UNKNOWN_WITHOUT_POLICY`,
   `ACCEPTANCE_SUPERSEDED`, `REVISION_DIR_MISMATCH`, `PACKAGE_DIR_WRITABLE`.
Warnings never change `ok`; errors do.

## 5. Acceptance Criteria

- `cargo test --workspace` green (new crate adds ≥ 40 meaningful tests: SHA-256 vectors, PNG
  parser cases, manifest strictness, every level-1 diagnostic class, level-2 completeness
  matrix, determinism/sorting, report shape).
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` clean.
- `michelangelo input validate --package examples/input_packages/valid_full` exits 0,
  `status = valid_complete`, two calibrated views with distinct directions, all hashes verified.
- `.../valid_partial` exits 0 with `status = valid_partial` and
  `INSUFFICIENT_VIEW_DIRECTIONS` in `completeness.missing`; the same command with
  `--require-complete` exits 1.
- A tampered copy (one byte changed in a mask) exits 1 with `HASH_MISMATCH` addressed to the
  exact resource; a package with a path escaping the package directory exits 1 with
  `PATH_OUTSIDE_PACKAGE` and never reads the file.
- A manifest JSON generated from a local PoC dataset (`solver_input/{cameras,views}` facts for
  `harbor_lantern_v01`, 6 calibrated views, 1024×1024, ortho scale 1.0 m, units m, known
  dimensions) validates as `valid_complete` with all six resources hash-verified — run manually
  by the Lead, evidence recorded in this TZ; the dataset stays untracked.
- `docs/specs/INPUT_PACKAGE.md` documents every field/enum/limit/code from §3 and the report and
  exit codes; the JSON Schema parses and mirrors the same required fields/enums
  (`python -m json.tool` on the schema, plus manual field-by-field check by the reviewer).
- No changes in `crates/michelangelo_protocol`, `crates/michelangelo_core`,
  `crates/michelangelo_workspace`, `blender/`, `docs/specs/CORE_PROTOCOL.md`,
  `docs/specs/BLENDER_WORKER.md`.
- Commit on `dev` only, explicit pathspecs, staged diff inspected, no push.

## 6. Evidence

Iteration executed 2026-11-07 on `dev` (base `78429b4`), single change set. Roles: T-A1 worker
(child `1212aca4`, route `minimax-cn/MiniMax-M3.1-Flash`, preset `worker`) implemented the crate;
T-A2 worker (child `72abf069`, same route/preset) implemented CLI, spec and JSON Schema; Lead
reconciled and verified; independent review by child `a2bc3782` (route `xiaomi/mimo-v2.6-pro`,
preset `reviewer`).

- Ladder (Lead, after the final edits): `cargo fmt --all -- --check` exit 0;
  `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `cargo test --workspace`
  green — 387 tests total (input crate: 105; CLI: 23 unit + 10 integration; existing suites
  unchanged); `cargo build -p michelangelo_cli` exit 0.
- CLI acceptance: `input validate --package examples/input_packages/valid_full` → exit 0,
  `valid_complete`, 2 calibrated views, 2 distinct directions, 4/4 resources verified;
  `valid_partial` → exit 0 `valid_partial` with `INSUFFICIENT_VIEW_DIRECTIONS`, and exit 1 with
  `--require-complete`; tampered mask copy → exit 1 with `HASH_MISMATCH` at
  `views[0].mask.sha256`; `input describe` → exit 0.
- Dataset-derived package (local untracked `I:\prog\Michelangelo_PoC\harbor_lantern_v01`,
  `solver_input` only): package built from the 3 calibrated orthographic views
  (`front`/`right`/`top`, 1024×1024, real SHA-256) plus the 3 perspective views as `reference`;
  `input validate` → exit 0, `valid_complete`, 3 calibrated views, 3 distinct directions,
  axes `x,y,z`, **12/12 resources verified**, zero diagnostics. Evidence:
  `target/techlead/IT-4/poc_report.json` (gitignored), generator
  `target/techlead/IT-4/make_poc_package.py`. The dataset itself stays untracked.
- Environment sanity for the next iteration: `python scripts/smoke_blender_scene.py
  --require-blender` (Blender 5.2.3 LTS) → 119 checks passed, 0 skipped.
- Independent reviewer (read-only, no shell tool): confirmed the three contract-critical
  mechanisms with file:line evidence — (1) declared hash compared against a hash computed from
  the read file bytes (`validate.rs:1259,1276,1287`), (2) PNG IHDR-based dimension and color-type
  checks rather than trusting the manifest (`validate.rs:1300,1318,1335`), (3) pre-read path
  classification plus canonicalization/containment (`validate.rs:1716,1120,1157,1174`); scope
  check of `Cargo.toml` members. Reviewer verdict: **GREEN** on the code evidence, with execution
  of the command-level checks delegated to the Lead (its preset has no shell), and an explicit
  conversion condition — the Lead's runs above reproduce exactly the expected 0/`valid_complete`
  and 1/`HASH_MISMATCH` results, and the scope/version checks pass.
- Known process limitation (recorded, not blocking): the `reviewer` preset has no shell tool, so
  command-level evidence in this repository is executed by the Lead and reviewed statically by the
  independent reviewer; a future iteration should give the reviewer a bounded command toolset.
- Commit: this change set itself (explicit pathspecs, `dev`, no push).

### Review log

| attempt | reviewer | route | verdict | findings |
|---|---|---|---|---|
| 1 | child `a2bc3782` (read-only) | `xiaomi/mimo-v2.6-pro`, preset `reviewer` | GREEN (code) + Lead-executed commands | none blocking; process limitation recorded above |

