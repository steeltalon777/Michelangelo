#!/usr/bin/env python3
"""ADR 0005 draft schema check (GLM-1 contracts lane).

Validates every schema in schemas/adr5 against the draft 2020-12 meta-schema,
then checks examples: positive/*.json must pass, negative/*.json must fail.
Every negative fixture is additionally pinned to the exact instance path of the
rule it was written for (NEGATIVE_EXPECTED_PATH): failing on a different —
typically earlier — rule counts as a failure, so a fixture can never "pass" by
accident (e.g. on a digest pattern before its intended invariant).

Run from the repository root:
    python docs/adr5-streams/contracts/tools/check_schemas.py

Requires: python >= 3.10, jsonschema >= 4.18.
Exit code 0 = all checks passed; 1 = failures (details on stderr).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from jsonschema import Draft202012Validator
from jsonschema.validators import validator_for

ROOT = Path(__file__).resolve().parents[4]
SCHEMA_DIR = ROOT / "schemas" / "adr5"
POSITIVE = SCHEMA_DIR / "examples" / "positive"
NEGATIVE = SCHEMA_DIR / "examples" / "negative"

# Negative fixture -> absolute_path of the FIRST error (as sorted below) that
# must reject it. Each fixture exists to demonstrate one specific contract
# rule; if it fails on another rule (a digest pattern, a missing required
# field, ...) the fixture itself is broken. Every file in NEGATIVE must have
# an entry here (an entry is required, not optional).
NEGATIVE_EXPECTED_PATH: dict[str, list] = {
    "acceptance_missing_report.json": ["basis"],
    "batch_plan_not_acyclic.json": ["cycle_check", "acyclic"],
    # `required` always reports on the containing object, so the pin is the
    # root path; the OK line shows the message ('expected_current_revision'
    # is a required property).
    "bundle_missing_expected_current_revision.json": [],
    "bundle_path_escape.json": ["artifacts", 0, "path"],
    "lock_floating_latest.json": ["floating_latest_allowed"],
    "lock_tool_version_latest.json": ["tool_versions", "blender"],
    "profile_unknown_check.json": ["required_checks", 1],
    "recipe_arbitrary_code.json": ["scope", "allow_arbitrary_code"],
    "report_not_required_missing_ref.json": ["checks", 1],
    "report_not_required_verdict.json": ["verdict"],
    "source_set_held_out_build_role.json": ["sources", 0, "roles", 0],
    "source_set_invalid_role.json": ["sources", 0, "roles", 0],
    "source_set_ivr_not_held_out.json": ["sources", 0, "usage"],
    "source_snapshot_fbx.json": ["source", "original_format"],
    "variant_request_scope_field.json": [],
}


def load(path: Path):
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def main() -> int:
    if not SCHEMA_DIR.is_dir():
        print(f"schemas dir not found: {SCHEMA_DIR}", file=sys.stderr)
        return 1

    schemas = sorted(SCHEMA_DIR.glob("*.schema.json"))
    if not schemas:
        print(f"no schemas found in {SCHEMA_DIR}", file=sys.stderr)
        return 1

    failures: list[str] = []
    validators: dict[str, Draft202012Validator] = {}

    for schema_path in schemas:
        schema = load(schema_path)
        cls = validator_for(schema)
        cls.check_schema(schema)
        name = schema.get("properties", {}).get("schema", {}).get("const")
        if not name:
            failures.append(f"{schema_path.name}: missing top-level schema const")
            continue
        if not name.endswith("/draft-0.1"):
            failures.append(f"{schema_path.name}: schema tag {name!r} must end with /draft-0.1")
        validators[name] = cls(schema)
        print(f"OK meta   {schema_path.name} -> {name}")

    def instance_files(directory: Path) -> list[Path]:
        return sorted(directory.glob("*.json")) if directory.is_dir() else []

    for ex_path in instance_files(POSITIVE):
        instance = load(ex_path)
        tag = instance.get("schema")
        validator = validators.get(tag)
        if validator is None:
            failures.append(f"{ex_path.name}: unknown schema tag {tag!r}")
            continue
        errors = sorted(validator.iter_errors(instance), key=lambda e: list(e.absolute_path))
        if errors:
            first = errors[0]
            failures.append(
                f"{ex_path.name}: positive example FAILED at {list(first.absolute_path)}: {first.message}"
            )
        else:
            print(f"OK pos    {ex_path.name}")

    for ex_path in instance_files(NEGATIVE):
        instance = load(ex_path)
        expected = NEGATIVE_EXPECTED_PATH.get(ex_path.name)
        if expected is None:
            failures.append(
                f"{ex_path.name}: no NEGATIVE_EXPECTED_PATH entry — pin the rule this fixture must fail on"
            )
            continue
        tag = instance.get("schema")
        validator = validators.get(tag)
        if validator is None:
            failures.append(f"{ex_path.name}: unknown schema tag {tag!r}")
            continue
        errors = list(validator.iter_errors(instance))
        if not errors:
            failures.append(f"{ex_path.name}: negative example was ACCEPTED but must fail")
            continue
        first = sorted(errors, key=lambda e: list(e.absolute_path))[0]
        path = list(first.absolute_path)
        if path != expected:
            failures.append(
                f"{ex_path.name}: rejected at {path} but must fail at {expected}: {first.message[:90]}"
            )
        else:
            print(f"OK neg    {ex_path.name} (rejected at {path}: {first.message[:90]})")

    print()
    if failures:
        for line in failures:
            print(f"FAIL {line}", file=sys.stderr)
        print(f"RESULT: {len(failures)} failure(s)", file=sys.stderr)
        return 1
    print("RESULT: all schema/example checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
