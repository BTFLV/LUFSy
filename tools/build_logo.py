#!/usr/bin/env python3
"""Build the LUFSy logo files as outlined SVG paths.

The wordmark is set in Chakra Petch Bold and converted to paths, so it renders
the same on GitHub and anywhere else without the font installed. The icon is
drawn from rectangles only (assets/branding/lufsy-icon.svg).

    python3 tools/build_logo.py

Output in assets/branding/:
    lufsy-wordmark-dark.svg   white "LUFS" + yellow "y", for dark backgrounds
    lufsy-wordmark-light.svg  graphite, for light backgrounds
    lufsy-lockup-dark.svg     icon + wordmark
    lufsy-lockup-light.svg
"""
import re
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
FONT = ROOT / "assets/fonts/ChakraPetch-Bold.ttf"
OUT = ROOT / "assets/branding"

WHITE = "#FFFFFF"
YELLOW = "#FFE500"
GRAPHITE = "#121315"


def wordmark():
    """Returns [(role, path_d)] and ink bounds for "LUFSy" in font units."""
    tt = TTFont(FONT)
    glyphs = tt.getGlyphSet()
    cmap = tt.getBestCmap()
    hmtx = tt["hmtx"]
    x = 0
    parts, bounds = [], None
    for ch in "LUFSy":
        name = cmap[ord(ch)]
        transform = (1, 0, 0, -1, x, 0)
        pen = SVGPathPen(glyphs)
        glyphs[name].draw(TransformPen(pen, transform))
        bpen = BoundsPen(glyphs)
        glyphs[name].draw(TransformPen(bpen, transform))
        parts.append(("accent" if ch == "y" else "base", pen.getCommands()))
        b = bpen.bounds
        bounds = b if bounds is None else (min(bounds[0], b[0]), min(bounds[1], b[1]), max(bounds[2], b[2]), max(bounds[3], b[3]))
        x += hmtx[name][0]
    return parts, bounds


def paths(parts, base, accent, dx, dy):
    out = []
    for role, d in parts:
        fill = accent if role == "accent" else base
        out.append(f'<path fill="{fill}" d="{d}"/>')
    return f'<g transform="translate({dx:.1f} {dy:.1f})">{"".join(out)}</g>'


def write(name, w, h, body):
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0f} {h:.0f}" role="img" aria-label="LUFSy">'
        f"<title>LUFSy</title>{body}</svg>\n"
    )
    (OUT / name).write_text(svg)
    print(f"wrote {name}")


def main():
    parts, (x0, y0, x1, y1) = wordmark()
    pad = 40
    w, h = x1 - x0 + 2 * pad, y1 - y0 + 2 * pad

    for theme, base, accent in (("dark", WHITE, YELLOW), ("light", GRAPHITE, GRAPHITE)):
        write(f"lufsy-wordmark-{theme}.svg", w, h, paths(parts, base, accent, pad - x0, pad - y0))

    # Lockup: icon square at cap height scale, then the wordmark.
    icon = (OUT / "lufsy-icon.svg").read_text()
    inner = re.search(r"<svg[^>]*>(.*)</svg>", icon, re.S).group(1)
    inner = re.sub(r"<!--.*?-->", "", inner, flags=re.S).strip()
    size = (y1 - y0) * 1.5
    gap = size * 0.28
    lw = size + gap + (x1 - x0)
    lh = size
    for theme, base, accent in (("dark", WHITE, YELLOW), ("light", GRAPHITE, GRAPHITE)):
        body = (
            f'<g transform="scale({size / 1024:.5f})">{inner}</g>'
            + paths(parts, base, accent, size + gap - x0, (size - (y1 - y0)) / 2 - y0)
        )
        write(f"lufsy-lockup-{theme}.svg", lw, lh, body)


if __name__ == "__main__":
    main()
