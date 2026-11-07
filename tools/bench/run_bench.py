#!/usr/bin/env python3
"""Run the Stage B reconstruction benchmark on one dataset-shaped case.

For every case the script:
  1. creates (or reuses) a project workspace,
  2. converts the case's `solver_input/` into a Stage A package and its `evaluation/`
     into a held-out evaluation package (`tools/bench/package_from_dataset.py`),
  3. runs `michelangelo reconstruct run` and `michelangelo reconstruct score`,
  4. checks the acceptance thresholds, prints a summary table and writes a JSON summary.

The dataset content (the untracked PoC directories) is never copied into the repository:
by default the workspace lives under `target/techlead/IT-5/bench/<case>/workspace`.

Examples:
  python tools/bench/run_bench.py --dataset I:\\prog\\Michelangelo_PoC\\harbor_lantern_v01
  python tools/bench/run_bench.py --dataset ... --supplements tools/bench/supplements/harbor_lantern.json
  python tools/bench/run_bench.py --synthetic target/techlead/IT-5/synthetic/blind_recess
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CONVERTER = REPO / "tools" / "bench" / "package_from_dataset.py"
DEFAULT_CLI = REPO / "target" / "debug" / "michelangelo.exe"


def resolve_blender(value):
    if value:
        return value
    env = os.environ.get("BLENDER_BIN")
    if env:
        return env
    candidate = Path(r"I:\Tools\Blender\blender.exe")
    return str(candidate) if candidate.is_file() else "blender"


def run(command, *, cwd=None, timeout=None):
    result = subprocess.run(
        [str(part) for part in command],
        capture_output=True,
        text=True,
        cwd=cwd,
        timeout=timeout,
    )
    return result


def ensure_workspace(cli: Path, workspace: Path, name: str):
    meta = workspace / ".michelangelo" / "project.json"
    if meta.is_file():
        return
    workspace.mkdir(parents=True, exist_ok=True)
    payload = (
        json.dumps(
            {
                "id": "c1",
                "method": "project.create",
                "params": {"path": str(workspace), "name": name},
            }
        )
        + "\n"
    )
    result = subprocess.run(
        [str(cli), "core", "stdio"], input=payload, text=True, capture_output=True
    )
    responses = [line for line in result.stdout.splitlines() if line.strip()]
    if not responses or "error" in json.loads(responses[0]):
        raise SystemExit(f"[bench] project.create failed: {result.stdout}\n{result.stderr}")


def convert(source_dir: Path, out_dir: Path, *, kind: str, supplements: Path | None):
    command = [
        sys.executable,
        str(CONVERTER),
        "--dataset",
        str(source_dir),
        "--out",
        str(out_dir),
        "--source",
        kind,
    ]
    if supplements is not None and kind == "solver":
        command.extend(["--supplements", str(supplements)])
    result = run(command)
    if result.returncode != 0:
        raise SystemExit(f"[bench] package conversion failed:\n{result.stdout}\n{result.stderr}")
    return result.stdout.strip()


def read_json(path: Path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Stage B reconstruction benchmark")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--dataset", type=Path, help="local PoC dataset directory")
    source.add_argument("--synthetic", type=Path, help="synthetic dataset-shaped case directory")
    parser.add_argument("--case-name", default=None)
    parser.add_argument("--workspace", type=Path, default=None)
    parser.add_argument("--cli", type=Path, default=None)
    parser.add_argument("--blender", default=None)
    parser.add_argument("--resolution", type=int, default=96)
    parser.add_argument("--timeout-ms", type=int, default=300000)
    parser.add_argument("--supplements", type=Path, default=None)
    parser.add_argument("--summary", type=Path, default=None)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args(argv)

    cli = (args.cli or DEFAULT_CLI).resolve()
    if not cli.is_file():
        raise SystemExit(f"[bench] CLI not found: {cli} (build with cargo build -p michelangelo_cli)")

    case_dir = (args.dataset or args.synthetic).resolve()
    case_name = args.case_name or case_dir.name
    workspace = args.workspace
    temp_workspace = None
    if workspace is None:
        temp_workspace = tempfile.mkdtemp(prefix=f"bench-{case_name}-", dir=str(REPO / "target" / "techlead"))
        workspace = Path(temp_workspace)
    workspace = workspace.resolve()
    ensure_workspace(cli, workspace, f"bench-{case_name}")

    package_dir = workspace / "input" / "r0001"
    evaluation_dir = workspace / "evaluation" / "r0001"
    if package_dir.exists():
        shutil.rmtree(package_dir)
    if evaluation_dir.exists():
        shutil.rmtree(evaluation_dir)
    convert(case_dir, package_dir, kind="solver", supplements=args.supplements)
    eval_stdout = ""
    has_evaluation = (case_dir / "evaluation" / "cameras.json").is_file()
    if has_evaluation:
        eval_stdout = convert(case_dir, evaluation_dir, kind="evaluation", supplements=None)

    blender = resolve_blender(args.blender)
    run_args = [
        str(cli),
        "reconstruct",
        "run",
        "--package",
        str(package_dir),
        "--workspace",
        str(workspace),
        "--resolution",
        str(args.resolution),
        "--blender",
        blender,
        "--timeout-ms",
        str(args.timeout_ms),
        "--json",
    ]
    run_result = run(run_args, timeout=args.timeout_ms / 1000.0 + 120)
    if run_result.returncode not in (0, 1):
        raise SystemExit(
            f"[bench] reconstruct run failed ({run_result.returncode}):\n"
            f"{run_result.stdout[-4000:]}\n{run_result.stderr[-4000:]}"
        )
    report_path = None
    for line in run_result.stdout.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                candidate = json.loads(line)
            except json.JSONDecodeError:
                continue
            if candidate.get("schema") == "michelangelo.reconstruction-report/0":
                report_path = candidate
    run_dirs = sorted((workspace / "recon" / "r0001").glob("run-*"))
    if not run_dirs:
        raise SystemExit("[bench] no run directory produced")
    run_dir = run_dirs[-1]
    report = report_path or read_json(run_dir / "report.json")

    evaluation_quality = None
    score_exit = None
    if has_evaluation:
        score_result = run(
            [
                str(cli),
                "reconstruct",
                "score",
                "--run",
                str(run_dir),
                "--evaluation",
                str(evaluation_dir),
                "--blender",
                blender,
                "--json",
            ],
            timeout=args.timeout_ms / 1000.0 + 120,
        )
        score_exit = score_result.returncode
        evaluation_path = run_dir / "evaluation_quality_report.json"
        if evaluation_path.is_file():
            evaluation_quality = read_json(evaluation_path)
        else:
            print(score_result.stdout[-2000:], file=sys.stderr)
            print(score_result.stderr[-2000:], file=sys.stderr)

    quality = report.get("quality", {})
    source_quality = quality.get("source") or {}
    summary = {
        "case": case_name,
        "case_dir": str(case_dir),
        "workspace": str(workspace),
        "run_dir": str(run_dir),
        "resolution": args.resolution,
        "supplements": str(args.supplements) if args.supplements else None,
        "package_validation": report.get("package", {}).get("validation_status"),
        "run_status": report.get("status"),
        "run_exit_code": run_result.returncode,
        "source_iou": {
            "mean": source_quality.get("mean_iou"),
            "min": source_quality.get("min_iou"),
            "views": [
                {"view_id": view.get("view_id"), "iou": view.get("iou")}
                for view in source_quality.get("views", [])
            ],
        },
        "evaluation_iou": None
        if evaluation_quality is None
        else {
            "mean": evaluation_quality.get("evaluation", {}).get("mean_iou"),
            "min": evaluation_quality.get("evaluation", {}).get("min_iou"),
            "views": [
                {"view_id": view.get("view_id"), "iou": view.get("iou")}
                for view in evaluation_quality.get("evaluation", {}).get("views", [])
            ],
        },
        "score_exit_code": score_exit,
        "quality_status": quality.get("status"),
        "evaluation_quality_status": None
        if evaluation_quality is None
        else evaluation_quality.get("thresholds", {}).get("status"),
        "thresholds": quality.get("thresholds"),
        "measurements": report.get("build", {}).get("measurements", []),
        "supplements_applied": [
            item for item in report.get("build", {}).get("supplements", [])
            if item.get("status") == "applied"
        ],
        "unknowns": report.get("build", {}).get("unknowns", []),
        "unresolved": report.get("unresolved", []),
    }
    summary_path = args.summary
    if summary_path is None:
        summary_path = REPO / "target" / "techlead" / "IT-5" / "bench" / f"{case_name}-summary.json"
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    summary_path.write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n"
    )

    print(f"case: {case_name}")
    print(f"  package: {summary['package_validation']}, run status: {summary['run_status']}")
    for view in summary["source_iou"]["views"]:
        print(f"  source {view['view_id']}: IoU {view['iou']:.4f}")
    if summary["evaluation_iou"]:
        for view in summary["evaluation_iou"]["views"]:
            print(f"  held-out {view['view_id']}: IoU {view['iou']:.4f}")
    if summary["thresholds"]:
        print(f"  thresholds: {summary['thresholds']}")
    print(f"  summary: {summary_path}")

    if not args.keep and temp_workspace is not None:
        shutil.rmtree(workspace, ignore_errors=True)
    ok = summary["run_status"] == "pass" and summary["quality_status"] != "fail"
    if summary["evaluation_quality_status"] is not None:
        ok = ok and summary["evaluation_quality_status"] == "pass"
    if not ok:
        print("[bench] FAIL: acceptance thresholds not met", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
