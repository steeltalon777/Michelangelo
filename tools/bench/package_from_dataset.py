#!/usr/bin/env python3
"""Convert a local PoC dataset into a Stage A input package (bench tool, stdlib only).

Usage:
  python tools/bench/package_from_dataset.py --dataset <dir> --out <package_dir>
      [--source solver|evaluation] [--supplements <file.json>] [--revision 1]

The dataset layout follows the local (untracked) PoC datasets:
  <dataset>/solver_input/cameras.json, external_dimensions.json, views/<view>*.png
  <dataset>/evaluation/cameras.json, views/<control>*.png

The produced package contains only calibrated ORTHOGRAPHIC views as metric constraints;
perspective views are carried as `reference` views (image + mask, no projection), per
TZ_RECON_POC_STAGE_B §3.1/§3.3. Measurements come from `external_dimensions.json`
(overall bbox extents with a relative tolerance).

`--supplements` merges an authored supplement set:
  {"parts": [...], "supplements": [...], "unknowns": [...], "acceptance": [...]}

The tool never reads `ground_truth/`, never writes absolute paths into the manifest,
and is deterministic for a given dataset and options.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import struct
import sys
from pathlib import Path

PACKAGE_SCHEMA = "michelangelo.input-package/0"
MEASUREMENT_TOLERANCE_RATIO = 0.05


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def png_size(path: Path):
    data = path.read_bytes()[:33]
    if len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG file")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def load_json(path: Path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def dataset_cameras(dataset: Path, source: str):
    base = dataset / ("solver_input" if source == "solver" else "evaluation")
    cameras_path = base / "cameras.json"
    if not cameras_path.is_file():
        raise SystemExit(f"[package_from_dataset] missing {cameras_path}")
    cameras = load_json(cameras_path)
    view_dir = base / "views"
    views = []
    for view in cameras["views"]:
        # groups are optional in some dataset variants; the source directory decides.
        files = view["files"]
        image_name = files.get("beauty_png") or f"{view['view_id']}.png"
        mask_name = files.get("mask_png") or f"{view['view_id']}_mask.png"
        image_path = view_dir / image_name
        mask_path = view_dir / mask_name
        if not image_path.is_file() or not mask_path.is_file():
            raise SystemExit(f"[package_from_dataset] missing view files for {view['view_id']}")
        views.append((view, image_path, mask_path))
    return cameras, views


def build_view(view, image_path: Path, mask_path: Path, images_dir: Path, masks_dir: Path):
    view_id = view["view_id"]
    width, height = png_size(image_path)
    mask_width, mask_height = png_size(mask_path)
    if (width, height) != (mask_width, mask_height):
        raise SystemExit(f"[package_from_dataset] {view_id}: image/mask dimensions differ")
    image_rel = f"images/{view_id}.png"
    mask_rel = f"masks/{view_id}.png"
    shutil.copyfile(image_path, images_dir / f"{view_id}.png")
    shutil.copyfile(mask_path, masks_dir / f"{view_id}.png")
    entry = {
        "view_id": view_id,
        "kind": "calibrated" if view.get("projection") == "orthographic" else "reference",
        "image": {
            "path": image_rel,
            "sha256": sha256_file(images_dir / f"{view_id}.png"),
            "width_px": width,
            "height_px": height,
        },
        "mask": {
            "path": mask_rel,
            "sha256": sha256_file(masks_dir / f"{view_id}.png"),
            "format": "grayscale8",
            "foreground": "white",
        },
        "provenance": {
            "origin": "ai_author",
            "detail": f"converted from local PoC dataset ({view.get('role', 'view')})",
        },
    }
    if entry["kind"] == "calibrated":
        axes = view["camera_axes_world"]
        entry["projection"] = {
            "type": "orthographic",
            "location_m": view["camera_location_m"],
            "look_at_m": view["target_m"],
            "up_world": axes["up"],
            "ortho_scale_m": view["ortho_scale_m"],
            "shift_x": view.get("shift_x", 0.0),
            "shift_y": view.get("shift_y", 0.0),
            "clip_start_m": view.get("clip_start_m", 0.01),
            "clip_end_m": view.get("clip_end_m", 100.0),
        }
    return entry


def overall_sizes(dataset: Path):
    path = dataset / "solver_input" / "external_dimensions.json"
    if not path.is_file():
        return {}
    dims = load_json(path)
    bbox = dims.get("overall_bbox_m")
    if not isinstance(bbox, dict):
        return {}
    if "size_m" in bbox:
        return {
            "width": bbox["size_m"][0],
            "depth": bbox["size_m"][1],
            "height": bbox["size_m"][2],
        }
    sizes = {}
    for axis, kind in (("x", "width"), ("y", "depth"), ("z", "height")):
        if axis in bbox:
            sizes[kind] = bbox[axis]
    return sizes


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Build a Stage A package from a local dataset")
    parser.add_argument("--dataset", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--source", choices=("solver", "evaluation"), default="solver")
    parser.add_argument("--supplements", type=Path, default=None)
    parser.add_argument("--revision", type=int, default=1)
    parser.add_argument("--package-id", default=None)
    args = parser.parse_args(argv)

    dataset = args.dataset.resolve()
    out = args.out.resolve()
    if not dataset.is_dir():
        print(f"[package_from_dataset] dataset not found: {dataset}", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    images_dir = out / "images"
    masks_dir = out / "masks"
    images_dir.mkdir(exist_ok=True)
    masks_dir.mkdir(exist_ok=True)

    cameras, views = dataset_cameras(dataset, args.source)
    entries = [build_view(view, image, mask, images_dir, masks_dir) for view, image, mask in views]
    package_id = args.package_id
    if package_id is None:
        suffix = "" if args.source == "solver" else "-eval"
        package_id = dataset.name + suffix

    package = {
        "schema": PACKAGE_SCHEMA,
        "package_id": package_id,
        "object": {
            "name": str(cameras.get("asset_id", dataset.name)),
            "notes": "converted from a local untracked PoC dataset; evaluation data excluded",
        },
        "revision": args.revision,
        "units": "m",
        "coordinate_system": {
            "handedness": "right",
            "up": "+Z",
            "right": "+X",
            "forward": "-Y",
        },
        "views": entries,
    }

    if args.source == "solver":
        sizes = overall_sizes(dataset)
        measurements = []
        for kind in ("width", "depth", "height"):
            if kind not in sizes:
                continue
            value = float(sizes[kind])
            measurements.append(
                {
                    "id": f"overall_{kind}",
                    "label": f"overall {kind}",
                    "kind": kind,
                    "value_m": value,
                    "tolerance_m": round(MEASUREMENT_TOLERANCE_RATIO * value, 6),
                    "origin": "human",
                }
            )
        package["measurements"] = measurements
        package["uncertainty_policy"] = {
            "allowed_completion_rules": ["thickness", "closure", "symmetry"],
            "stub_policy": "allowed",
            "require_author_acceptance": True,
        }

    if args.supplements is not None:
        extra = load_json(args.supplements.resolve())
        for key in ("parts", "supplements", "unknowns", "acceptance"):
            if key in extra:
                package[key] = extra[key]

    package["provenance"] = {
        "origin": "ai_author",
        "detail": "bench conversion of a local PoC dataset (solver_input only)",
    }

    manifest_path = out / "package.json"
    manifest_path.write_text(
        json.dumps(package, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    calibrated = sum(1 for entry in entries if entry["kind"] == "calibrated")
    print(
        f"[package_from_dataset] {manifest_path}: {len(entries)} views "
        f"({calibrated} calibrated), {len(package.get('measurements', []))} measurements"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
