#!/usr/bin/env python3
"""Run the full Stage B benchmark matrix and aggregate the summaries.

Cases (ADR §13.2 + the local PoC datasets):
  harbor_lantern_v01  composite lamp (dataset)
  naval_cannon_v01    blind recess / silhouette limit (dataset)
  wagon_wheel_v01     through-holes, thin spokes (dataset)
  alien_plant         novel organic object (synthetic)
  blind_recess        control body with a blind recess (synthetic)
  composite_lamp      organic lamp on a mechanical base (synthetic)

Every dataset case runs twice: hull only, and with authored supplements. Synthetic cases use the
matching `tools/bench/supplements/<case>.json` when present.

Usage: python tools/bench/run_all.py [--resolution 96] [--only case1,case2] [--keep]
Outputs `target/techlead/IT-5/bench/matrix-summary.json`.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
POC = Path(os.environ.get("MICHELANGELO_POC_DATASETS", r"I:\prog\Michelangelo_PoC"))
SYNTHETIC = REPO / "target" / "techlead" / "IT-5" / "synthetic"
OUT = REPO / "target" / "techlead" / "IT-5" / "bench"

DATASET_CASES = [
    ("harbor_lantern_v01", "harbor_lantern"),
    ("naval_cannon_v01", "naval_cannon"),
    ("wagon_wheel_v01", "wagon_wheel"),
]
SYNTHETIC_CASES = [
    ("alien_plant", "alien_plant"),
    ("blind_recess", "blind_recess"),
    ("composite_lamp", "composite_lamp"),
]


def run_case(label: str, source_args, supplements: Path | None, resolution: int, keep: bool):
    workspace = OUT / label / "workspace"
    argv = [
        sys.executable,
        str(REPO / "tools" / "bench" / "run_bench.py"),
        *source_args,
        "--workspace",
        str(workspace),
        "--resolution",
        str(resolution),
        "--summary",
        str(OUT / f"{label}-summary.json"),
    ]
    if supplements is not None and supplements.is_file():
        argv.extend(["--supplements", str(supplements)])
    if keep:
        argv.append("--keep")
    result = subprocess.run([str(part) for part in argv], capture_output=True, text=True)
    print(result.stdout)
    if result.stderr.strip():
        print(result.stderr.strip(), file=sys.stderr)
    summary_path = OUT / f"{label}-summary.json"
    summary = json.loads(summary_path.read_text(encoding="utf-8")) if summary_path.is_file() else {}
    return result.returncode, summary


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--resolution", type=int, default=96)
    parser.add_argument("--only", default=None)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args(argv)
    only = set(args.only.split(",")) if args.only else None
    OUT.mkdir(parents=True, exist_ok=True)

    rows = []
    failures = 0
    for dataset, supplement_name in DATASET_CASES:
        if only and dataset not in only:
            continue
        dataset_dir = POC / dataset
        if not dataset_dir.is_dir():
            print(f"[matrix] skip {dataset}: {dataset_dir} not found", file=sys.stderr)
            continue
        for variant, supplements in (
            ("hull", None),
            ("supplements", REPO / "tools" / "bench" / "supplements" / f"{supplement_name}.json"),
        ):
            label = f"{dataset}-{variant}"
            code, summary = run_case(
                label, ["--dataset", str(dataset_dir)], supplements, args.resolution, args.keep
            )
            failures += 1 if code != 0 else 0
            rows.append(
                {
                    "case": dataset,
                    "variant": variant,
                    "label": label,
                    "exit_code": code,
                    "package_validation": summary.get("package_validation"),
                    "run_status": summary.get("run_status"),
                    "quality_status": summary.get("quality_status"),
                    "evaluation_quality_status": summary.get("evaluation_quality_status"),
                    "source_mean_iou": summary.get("source_iou", {}).get("mean"),
                    "source_min_iou": summary.get("source_iou", {}).get("min"),
                    "evaluation_mean_iou": (summary.get("evaluation_iou") or {}).get("mean"),
                    "measurements": summary.get("measurements"),
                    "supplements_applied": [item.get("supplement_id") for item in summary.get("supplements_applied", [])],
                    "unknowns": summary.get("unknowns"),
                }
            )

    for case, supplement_name in SYNTHETIC_CASES:
        if only and case not in only:
            continue
        case_dir = SYNTHETIC / case
        if not case_dir.is_dir():
            print(f"[matrix] skip synthetic {case}: {case_dir} not found", file=sys.stderr)
            continue
        for variant, supplements in (
            ("hull", None),
            ("supplements", REPO / "tools" / "bench" / "supplements" / f"{supplement_name}.json"),
        ):
            if variant == "supplements" and supplements is not None and not supplements.is_file():
                continue
            label = f"{case}-{variant}"
            code, summary = run_case(
                label, ["--synthetic", str(case_dir)], supplements, args.resolution, args.keep
            )
            failures += 1 if code != 0 else 0
            rows.append(
                {
                    "case": case,
                    "variant": variant,
                    "label": label,
                    "exit_code": code,
                    "package_validation": summary.get("package_validation"),
                    "run_status": summary.get("run_status"),
                    "quality_status": summary.get("quality_status"),
                    "evaluation_quality_status": summary.get("evaluation_quality_status"),
                    "source_mean_iou": summary.get("source_iou", {}).get("mean"),
                    "source_min_iou": summary.get("source_iou", {}).get("min"),
                    "evaluation_mean_iou": (summary.get("evaluation_iou") or {}).get("mean"),
                    "measurements": summary.get("measurements"),
                    "supplements_applied": [item.get("supplement_id") for item in summary.get("supplements_applied", [])],
                    "unknowns": summary.get("unknowns"),
                }
            )

    matrix_path = OUT / "matrix-summary.json"
    matrix_path.write_text(
        json.dumps({"schema": "michelangelo.bench-matrix/0", "resolution": args.resolution, "rows": rows}, indent=2)
        + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"[matrix] {len(rows)} runs, {failures} failed; summary: {matrix_path}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
