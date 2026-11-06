#!/usr/bin/env python3
"""Draw DraftCraft's app icon: a drafting compass that has just drawn an arc, on a dark blueprint-blue
tile ruled with a drafting grid.

Original artwork made for DraftCraft (MIT OR Apache-2.0). Writes the master SVGs and renders every icon
size CADCraft's packaging uses, at the same paths, so nothing else needs to change:

    python3 packaging/compass-icon.py

writes assets/app-icon/draftcraft.svg (the full tile), draftcraft-macos.svg (the tile on Apple's icon
grid, with its margin), cadcraft-1024.png, cadcraft-macos-512.png, cadcraft-256.png, cadcraft-64.png,
the hicolor PNGs, cadcraft.ico and cadcraft.icns. Needs Python 3 with Pillow and Playwright (its
Chromium renders the SVG).
"""

import io
import math
import os
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "app-icon")
ID = "dev.cadcraft.draft"

# Colours.
DEEP = "#081d38"  # the tile's darkest blueprint blue
BLUE = "#0e3463"  # its lightest
GRID = "#9cc3ff"  # grid lines (drawn translucent)
INK = "#08121f"  # outlines
STEEL = "#e9eef4"  # the compass's legs
STEEL_SHADE = "#b7c3d1"
BRASS = "#d9a441"  # hinge and handle
LINE = "#f4f7fb"  # the drawn arc
ACCENT = "#ff8a3d"  # the pencil lead, CADCraft's orange

S = 1024


def leg(x0, y0, x1, y1, w0, w1):
    """A tapered leg from (x0, y0) (width w0) to (x1, y1) (width w1), as a polygon's points."""
    dx, dy = x1 - x0, y1 - y0
    n = math.hypot(dx, dy)
    px, py = -dy / n, dx / n
    pts = [
        (x0 + px * w0 / 2, y0 + py * w0 / 2),
        (x1 + px * w1 / 2, y1 + py * w1 / 2),
        (x1 - px * w1 / 2, y1 - py * w1 / 2),
        (x0 - px * w0 / 2, y0 - py * w0 / 2),
    ]
    return " ".join(f"{x:.1f},{y:.1f}" for x, y in pts)


def along(x0, y0, x1, y1, t):
    return x0 + (x1 - x0) * t, y0 + (y1 - y0) * t


def tile(radius=228):
    """The blueprint tile with its drafting grid: fine lines every 32, bold every 128."""
    fine = "".join(
        f'<path d="M{v} 0V{S}M0 {v}H{S}"/>' for v in range(32, S, 32) if v % 128
    )
    bold = "".join(f'<path d="M{v} 0V{S}M0 {v}H{S}"/>' for v in range(128, S, 128))
    return f"""<defs>
    <linearGradient id="bp" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="{BLUE}"/>
      <stop offset="1" stop-color="{DEEP}"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.5" cy="0.42" r="0.62">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.10"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </radialGradient>
    <clipPath id="t"><rect width="{S}" height="{S}" rx="{radius}"/></clipPath>
    <linearGradient id="steel" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="{STEEL}"/>
      <stop offset="0.55" stop-color="{STEEL}"/>
      <stop offset="1" stop-color="{STEEL_SHADE}"/>
    </linearGradient>
  </defs>
  <g clip-path="url(#t)">
    <rect width="{S}" height="{S}" fill="url(#bp)"/>
    <g stroke="{GRID}" stroke-opacity="0.16" stroke-width="2" fill="none">{fine}</g>
    <g stroke="{GRID}" stroke-opacity="0.34" stroke-width="3.5" fill="none">{bold}</g>
    <rect width="{S}" height="{S}" fill="url(#glow)"/>
  </g>"""


def compass():
    hx, hy = 512, 214  # hinge
    nx, ny = 366, 660  # needle point
    px, py = 658, 660  # pencil lead
    r = px - nx  # the arc's radius: the compass's spread
    # The arc it drew: from the lead, sweeping down and round the needle.
    a0, a1 = math.radians(0), math.radians(74)
    ax0, ay0 = nx + r * math.cos(a0), ny + r * math.sin(a0)
    ax1, ay1 = nx + r * math.cos(a1), ny + r * math.sin(a1)
    arc = f'<path d="M{ax0:.1f} {ay0:.1f}A{r} {r} 0 0 1 {ax1:.1f} {ay1:.1f}" fill="none" stroke="{LINE}" stroke-width="30" stroke-linecap="round"/>'
    # A faint construction circle the arc belongs to.
    construction = f'<circle cx="{nx}" cy="{ny}" r="{r}" fill="none" stroke="{GRID}" stroke-opacity="0.38" stroke-width="5" stroke-dasharray="26 18"/>'
    # Legs: from the hinge to just above each point.
    lx, ly = along(hx, hy, nx, ny, 0.86)
    rx, ry = along(hx, hy, px, py, 0.80)
    left = f'<polygon points="{leg(hx, hy, lx, ly, 104, 58)}" fill="url(#steel)" stroke="{INK}" stroke-width="18" stroke-linejoin="round"/>'
    right = f'<polygon points="{leg(hx, hy, rx, ry, 104, 64)}" fill="url(#steel)" stroke="{INK}" stroke-width="16" stroke-linejoin="round"/>'
    # The needle: a thin steel point.
    needle = f'<polygon points="{leg(lx, ly, nx, ny, 34, 2)}" fill="{STEEL_SHADE}" stroke="{INK}" stroke-width="10" stroke-linejoin="round"/>'
    # The pencil holder (a collar) and the lead.
    cx0, cy0 = along(hx, hy, px, py, 0.80)
    cx1, cy1 = along(hx, hy, px, py, 0.88)
    collar = f'<polygon points="{leg(cx0, cy0, cx1, cy1, 78, 72)}" fill="{BRASS}" stroke="{INK}" stroke-width="14" stroke-linejoin="round"/>'
    lead = f'<polygon points="{leg(cx1, cy1, px, py, 52, 4)}" fill="{ACCENT}" stroke="{INK}" stroke-width="10" stroke-linejoin="round"/>'
    # The handle on top and the hinge.
    handle = f'<rect x="{hx - 32}" y="{hy - 158}" width="64" height="124" rx="26" fill="{BRASS}" stroke="{INK}" stroke-width="16"/>'
    hinge = (
        f'<circle cx="{hx}" cy="{hy}" r="72" fill="{BRASS}" stroke="{INK}" stroke-width="16"/>'
        f'<circle cx="{hx}" cy="{hy}" r="18" fill="{INK}"/>'
    )
    # Where the needle stands: a small centre mark.
    mark = f'<path d="M{nx - 34} {ny}H{nx + 34}M{nx} {ny - 34}V{ny + 34}" stroke="{LINE}" stroke-opacity="0.7" stroke-width="7" stroke-linecap="round"/>'
    return construction + arc + mark + left + right + needle + collar + lead + handle + hinge


def svg(margin=0):
    """The icon; `margin` insets the tile on its canvas (Apple's grid puts 100 of 1024 around it)."""
    inner = S - 2 * margin
    k = inner / S
    body = tile() + compass()
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{S}" height="{S}">'
        f'<g transform="translate({margin} {margin}) scale({k:.6f})">{body}</g></svg>'
    )


def render(svg_text, size):
    """`svg_text` as a size x size PNG (bytes), by Chromium."""
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        b = p.chromium.launch()
        pg = b.new_page(viewport={"width": size, "height": size}, device_scale_factor=1)
        pg.set_content(
            f'<html><body style="margin:0;background:transparent">'
            f'<div style="width:{size}px;height:{size}px">{svg_text.replace(f"width=\"{S}\" height=\"{S}\"", f"width=\"{size}\" height=\"{size}\"")}</div></body></html>'
        )
        png = pg.screenshot(omit_background=True, clip={"x": 0, "y": 0, "width": size, "height": size})
        b.close()
    return png


def resized(master_png, size):
    from PIL import Image

    im = Image.open(io.BytesIO(master_png)).convert("RGBA").resize((size, size), Image.LANCZOS)
    out = io.BytesIO()
    im.save(out, "PNG", optimize=True)
    return out.getvalue()


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def ico(pngs):
    """A Windows .ico holding the PNGs (each stored as PNG, as Vista and later read them)."""
    head = struct.pack("<HHH", 0, 1, len(pngs))
    entries, blobs = b"", b""
    offset = 6 + 16 * len(pngs)
    for size, data in pngs:
        d = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", d, d, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return head + entries + blobs


def icns(pngs):
    """A macOS .icns of PNG entries: ic07 128, ic08 256, ic09 512, ic10 1024 (512@2x), ic11-14 @2x."""
    kinds = {16: [b"icp4"], 32: [b"icp5", b"ic11"], 64: [b"icp6", b"ic12"], 128: [b"ic07"], 256: [b"ic08", b"ic13"], 512: [b"ic09", b"ic14"], 1024: [b"ic10"]}
    body = b""
    for size, data in pngs:
        for k in kinds.get(size, []):
            body += k + struct.pack(">I", 8 + len(data)) + data
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main(out=OUT):
    full, mac = svg(), svg(margin=100)
    write(os.path.join(out, "draftcraft.svg"), full.encode())
    write(os.path.join(out, "draftcraft-macos.svg"), mac.encode())
    master = render(full, S)
    mac_master = render(mac, S)
    write(os.path.join(out, "cadcraft-1024.png"), master)
    write(os.path.join(out, "cadcraft-macos-512.png"), resized(mac_master, 512))
    write(os.path.join(out, "cadcraft-256.png"), resized(master, 256))
    write(os.path.join(out, "cadcraft-64.png"), resized(master, 64))
    for s in (16, 24, 32, 48, 64, 128, 256, 512):
        write(os.path.join(out, "hicolor", f"{s}x{s}", "apps", f"{ID}.png"), resized(master, s))
    write(os.path.join(out, "cadcraft.ico"), ico([(s, resized(master, s)) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)]))
    write(os.path.join(out, "cadcraft.icns"), icns([(s, resized(mac_master, s)) for s in (16, 32, 64, 128, 256, 512, 1024)]))
    print(f"icons written to {out}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else OUT)
