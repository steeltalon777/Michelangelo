#!/usr/bin/env python3
"""ADR 0005 draft schema check (GLM-1 contracts lane).

Validates every schema in schemas/adr5 against the draft 2020-12 meta-schema,
then checks examples: positive/*.json must pass, negative/*.json must fail.

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
        tag = instance.get("schema")
        validator = validators.get(tag)
        if validator is None:
            failures.append(f"{ex_path.name}: unknown schema tag {tag!r}")
            continue
        errors = list(validator.iter_errors(instance))
        if errors:
            first = sorted(errors, key=lambda e: list(e.absolute_path))[0]
            print(
                f"OK neg    {ex_path.name} (rejected at {list(first.absolute_path)}: {first.message[:90]})"
            )
        else:
            failures.append(f"{ex_path.name}: negative example was ACCEPTED but must fail")

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
