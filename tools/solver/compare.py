"""Stage B quality comparator (stdlib only).

Usage:
    python tools/solver/compare.py --package <pkg> --renders <dir> --out <dir>
                                   [--evaluation-package <pkg>
                                    --evaluation-renders <dir>] [--json]

The comparator compares the package masks with the silhouette renders of the
built blockout (RGBA8, alpha >= 128 = object) and writes quality_report.json
(schema michelangelo.quality-report/0).

Exit codes: 0 pass, 1 fail, 2 usage/IO error.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

import carve  # noqa: E402
import pngread  # noqa: E402
import solve as solver  # noqa: E402
from mask import UNIT_TO_METRES, View  # noqa: E402

QUALITY_REPORT_SCHEMA = "michelangelo.quality-report/0"

DEFAULT_SOURCE_MEAN_IOU = 0.90
DEFAULT_SOURCE_MIN_IOU = 0.85
DEFAULT_EVALUATION_MEAN_IOU = 0.80
DEFAULT_EVALUATION_MIN_IOU = 0.70
DEFAULT_BBOX_RELATIVE_ERROR_MAX = 0.05

EXIT_OK = 0
EXIT_FAIL = 1
EXIT_USAGE = 2


class CompareError(Exception):
    """The comparator cannot run (usage or IO problem)."""


def _diagnostic(code, severity, message, address=None):
    entry = {"code": code, "severity": severity, "message": message}
    if address is not None:
        entry["address"] = address
    return entry


def _load_views(package_dir):
    """Return (manifest, [(view, mask)]) of the calibrated views of a package."""
    manifest = solver.load_manifest(package_dir)
    units = str(manifest.get("units", "m"))
    if units not in UNIT_TO_METRES:
        raise CompareError("unsupported package units %r" % units)
    diagnostics = []
    views = []
    masks = []
    for entry in manifest.get("views") or []:
        if str(entry.get("kind", "")).lower() != "calibrated":
            continue
        view_id = str(entry.get("view_id", "?"))
        view = View.from_manifest(entry, units)
        mask = solver.load_mask(package_dir, entry, diagnostics)
        views.append(view)
        masks.append(mask)
    ordered = sorted(zip(views, masks), key=lambda item: item[0].view_id)
    return (manifest, [(v, m) for v, m in ordered], diagnostics)


def _load_render_mask(render_dir, view_id):
    path = os.path.join(render_dir, "%s.png" % view_id)
    if not os.path.isfile(path):
        return None
    try:
        image = pngread.read_png(path)
    except pngread.PngError as exc:
        raise CompareError("render %s: %s" % (path, exc)) from exc
    if image.mode in ("rgba", "gray_alpha"):
        return pngread.mask_from_alpha(image)
    if image.mode == "gray":
        return pngread.mask_from_gray(image)
    raise CompareError("render %s has an unsupported mode %r" % (path, image.mode))


def _view_metrics(view, mask, render_mask):
    from mask import mask_counts

    intersection, union, mask_px, render_px = mask_counts(mask, render_mask)
    iou = 1.0 if union == 0 else intersection / union
    total = float(mask.width * mask.height)
    return {
        "view_id": view.view_id,
        "iou": iou,
        "mask_px": mask_px,
        "render_px": render_px,
        "intersection_px": intersection,
        "union_px": union,
        "mask_coverage": mask_px / total,
        "render_coverage": render_px / total,
    }


def _compare_set(views_and_masks, render_dir, diagnostics, label):
    metrics = []
    renders = []
    for view, mask in views_and_masks:
        render_mask = _load_render_mask(render_dir, view.view_id)
        if render_mask is None:
            diagnostics.append(
                _diagnostic(
                    "RENDER_MISSING",
                    "warning",
                    "%s render %s.png is missing" % (label, view.view_id),
                    "renders/%s.png" % view.view_id,
                )
            )
            continue
        if render_mask.width != mask.width or render_mask.height != mask.height:
            diagnostics.append(
                _diagnostic(
                    "RENDER_SIZE_MISMATCH",
                    "warning",
                    "%s render %s is %dx%d but the mask is %dx%d"
                    % (
                        label,
                        view.view_id,
                        render_mask.width,
                        render_mask.height,
                        mask.width,
                        mask.height,
                    ),
                    "renders/%s.png" % view.view_id,
                )
            )
            continue
        renders.append((view, render_mask))
        metrics.append(_view_metrics(view, mask, render_mask))
    if metrics:
        mean_iou = sum(entry["iou"] for entry in metrics) / len(metrics)
        min_iou = min(entry["iou"] for entry in metrics)
    else:
        mean_iou = 0.0
        min_iou = 0.0
    return ({"views": metrics, "mean_iou": mean_iou, "min_iou": min_iou}, renders)


def _render_bbox(views_and_masks, renders):
    """World bbox derived from the render masks (metres)."""
    pairs = []
    for view, mask in views_and_masks:
        for rview, rmask in renders:
            if rview.view_id == view.view_id:
                pairs.append((view, rmask))
                break
    if len(pairs) < 2:
        return None
    try:
        return carve.derive_hull_bbox(pairs)
    except carve.CarveError:
        return None


def run(package_dir, renders_dir, out_dir, evaluation_package=None,
        evaluation_renders=None, thresholds=None):
    """Run the comparator; returns (report, status)."""
    diagnostics = []
    manifest, views_and_masks, load_diagnostics = _load_views(package_dir)
    diagnostics.extend(load_diagnostics)
    if not views_and_masks:
        raise CompareError("the package has no calibrated views")
    thresholds = dict(thresholds or {})

    source, source_renders = _compare_set(
        views_and_masks, renders_dir, diagnostics, "source"
    )
    evaluation = None
    evaluation_manifest = None
    if evaluation_package and evaluation_renders:
        evaluation_manifest, eval_views, eval_diagnostics = _load_views(
            evaluation_package
        )
        diagnostics.extend(eval_diagnostics)
        evaluation, _ = _compare_set(
            eval_views, evaluation_renders, diagnostics, "evaluation"
        )

    measurements = []
    bbox = _render_bbox(views_and_masks, source_renders)
    if bbox is not None:
        unit_scale = UNIT_TO_METRES.get(str(manifest.get("units", "m")), 1.0)
        measurements = solver.evaluate_measurements(
            manifest, bbox, unit_scale, diagnostics
        )
    else:
        diagnostics.append(
            _diagnostic(
                "MEASUREMENTS_UNAVAILABLE",
                "warning",
                "the render bbox could not be derived; declared measurements were "
                "not re-checked",
            )
        )

    source_mean_threshold = thresholds.get(
        "source_mean_iou", DEFAULT_SOURCE_MEAN_IOU
    )
    source_min_threshold = thresholds.get("source_min_iou", DEFAULT_SOURCE_MIN_IOU)
    evaluation_mean_threshold = thresholds.get(
        "evaluation_mean_iou", DEFAULT_EVALUATION_MEAN_IOU
    )
    evaluation_min_threshold = thresholds.get(
        "evaluation_min_iou", DEFAULT_EVALUATION_MIN_IOU
    )
    bbox_relative_threshold = thresholds.get(
        "bbox_relative_error_max", DEFAULT_BBOX_RELATIVE_ERROR_MAX
    )

    failures = []
    if not source["views"]:
        failures.append("no source render could be compared")
    if source["mean_iou"] < source_mean_threshold:
        failures.append(
            "source mean IoU %.4f < %.4f" % (source["mean_iou"], source_mean_threshold)
        )
    if source["min_iou"] < source_min_threshold:
        failures.append(
            "source min IoU %.4f < %.4f" % (source["min_iou"], source_min_threshold)
        )
    if evaluation is not None:
        if not evaluation["views"]:
            failures.append("no evaluation render could be compared")
        if evaluation["mean_iou"] < evaluation_mean_threshold:
            failures.append(
                "evaluation mean IoU %.4f < %.4f"
                % (evaluation["mean_iou"], evaluation_mean_threshold)
            )
        if evaluation["min_iou"] < evaluation_min_threshold:
            failures.append(
                "evaluation min IoU %.4f < %.4f"
                % (evaluation["min_iou"], evaluation_min_threshold)
            )
    for measurement in measurements:
        relative = measurement.get("relative_error")
        if relative is None:
            continue
        if relative > bbox_relative_threshold:
            failures.append(
                "measurement %s relative error %.4f > %.4f"
                % (measurement["id"], relative, bbox_relative_threshold)
            )

    status = "pass" if not failures else "fail"
    for failure in failures:
        diagnostics.append(
            _diagnostic("THRESHOLD_NOT_MET", "warning", failure)
        )

    report = {
        "schema": QUALITY_REPORT_SCHEMA,
        "source": source,
        "evaluation": evaluation,
        "measurements": measurements,
        "thresholds": {
            "source_mean_iou": source_mean_threshold,
            "source_min_iou": source_min_threshold,
            "evaluation_mean_iou": evaluation_mean_threshold,
            "evaluation_min_iou": evaluation_min_threshold,
            "bbox_relative_error_max": bbox_relative_threshold,
            "status": status,
        },
        "diagnostics": sorted(
            diagnostics, key=lambda item: (item["code"], item.get("address", ""))
        ),
    }
    return (report, status)


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="compare.py",
        description="Compare package masks with silhouette renders of the blockout.",
    )
    parser.add_argument("--package", required=True, help="input package directory")
    parser.add_argument("--renders", required=True, help="directory with <view_id>.png")
    parser.add_argument("--out", required=True, help="output directory")
    parser.add_argument("--evaluation-package", default=None)
    parser.add_argument("--evaluation-renders", default=None)
    parser.add_argument(
        "--source-mean-iou", type=float, default=DEFAULT_SOURCE_MEAN_IOU
    )
    parser.add_argument(
        "--source-min-iou", type=float, default=DEFAULT_SOURCE_MIN_IOU
    )
    parser.add_argument(
        "--evaluation-mean-iou", type=float, default=DEFAULT_EVALUATION_MEAN_IOU
    )
    parser.add_argument(
        "--evaluation-min-iou", type=float, default=DEFAULT_EVALUATION_MIN_IOU
    )
    parser.add_argument(
        "--bbox-relative-error-max",
        type=float,
        default=DEFAULT_BBOX_RELATIVE_ERROR_MAX,
    )
    parser.add_argument("--json", action="store_true", help="print the quality report")
    args = parser.parse_args(argv)

    if bool(args.evaluation_package) != bool(args.evaluation_renders):
        sys.stderr.write(
            "compare.py: --evaluation-package and --evaluation-renders must be "
            "used together\n"
        )
        return EXIT_USAGE

    try:
        report, status = run(
            args.package,
            args.renders,
            args.out,
            evaluation_package=args.evaluation_package,
            evaluation_renders=args.evaluation_renders,
            thresholds={
                "source_mean_iou": args.source_mean_iou,
                "source_min_iou": args.source_min_iou,
                "evaluation_mean_iou": args.evaluation_mean_iou,
                "evaluation_min_iou": args.evaluation_min_iou,
                "bbox_relative_error_max": args.bbox_relative_error_max,
            },
        )
    except (CompareError, solver.PackageError) as exc:
        sys.stderr.write("compare.py: %s\n" % exc)
        return EXIT_USAGE
    except Exception as exc:  # noqa: BLE001 - unexpected failure
        sys.stderr.write("compare.py: failure: %s\n" % exc)
        return EXIT_USAGE

    os.makedirs(args.out, exist_ok=True)
    path = os.path.join(args.out, "quality_report.json")
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(report, handle, indent=2, ensure_ascii=True)
        handle.write("\n")
    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=True))
    else:
        print(
            "compare: status %s (source mean IoU %.4f, min IoU %.4f)"
            % (
                status,
                report["source"]["mean_iou"],
                report["source"]["min_iou"],
            )
        )
    return EXIT_OK if status == "pass" else EXIT_FAIL


if __name__ == "__main__":
    sys.exit(main())
