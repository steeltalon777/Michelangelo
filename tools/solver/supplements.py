"""Voxel space supplement rules for the Stage B solver (stdlib only).

Rules are applied to the voxel grid in manifest order:

* thickness  - keep a surface shell of value_m, the interior is hollowed out,
* cavity     - subtract interior voxels inside a bbox/points region, hull surface
               voxels are never removed,
* closure / connection - add a capsule between from and to,
* curve      - add a capsule chain along points_m,
* symmetry   - mirror about the given axis/plane and intersect with the hull,
* section / profile / primitive_fill - reported as unsupported (not guessed).

All geometry values are package units and are converted to metres by the caller.
"""

from __future__ import annotations

import math
import os
import sys
from collections import deque
from dataclasses import dataclass, field

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from carve import Grid, VoxelSet  # noqa: E402

#: supplement kinds that are known but not implemented in v0
UNSUPPORTED_KINDS = ("section", "profile", "primitive_fill")

#: relative size of a stub box (fraction of the hull bbox)
STUB_BOX_FRACTION = 0.1

_NEIGHBOURS = (
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
)


@dataclass
class SupplementOutcome:
    """Result of one supplement rule."""

    supplement_id: str
    status: str  # applied | unsupported | rejected
    rule: str
    detail: str
    mode: str = "none"  # added | modified | none
    voxels: "VoxelSet | None" = None


def _surface_distance(voxels: VoxelSet):
    """Distance (in voxels) of every solid voxel to the nearest empty cell.

    Surface voxels have distance 1.
    """
    grid = voxels.grid
    nx, ny, nz = grid.dims
    solid = voxels.indices
    distance = {}
    queue = deque()
    for index in solid:
        i, j, k = grid.coords(index)
        for di, dj, dk in _NEIGHBOURS:
            ni, nj, nk = i + di, j + dj, k + dk
            if not (0 <= ni < nx and 0 <= nj < ny and 0 <= nk < nz):
                distance[index] = 1
                queue.append(index)
                break
            elif grid.index(ni, nj, nk) not in solid:
                distance[index] = 1
                queue.append(index)
                break
    while queue:
        index = queue.popleft()
        i, j, k = grid.coords(index)
        step = distance[index] + 1
        for di, dj, dk in _NEIGHBOURS:
            ni, nj, nk = i + di, j + dj, k + dk
            if not (0 <= ni < nx and 0 <= nj < ny and 0 <= nk < nz):
                continue
            neighbour = grid.index(ni, nj, nk)
            if neighbour in solid and neighbour not in distance:
                distance[neighbour] = step
                queue.append(neighbour)
    return distance


def _capsule_voxels(grid: Grid, start, end, radius: float):
    """Voxel indices whose centre is within radius of the segment start..end."""
    ax, ay, az = (float(c) for c in start)
    bx, by, bz = (float(c) for c in end)
    dx, dy, dz = bx - ax, by - ay, bz - az
    length_sq = dx * dx + dy * dy + dz * dz
    nx, ny, nz = grid.dims
    size = grid.voxel_size
    ox, oy, oz = grid.min_corner
    radius_sq = radius * radius
    indices = set()
    for k in range(nz):
        pz = oz + (k + 0.5) * size
        for j in range(ny):
            py = oy + (j + 0.5) * size
            for i in range(nx):
                px = ox + (i + 0.5) * size
                if length_sq <= 1e-18:
                    t = 0.0
                else:
                    t = ((px - ax) * dx + (py - ay) * dy + (pz - az) * dz) / length_sq
                    t = 0.0 if t < 0.0 else (1.0 if t > 1.0 else t)
                cx = ax + t * dx
                cy = ay + t * dy
                cz = az + t * dz
                ex = px - cx
                ey = py - cy
                ez = pz - cz
                if ex * ex + ey * ey + ez * ez <= radius_sq:
                    indices.add(grid.index(i, j, k))
    return indices


def _default_radius(grid: Grid, start, end) -> float:
    distance = math.dist(tuple(float(c) for c in start), tuple(float(c) for c in end))
    return min(grid.voxel_size * 2.0, distance / 8.0)


def _region_voxels(grid: Grid, region_min, region_max):
    """Voxel indices whose centre lies inside an axis aligned region."""
    nx, ny, nz = grid.dims
    size = grid.voxel_size
    ox, oy, oz = grid.min_corner
    indices = set()
    for k in range(nz):
        pz = oz + (k + 0.5) * size
        if not (region_min[2] <= pz <= region_max[2]):
            continue
        for j in range(ny):
            py = oy + (j + 0.5) * size
            if not (region_min[1] <= py <= region_max[1]):
                continue
            for i in range(nx):
                px = ox + (i + 0.5) * size
                if region_min[0] <= px <= region_max[0]:
                    indices.add(grid.index(i, j, k))
    return indices


def _points_region(points):
    """Axis aligned bounding region of a list of points."""
    xs = [float(p[0]) for p in points]
    ys = [float(p[1]) for p in points]
    zs = [float(p[2]) for p in points]
    return (
        (min(xs), min(ys), min(zs)),
        (max(xs), max(ys), max(zs)),
    )


def _apply_thickness(hull: VoxelSet, geometry, unit_scale):
    value_m = float(geometry["value_m"]) * unit_scale
    if value_m <= 0.0:
        raise ValueError("thickness value_m must be positive")
    distance = _surface_distance(hull)
    size = hull.grid.voxel_size
    kept = set()
    for index, dist in distance.items():
        if (dist - 1) * size < value_m - 1e-12:
            kept.add(index)
    detail = (
        "kept %d of %d voxels as a surface shell of %.6g m; the hull is modified "
        "and this object carries the resulting hull"
        % (len(kept), hull.count(), value_m)
    )
    return (hull.with_indices(kept), detail)


def _apply_cavity(hull: VoxelSet, geometry, unit_scale, hull_bbox):
    if "bbox_m" in geometry:
        sizes = [float(v) * unit_scale for v in geometry["bbox_m"]]
        if len(sizes) != 3 or any(s <= 0.0 for s in sizes):
            raise ValueError("cavity bbox_m must be three positive sizes")
        centre = tuple((hull_bbox[0][a] + hull_bbox[1][a]) * 0.5 for a in range(3))
        region_min = tuple(centre[a] - sizes[a] * 0.5 for a in range(3))
        region_max = tuple(centre[a] + sizes[a] * 0.5 for a in range(3))
        origin = "bbox_m centred on the hull centre"
    elif "points_m" in geometry:
        points = geometry["points_m"]
        if len(points) < 4:
            raise ValueError("cavity points_m needs at least four points")
        scaled = [tuple(float(c) * unit_scale for c in point) for point in points]
        region_min, region_max = _points_region(scaled)
        origin = "bbox of points_m"
    else:
        raise ValueError("cavity needs bbox_m or points_m")
    distance = _surface_distance(hull)
    region = _region_voxels(hull.grid, region_min, region_max)
    removed = set()
    for index in region:
        if index not in hull.indices:
            continue
        if distance.get(index, 1) <= 1:
            continue  # never remove hull surface voxels
        removed.add(index)
    kept = set(hull.indices) - removed
    detail = (
        "removed %d interior voxels (%s); hull surface voxels kept; the hull is "
        "modified and this object carries the resulting hull"
        % (len(removed), origin)
    )
    return (hull.with_indices(kept), detail)


def _apply_capsule(hull: VoxelSet, geometry, unit_scale):
    start = tuple(float(c) * unit_scale for c in geometry["from"])
    end = tuple(float(c) * unit_scale for c in geometry["to"])
    if "radius_m" in geometry:
        radius = float(geometry["radius_m"]) * unit_scale
    else:
        radius = _default_radius(hull.grid, start, end)
    if radius <= 0.0:
        raise ValueError("capsule radius must be positive")
    added = _capsule_voxels(hull.grid, start, end, radius)
    detail = "added %d voxels (capsule radius %.6g m)" % (len(added), radius)
    return (added, detail)


def _apply_curve(hull: VoxelSet, geometry, unit_scale):
    points = geometry["points_m"]
    if len(points) < 2:
        raise ValueError("curve points_m needs at least two points")
    scaled = [tuple(float(c) * unit_scale for c in point) for point in points]
    added = set()
    for start, end in zip(scaled, scaled[1:]):
        if "radius_m" in geometry:
            radius = float(geometry["radius_m"]) * unit_scale
        else:
            radius = _default_radius(hull.grid, start, end)
        added |= _capsule_voxels(hull.grid, start, end, radius)
    detail = "added %d voxels along %d points" % (len(added), len(scaled))
    return (added, detail)


def _apply_symmetry(hull: VoxelSet, geometry, unit_scale, hull_bbox):
    axis = str(geometry.get("axis", "")).lower()
    if axis not in ("x", "y", "z"):
        raise ValueError("symmetry axis must be x, y or z")
    axis_index = {"x": 0, "y": 1, "z": 2}[axis]
    if "plane_offset_m" in geometry:
        offset = float(geometry["plane_offset_m"]) * unit_scale
    else:
        offset = (hull_bbox[0][axis_index] + hull_bbox[1][axis_index]) * 0.5
    grid = hull.grid
    size = grid.voxel_size
    mirrored = set()
    for index in hull.indices:
        i, j, k = grid.coords(index)
        coords = [i, j, k]
        centre = grid.min_corner[axis_index] + (coords[axis_index] + 0.5) * size
        mirrored_centre = 2.0 * offset - centre
        new_index = int(math.floor((mirrored_centre - grid.min_corner[axis_index]) / size))
        coords[axis_index] = new_index
        if not (0 <= coords[0] < grid.dims[0] and 0 <= coords[1] < grid.dims[1]
                and 0 <= coords[2] < grid.dims[2]):
            continue
        mirrored.add(grid.index(coords[0], coords[1], coords[2]))
    kept = set(hull.indices) & mirrored
    detail = (
        "mirrored about the %s plane at %.6g m and intersected with the hull: "
        "%d of %d voxels kept; the hull is modified and this object carries the "
        "resulting hull" % (axis, offset, len(kept), hull.count())
    )
    return (hull.with_indices(kept), detail)


def apply_supplements(supplements, hull: VoxelSet, grid: Grid, hull_bbox, unit_scale=1.0):
    """Apply the manifest supplements to the hull.

    Returns (hull, outcomes, diagnostics).  hull_bbox is the world bbox of the
    solid hull (min_corner, max_corner) in metres.
    """
    hull = hull
    outcomes = []
    diagnostics = []
    for supplement in supplements:
        supplement_id = str(supplement.get("supplement_id", "?"))
        kind = str(supplement.get("kind", "")).lower()
        geometry = supplement.get("geometry") or {}
        if kind in UNSUPPORTED_KINDS:
            detail = "supplement kind %r is not implemented in v0" % kind
            outcomes.append(
                SupplementOutcome(supplement_id, "unsupported", kind, detail)
            )
            diagnostics.append(
                {
                    "code": "SUPPLEMENT_UNSUPPORTED_KIND",
                    "severity": "warning",
                    "message": detail,
                    "address": "supplements[%s].kind" % supplement_id,
                }
            )
            continue
        try:
            if kind == "thickness":
                new_hull, detail = _apply_thickness(hull, geometry, unit_scale)
                outcomes.append(
                    SupplementOutcome(
                        supplement_id, "applied", kind, detail, "modified", new_hull
                    )
                )
                hull = new_hull
            elif kind == "cavity":
                new_hull, detail = _apply_cavity(hull, geometry, unit_scale, hull_bbox)
                outcomes.append(
                    SupplementOutcome(
                        supplement_id, "applied", kind, detail, "modified", new_hull
                    )
                )
                hull = new_hull
            elif kind in ("closure", "connection"):
                added, detail = _apply_capsule(hull, geometry, unit_scale)
                outcomes.append(
                    SupplementOutcome(
                        supplement_id, "applied", kind, detail, "added",
                        hull.with_indices(added),
                    )
                )
            elif kind == "curve":
                added, detail = _apply_curve(hull, geometry, unit_scale)
                outcomes.append(
                    SupplementOutcome(
                        supplement_id, "applied", kind, detail, "added",
                        hull.with_indices(added),
                    )
                )
            elif kind == "symmetry":
                new_hull, detail = _apply_symmetry(hull, geometry, unit_scale, hull_bbox)
                outcomes.append(
                    SupplementOutcome(
                        supplement_id, "applied", kind, detail, "modified", new_hull
                    )
                )
                hull = new_hull
            else:
                detail = "unknown supplement kind %r" % kind
                outcomes.append(
                    SupplementOutcome(supplement_id, "rejected", kind, detail)
                )
                diagnostics.append(
                    {
                        "code": "SUPPLEMENT_KIND_UNKNOWN",
                        "severity": "warning",
                        "message": detail,
                        "address": "supplements[%s].kind" % supplement_id,
                    }
                )
        except (KeyError, TypeError, ValueError) as exc:
            detail = "invalid geometry for supplement %r: %s" % (supplement_id, exc)
            outcomes.append(
                SupplementOutcome(supplement_id, "rejected", kind, detail)
            )
            diagnostics.append(
                {
                    "code": "SUPPLEMENT_GEOMETRY_INVALID",
                    "severity": "warning",
                    "message": detail,
                    "address": "supplements[%s].geometry" % supplement_id,
                }
            )
    return (hull, outcomes, diagnostics)


def stub_box(hull_bbox, fraction: float = STUB_BOX_FRACTION):
    """Deterministic stub box: fraction of the hull bbox, centred on the hull."""
    lo, hi = hull_bbox
    centre = tuple((lo[a] + hi[a]) * 0.5 for a in range(3))
    size = tuple((hi[a] - lo[a]) * fraction for a in range(3))
    if any(s <= 0.0 for s in size):
        return None
    min_corner = tuple(centre[a] - size[a] * 0.5 for a in range(3))
    max_corner = tuple(centre[a] + size[a] * 0.5 for a in range(3))
    return (min_corner, max_corner)


def resolve_unknowns(unknowns, hull_bbox, resolved_by_supplement):
    """Resolve the manifest unknowns into stubs / unresolved entries.

    resolved_by_supplement maps a part_id to the list of applied supplement ids.
    Returns (stubs, results, diagnostics); stubs is a list of
    (unknown_id, min_corner, max_corner, detail).
    """
    stubs = []
    results = []
    diagnostics = []
    for unknown in unknowns:
        unknown_id = str(unknown.get("id", "?"))
        policy = str(unknown.get("policy", "")).lower()
        affected = [str(p) for p in (unknown.get("affected_parts") or [])]
        applied = []
        for part_id in affected:
            applied.extend(resolved_by_supplement.get(part_id, []))
        if policy == "stub_allowed":
            box = stub_box(hull_bbox) if hull_bbox else None
            if box is None:
                results.append(
                    {
                        "id": unknown_id,
                        "status": "unresolved",
                        "detail": "no hull extent available for a stub box",
                    }
                )
                diagnostics.append(
                    {
                        "code": "UNKNOWN_STUB_IMPOSSIBLE",
                        "severity": "warning",
                        "message": "unknown %r cannot be stubbed: the hull is empty"
                        % unknown_id,
                        "address": "unknowns[%s]" % unknown_id,
                    }
                )
                continue
            detail = (
                "stub box of %.0f%% of the hull bbox centred on the hull centre "
                "(deterministic guess: input package v0 carries no geometry for "
                "unknowns)" % (STUB_BOX_FRACTION * 100.0)
            )
            stubs.append((unknown_id, box[0], box[1], detail))
            results.append(
                {
                    "id": unknown_id,
                    "status": "stub",
                    "stub_obj": "stub_%s.obj" % unknown_id,
                    "detail": detail,
                }
            )
        elif policy == "rule_allowed":
            if applied:
                results.append(
                    {
                        "id": unknown_id,
                        "status": "resolved",
                        "detail": "resolved by supplement(s) %s"
                        % ", ".join(sorted(applied)),
                    }
                )
            else:
                results.append(
                    {
                        "id": unknown_id,
                        "status": "unresolved",
                        "detail": "no supplement resolved this unknown; "
                        "an author decision is required",
                    }
                )
                diagnostics.append(
                    {
                        "code": "UNKNOWN_UNRESOLVED",
                        "severity": "warning",
                        "message": "unknown %r is unresolved" % unknown_id,
                        "address": "unknowns[%s]" % unknown_id,
                    }
                )
        else:
            results.append(
                {
                    "id": unknown_id,
                    "status": "unresolved",
                    "detail": "policy %r requires an author decision; the hull was "
                    "not modified" % (policy or "unset"),
                }
            )
            diagnostics.append(
                {
                    "code": "UNKNOWN_UNRESOLVED",
                    "severity": "warning",
                    "message": "unknown %r is unresolved" % unknown_id,
                    "address": "unknowns[%s]" % unknown_id,
                }
            )
    return (stubs, results, diagnostics)
