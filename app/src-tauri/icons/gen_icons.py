"""Generates Remeet's icon PNGs with no image-library dependency.

Two motifs:
- App icon: a soft cream rounded square with a clay recording dot.
- Tray icon: a monochrome dot, emitted as a macOS template image (black + alpha)
  so the menu bar tints it for light/dark automatically.

Run from this directory: `python3 gen_icons.py`
"""

import struct
import zlib
import os


def _png(width, height, pixels):
    """pixels: flat bytearray of RGBA, length width*height*4."""

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)  # no filter
        raw.extend(pixels[y * stride:(y + 1) * stride])

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def _blend(dst, src):
    """Alpha-composite src (r,g,b,a floats 0..1) over dst tuple."""
    sr, sg, sb, sa = src
    dr, dg, db, da = dst
    out_a = sa + da * (1 - sa)
    if out_a == 0:
        return (0.0, 0.0, 0.0, 0.0)
    out_r = (sr * sa + dr * da * (1 - sa)) / out_a
    out_g = (sg * sa + dg * da * (1 - sa)) / out_a
    out_b = (sb * sa + db * da * (1 - sa)) / out_a
    return (out_r, out_g, out_b, out_a)


def _coverage(px, py, cx, cy, r, softness=1.2):
    """Anti-aliased inside-circle coverage 0..1."""
    d = ((px - cx) ** 2 + (py - cy) ** 2) ** 0.5
    return max(0.0, min(1.0, (r - d) / softness + 0.5))


def _rounded_square_coverage(px, py, size, radius, softness=1.2):
    inset = 0.5
    x = min(max(px, inset + radius), size - inset - radius)
    y = min(max(py, inset + radius), size - inset - radius)
    d = ((px - x) ** 2 + (py - y) ** 2) ** 0.5
    return max(0.0, min(1.0, (radius - d) / softness + 0.5))


# Warm cream and clay, matching DESIGN.md (approx sRGB of the OKLCH tokens).
CREAM = (0.968, 0.955, 0.925)
CLAY = (0.72, 0.42, 0.30)


def app_icon(size):
    pixels = bytearray(size * size * 4)
    radius = size * 0.22
    dot_r = size * 0.16
    cx = cy = size / 2
    for y in range(size):
        for x in range(size):
            px, py = x + 0.5, y + 0.5
            out = (0.0, 0.0, 0.0, 0.0)
            sq = _rounded_square_coverage(px, py, size, radius, softness=size * 0.01)
            if sq > 0:
                out = _blend(out, (*CREAM, sq))
            dot = _coverage(px, py, cx, cy, dot_r, softness=size * 0.008)
            if dot > 0:
                out = _blend(out, (*CLAY, dot))
            i = (y * size + x) * 4
            pixels[i] = round(out[0] * 255)
            pixels[i + 1] = round(out[1] * 255)
            pixels[i + 2] = round(out[2] * 255)
            pixels[i + 3] = round(out[3] * 255)
    return _png(size, size, pixels)


def tray_icon(size):
    """Monochrome dot with a hollow ring: a record glyph. Black + alpha template."""
    pixels = bytearray(size * size * 4)
    cx = cy = size / 2
    outer = size * 0.34
    inner = size * 0.20
    for y in range(size):
        for x in range(size):
            px, py = x + 0.5, y + 0.5
            ring = _coverage(px, py, cx, cy, outer, softness=size * 0.05)
            hole = _coverage(px, py, cx, cy, inner, softness=size * 0.05)
            alpha = max(0.0, ring - hole)
            i = (y * size + x) * 4
            pixels[i] = pixels[i + 1] = pixels[i + 2] = 0
            pixels[i + 3] = round(alpha * 255)
    return _png(size, size, pixels)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    for size, name in [(32, "32x32.png"), (128, "128x128.png"),
                       (256, "128x128@2x.png"), (512, "icon.png")]:
        with open(os.path.join(here, name), "wb") as f:
            f.write(app_icon(size))
    with open(os.path.join(here, "tray.png"), "wb") as f:
        f.write(tray_icon(32))
    with open(os.path.join(here, "tray@2x.png"), "wb") as f:
        f.write(tray_icon(64))
    print("wrote app + tray icons")


if __name__ == "__main__":
    main()
