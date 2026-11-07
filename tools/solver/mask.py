"""Mask helpers and calibrated orthographic view geometry (stdlib only).

Pixel convention (Stage A input package v0, fixed and not negotiable):

    image origin is top-left, u points right, v points down,
    u_axis = normalize(cross(view_dir, up_world)), v_axis = up_world,
    u_px = (0.5 + u_cam / ortho_scale) * width_px,
    v_px = (0.5 - v_cam / ortho_scale) * width_px,

where u_cam / v_cam are camera space coordinates of the point relative to
look_at_m.  ortho_scale is the full visible width of the image.

All lengths of the manifest are expressed in the package units (m, cm, mm); the
helpers convert them to metres on construction of a View.
"""

from __future__ import annotations

import math

#: default foreground threshold (0 = background, >= 128 = object)
MASK_THRESHOLD = 128

#: conversion factor from package units to metres
UNIT_TO_METRES = {"m": 1.0, "cm": 0.01, "mm": 0.001}


class MaskError(Exception):
    """Raised for invalid mask data."""


class ViewError(Exception):
    """Raised for invalid or unsupported view definitions."""


class ShiftNotSupportedError(ViewError):
    """Raised when a view uses a non-zero camera shift (unsupported in v0)."""


# --- vector helpers --------------------------------------------------------
def vec_add(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def vec_sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def vec_scale(a, s):
    return (a[0] * s, a[1] * s, a[2] * s)


def vec_neg(a):
    return (-a[0], -a[1], -a[2])


def vec_dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def vec_cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def vec_norm(a):
    return math.sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])


def vec_unit(a):
    length = vec_norm(a)
    if length <= 0.0:
        raise ViewError("cannot normalise a zero length vector")
    return (a[0] / length, a[1] / length, a[2] / length)


# --- masks ------------------------------------------------------------------
class Mask:
    """A binary foreground mask (1 = object, 0 = background)."""

    __slots__ = ("width", "height", "data")

    def __init__(self, width: int, height: int, data=None):
        width = int(width)
        height = int(height)
        if width <= 0 or height <= 0:
            raise MaskError("mask dimensions must be positive")
        if data is None:
            buffer = bytearray(width * height)
        else:
            buffer = bytearray(data)
            if len(buffer) != width * height:
                raise MaskError(
                    "mask data length %d does not match %dx%d"
                    % (len(buffer), width, height)
                )
        for value in buffer:
            if value not in (0, 1):
                raise MaskError("mask data must contain only 0 or 1")
        self.width = width
        self.height = height
        self.data = buffer

    def at(self, x: int, y: int) -> bool:
        """True when pixel (x, y) is foreground; outside pixels are background."""
        if x < 0 or y < 0 or x >= self.width or y >= self.height:
            return False
        return self.data[y * self.width + x] != 0

    def set(self, x: int, y: int, value: bool = True) -> None:
        if x < 0 or y < 0 or x >= self.width or y >= self.height:
            raise MaskError("pixel outside mask")
        self.data[y * self.width + x] = 1 if value else 0

    def count(self) -> int:
        """Number of foreground pixels."""
        return sum(self.data)

    def coverage(self) -> float:
        """Foreground fraction of the mask area."""
        return self.count() / float(self.width * self.height)

    def bbox(self):
        """Inclusive pixel bbox (x0, y0, x1, y1) or None when the mask is empty."""
        x0 = y0 = None
        x1 = y1 = None
        data = bytes(self.data)
        width = self.width
        for y in range(self.height):
            row = data[y * width:(y + 1) * width]
            first = row.find(1)
            if first < 0:
                continue
            last = row.rfind(1)
            if x0 is None or first < x0:
                x0 = first
            if x1 is None or last > x1:
                x1 = last
            if y0 is None:
                y0 = y
            y1 = y
        if x0 is None:
            return None
        return (x0, y0, x1, y1)

    def __eq__(self, other):
        if not isinstance(other, Mask):
            return NotImplemented
        return (
            self.width == other.width
            and self.height == other.height
            and bytes(self.data) == bytes(other.data)
        )

    def __repr__(self):  # pragma: no cover - debugging helper
        return "Mask(%dx%d, %d px)" % (self.width, self.height, self.count())


def mask_from_values(width: int, height: int, values, threshold: int = MASK_THRESHOLD):
    """Build a mask from a flat sequence of sample values."""
    mask = Mask(width, height)
    values = list(values)
    if len(values) != width * height:
        raise MaskError("value count does not match mask dimensions")
    for i, value in enumerate(values):
        if value >= threshold:
            mask.data[i] = 1
    return mask


def mask_iou(a: Mask, b: Mask) -> float:
    """Deterministic intersection-over-union of two masks of equal size."""
    if a.width != b.width or a.height != b.height:
        raise MaskError("mask dimensions differ: %dx%d vs %dx%d"
                        % (a.width, a.height, b.width, b.height))
    ia = int.from_bytes(bytes(a.data), "little")
    ib = int.from_bytes(bytes(b.data), "little")
    union = (ia | ib).bit_count()
    if union == 0:
        return 1.0
    return (ia & ib).bit_count() / union


def mask_counts(a: Mask, b: Mask):
    """Return (intersection, union, mask_px, render_px) for two masks."""
    if a.width != b.width or a.height != b.height:
        raise MaskError("mask dimensions differ: %dx%d vs %dx%d"
                        % (a.width, a.height, b.width, b.height))
    ia = int.from_bytes(bytes(a.data), "little")
    ib = int.from_bytes(bytes(b.data), "little")
    intersection = (ia & ib).bit_count()
    union = (ia | ib).bit_count()
    return intersection, union, a.count(), b.count()


def sample_mask(mask: Mask, u_px: float, v_px: float) -> bool:
    """Sample a mask at fractional pixel coordinates (floor rounding)."""
    x = int(u_px)
    y = int(v_px)
    return mask.at(x, y)


# --- calibrated orthographic views -----------------------------------------
class View:
    """A calibrated orthographic view (metres, pixels)."""

    def __init__(
        self,
        view_id: str,
        location_m,
        look_at_m,
        up_world,
        ortho_scale_m: float,
        width_px: int,
        height_px: int,
        shift_x: float = 0.0,
        shift_y: float = 0.0,
    ):
        self.view_id = view_id
        self.location_m = tuple(float(c) for c in location_m)
        self.look_at_m = tuple(float(c) for c in look_at_m)
        self.up_world = tuple(float(c) for c in up_world)
        self.ortho_scale_m = float(ortho_scale_m)
        self.width_px = int(width_px)
        self.height_px = int(height_px)
        self.shift_x = float(shift_x)
        self.shift_y = float(shift_y)
        if self.ortho_scale_m <= 0.0:
            raise ViewError("ortho_scale must be positive")
        if self.width_px <= 0 or self.height_px <= 0:
            raise ViewError("view dimensions must be positive")
        if abs(self.shift_x) > 1e-12 or abs(self.shift_y) > 1e-12:
            raise ShiftNotSupportedError(
                "view %r uses a non-zero camera shift; shifts are not supported in v0"
                % self.view_id
            )
        direction = vec_sub(self.look_at_m, self.location_m)
        if vec_norm(direction) <= 0.0:
            raise ViewError("view %r has a degenerate camera direction" % self.view_id)
        self.forward = vec_unit(direction)
        up = vec_unit(self.up_world)
        if abs(vec_dot(self.forward, up)) >= 0.999:
            raise ViewError(
                "view %r: up_world is parallel to the view direction" % self.view_id
            )
        self.v_axis = up
        self.u_axis = vec_unit(vec_cross(self.forward, up))

    # -- geometry ---------------------------------------------------------
    def project_point(self, point_m):
        """Project a world point (metres) to pixel coordinates (u_px, v_px)."""
        if abs(self.shift_x) > 1e-12 or abs(self.shift_y) > 1e-12:
            raise ShiftNotSupportedError(
                "view %r uses a non-zero camera shift; shifts are not supported in v0"
                % self.view_id
            )
        rel = vec_sub(tuple(float(c) for c in point_m), self.look_at_m)
        u_cam = vec_dot(rel, self.u_axis)
        v_cam = vec_dot(rel, self.v_axis)
        u_px = (0.5 + u_cam / self.ortho_scale_m) * self.width_px
        v_px = (0.5 - v_cam / self.ortho_scale_m) * self.width_px
        return (u_px, v_px)

    def unproject_pixel(self, u_px: float, v_px: float):
        """Unproject a pixel to a world point (metres) on the view plane.

        The pixel scale is the same on both axes (`width_px / ortho_scale_m`):
        the visible height follows from the image aspect ratio, so v is scaled
        by `width_px`, not by `height_px` (see the Stage A pixel convention).
        """
        u_cam = (u_px / self.width_px - 0.5) * self.ortho_scale_m
        v_cam = (0.5 - v_px / self.width_px) * self.ortho_scale_m
        point = vec_add(
            self.look_at_m,
            vec_add(vec_scale(self.u_axis, u_cam), vec_scale(self.v_axis, v_cam)),
        )
        return point

    def pixel_bbox_to_camera(self, bbox):
        """Convert an inclusive pixel bbox to camera space (u_lo, u_hi, v_lo, v_hi).

        Both axes use the same pixel scale (`width_px / ortho_scale_m`).
        """
        x0, y0, x1, y1 = bbox
        u_lo = (x0 / self.width_px - 0.5) * self.ortho_scale_m
        u_hi = ((x1 + 1) / self.width_px - 0.5) * self.ortho_scale_m
        v_lo = (0.5 - (y1 + 1) / self.width_px) * self.ortho_scale_m
        v_hi = (0.5 - y0 / self.width_px) * self.ortho_scale_m
        return (u_lo, u_hi, v_lo, v_hi)

    # -- construction -----------------------------------------------------
    @classmethod
    def from_manifest(cls, view: dict, units: str = "m") -> "View":
        """Build a View from a Stage A manifest view entry.

        Manifest lengths are package units and are converted to metres.
        """
        if not isinstance(view, dict):
            raise ViewError("view entry must be an object")
        view_id = str(view.get("view_id", "?"))
        projection = view.get("projection")
        if not isinstance(projection, dict):
            raise ViewError("view %r has no projection" % view_id)
        if projection.get("type") != "orthographic":
            raise ViewError(
                "view %r uses an unsupported projection type %r"
                % (view_id, projection.get("type"))
            )
        factor = UNIT_TO_METRES.get(units)
        if factor is None:
            raise ViewError("unsupported package units %r" % units)
        image = view.get("image") or {}
        width_px = int(image.get("width_px", 0))
        height_px = int(image.get("height_px", 0))
        if width_px <= 0 or height_px <= 0:
            raise ViewError("view %r has no usable image dimensions" % view_id)
        try:
            location = tuple(float(c) * factor for c in projection["location_m"])
            look_at = tuple(float(c) * factor for c in projection["look_at_m"])
            up = tuple(float(c) for c in projection["up_world"])
            ortho_scale = float(projection["ortho_scale_m"]) * factor
        except KeyError as exc:
            raise ViewError("view %r misses projection field %s" % (view_id, exc)) from exc
        except (TypeError, ValueError) as exc:
            raise ViewError("view %r has an invalid projection" % view_id) from exc
        shift_x = float(projection.get("shift_x", 0.0) or 0.0)
        shift_y = float(projection.get("shift_y", 0.0) or 0.0)
        return cls(
            view_id=view_id,
            location_m=location,
            look_at_m=look_at,
            up_world=up,
            ortho_scale_m=ortho_scale,
            width_px=width_px,
            height_px=height_px,
            shift_x=shift_x,
            shift_y=shift_y,
        )

    def __repr__(self):  # pragma: no cover - debugging helper
        return "View(%r, %dx%d, ortho=%.6g)" % (
            self.view_id,
            self.width_px,
            self.height_px,
            self.ortho_scale_m,
        )
