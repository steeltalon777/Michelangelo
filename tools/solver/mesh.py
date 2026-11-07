"""Blocky (voxel) surface meshing to Wavefront OBJ (stdlib only).

Only faces between a solid and an empty cell are emitted, the winding is
counter clockwise seen from outside, vertices are deduplicated deterministically
on the grid corners and the object is written as a single OBJ object.
"""

from __future__ import annotations

import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from carve import Grid, VoxelSet  # noqa: E402

#: face order used for every voxel (deterministic output)
FACE_ORDER = ("-X", "+X", "-Y", "+Y", "-Z", "+Z")


def _corner_index(grid: Grid, i: int, j: int, k: int) -> int:
    nx, ny, _ = grid.dims
    return i + (nx + 1) * (j + (ny + 1) * k)


def _corner_position(grid: Grid, i: int, j: int, k: int) -> tuple:
    size = grid.voxel_size
    return (
        grid.min_corner[0] + i * size,
        grid.min_corner[1] + j * size,
        grid.min_corner[2] + k * size,
    )


def _quad_corners(grid: Grid, i: int, j: int, k: int, face: str):
    """Corner offsets of a voxel face, counter clockwise seen from outside."""
    if face == "-X":
        return ((i, j, k + 1), (i, j + 1, k + 1), (i, j + 1, k), (i, j, k))
    if face == "+X":
        return ((i + 1, j, k), (i + 1, j + 1, k), (i + 1, j + 1, k + 1), (i + 1, j, k + 1))
    if face == "-Y":
        return ((i, j, k), (i + 1, j, k), (i + 1, j, k + 1), (i, j, k + 1))
    if face == "+Y":
        return ((i, j + 1, k), (i, j + 1, k + 1), (i + 1, j + 1, k + 1), (i + 1, j + 1, k))
    if face == "-Z":
        return ((i, j, k), (i, j + 1, k), (i + 1, j + 1, k), (i + 1, j, k))
    if face == "+Z":
        return ((i, j, k + 1), (i + 1, j, k + 1), (i + 1, j + 1, k + 1), (i, j + 1, k + 1))
    raise ValueError("unknown face %r" % face)


def voxel_surface(voxels: VoxelSet):
    """Return (vertices, faces) for the surface of a voxel set.

    vertices is a list of (x, y, z) tuples in metres, faces a list of 4-tuples of
    zero based vertex indices.
    """
    grid = voxels.grid
    nx, ny, nz = grid.dims
    solid = voxels.indices
    corner_map = {}
    vertices = []
    faces = []
    for index in voxels.sorted_indices():
        k = index // (nx * ny)
        rest = index - k * nx * ny
        j = rest // nx
        i = rest - j * nx
        neighbours = {
            "-X": (i - 1, j, k),
            "+X": (i + 1, j, k),
            "-Y": (i, j - 1, k),
            "+Y": (i, j + 1, k),
            "-Z": (i, j, k - 1),
            "+Z": (i, j, k + 1),
        }
        for face in FACE_ORDER:
            ni, nj, nk = neighbours[face]
            if 0 <= ni < nx and 0 <= nj < ny and 0 <= nk < nz:
                if grid.index(ni, nj, nk) in solid:
                    continue
            quad = []
            for corner in _quad_corners(grid, i, j, k, face):
                corner_id = _corner_index(grid, *corner)
                vertex_id = corner_map.get(corner_id)
                if vertex_id is None:
                    vertex_id = len(vertices)
                    corner_map[corner_id] = vertex_id
                    vertices.append(_corner_position(grid, *corner))
                quad.append(vertex_id)
            faces.append(tuple(quad))
    return (vertices, faces)


def write_obj_mesh(path: str, name: str, vertices, faces) -> None:
    """Write a single object OBJ file (metres).

    OBJ files are Y-up by convention; the solver works in the package frame
    (Z-up, metres).  The stored coordinates are therefore (x, z, -y): a proper
    rotation (det = +1), so face winding is preserved, and Blender's default OBJ
    importer (Y-up -> Z-up) restores the package frame exactly.
    """
    lines = ["# michelangelo solver output (OBJ Y-up; package frame Z-up)", "o %s" % name]
    for x, y, z in vertices:
        lines.append("v %.6f %.6f %.6f" % (x, z, -y))
    for face in faces:
        lines.append("f " + " ".join(str(index + 1) for index in face))
    directory = os.path.dirname(os.path.abspath(path))
    if directory and not os.path.isdir(directory):
        os.makedirs(directory, exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write("\n".join(lines))
        handle.write("\n")


def write_obj(path: str, name: str, voxels: VoxelSet, grid: Grid = None) -> None:
    """Write the blocky surface of a voxel set to an OBJ file."""
    if grid is None:
        grid = voxels.grid
    vertices, faces = voxel_surface(voxels)
    write_obj_mesh(path, name, vertices, faces)


def box_mesh(min_corner, max_corner):
    """Return (vertices, faces) of an axis aligned box with outward winding."""
    x0, y0, z0 = (float(c) for c in min_corner)
    x1, y1, z1 = (float(c) for c in max_corner)
    vertices = [
        (x0, y0, z0),
        (x1, y0, z0),
        (x1, y1, z0),
        (x0, y1, z0),
        (x0, y0, z1),
        (x1, y0, z1),
        (x1, y1, z1),
        (x0, y1, z1),
    ]
    faces = [
        (0, 3, 2, 1),  # -Z
        (4, 5, 6, 7),  # +Z
        (4, 7, 3, 0),  # -X
        (1, 2, 6, 5),  # +X
        (0, 1, 5, 4),  # -Y
        (3, 7, 6, 2),  # +Y
    ]
    return (vertices, faces)
