# Input Package Contract (v0)

Normative contract of the Michelangelo **input package**: a self-contained, versioned bundle of
calibrated views (image + mask + camera) plus the structured information needed to complete a
partial reconstruction.

Status: implemented in Stage A (crate `michelangelo_input`, CLI `michelangelo input`, JSON Schema
`docs/specs/schemas/input_package_v0.schema.json`). The manifest schema is
`michelangelo.input-package/0`; the validation report schema is
`michelangelo.input-package-report/0`.

## Purpose and ADR reference

ADR 0004 (`docs/adr`, §13.1 *Stage A*) requires a validated input package **before** any
reconstruction work: the mandatory information for the supported orthographic mode, the handling of
partial supplements, view registration, conflict and revision policy. ADR §6 defines the two
validation levels implemented here:

- **Level 1 — format / safety**: the package is well formed, safe to read and internally
  consistent.
- **Level 2 — sufficiency**: the package contains enough information for a *metric* reconstruction.

A partial package is **not** an error: independent valid parts stay usable and the report states
exactly which information blocks the metric reconstruction (ADR §6.1).

## Package directory layout

```text
<workspace>/input/r0001/package.json
<workspace>/input/r0001/images/<view_id>.png      # RGB8 or RGBA8
<workspace>/input/r0001/masks/<view_id>.png       # gray8 or RGBA8
```

- The manifest is always `package.json` inside the package directory.
- Resource paths in the manifest are **relative to the package directory**, use `/`, are at most
  1024 characters and contain no `..`, no leading `/`, no backslash, no drive/UNC form and no
  NUL byte. After symlink resolution they must stay **inside** the canonicalized package directory
  and point at a regular file.
- The package directory name, when it matches `^r[0-9]{4}$`, must equal the manifest `revision`
  (`r0001` ↔ `revision: 1`); otherwise a level-2 warning `REVISION_DIR_MISMATCH` is reported.
- Images and masks are **PNG only**. JPEG or any other container is rejected.

## Manifest v0 (`michelangelo.input-package/0`)

The manifest is a JSON object. **Unknown fields are rejected** (`deny_unknown_fields` in the Rust
types, `additionalProperties: false` in the JSON Schema).

### Top level

| Field | Type | Rules |
|---|---|---|
| `schema` | string | exactly `michelangelo.input-package/0` |
| `package_id` | string | `^[a-z0-9][a-z0-9._-]{0,63}$` |
| `object` | object | `name` 1..200 chars; optional `notes` ≤ 2000 chars |
| `revision` | integer | 1..1 000 000 |
| `units` | string | `m` | `cm` | `mm` — the unit of every length in the manifest |
| `coordinate_system` | object | `handedness`: `right`; `up`, `right`, `forward`: signed axes, pairwise distinct |
| `views` | array | 1..64 entries, `view_id` unique |
| `parts` | array | 0..256 entries, `part_id` unique (optional) |
| `supplements` | array | 0..256 entries, `supplement_id` unique (optional) |
| `unknowns` | array | 0..256 entries, `id` unique (optional) |
| `measurements` | array | 0..64 entries, `id` unique (optional) |
| `uncertainty_policy` | object | optional, see below |
| `provenance` | object | **required**; `origin` ∈ `human` | `ai_author` | `mixed`; optional `detail` ≤ 2000 |
| `acceptance` | array | 0..512 entries (optional) |

> **Unit note.** The length fields keep the historical `_m` suffix (`ortho_scale_m`,
> `value_m`, …) even when `units` is `cm` or `mm`. The `units` field declares the real unit
> of the numbers; the suffix is a naming convention only and is not validated.

### View

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

- `view_id`: `^[a-z][a-z0-9_]{0,31}$`, unique inside the package.
- `kind`: `calibrated` (usable as a metric constraint) or `reference` (visual reference only).
- `image` and `mask` are both **required**. `sha256` is required (64 lowercase hex characters).
  The hash is verified for **all** views, `calibrated` and `reference` alike; `--no-hash-verify`
  downgrades a mismatch from an error to a warning.
- `width_px` / `height_px`: 1..16 384 and must equal the PNG IHDR dimensions.
- `projection`: **required** for `calibrated`, optional for `reference` (a projection on a
  reference view is ignored and reported as level-2 warning `REFERENCE_PROJECTION_IGNORED`).
  - `type` is exactly `orthographic`.
  - `location_m`, `look_at_m`, `up_world`: finite, each component `|x| ≤ 1e6`.
  - `up_world` length ∈ [0.99, 1.01]; `look_at_m - location_m` must be non-zero and not
    parallel to `up_world` (`|dot(unit(dir), unit(up))| < 0.999`).
  - `ortho_scale_m`: `> 0`, `≤ 1e6`; it is the **visible world width** of the image.
    Visible height = `ortho_scale_m * height_px / width_px`.
  - `shift_x` / `shift_y`: ∈ [-10, 10] (default `0`).
  - `clip_start_m`: `> 0` (default `0.1`); `clip_end_m`: `> clip_start_m` (default `1000`).
- `provenance` (optional): `origin` ∈ `human` | `ai_author` | `mixed`, optional `detail`
  ≤ 1000 chars.

### Part

```json
{"part_id": "lid", "label": "lid", "parent": null, "protected": false}
```

- `part_id` and `label`: `^[A-Za-z0-9][A-Za-z0-9 _.-]{0,63}$`; `part_id` unique.
- `parent`: optional reference to an existing `part_id`; the parent graph must be acyclic.
- `protected`: optional bool (default `false`); used by a later stage, validated here for shape
  only.

### Supplement (authored completion of hidden shape)

```json
{
  "supplement_id": "lid_thickness",
  "part_id": "lid",
  "kind": "thickness",
  "geometry": {"value_m": 0.012},
  "provenance": {"origin": "human", "detail": "author decision"}
}
```

- `supplement_id`: unique, non-empty (v0 constrains the charset only through uniqueness).
- `part_id`: must reference an existing part (level-1 error `SUPPLEMENT_PART_UNKNOWN`).
- `kind` ∈ `thickness` | `curve` | `section` | `profile` | `closure` | `cavity` |
  `connection` | `symmetry`.
- `geometry`: a **non-empty JSON object**; all numbers finite with `|x| ≤ 1e6`; nesting depth
  ≤ 8; serialized size ≤ 64 KiB.
- Required numbers per kind (level-1 error `SUPPLEMENT_GEOMETRY_INVALID`):
  - `thickness`: `value_m` > 0.
  - `curve`: `points_m` with 2..256 points, each a finite `[x, y, z]`.
  - `section` / `profile`: `points_m` with 2..256 finite 2D or 3D points, optional `plane`
    ∈ `xy` | `yz` | `xz`.
  - `cavity`: `bbox_m` = `[x, y, z]` sizes, all > 0, **or** `points_m` with ≥ 4 points.
  - `closure` / `connection`: `from` and `to`, each a finite `[x, y, z]`.
  - `symmetry`: `axis` ∈ `x` | `y` | `z`, optional finite `plane_offset_m`.
- `provenance` is required: `origin` ∈ `human` | `ai_author` | `computed`.

### Unknown

```json
{"id": "u_lid_inside", "description": "hidden lid interior", "affected_parts": ["lid"], "policy": "stub_allowed"}
```

- `id`: unique, `^[a-z][a-z0-9_]{0,63}$`.
- `description`: 1..500 chars.
- `affected_parts`: 0..64 existing part ids.
- `policy` ∈ `author_required` | `rule_allowed` | `stub_allowed`; a missing policy is a
  level-2 warning (`UNKNOWN_WITHOUT_POLICY`).

### Measurement (known dimension)

```json
{"id": "height", "label": "overall height", "kind": "height", "value_m": 0.645986, "tolerance_m": 0.002, "origin": "human"}
```

- `id`: unique, `^[a-z][a-z0-9_]{0,63}$`; `label` optional.
- `kind` ∈ `height` | `width` | `depth` | `distance` | `bbox`.
- `value_m`: finite, > 0; `tolerance_m`: finite, ≥ 0 (default `0`).
- `origin` ∈ `human` | `ai_author` | `computed`.

### Uncertainty policy

```json
{
  "allowed_completion_rules": ["thickness", "closure", "symmetry", "primitive_fill"],
  "stub_policy": "allowed",
  "require_author_acceptance": true
}
```

- `allowed_completion_rules`: 0..8 entries from the supplement `kind` set plus `primitive_fill`.
- `stub_policy` ∈ `allowed` | `forbidden`.
- `require_author_acceptance`: bool (default `true`).

### Acceptance

```json
{"target": "supplements:lid_thickness", "status": "accepted", "by": "owner", "at": "2026-11-07T12:00:00Z"}
```

- `target`: `<views|parts|supplements|unknowns|measurements>:<id>`, must reference an existing
  element (level-1 error `ACCEPTANCE_TARGET_UNKNOWN`).
- `status` ∈ `proposed` | `accepted` | `needs_revision`.
- `by`: 1..128 chars; `at`: RFC 3339 timestamp.
- Duplicate targets: only the **last** entry is authoritative. An earlier `accepted` followed by
  anything else is a level-2 warning (`ACCEPTANCE_SUPERSEDED`).
- If `uncertainty_policy.require_author_acceptance` is true, every supplement with
  `provenance.origin = ai_author` needs an `accepted` acceptance entry (level-2 warning
  `SUPPLEMENT_NOT_ACCEPTED`).

## Pixel and projection convention

Fixed in v0; it is not expressible differently:

- Image origin is **top-left**, `u` points right, `v` points **down**.
- The camera looks at `look_at_m`;
  `u_axis = normalize(cross(view_dir, up_world))`, `v_axis = up_world`.
- `u_px = (0.5 + u_cam / ortho_scale_m) * width_px`
- `v_px = (0.5 - v_cam / ortho_scale_m) * width_px`

where `u_cam` / `v_cam` are camera-space coordinates in package units. This is the same mapping
as `BLENDER_WORKER.md` § *Camera projection contract*, with the v axis flipped to the image
convention (top-left origin).

## Mask convention

- A mask is a PNG with bit depth 8 and either color type 0 (grayscale) or color type 6 (RGBA).
- `format: "grayscale8"` pairs with `foreground: "white"`: pixel luminance `0` = background,
  `≥ 128` = object.
- `format: "rgba8"` pairs with `foreground: "alpha"`: alpha `0` = background, `≥ 128` = object.
- Mask dimensions must equal the dimensions of the image of the same view.

## Validation levels

### Level 1 — format / safety

- schema id, JSON syntax, types, enums, ranges, unique ids, reference integrity;
- unknown fields are rejected (`SCHEMA_UNKNOWN_FIELD`);
- resource existence, regular-file check, containment inside the package, PNG container format
  (signature, IHDR, bit depth 8, color type, IEND), dimensions and SHA-256 hashes;
- limits: manifest ≤ 1 MiB, single resource ≤ 64 MiB, total referenced bytes ≤ 512 MiB,
  ≤ 64 views, ≤ 512 acceptance entries, path ≤ 1024 chars, dimensions ≤ 16 384 px;
- no path traversal, no non-finite numbers.

### Level 2 — sufficiency

- The **measurable set** is the set of `calibrated` views that have a valid projection, image and
  mask.
- A package is `complete` when it has **≥ 2 calibrated views** with **pairwise distinct
  directions** (`|dot(unit(d1), unit(d2))| ≤ 0.9`, i.e. at least ~26° apart) and those
  directions span **at least two coordinate axes** (the dominant axis of each direction cluster).
- Otherwise the package is `partial` and the report lists the missing reasons:
  - `INSUFFICIENT_CALIBRATION` — no usable calibrated view at all;
  - `INSUFFICIENT_VIEW_DIRECTIONS` — fewer than two independent directions or fewer than two
    spanned axes.
- Two calibrated views with nearly the same direction (`|dot| > 0.999`) are a level-2 warning
  (`DUPLICATE_VIEW_DIRECTION`): they are not independent silhouette constraints (ADR §9.1).
- Level 2 also checks the acceptance rules and the reference-projection rule above.

**Severity policy.** A level-1 error always makes the package `invalid`. Level-2 findings are
warnings by default and only become errors when `--require-complete` is passed — an incomplete
package does not block the independent valid parts (ADR §6.1).

## Diagnostic codes

| Level | Code | Severity | Meaning |
|---|---|---|---|
| 1 | `SCHEMA_UNKNOWN_FIELD` | error | the manifest carries a field that is not part of v0 |
| 1 | `SCHEMA_TYPE` | error | a field has the wrong JSON type (also: the manifest is not JSON, or a calibrated view has no projection) |
| 1 | `SCHEMA_ENUM` | error | a field has a value outside its enumeration (also: the schema id is wrong) |
| 1 | `SCHEMA_RANGE` | error | a number, length or array size is outside its permitted range |
| 1 | `ID_INVALID` | error | an id does not match its pattern (or is empty) |
| 1 | `ID_DUPLICATE` | error | the same id is used twice in one collection |
| 1 | `PATH_INVALID` | error | the resource path is empty, too long, contains NUL, a backslash or a drive letter |
| 1 | `PATH_OUTSIDE_PACKAGE` | error | the resource path is absolute or contains `..`; the file is **never read** |
| 1 | `RESOURCE_MISSING` | error | the manifest or a resource cannot be found or read |
| 1 | `RESOURCE_NOT_REGULAR` | error | the resource is not a regular file |
| 1 | `RESOURCE_TOO_LARGE` | error | the resource exceeds 64 MiB |
| 1 | `PNG_SIGNATURE` | error | the file does not start with the PNG signature |
| 1 | `PNG_IHDR_INVALID` | error | the IHDR chunk is malformed, has a bad CRC, an unknown critical chunk is present or the file is truncated |
| 1 | `PNG_COLOR_TYPE` | error | the PNG color type is not allowed for this resource kind |
| 1 | `PNG_DIMENSION_MISMATCH` | error | the declared dimensions differ from the PNG IHDR dimensions |
| 1 | `PNG_TRUNCATED` | error | the PNG file ends in the middle of a chunk or has no IEND |
| 1 | `HASH_MISMATCH` | error (warning with `--no-hash-verify`) | the file content does not hash to the declared `sha256` |
| 1 | `HASH_INVALID` | error | `sha256` is not 64 lowercase hexadecimal characters |
| 1 | `REFERENCE_UNKNOWN` | error | a reference (`parent`, `affected_parts`) points at a non-existing element |
| 1 | `PARENT_CYCLE` | error | the part parent graph contains a cycle |
| 1 | `GEOMETRY_INVALID` | error | the supplement geometry is not a non-empty object, is too large, too deep or holds non-finite numbers |
| 1 | `SUPPLEMENT_GEOMETRY_INVALID` | error | the geometry does not satisfy the requirements of its `kind` |
| 1 | `SUPPLEMENT_PART_UNKNOWN` | error | the supplement references a part that does not exist |
| 1 | `MEASUREMENT_INVALID` | error | `value_m` or `tolerance_m` is not a finite number in range |
| 1 | `ACCEPTANCE_TARGET_UNKNOWN` | error | the acceptance target does not reference an existing element |
| 1 | `MANIFEST_TOO_LARGE` | error | `package.json` exceeds 1 MiB |
| 1 | `TOTAL_BYTES_EXCEEDED` | error | the referenced resources total more than 512 MiB |
| 2 | `INSUFFICIENT_CALIBRATION` | warning (error with `--require-complete`) | no usable calibrated view |
| 2 | `INSUFFICIENT_VIEW_DIRECTIONS` | warning (error with `--require-complete`) | fewer than two independent view directions or fewer than two spanned axes |
| 2 | `DUPLICATE_VIEW_DIRECTION` | warning | two calibrated views share the same direction |
| 2 | `REFERENCE_PROJECTION_IGNORED` | warning | a reference view carries a projection that is not used |
| 2 | `SUPPLEMENT_NOT_ACCEPTED` | warning | an AI-authored supplement has no `accepted` acceptance entry |
| 2 | `UNKNOWN_WITHOUT_POLICY` | warning | an unknown has no resolution policy |
| 2 | `ACCEPTANCE_SUPERSEDED` | warning | a previous acceptance of the same target was superseded |
| 2 | `REVISION_DIR_MISMATCH` | warning | the package directory name does not match the manifest revision |
| 2 | `PACKAGE_DIR_WRITABLE` | warning | the directory name is not a revision name and the manifest is newer than all resources |

Diagnostics are deterministic and sorted by `(level, address, code)`. Addresses are JSON-path
like `views[2].mask.path` or `supplements[0].geometry.value_m`.

## Report JSON (`michelangelo.input-package-report/0`)

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
    "axes_covered": ["x", "y"],
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

| Key | Type | Meaning |
|---|---|---|
| `schema` | string | always `michelangelo.input-package-report/0` |
| `package_id` | string | the manifest package id (empty when the manifest is unreadable) |
| `revision` | integer | the manifest revision (0 when the manifest is unreadable) |
| `status` | string | `valid_complete`, `valid_partial` or `invalid` |
| `levels.format_safety` | object | `{ok, errors, warnings}` for level 1 |
| `levels.sufficiency` | object | `{ok, errors, warnings}` for level 2 |
| `completeness.status` | string | `complete` or `partial` |
| `completeness.calibrated_views` | integer | number of usable calibrated views |
| `completeness.distinct_directions` | integer | number of independent direction clusters |
| `completeness.axes_covered` | array of string | axes spanned by the clusters (`x`, `y`, `z`) |
| `completeness.missing` | array of string | diagnostic codes that describe what is missing |
| `diagnostics[]` | array | `{level, code, severity, address, message}`, sorted by `(level, address, code)` |
| `resources[]` | array | `{address, path, sha256, bytes, verified}` — one entry per referenced resource |

`status` is `invalid` when a level-1 error exists (or when a level-2 error exists under
`--require-complete`); otherwise it is `valid_complete` or `valid_partial`.

## CLI

```text
michelangelo input validate --package <DIR> [--require-complete] [--no-hash-verify]
                            [--report <FILE>] [--quiet]
michelangelo input describe --package <DIR>
```

- `validate` prints the **report JSON to stdout**: pretty-printed with two-space indentation,
  UTF-8, terminated by a newline. The JSON is the stable machine interface; human diagnostics go
  to stderr.
- `--report <FILE>` writes the same JSON to a file. The path must **not** be inside the package
  revision directory — the CLI never writes into a package. A path inside the package is refused
  with exit code 2.
- `--quiet` prints nothing on stdout; a single one-line summary is written to stderr.
- `--require-complete` turns a partial package into a failure (exit 1).
- `--no-hash-verify` keeps the hash check but downgrades a mismatch from an error to a warning.
- `describe` prints a short human summary (package id, revision, status, completeness, view ids
  and kinds, part / supplement / unknown / measurement counts) and exits 0 for a level-1 valid
  package, 1 otherwise.
- Both commands work without an open project; no `--workspace` is needed because the package
  directory is given directly.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | the package is valid (complete or partial; warnings are allowed) |
| `1` | the package is invalid, or it is partial while `--require-complete` is set |
| `2` | usage / IO error: the package directory does not exist, the manifest is unreadable, the report path is inside the package or the report file cannot be written |

The JSONL protocol mode (`michelangelo core stdio`) is untouched by these commands: stdout
carries protocol messages only, and every human diagnostic of the CLI goes to stderr.

## Revision and immutability conventions

- A package revision lives in `<workspace>/input/<revision>/` and the directory is named
  `r0001`, `r0002`, … (`revision` 1, 2, …).
- A revision directory is an **immutable snapshot**. The validator never writes into the package,
  and it reports the level-2 warning `PACKAGE_DIR_WRITABLE` when the directory name is not a
  revision name and `package.json` is newer than all resources — a documented convention, not an
  ACL enforcement.
- A new revision is a new directory with a new `revision` number; an existing revision is never
  edited in place.
- The manifest carries the SHA-256 of every resource, so a snapshot can be verified at any time.

## Scope

### Done (Stage A)

- strict, versioned JSON manifest + JSON Schema + Rust validator (levels 1 and 2);
- safe resource resolution (containment, regular files, path rules) and SHA-256 verification;
- deterministic, addressable diagnostics and a stable report JSON;
- the `michelangelo input validate` / `input describe` CLI commands.

### Not done (out of scope)

- **no perspective views** — v0 supports orthographic projections only;
- **no pixel analysis** — no bounding box, connected components, contours or IoU; the validator
  checks the PNG *container* only (Stage B / solver work);
- **no wire methods** — no JSONL method, DTO, protocol error code or event was added; the
  functionality is reachable only through the CLI, and `CORE_PROTOCOL` / `BLENDER_WORKER` are
  unchanged;
- no Blender operations, no SQLite migration, no Asset Graph indexing, no package import from the
  local PoC dataset.
