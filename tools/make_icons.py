"""The application's icons and the panel's brand images, from the artwork.

The source is `Logos/Aether HF Icon.svg`: the tile (a `<rect>` with `rx`) and the mark (the
`<g id="mark">`, closed outlines) on a 1024-unit square. This script reads that file and
fills the shapes itself, so every size is drawn from the outlines rather than shrunk from one
big picture: each is filled at up to sixteen times its size and averaged down, which gives a
16 px icon the shapes' true coverage. That keeps the tool to Pillow and the standard library.
The reader knows only what the artwork uses — one `<rect>` with `rx`, and `<path>`s of
absolute M, L, C and Z, one closed outline each (no holes) — and refuses anything else with a
message rather than drawing it wrong.

    python tools/make_icons.py

Writes `app/src-tauri/icons/` (every size the bundler wants; each size in the `.ico` is drawn,
not scaled), the panel's `mark.png` (the tile: the header, About and the shell's windows) and
`favicon.png`, and `mark.svg` (the mark alone, cropped to it: the splash).

Needs Pillow (`python -m uv run --with pillow python tools/make_icons.py` works without
installing it).
"""

from __future__ import annotations

import math
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
ARTWORK = ROOT / "Logos" / "Aether HF Icon.svg"
ICONS = ROOT / "app" / "src-tauri" / "icons"
UI = ROOT / "app" / "ui"

SVG_NS = "http://www.w3.org/2000/svg"
# the four points of a cubic that follows a quarter circle most closely
QUARTER = 4 * (math.sqrt(2) - 1) / 3
# straight pieces each cubic is cut into: at 4096 px a chord's sag is well under a hundredth
CUBIC_STEPS = 64
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)

Point = tuple[float, float]


@dataclass
class Artwork:
    size: float
    tile: list[Point]
    tile_fill: str
    mark: list[list[Point]]
    mark_fill: str
    mark_paths: list[str]


def cubic(p0: Point, p1: Point, p2: Point, p3: Point) -> list[Point]:
    """The cubic from `p0` to `p3` as straight pieces, without `p0` (the outline has it)."""
    out = []
    for i in range(1, CUBIC_STEPS + 1):
        t = i / CUBIC_STEPS
        a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t * t, t**3
        out.append(
            (
                a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
            )
        )
    return out


def path_outline(d: str) -> list[Point]:
    """One closed outline from a path's `d`: absolute M, L, C and Z only."""
    tokens = re.findall(r"[A-Za-z]|-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?", d)
    unknown = sorted({t for t in tokens if t.isalpha() and t not in "MLCZ"})
    if unknown:
        raise SystemExit(f"{ARTWORK.name}: path commands {unknown} are not read here (M L C Z)")
    one_outline = tokens.count("M") == tokens.count("Z") == 1
    if not one_outline or tokens[0] != "M" or tokens[-1] != "Z":
        raise SystemExit(f"{ARTWORK.name}: each path must be one outline: M … Z")
    points: list[Point] = []
    i = 0
    while i < len(tokens):
        command = tokens[i]
        i += 1
        if command == "Z":
            break
        arity = 6 if command == "C" else 2
        numbers = [float(t) for t in tokens[i : i + arity]]
        if len(numbers) != arity or any(t.isalpha() for t in tokens[i : i + arity]):
            raise SystemExit(f"{ARTWORK.name}: {command} needs {arity} numbers in {d!r}")
        i += arity
        pairs = list(zip(numbers[::2], numbers[1::2], strict=True))
        if command == "C":
            points.extend(cubic(points[-1], pairs[0], pairs[1], pairs[2]))
        else:
            points.append(pairs[0])
    return points


def rounded_rect(x: float, y: float, w: float, h: float, r: float) -> list[Point]:
    """A rectangle with circular corners of radius `r`, as an outline."""
    k = r * QUARTER
    points: list[Point] = [(x + r, y), (x + w - r, y)]
    points += cubic(points[-1], (x + w - r + k, y), (x + w, y + r - k), (x + w, y + r))
    points.append((x + w, y + h - r))
    points += cubic(points[-1], (x + w, y + h - r + k), (x + w - r + k, y + h), (x + w - r, y + h))
    points.append((x + r, y + h))
    points += cubic(points[-1], (x + r - k, y + h), (x, y + h - r + k), (x, y + h - r))
    points.append((x, y + r))
    points += cubic(points[-1], (x, y + r - k), (x + r - k, y), (x + r, y))
    return points


def read_artwork() -> Artwork:
    root = ET.parse(ARTWORK).getroot()

    def q(tag: str) -> str:
        return f"{{{SVG_NS}}}{tag}"

    box = [float(v) for v in root.get("viewBox", "").split()]
    if len(box) != 4 or box[0] or box[1] or box[2] != box[3]:
        raise SystemExit(f"{ARTWORK.name}: the viewBox must be a square from 0 0")
    rect = root.find(q("rect"))
    group = root.find(f"{q('g')}[@id='mark']")
    if rect is None or group is None:
        raise SystemExit(f"{ARTWORK.name}: wants a <rect> (the tile) and <g id='mark'>")
    x, y, w, h = (float(rect.get(a, "0")) for a in ("x", "y", "width", "height"))
    r = float(rect.get("rx", "0"))
    paths = [p.get("d", "") for p in group.findall(q("path"))]
    if not paths:
        raise SystemExit(f"{ARTWORK.name}: the mark has no paths")
    return Artwork(
        size=box[2],
        tile=rounded_rect(x, y, w, h, r),
        tile_fill=rect.get("fill", "#000"),
        mark=[path_outline(d) for d in paths],
        mark_fill=group.get("fill", "#fff"),
        mark_paths=paths,
    )


def coverage(outlines: list[list[Point]], scale: float, size: int) -> Image.Image:
    """The share of each pixel the outlines cover, filled large and averaged down."""
    over = max(4, min(16, 4096 // size))
    big = Image.new("L", (size * over, size * over), 0)
    draw = ImageDraw.Draw(big)
    k = scale * over
    for outline in outlines:
        # an outline's coordinates are edges; Pillow's are pixel centres
        draw.polygon([(px * k - 0.5, py * k - 0.5) for px, py in outline], fill=255)
    return big.reduce(over)


def icon(art: Artwork, size: int) -> Image.Image:
    scale = size / art.size
    tile = coverage([art.tile], scale, size)
    mark = coverage(art.mark, scale, size)
    picture = Image.composite(
        Image.new("RGB", (size, size), art.mark_fill),
        Image.new("RGB", (size, size), art.tile_fill),
        mark,
    )
    picture.putalpha(tile)
    return picture


def mark_svg(art: Artwork) -> str:
    """The mark alone, its viewBox the whole units around it: what the splash shows."""
    xs = [px for outline in art.mark for px, _ in outline]
    ys = [py for outline in art.mark for _, py in outline]
    left, top = math.floor(min(xs)), math.floor(min(ys))
    width, height = math.ceil(max(xs)) - left, math.ceil(max(ys)) - top
    paths = "\n".join(f'  <path d="{d}"/>' for d in art.mark_paths)
    return (
        f'<svg xmlns="{SVG_NS}" width="{width}" height="{height}" '
        f'viewBox="{left} {top} {width} {height}" fill="{art.mark_fill}">\n'
        f"  <!-- written by tools/make_icons.py from Logos/{ARTWORK.name}: edit that -->\n"
        f"{paths}\n</svg>\n"
    )


def main() -> None:
    art = read_artwork()
    drawn = {size: icon(art, size) for size in {32, 96, 128, 256, 512, *ICO_SIZES}}
    drawn[32].save(ICONS / "32x32.png")
    drawn[128].save(ICONS / "128x128.png")
    drawn[256].save(ICONS / "128x128@2x.png")
    drawn[512].save(ICONS / "icon.png")
    largest = drawn[ICO_SIZES[-1]]
    largest.save(
        ICONS / "icon.ico",
        sizes=[(s, s) for s in ICO_SIZES],
        append_images=[drawn[s] for s in ICO_SIZES[:-1]],
    )
    drawn[96].save(UI / "mark.png")
    drawn[32].save(UI / "favicon.png")
    (UI / "mark.svg").write_text(mark_svg(art), encoding="utf-8", newline="\n")
    print("icons, mark, favicon and the splash's mark written")


if __name__ == "__main__":
    main()
