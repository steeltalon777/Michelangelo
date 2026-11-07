"""Visual hull derivation and voxel carving (stdlib only).

The hull AABB is derived from the calibrated view mask bboxes: every mask bbox is
unprojected onto the world axes of its view plane, giving four half space planes
per view.  The intersection of those half spaces is the set of world points that
are compatible with every silhouette; its axis aligned bounding box is the hull
AABB.  Degenerate cases (unbounded along an axis, empty intersection) raise
CarveError.
"""

from __future__ import annotations

import math
import os
import sys
from dataclasses import dataclass

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from mask import Mask, View, vec_cross, vec_dot, vec_neg, vec_norm  # noqa: E402

#: default number of voxels along the longest axis of the hull
DEFAULT_RESOLUTION = 64
#: hard cap for the requested resolution
MAX_RESOLUTION = 192
#: hard cap for the total number of voxels
MAX_VOXELS = 2_000_000

_EPS = 1e-9


class CarveError(Exception):
    """Raised when the hull cannot be derived or carved."""


@dataclass(frozen=True)
class Grid:
    """A uniform, axis aligned voxel grid in metres."""

    min_corner: tuple
    max_corner: tuple
    dims: tuple
    voxel_size: float

    @property
    def nx(self) -> int:
        return self.dims[0]

    @property
    def ny(self) -> int:
        return self.dims[1]

    @property
    def nz(self) -> int:
        return self.dims[2]

    def count(self) -> int:
        return self.dims[0] * self.dims[1] * self.dims[2]

    def index(self, i: int, j: int, k: int) -> int:
        return i + self.dims[0] * (j + self.dims[1] * k)

    def coords(self, index: int):
        nx, ny, nz = self.dims
        k = index // (nx * ny)
        rest = index - k * nx * ny
        j = rest // nx
        i = rest - j * nx
        return (i, j, k)

    def voxel_center(self, i: int, j: int, k: int) -> tuple:
        s = self.voxel_size
        return (
            self.min_corner[0] + (i + 0.5) * s,
            self.min_corner[1] + (j + 0.5) * s,
            self.min_corner[2] + (k + 0.5) * s,
        )

    def world_bounds(self, index: int):
        """Return (min_corner, max_corner) of the voxel with the given index."""
        i, j, k = self.coords(index)
        s = self.voxel_size
        lo = (
            self.min_corner[0] + i * s,
            self.min_corner[1] + j * s,
            self.min_corner[2] + k * s,
        )
        hi = (lo[0] + s, lo[1] + s, lo[2] + s)
        return (lo, hi)


@dataclass(frozen=True)
class VoxelSet:
    """An immutable set of voxels of a Grid."""

    grid: Grid
    indices: frozenset

    def count(self) -> int:
        return len(self.indices)

    def contains(self, i: int, j: int, k: int) -> bool:
        return self.grid.index(i, j, k) in self.indices

    def contains_index(self, index: int) -> bool:
        return index in self.indices

    def sorted_indices(self):
        return sorted(self.indices)

    def with_indices(self, indices) -> "VoxelSet":
        return VoxelSet(self.grid, frozenset(indices))

    def bbox(self):
        """World bbox (min_corner, max_corner) of the solid voxels or None."""
        if not self.indices:
            return None
        nx, ny, nz = self.grid.dims
        mins = [None, None, None]
        maxs = [None, None, None]
        for index in self.indices:
            k = index // (nx * ny)
            rest = index - k * nx * ny
            j = rest // nx
            i = rest - j * nx
            coords = (i, j, k)
            for axis, value in enumerate(coords):
                if mins[axis] is None or value < mins[axis]:
                    mins[axis] = value
                if maxs[axis] is None or value > maxs[axis]:
                    maxs[axis] = value
        s = self.grid.voxel_size
        lo = tuple(self.grid.min_corner[a] + mins[a] * s for a in range(3))
        hi = tuple(self.grid.min_corner[a] + (maxs[a] + 1) * s for a in range(3))
        return (lo, hi)


# --- linear algebra helpers -------------------------------------------------
def _det3(a, b, c) -> float:
    return (
        a[0] * (b[1] * c[2] - b[2] * c[1])
        - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0])
    )


def _solve3(n1, n2, n3, c1, c2, c3):
    """Solve the 3x3 system n_i . x = c_i; returns None when singular."""
    det = _det3(n1, n2, n3)
    if abs(det) < 1e-12:
        return None
    x = _det3((c1, n1[1], n1[2]), (c2, n2[1], n2[2]), (c3, n3[1], n3[2])) / det
    y = _det3((n1[0], c1, n1[2]), (n2[0], c2, n2[2]), (n3[0], c3, n3[2])) / det
    z = _det3((n1[0], n1[1], c1), (n2[0], n2[1], c2), (n3[0], n3[1], c3)) / det
    return (x, y, z)


def _planes_for_view(view: View, mask: Mask):
    """Return the four half space planes implied by one view mask bbox."""
    bbox = mask.bbox()
    if bbox is None:
        raise CarveError("view %r has an empty mask" % view.view_id)
    u_lo, u_hi, v_lo, v_hi = view.pixel_bbox_to_camera(bbox)
    du = vec_dot(view.u_axis, view.look_at_m)
    dv = vec_dot(view.v_axis, view.look_at_m)
    return [
        (view.u_axis, du + u_hi),
        (vec_neg(view.u_axis), -(du + u_lo)),
        (view.v_axis, dv + v_hi),
        (vec_neg(view.v_axis), -(dv + v_lo)),
    ]


def _feasible(planes, point, tol=1e-7) -> bool:
    for normal, bound in planes:
        if vec_dot(normal, point) > bound + tol:
            return False
    return True


def _recession_rays(planes):
    """Extreme rays of the recession cone of the half space system."""
    normals = [normal for normal, _ in planes]
    rays = []
    count = len(normals)
    for i in range(count):
        for j in range(i + 1, count):
            ray = vec_cross(normals[i], normals[j])
            length = vec_norm(ray)
            if length < 1e-12:
                continue
            ray = (ray[0] / length, ray[1] / length, ray[2] / length)
            for candidate in (ray, vec_neg(ray)):
                if all(vec_dot(normal, candidate) <= 1e-9 for normal in normals):
                    rays.append(candidate)
    return rays


def _unbounded_axes(planes):
    """Return the list of world axes along which the intersection is unbounded."""
    rays = _recession_rays(planes)
    axes = []
    for axis in range(3):
        low = min((ray[axis] for ray in rays), default=0.0)
        high = max((ray[axis] for ray in rays), default=0.0)
        if high > 1e-9 or low < -1e-9:
            axes.append(axis)
    return axes


def derive_hull_bbox(views_and_masks):
    """Derive the world AABB (metres) from calibrated view masks.

    views_and_masks is a sequence of (View, Mask) pairs.
    """
    views_and_masks = list(views_and_masks)
    if not views_and_masks:
        raise CarveError("no calibrated views with masks")
    planes = []
    for view, mask in views_and_masks:
        planes.extend(_planes_for_view(view, mask))
    if len(planes) < 3:
        raise CarveError("at least three half space planes are required")
    unbounded = _unbounded_axes(planes)
    if unbounded:
        names = "xyz"
        raise CarveError(
            "the silhouette intersection is unbounded along axis %s; "
            "more non-parallel calibrated views are required"
            % ", ".join(names[a] for a in unbounded)
        )
    vertices = []
    count = len(planes)
    for i in range(count):
        for j in range(i + 1, count):
            for k in range(j + 1, count):
                n1, c1 = planes[i]
                n2, c2 = planes[j]
                n3, c3 = planes[k]
                point = _solve3(n1, n2, n3, c1, c2, c3)
                if point is None:
                    continue
                if not all(math.isfinite(value) for value in point):
                    continue
                if _feasible(planes, point):
                    vertices.append(point)
    if len(vertices) < 4:
        raise CarveError(
            "the silhouette intersection is degenerate (only %d vertex/vertices)"
            % len(vertices)
        )
    lo = tuple(min(v[a] for v in vertices) for a in range(3))
    hi = tuple(max(v[a] for v in vertices) for a in range(3))
    for axis in range(3):
        if hi[axis] - lo[axis] <= 1e-12:
            raise CarveError(
                "the derived hull has a degenerate extent along axis %d" % axis
            )
    return (lo, hi)


def make_grid(min_corner, max_corner, resolution: int = DEFAULT_RESOLUTION):
    """Create a uniform voxel grid; returns (Grid, clamped_resolution)."""
    resolution = int(resolution)
    if resolution < 1:
        raise CarveError("resolution must be >= 1")
    clamped = min(resolution, MAX_RESOLUTION)
    extents = [max_corner[a] - min_corner[a] for a in range(3)]
    longest = max(extents)
    if longest <= 0.0:
        raise CarveError("the derived hull has a zero extent")
    voxel_size = longest / float(clamped)
    dims = None
    while True:
        dims = tuple(
            max(1, int(math.ceil(extents[a] / voxel_size - 1e-9))) for a in range(3)
        )
        total = dims[0] * dims[1] * dims[2]
        if total <= MAX_VOXELS:
            break
        voxel_size *= 1.05
    grid_max = tuple(min_corner[a] + dims[a] * voxel_size for a in range(3))
    grid = Grid(
        min_corner=tuple(float(c) for c in min_corner),
        max_corner=grid_max,
        dims=dims,
        voxel_size=voxel_size,
    )
    return (grid, clamped)


def carve(grid: Grid, views_and_masks) -> VoxelSet:
    """Carve the grid: a voxel is solid iff it projects inside every mask."""
    views_and_masks = list(views_and_masks)
    if not views_and_masks:
        raise CarveError("no calibrated views with masks")
    nx, ny, nz = grid.dims
    size = grid.voxel_size
    ox, oy, oz = grid.min_corner
    prepared = []
    for view, mask in views_and_masks:
        prepared.append(
            (
                view.u_axis[0], view.u_axis[1], view.u_axis[2],
                view.v_axis[0], view.v_axis[1], view.v_axis[2],
                view.look_at_m[0], view.look_at_m[1], view.look_at_m[2],
                view.ortho_scale_m,
                float(view.width_px), float(view.height_px),
                mask,
            )
        )
    solid = set()
    for k in range(nz):
        z = oz + (k + 0.5) * size
        for j in range(ny):
            y = oy + (j + 0.5) * size
            for i in range(nx):
                x = ox + (i + 0.5) * size
                inside = True
                for (
                    ux, uy, uz, vx, vy, vz, lx, ly, lz, scale, width, height, mask
                ) in prepared:
                    rx = x - lx
                    ry = y - ly
                    rz = z - lz
                    u_cam = rx * ux + ry * uy + rz * uz
                    v_cam = rx * vx + ry * vy + rz * vz
                    u_px = int((0.5 + u_cam / scale) * width)
                    v_px = int((0.5 - v_cam / scale) * width)
                    if u_px < 0 or v_px < 0 or u_px >= mask.width or v_px >= mask.height:
                        inside = False
                        break
                    if not mask.at(u_px, v_px):
                        inside = False
                        break
                if inside:
                    solid.add(i + nx * (j + ny * k))
    return VoxelSet(grid, frozenset(solid))
