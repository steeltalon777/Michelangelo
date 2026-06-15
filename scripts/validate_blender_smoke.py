#!/usr/bin/env python3
"""
Validate Michelangelo JSONL smoke output structurally.

Reads JSONL from stdin, parses each line, and validates:
- All lines are valid JSON.
- Project create response is successful.
- blender.run_job(wait=true) response exists.
- Terminal response has result.status == "completed".
- No terminal job.failed event for the success path.
- Expected artifact exists on disk with size > 0 (if --artifact is given).

Exit 0 on pass, 1 on failure.
"""

import argparse
import json
import os
import sys
from pathlib import Path


def validate(lines, workspace_dir, expected_artifact):
    events = []
    responses = []
    errors = []

    for i, line in enumerate(lines):
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as e:
            errors.append(f"Line {i+1}: invalid JSON — {e}")
            continue

        if "event" in obj:
            events.append(obj)
        elif "id" in obj:
            responses.append(obj)
        else:
            errors.append(f"Line {i+1}: line has no 'event' or 'id' — {line[:80]}")

    # Check 1: all lines are valid JSON
    if errors:
        for e in errors:
            print(f"  FAIL: {e}", file=sys.stderr)
        return False

    # Check 2: project create response is successful
    create_resps = [r for r in responses if r.get("result") is not None and
                    isinstance(r.get("result"), dict) and
                    r["result"].get("name") is not None]
    if not create_resps:
        print("  FAIL: no successful project.create response found", file=sys.stderr)
        return False
    print(f"  PASS: project.create response OK (name={create_resps[0]['result'].get('name')})")

    # Check 3: blender.run_job response exists
    job_resps = [r for r in responses if r.get("result") is not None and
                 isinstance(r.get("result"), dict) and
                 "status" in r["result"]]
    if not job_resps:
        print("  FAIL: no blender.run_job response found", file=sys.stderr)
        return False

    final_resp = job_resps[-1]  # last response is the blender.run_job response
    status = final_resp["result"].get("status")

    # Check 4: terminal status is "completed"
    if status != "completed":
        error_msg = final_resp["result"].get("error", "no error detail")
        print(f"  FAIL: terminal job status is '{status}', expected 'completed'. error: {error_msg}", file=sys.stderr)
        return False
    print(f"  PASS: terminal job status = 'completed'")

    # Check 5: no job.failed event
    failed_events = [e for e in events if e.get("event") == "job.failed"]
    if failed_events:
        print(f"  FAIL: found job.failed event(s) — {len(failed_events)}", file=sys.stderr)
        return False

    # Check 6: required events present
    event_names = [e["event"] for e in events]
    for req in ("job.queued", "job.started", "job.completed"):
        if req in event_names:
            print(f"  PASS: event '{req}' found")
        else:
            print(f"  FAIL: missing event '{req}'", file=sys.stderr)
            return False

    # Check 6b: job.completed event contains artifact metadata
    completed_events = [e for e in events if e.get("event") == "job.completed"]
    if completed_events:
        artifacts = completed_events[0].get("data", {}).get("artifacts", [])
        if not artifacts:
            print("  FAIL: job.completed event has no artifacts", file=sys.stderr)
            return False
        for a in artifacts:
            path = a.get("path", "")
            atype = a.get("type", "")
            size = a.get("size_bytes")
            if not path or not atype or size is None:
                print(f"  FAIL: job.completed artifact missing path/type/size_bytes: {a}", file=sys.stderr)
                return False
        print(f"  PASS: job.completed event contains {len(artifacts)} artifact(s) with valid metadata")

    # Check 6c: response contains artifact info in output_json
    resp_artifacts = final_resp.get("result", {}).get("job", {}).get("output_json")
    if resp_artifacts:
        print("  PASS: response contains output_json")
    else:
        print("  WARN: response has no output_json")

    # Check 7: expected artifact on disk
    if expected_artifact:
        art_path = Path(workspace_dir) / expected_artifact
        if not art_path.exists():
            print(f"  FAIL: expected artifact not found — {art_path}", file=sys.stderr)
            return False
        if art_path.stat().st_size == 0:
            print(f"  FAIL: artifact exists but size is 0 — {art_path}", file=sys.stderr)
            return False
        print(f"  PASS: artifact '{expected_artifact}' exists ({art_path.stat().st_size} bytes)")

    # Check 8: response contains result.status == "completed" (redundant but explicit)
    print(f"  PASS: smoke JSONL validation complete ({len(events)} events, {len(responses)} responses)")
    return True


def main():
    parser = argparse.ArgumentParser(description="Validate Michelangelo JSONL smoke output")
    parser.add_argument("--workspace", required=True, help="Temporary workspace directory")
    parser.add_argument("--artifact", default=None, help="Expected artifact path relative to workspace")
    args = parser.parse_args()

    lines = sys.stdin.readlines()
    if not lines:
        print("  FAIL: no output to validate", file=sys.stderr)
        sys.exit(1)

    ok = validate(lines, args.workspace, args.artifact)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
