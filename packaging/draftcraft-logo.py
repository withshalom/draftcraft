#!/usr/bin/env python3
"""Draw DraftCraft's logo: the app icon beside the wordmark "DraftCraft", Draft in bold.

Original artwork made for DraftCraft (MIT OR Apache-2.0). The wordmark is set in Source Sans 3 (SIL Open
Font License 1.1, from storytold/craft-fonts), Bold for Draft and Regular for Craft, as DRAWCraft's is, and converted to
outlines, so the logo needs no font to show and no font file lives in this repository. The icon is
assets/app-icon/draftcraft.svg (packaging/compass-icon.py).

    python3 packaging/draftcraft-logo.py [FONTS_DIR]

FONTS_DIR is a checkout of craft-fonts (default: $CRAFT_FONTS_DIR). Writes docs/images/draftcraft-logo.svg
(navy type, for light backgrounds) and draftcraft-logo-dark.svg (light type, for dark ones). Needs
fontTools (pip install fonttools).
"""

import os
import re
import sys

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NAVY = "#0b2547"
LIGHT = "#eaf1fb"

ICON = 360  # the icon's side in the logo
GAP = 52  # icon to wordmark
CAP = 150  # cap height of the wordmark


def word(path, text, x, baseline, cap_height):
    """`text` in the font at `path` as one SVG path: (d, advance)."""
    font = TTFont(path)
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    cap = getattr(font["OS/2"], "sCapHeight", 0) or font["head"].unitsPerEm * 0.7
    k = cap_height / cap
    pen = SVGPathPen(glyphs, lambda v: f"{v:.1f}".rstrip("0").rstrip("."))
    adv = 0.0
    for ch in text:
        name = cmap[ord(ch)]
        tp = TransformPen(pen, (k, 0, 0, -k, x + adv * k, baseline))
        glyphs[name].draw(tp)
        adv += glyphs[name].width
    return pen.getCommands(), adv * k


def logo(fonts, ink):
    icon = open(os.path.join(ROOT, "assets", "app-icon", "draftcraft.svg")).read()
    inner = re.sub(r"^<svg[^>]*>|</svg>$", "", icon.strip())
    baseline = ICON / 2 + CAP / 2
    x = ICON + GAP
    d1, a1 = word(os.path.join(fonts, "fonts", "source-sans-3", "SourceSans3-Bold.ttf"), "Draft", x, baseline, CAP)
    d2, a2 = word(os.path.join(fonts, "fonts", "source-sans-3", "SourceSans3-Regular.ttf"), "Craft", x + a1 + CAP * 0.04, baseline, CAP)
    width = x + a1 + CAP * 0.04 + a2 + 8
    k = ICON / 1024
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.0f} {ICON}" width="{width:.0f}" height="{ICON}" role="img" aria-label="DraftCraft">'
        f'<g transform="scale({k:.6f})">{inner}</g>'
        f'<path fill="{ink}" d="{d1}"/><path fill="{ink}" d="{d2}"/></svg>\n'
    )


def main():
    fonts = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("CRAFT_FONTS_DIR", "")
    if not os.path.isfile(os.path.join(fonts, "fonts", "source-sans-3", "SourceSans3-Bold.ttf")):
        sys.exit("usage: draftcraft-logo.py FONTS_DIR (a checkout of storytold/craft-fonts)")
    out = os.path.join(ROOT, "docs", "images")
    os.makedirs(out, exist_ok=True)
    for name, ink in (("draftcraft-logo.svg", NAVY), ("draftcraft-logo-dark.svg", LIGHT)):
        with open(os.path.join(out, name), "w") as f:
            f.write(logo(fonts, ink))
    print(f"logos written to {out}")


if __name__ == "__main__":
    main()
