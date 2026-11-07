"""Stage B solver CLI: visual hull carving and supplement rules (stdlib only).

Usage:
    python tools/solver/solve.py --package <package_dir> --out <out_dir>
                                 [--resolution N] [--json]

Exit codes: 0 success, 2 invalid/unusable package, 3 solver failure.
All outputs are deterministic: no timestamps, no absolute paths, sorted lists.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

import carve  # noqa: E402
import mesh as meshmod  # noqa: E402
import pngread  # noqa: E402
import supplements as supp  # noqa: E402
from mask import UNIT_TO_METRES, ShiftNotSupportedError, View, ViewError  # noqa: E402

BUILD_REPORT_SCHEMA = "michelangelo.build-report/0"
PACKAGE_SCHEMA = "michelangelo.input-package/0"
SOLVER_NAME = "voxel-carve"
SOLVER_VERSION = 1

EXIT_OK = 0
EXIT_PACKAGE = 2
EXIT_SOLVER = 3

LIMITS = {
    "silhouette_hull": (
        "blind recesses, undercuts and internal voids are not recoverable "
        "from silhouettes"
    ),
    "meshing": "voxel blockout",
}


class PackageError(Exception):
    """The input package is invalid or unusable."""


class SolverError(Exception):
    """The solver itself failed."""


# --- helpers ----------------------------------------------------------------
def _diagnostic(code, severity, message, address=None):
    entry = {"code": code, "severity": severity, "message": message}
    if address is not None:
        entry["address"] = address
    return entry


def _round_list(values, digits=9):
    return [round(float(v) + 0.0, digits) for v in values]


def _resolve_resource(package_dir, relative, address):
    if not isinstance(relative, str) or not relative:
        raise PackageError("%s: missing resource path" % address)
    if relative.startswith("/") or "\\" in relative or ".." in relative.split("/"):
        raise PackageError("%s: unsafe resource path %r" % (address, relative))
    base = os.path.realpath(package_dir)
    path = os.path.realpath(os.path.join(base, relative.replace("/", os.sep)))
    if not (path == base or path.startswith(base + os.sep)):
        raise PackageError("%s: resource path escapes the package" % address)
    if not os.path.isfile(path):
        raise PackageError("%s: resource %r is missing" % (address, relative))
    return path


def load_manifest(package_dir):
    if not os.path.isdir(package_dir):
        raise PackageError("package directory %r does not exist" % package_dir)
    manifest_path = os.path.join(package_dir, "package.json")
    if not os.path.isfile(manifest_path):
        raise PackageError("package manifest %r is missing" % "package.json")
    try:
        with open(manifest_path, "r", encoding="utf-8") as handle:
            manifest = json.load(handle)
    except (OSError, ValueError) as exc:
        raise PackageError("cannot read the package manifest: %s" % exc) from exc
    if not isinstance(manifest, dict):
        raise PackageError("the package manifest must be a JSON object")
    schema = manifest.get("schema")
    if schema != PACKAGE_SCHEMA:
        raise PackageError(
            "unsupported package schema %r (expected %r)" % (schema, PACKAGE_SCHEMA)
        )
    return manifest


def load_mask(package_dir, view_entry, diagnostics):
    view_id = str(view_entry.get("view_id", "?"))
    mask_entry = view_entry.get("mask") or {}
    path = _resolve_resource(package_dir, mask_entry.get("path"),
                             "views[%s].mask.path" % view_id)
    try:
        image = pngread.read_png(path)
    except pngread.PngError as exc:
        raise PackageError("views[%s].mask: %s" % (view_id, exc)) from exc
    fmt = str(mask_entry.get("format", "")).lower()
    try:
        if fmt == "grayscale8":
            if image.mode != "gray":
                raise PackageError(
                    "views[%s].mask declares grayscale8 but the PNG is %r"
                    % (view_id, image.mode)
                )
            mask = pngread.mask_from_gray(image)
        elif fmt == "alpha":
            if image.mode not in ("rgba", "gray_alpha"):
                raise PackageError(
                    "views[%s].mask declares alpha but the PNG is %r"
                    % (view_id, image.mode)
                )
            mask = pngread.mask_from_alpha(image)
        else:
            mask = pngread.mask_from_png(path)
    except pngread.PngError as exc:
        raise PackageError("views[%s].mask: %s" % (view_id, exc)) from exc
    image_entry = view_entry.get("image") or {}
    width = image_entry.get("width_px")
    height = image_entry.get("height_px")
    if width and height and (int(width) != image.width or int(height) != image.height):
        diagnostics.append(
            _diagnostic(
                "MASK_DIMENSION_MISMATCH",
                "warning",
                "view %r: mask is %dx%d but the manifest declares %sx%s"
                % (view_id, image.width, image.height, width, height),
                "views[%s].mask.path" % view_id,
            )
        )
    return mask


def _axis_indices(manifest):
    """Return (width_axis, depth_axis, height_axis) from the coordinate system."""
    mapping = {"X": 0, "Y": 1, "Z": 2}
    coordinate_system = manifest.get("coordinate_system") or {}

    def _axis(value, default):
        if not isinstance(value, str):
            return default
        return mapping.get(value.strip().upper().lstrip("+-"), default)

    width_axis = _axis(coordinate_system.get("right"), 0)
    depth_axis = _axis(coordinate_system.get("forward"), 1)
    height_axis = _axis(coordinate_system.get("up"), 2)
    return (width_axis, depth_axis, height_axis)


def _measurement_targets(manifest):
    width_axis, depth_axis, height_axis = _axis_indices(manifest)
    return {
        "width": width_axis,
        "depth": depth_axis,
        "height": height_axis,
    }


def _measured_extents(bbox):
    lo, hi = bbox
    return [hi[a] - lo[a] for a in range(3)]


def _measurements(manifest, bbox, unit_scale, diagnostics):
    width_axis, depth_axis, height_axis = _axis_indices(manifest)
    extents = _measured_extents(bbox)
    diagonal = math.sqrt(sum(value * value for value in extents))
    results = []
    for entry in manifest.get("measurements") or []:
        measurement_id = str(entry.get("id", "?"))
        kind = str(entry.get("kind", "")).lower()
        try:
            declared = float(entry.get("value_m")) * unit_scale
        except (TypeError, ValueError):
            declared = None
        tolerance = float(entry.get("tolerance_m", 0.0) or 0.0) * unit_scale
        measured = None
        if declared is None:
            detail = "measurement %r has no usable value_m" % measurement_id
            results.append(
                {
                    "id": measurement_id,
                    "declared": None,
                    "measured": None,
                    "absolute_error": None,
                    "relative_error": None,
                    "within_tolerance": None,
                }
            )
            diagnostics.append(
                _diagnostic(
                    "MEASUREMENT_INVALID",
                    "warning",
                    detail,
                    "measurements[%s].value_m" % measurement_id,
                )
            )
            continue
        if kind == "width":
            measured = extents[width_axis]
        elif kind == "depth":
            measured = extents[depth_axis]
        elif kind == "height":
            measured = extents[height_axis]
        elif kind == "bbox":
            measured = diagonal
        else:
            measured = None
        if measured is None:
            results.append(
                {
                    "id": measurement_id,
                    "declared": declared,
                    "measured": None,
                    "absolute_error": None,
                    "relative_error": None,
                    "within_tolerance": None,
                }
            )
            diagnostics.append(
                _diagnostic(
                    "MEASUREMENT_UNCHECKED",
                    "warning",
                    "measurement %r of kind %r cannot be checked against the hull "
                    "bbox" % (measurement_id, kind),
                    "measurements[%s].kind" % measurement_id,
                )
            )
            continue
        absolute_error = abs(measured - declared)
        relative_error = absolute_error / declared if declared > 0 else None
        within = absolute_error <= tolerance + 1e-12
        results.append(
            {
                "id": measurement_id,
                "declared": declared,
                "measured": measured,
                "absolute_error": absolute_error,
                "relative_error": relative_error,
                "within_tolerance": bool(within),
            }
        )
        if not within:
            diagnostics.append(
                _diagnostic(
                    "MEASUREMENT_OUT_OF_TOLERANCE",
                    "warning",
                    "measurement %r: measured %.6g m, declared %.6g m "
                    "(tolerance %.6g m)"
                    % (measurement_id, measured, declared, tolerance),
                    "measurements[%s].value_m" % measurement_id,
                )
            )
    return results


def evaluate_measurements(manifest, bbox, unit_scale, diagnostics):
    """Public wrapper around the measurement check (used by compare.py)."""
    return _measurements(manifest, bbox, unit_scale, diagnostics)


def _directions(views):
    return [_round_list(view.forward, 6) for view in views]


def _distinct_directions(views):
    count = 0
    for index, view in enumerate(views):
        if all(
            abs(sum(a * b for a, b in zip(view.forward, other.forward))) <= 0.9
            for other in views[:index]
        ):
            count += 1
    return count


# --- main solve -------------------------------------------------------------
def solve_package(package_dir, out_dir, resolution=carve.DEFAULT_RESOLUTION,
                  print_only=False):
    """Solve a package; returns (report, exit_code)."""
    diagnostics = []
    manifest = load_manifest(package_dir)
    units = str(manifest.get("units", "m"))
    unit_scale = UNIT_TO_METRES.get(units)
    if unit_scale is None:
        raise PackageError("unsupported package units %r" % units)
    package_info = {
        "package_id": str(manifest.get("package_id", "")),
        "revision": int(manifest.get("revision", 0) or 0),
    }

    views = []
    view_masks = []
    for entry in manifest.get("views") or []:
        kind = str(entry.get("kind", "")).lower()
        view_id = str(entry.get("view_id", "?"))
        if kind != "calibrated":
            continue
        try:
            view = View.from_manifest(entry, units)
        except ShiftNotSupportedError as exc:
            raise PackageError("views[%s]: %s" % (view_id, exc)) from exc
        except ViewError as exc:
            raise PackageError("views[%s]: %s" % (view_id, exc)) from exc
        mask = load_mask(package_dir, entry, diagnostics)
        views.append(view)
        view_masks.append(mask)

    if not views:
        raise PackageError("the package has no calibrated views")
    if len(views) < 2:
        raise PackageError(
            "the package has only %d calibrated view; at least two are required"
            % len(views)
        )

    ordered = sorted(zip(views, view_masks), key=lambda item: item[0].view_id)
    views = [item[0] for item in ordered]
    view_masks = [item[1] for item in ordered]

    try:
        min_corner, max_corner = carve.derive_hull_bbox(list(zip(views, view_masks)))
    except carve.CarveError as exc:
        raise PackageError(str(exc)) from exc

    grid, used_resolution = carve.make_grid(min_corner, max_corner, resolution)
    if used_resolution != int(resolution):
        diagnostics.append(
            _diagnostic(
                "RESOLUTION_CAPPED",
                "info",
                "the requested resolution %d was capped at %d"
                % (int(resolution), used_resolution),
            )
        )
    hull = carve.carve(grid, list(zip(views, view_masks)))
    if hull.count() == 0:
        diagnostics.append(
            _diagnostic(
                "HULL_EMPTY",
                "warning",
                "the carved visual hull is empty",
            )
        )

    hull_bbox = hull.bbox()
    supplements = manifest.get("supplements") or []
    hull, outcomes, supp_diagnostics = supp.apply_supplements(
        supplements, hull, grid, hull_bbox, unit_scale
    )
    diagnostics.extend(supp_diagnostics)
    final_bbox = hull.bbox() or hull_bbox

    resolved_by_supplement = {}
    for outcome in outcomes:
        if outcome.status != "applied":
            continue
        for supplement in supplements:
            if str(supplement.get("supplement_id")) != outcome.supplement_id:
                continue
            part_id = supplement.get("part_id")
            if part_id:
                resolved_by_supplement.setdefault(str(part_id), []).append(
                    outcome.supplement_id
                )
    stubs, unknown_results, unknown_diagnostics = supp.resolve_unknowns(
        manifest.get("unknowns") or [], final_bbox, resolved_by_supplement
    )
    diagnostics.extend(unknown_diagnostics)

    components = []
    outputs = {}
    if not print_only:
        os.makedirs(out_dir, exist_ok=True)

    # hull blockout
    blockout_name = "blockout.obj"
    vertices, faces = meshmod.voxel_surface(hull)
    if not print_only:
        meshmod.write_obj_mesh(
            os.path.join(out_dir, blockout_name), "blockout", vertices, faces
        )
    components.append(
        {
            "name": "blockout",
            "obj": blockout_name,
            "voxels": hull.count(),
            "triangles": 2 * len(faces),
        }
    )

    # supplement objects
    for outcome in sorted(outcomes, key=lambda item: item.supplement_id):
        if outcome.status != "applied" or outcome.voxels is None:
            continue
        obj_name = "supplement_%s.obj" % outcome.supplement_id
        vertices, faces = meshmod.voxel_surface(outcome.voxels)
        if not print_only:
            meshmod.write_obj_mesh(
                os.path.join(out_dir, obj_name),
                "supplement_%s" % outcome.supplement_id,
                vertices,
                faces,
            )
        components.append(
            {
                "name": "supplement_%s" % outcome.supplement_id,
                "obj": obj_name,
                "voxels": outcome.voxels.count(),
                "triangles": 2 * len(faces),
            }
        )

    # stubs
    for unknown_id, stub_min, stub_max, _detail in sorted(stubs, key=lambda i: i[0]):
        obj_name = "stub_%s.obj" % unknown_id
        vertices, faces = meshmod.box_mesh(stub_min, stub_max)
        if not print_only:
            meshmod.write_obj_mesh(
                os.path.join(out_dir, obj_name), "stub_%s" % unknown_id, vertices, faces
            )
        inside = 0
        size = grid.voxel_size
        for index in range(grid.count()):
            lo, hi = grid.world_bounds(index)
            if (
                lo[0] >= stub_min[0] - 1e-12
                and lo[1] >= stub_min[1] - 1e-12
                and lo[2] >= stub_min[2] - 1e-12
                and hi[0] <= stub_max[0] + 1e-12
                and hi[1] <= stub_max[1] + 1e-12
                and hi[2] <= stub_max[2] + 1e-12
            ):
                inside += 1
        components.append(
            {
                "name": "stub_%s" % unknown_id,
                "obj": obj_name,
                "voxels": inside,
                "triangles": 2 * len(faces),
            }
        )

    measurement_results = _measurements(
        manifest, final_bbox, unit_scale, diagnostics
    )

    report = {
        "schema": BUILD_REPORT_SCHEMA,
        "package": package_info,
        "solver": {
            "name": SOLVER_NAME,
            "version": SOLVER_VERSION,
            "resolution": used_resolution,
            "voxel_size": grid.voxel_size,
            "grid": [grid.dims[0], grid.dims[1], grid.dims[2]],
            "origin": _round_list(grid.min_corner, 6),
            "obj_units": "m",
            "obj_up": "y",
        },
        "basis": {
            "calibrated_views": [view.view_id for view in views],
            "directions": _directions(views),
            "complete": bool(_distinct_directions(views) >= 2),
        },
        "components": components,
        "supplements": [
            {
                "supplement_id": outcome.supplement_id,
                "status": outcome.status,
                "rule": outcome.rule,
                "detail": outcome.detail,
            }
            for outcome in sorted(outcomes, key=lambda item: item.supplement_id)
        ],
        "unknowns": sorted(unknown_results, key=lambda item: item["id"]),
        "measurements": measurement_results,
        "diagnostics": sorted(
            diagnostics, key=lambda item: (item["code"], item.get("address", ""))
        ),
        "limits": dict(LIMITS),
    }
    return (report, EXIT_OK)


def _error_report(package_info, code, message, diagnostics=None):
    entries = list(diagnostics or [])
    entries.append(_diagnostic(code, "error", message))
    return {
        "schema": BUILD_REPORT_SCHEMA,
        "package": package_info or {"package_id": "", "revision": 0},
        "solver": {
            "name": SOLVER_NAME,
            "version": SOLVER_VERSION,
            "resolution": 0,
            "voxel_size": 0.0,
            "grid": [0, 0, 0],
            "origin": [0.0, 0.0, 0.0],
        },
        "basis": {"calibrated_views": [], "directions": [], "complete": False},
        "components": [],
        "supplements": [],
        "unknowns": [],
        "measurements": [],
        "diagnostics": entries,
        "limits": dict(LIMITS),
    }


def write_report(out_dir, report):
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, "build_report.json")
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(report, handle, indent=2, ensure_ascii=True)
        handle.write("\n")
    return path


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="solve.py",
        description="Stage B solver: visual hull carving with supplement rules.",
    )
    parser.add_argument("--package", required=True, help="input package directory")
    parser.add_argument("--out", required=True, help="output directory")
    parser.add_argument(
        "--resolution",
        type=int,
        default=carve.DEFAULT_RESOLUTION,
        help="voxels along the longest hull axis (default %d, capped at %d)"
        % (carve.DEFAULT_RESOLUTION, carve.MAX_RESOLUTION),
    )
    parser.add_argument("--json", action="store_true", help="print the build report")
    parser.add_argument(
        "--print-only",
        action="store_true",
        help="do not write any file, print the build report only",
    )
    args = parser.parse_args(argv)

    package_info = {"package_id": "", "revision": 0}
    try:
        report, code = solve_package(
            args.package,
            args.out,
            resolution=args.resolution,
            print_only=args.print_only,
        )
    except PackageError as exc:
        try:
            manifest = load_manifest(args.package)
            package_info = {
                "package_id": str(manifest.get("package_id", "")),
                "revision": int(manifest.get("revision", 0) or 0),
            }
        except Exception:  # noqa: BLE001 - best effort only
            package_info = {"package_id": "", "revision": 0}
        report = _error_report(package_info, "PACKAGE_UNUSABLE", str(exc))
        if not args.print_only:
            try:
                write_report(args.out, report)
            except OSError:
                pass
        sys.stderr.write("solve.py: %s\n" % exc)
        if args.json:
            print(json.dumps(report, indent=2, ensure_ascii=True))
        return EXIT_PACKAGE
    except Exception as exc:  # noqa: BLE001 - solver failure
        sys.stderr.write("solve.py: solver failure: %s\n" % exc)
        return EXIT_SOLVER

    if not args.print_only:
        write_report(args.out, report)
    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=True))
    else:
        print(
            "solver: %d voxels, grid %dx%dx%d, %d components"
            % (
                report["solver"]["grid"][0]
                * report["solver"]["grid"][1]
                * report["solver"]["grid"][2],
                report["solver"]["grid"][0],
                report["solver"]["grid"][1],
                report["solver"]["grid"][2],
                len(report["components"]),
            )
        )
    return code


if __name__ == "__main__":
    sys.exit(main())
