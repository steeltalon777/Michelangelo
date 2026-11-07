"""PNG decoding for the Michelangelo Stage B solver (Python standard library only).

Supported subset: 8-bit greyscale (colour type 0), greyscale+alpha (4), RGB (2) and
RGBA (6), non-interlaced images.  Palette (3) and 16-bit images are rejected with
explicit errors.  Only zlib is used for decompression; all five PNG filter types
are implemented.

The reader also decodes the silhouette renders produced by the Blender worker
(RGBA8, alpha channel carries the silhouette).
"""

from __future__ import annotations

import os
import struct
import sys
import zlib
from dataclasses import dataclass

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from mask import Mask  # noqa: E402  (sibling module, path fixed above)

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"

#: channel count per supported PNG colour type
MODE_CHANNELS = {"gray": 1, "gray_alpha": 2, "rgb": 3, "rgba": 4}

#: default foreground threshold (0 = background, >= 128 = object)
DEFAULT_THRESHOLD = 128


class PngError(Exception):
    """Raised for malformed or unsupported PNG data."""


@dataclass(frozen=True)
class Image:
    """Decoded 8-bit PNG image.

    pixels holds the raw samples in row-major order, channels samples per pixel,
    without any filter bytes.
    """

    width: int
    height: int
    mode: str
    pixels: bytes

    @property
    def channels(self) -> int:
        return MODE_CHANNELS[self.mode]

    def sample(self, x: int, y: int) -> tuple[int, ...]:
        """Return the channel tuple of pixel (x, y)."""
        if x < 0 or y < 0 or x >= self.width or y >= self.height:
            raise IndexError("pixel outside image")
        base = (y * self.width + x) * self.channels
        return tuple(self.pixels[base:base + self.channels])


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa = abs(p - a)
    pb = abs(p - b)
    pc = abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def _unfilter(raw: bytes, width: int, height: int, bpp: int) -> bytes:
    stride = width * bpp
    expected = height * (stride + 1)
    if len(raw) < expected:
        raise PngError(
            "truncated PNG image data: expected %d bytes, got %d" % (expected, len(raw))
        )
    out = bytearray()
    prev = bytearray(stride)
    pos = 0
    for _ in range(height):
        ftype = raw[pos]
        pos += 1
        line = bytearray(raw[pos:pos + stride])
        pos += stride
        if ftype == 0:
            pass
        elif ftype == 1:  # Sub
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 0xFF
        elif ftype == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:  # Average
            for i in range(stride):
                left = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:  # Paeth
            for i in range(stride):
                left = line[i - bpp] if i >= bpp else 0
                up_left = prev[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + _paeth(left, prev[i], up_left)) & 0xFF
        else:
            raise PngError("unknown PNG filter type %d" % ftype)
        out += line
        prev = line
    return bytes(out)


def read_png_bytes(data: bytes) -> Image:
    """Decode PNG bytes into an Image."""
    if not data.startswith(PNG_SIGNATURE):
        raise PngError("not a PNG file: bad signature")
    pos = len(PNG_SIGNATURE)
    header = None
    idat = bytearray()
    seen_iend = False
    while pos < len(data):
        if pos + 8 > len(data):
            raise PngError("truncated PNG: incomplete chunk header")
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        ctype = data[pos + 4:pos + 8]
        pos += 8
        if length > 0x7FFFFFFF:
            raise PngError("invalid PNG chunk length")
        if pos + length + 4 > len(data):
            raise PngError("truncated PNG chunk %s" % ctype.decode("ascii", "replace"))
        chunk = data[pos:pos + length]
        pos += length + 4
        if ctype == b"IHDR":
            if header is not None:
                raise PngError("duplicate PNG IHDR chunk")
            if length != 13:
                raise PngError("invalid PNG IHDR chunk")
            header = struct.unpack(">IIBBBBB", chunk)
        elif ctype == b"IDAT":
            if header is None:
                raise PngError("PNG IDAT chunk before IHDR")
            idat += chunk
        elif ctype == b"IEND":
            seen_iend = True
            break
        elif ctype[:1].isupper():
            raise PngError(
                "unsupported critical PNG chunk %s" % ctype.decode("ascii", "replace")
            )
        # ancillary chunks (tEXt, pHYs, gAMA, ...) are ignored
    if header is None:
        raise PngError("PNG without IHDR chunk")
    if not seen_iend:
        raise PngError("truncated PNG: missing IEND chunk")
    width, height, bit_depth, color_type, compression, filter_method, interlace = header
    if width <= 0 or height <= 0:
        raise PngError("invalid PNG dimensions %dx%d" % (width, height))
    if bit_depth != 8:
        raise PngError(
            "unsupported PNG bit depth %d (only 8-bit is supported)" % bit_depth
        )
    if color_type == 3:
        raise PngError("unsupported PNG colour type 3 (palette); use gray8 or RGBA8")
    if color_type == 5:
        raise PngError("unsupported PNG colour type 5 (gray+alpha)")
    if color_type not in (0, 2, 4, 6):
        raise PngError("unsupported PNG colour type %d" % color_type)
    if compression != 0:
        raise PngError("unsupported PNG compression method %d" % compression)
    if filter_method != 0:
        raise PngError("unsupported PNG filter method %d" % filter_method)
    if interlace != 0:
        raise PngError("interlaced PNG is not supported")
    if not idat:
        raise PngError("PNG without IDAT data")
    try:
        raw = zlib.decompress(bytes(idat))
    except zlib.error as exc:
        raise PngError("PNG zlib error: %s" % exc) from exc
    channels = {0: 1, 2: 3, 4: 2, 6: 4}[color_type]
    mode = {0: "gray", 2: "rgb", 4: "gray_alpha", 6: "rgba"}[color_type]
    pixels = _unfilter(raw, width, height, channels)
    return Image(width=width, height=height, mode=mode, pixels=pixels)


def read_png(path: str) -> Image:
    """Read and decode the PNG file at the given path."""
    try:
        with open(path, "rb") as handle:
            data = handle.read()
    except OSError as exc:
        raise PngError("cannot read PNG %s: %s" % (path, exc)) from exc
    return read_png_bytes(data)


def mask_from_gray(img: Image, threshold: int = DEFAULT_THRESHOLD) -> Mask:
    """Build a foreground mask from an 8-bit greyscale image."""
    if img.mode != "gray":
        raise PngError("mask_from_gray requires a greyscale image, got %r" % img.mode)
    data = bytearray(img.width * img.height)
    raw = img.pixels
    for i, value in enumerate(raw):
        if value >= threshold:
            data[i] = 1
    return Mask(img.width, img.height, data)


def mask_from_alpha(img: Image, threshold: int = DEFAULT_THRESHOLD) -> Mask:
    """Build a foreground mask from the alpha channel of an RGBA/gray+alpha image."""
    if img.mode not in ("rgba", "gray_alpha"):
        raise PngError("mask_from_alpha requires an image with alpha, got %r" % img.mode)
    offset = 3 if img.mode == "rgba" else 1
    data = bytearray(img.width * img.height)
    raw = img.pixels
    for i in range(img.width * img.height):
        if raw[i * img.channels + offset] >= threshold:
            data[i] = 1
    return Mask(img.width, img.height, data)


def mask_from_png(path: str, threshold: int = DEFAULT_THRESHOLD) -> Mask:
    """Read a PNG and convert it to a foreground mask, dispatching on its mode."""
    img = read_png(path)
    if img.mode == "gray":
        return mask_from_gray(img, threshold)
    if img.mode in ("rgba", "gray_alpha"):
        return mask_from_alpha(img, threshold)
    raise PngError("cannot build a mask from a %r PNG" % img.mode)
