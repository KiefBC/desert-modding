#!/usr/bin/env python3
"""Rasterise assets/logo.svg into the raw RGBA bytes the overlay embeds.

    python3 tools/logo-to-rgba.py            writes desert-overlay/src/logo.rgba
    python3 tools/logo-to-rgba.py --png out.png   also writes a PNG to look at

The logo is pixel art: every path is made of M/m, h, v and z commands on an
integer grid inside a 64 x 64 viewBox, with crispEdges rendering. That is
simple enough to fill by hand (even-odd rule at pixel centres, paths painted in
document order) and keeps an SVG library out of both the build and the DLL.

The output is the picture scaled up SCALE times with nearest-neighbour, as
SIZE x SIZE pixels of straight (non-premultiplied) RGBA, row-major, top-left
first: exactly what hudhook's `RenderContext::load_texture` takes. Scaling up
here means the linear sampler in the game shrinks a sharp image rather than
enlarging a tiny one, which keeps the pixel look.

Rerun after editing the SVG; the overlay's unit test checks the byte count.
"""
import re
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SVG = ROOT / "assets" / "logo.svg"
OUT = ROOT / "desert-overlay" / "src" / "logo.rgba"
GRID = 64
SCALE = 2
SIZE = GRID * SCALE

TOKEN = re.compile(r"([MmHhVvZz])|(-?\d+(?:\.\d+)?)")


def subpaths(d):
    """Yield each closed subpath as a list of (x, y) vertices."""
    tokens = TOKEN.findall(d)
    cmd = None
    x = y = 0.0
    start = (0.0, 0.0)
    pts = []
    i = 0
    nums = []
    for letter, num in tokens:
        if letter:
            if letter in "Zz":
                if pts:
                    yield pts
                pts = []
                x, y = start
            cmd = letter
            continue
        nums.append(float(num))
        need = 2 if cmd in "Mm" else 1
        if len(nums) < need:
            continue
        if cmd == "M":
            x, y = nums
            start = (x, y)
            pts = [(x, y)]
            cmd = "L"  # further pairs would be lineto; the logo has none
        elif cmd == "m":
            x, y = x + nums[0], y + nums[1]
            start = (x, y)
            pts = [(x, y)]
            cmd = "l"
        elif cmd == "H":
            x = nums[0]
            pts.append((x, y))
        elif cmd == "h":
            x += nums[0]
            pts.append((x, y))
        elif cmd == "V":
            y = nums[0]
            pts.append((x, y))
        elif cmd == "v":
            y += nums[0]
            pts.append((x, y))
        else:
            sys.exit(f"logo-to-rgba: unsupported path command {cmd!r}")
        nums = []
    if pts:
        yield pts


def inside(poly, px, py):
    """Even-odd point-in-polygon for a rectilinear polygon."""
    hit = False
    n = len(poly)
    for i in range(n):
        x1, y1 = poly[i]
        x2, y2 = poly[(i + 1) % n]
        if (y1 > py) != (y2 > py):
            xi = x1 + (py - y1) * (x2 - x1) / (y2 - y1)
            if px < xi:
                hit = not hit
    return hit


def parse_fill(s):
    s = s.lstrip("#")
    return tuple(int(s[i : i + 2], 16) for i in (0, 2, 4))


def main():
    text = SVG.read_text()
    paths = re.findall(r'<path fill="(#[0-9a-fA-F]{6})" d="([^"]+)"', text)
    if not paths:
        sys.exit("logo-to-rgba: no <path fill= d=> elements found")

    grid = [[(0, 0, 0, 0)] * GRID for _ in range(GRID)]
    for fill, d in paths:
        rgb = parse_fill(fill)
        for poly in subpaths(d):
            xs = [p[0] for p in poly]
            ys = [p[1] for p in poly]
            for y in range(max(0, int(min(ys))), min(GRID, int(max(ys)) + 1)):
                for x in range(max(0, int(min(xs))), min(GRID, int(max(xs)) + 1)):
                    if inside(poly, x + 0.5, y + 0.5):
                        grid[y][x] = rgb + (255,)

    out = bytearray()
    for y in range(SIZE):
        row = grid[y // SCALE]
        for x in range(SIZE):
            out += bytes(row[x // SCALE])
    OUT.write_bytes(out)
    opaque = sum(1 for row in grid for px in row if px[3])
    print(f"logo-to-rgba: {OUT.relative_to(ROOT)}: {SIZE}x{SIZE} RGBA, {len(out)} bytes, {opaque} of {GRID * GRID} source pixels opaque")

    if len(sys.argv) >= 3 and sys.argv[1] == "--png":
        raw = b"".join(b"\x00" + bytes(out[y * SIZE * 4 : (y + 1) * SIZE * 4]) for y in range(SIZE))

        def chunk(tag, data):
            body = tag + data
            return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

        png = b"\x89PNG\r\n\x1a\n"
        png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(raw, 9))
        png += chunk(b"IEND", b"")
        Path(sys.argv[2]).write_bytes(png)
        print(f"logo-to-rgba: wrote {sys.argv[2]}")


if __name__ == "__main__":
    main()
