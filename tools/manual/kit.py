# SPDX-License-Identifier: MIT OR Apache-2.0
"""The drawing kit the manual's schematics share.

It is RadioPost's kit (tools/figures/style.py in KK4ODA/RadioPost) with Aether's roles, so the two
manuals read as one family: the same line for "over the air" and for "on the same computer", the
same boxes, the same type. A role colour per kind of thing (your station, the other station,
Aether, a program, RadioPost, hardware) and a line style per kind of connection (over the air,
audio, keying and CAT, a network port on the computer, the Internet), so a reader learns them
once. Every figure is a plate with no title of its own: the manual's caption under it names it,
so a figure can move without being redrawn. Standard library only; the output is the same bytes
on every system, so CI can hold the committed files to the script.
"""

from __future__ import annotations

import math
from collections.abc import Iterable, Sequence
from itertools import pairwise
from xml.sax.saxutils import escape

FONT = "Inter, 'Segoe UI', 'Helvetica Neue', Helvetica, Arial, sans-serif"
MONO = "'JetBrains Mono', 'Cascadia Mono', Consolas, 'DejaVu Sans Mono', Menlo, monospace"

INK = "#1e293b"
MUTED = "#475569"
FAINT = "#94a3b8"
HAIR = "#cbd5e1"
WASH = "#f8fafc"
PAPER = "#ffffff"
GRAPHITE = "#16181b"
"""The panel's own background, for drawings of the panel."""

# fill, stroke
ROLES = {
    "you": ("#eff6ff", "#1d4ed8"),
    "peer": ("#fff7ed", "#c2410c"),
    "aether": ("#f0fdfa", "#0f766e"),
    "program": ("#f5f3ff", "#6d28d9"),
    "radiopost": ("#ecfdf5", "#047857"),
    "neutral": ("#f8fafc", "#475569"),
    "warn": ("#fef2f2", "#b91c1c"),
    "ok": ("#f0fdf4", "#15803d"),
    "caution": ("#fffbeb", "#b45309"),
    "planned": ("#ffffff", "#94a3b8"),
}

# colour, dash, width
LINKS = {
    "radio": ("#c2410c", "8 5", 2.2),
    "audio": ("#0f766e", None, 2.0),
    "key": ("#b45309", "4 3", 1.8),
    "local": ("#64748b", "2 4", 1.8),
    "internet": ("#334155", None, 2.0),
    "cable": ("#334155", None, 2.0),
    "fail": ("#b91c1c", "5 4", 2.0),
    "plan": ("#94a3b8", "6 4", 1.8),
}

Point = tuple[float, float]


def num(v: float) -> str:
    """A coordinate, to a tenth, without a trailing .0 or a −0."""
    s = f"{v:.1f}"
    s = s.removesuffix(".0")
    return "0" if s == "-0" else s


class Plate:
    """An SVG drawing with the shared style and no title of its own."""

    def __init__(self, width: int, height: int, description: str) -> None:
        self.w = width
        self.h = height
        self.body: list[str] = []
        self.description = description
        self.defs: list[str] = []

    # -- primitives -----------------------------------------------------------------------------

    def add(self, s: str) -> None:
        self.body.append(s)

    def text(
        self,
        x: float,
        y: float,
        s: str,
        *,
        size: float = 13.5,
        weight: int = 400,
        color: str = INK,
        anchor: str = "start",
        mono: bool = False,
        halo: bool = False,
        italic: bool = False,
        halo_color: str = PAPER,
    ) -> None:
        family = MONO if mono else FONT
        extra = ""
        if halo:
            extra += (
                f' paint-order="stroke" stroke="{halo_color}" stroke-width="5"'
                ' stroke-linejoin="round"'
            )
        if italic:
            extra += ' font-style="italic"'
        self.add(
            f'<text xml:space="preserve" x="{num(x)}" y="{num(y)}" font-family="{family}" '
            f'font-size="{size}" font-weight="{weight}" fill="{color}" '
            f'text-anchor="{anchor}"{extra}>{escape(s)}</text>'
        )

    def lines(
        self,
        x: float,
        y: float,
        rows: Iterable[str],
        size: float = 13.5,
        gap: float = 1.45,
        **kw: object,
    ) -> float:
        """Draw rows of text from baseline ``y``; return the baseline after the last."""
        for row in rows:
            self.text(x, y, row, size=size, **kw)  # type: ignore[arg-type]
            y += size * gap
        return y

    def rect(
        self,
        x: float,
        y: float,
        w: float,
        h: float,
        fill: str,
        stroke: str,
        rx: float = 10,
        sw: float = 1.5,
        dash: str | None = None,
        opacity: float = 1.0,
    ) -> None:
        d = f' stroke-dasharray="{dash}"' if dash else ""
        o = f' fill-opacity="{opacity}"' if opacity < 1 else ""
        self.add(
            f'<rect x="{num(x)}" y="{num(y)}" width="{num(w)}" height="{num(h)}" rx="{rx}" '
            f'fill="{fill}"{o} stroke="{stroke}" stroke-width="{sw}"{d}/>'
        )

    def line(
        self,
        x1: float,
        y1: float,
        x2: float,
        y2: float,
        color: str = HAIR,
        width: float = 1.0,
        dash: str | None = None,
    ) -> None:
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(
            f'<line x1="{num(x1)}" y1="{num(y1)}" x2="{num(x2)}" y2="{num(y2)}" '
            f'stroke="{color}" stroke-width="{width}"{d}/>'
        )

    def path(
        self,
        d: str,
        *,
        fill: str = "none",
        stroke: str = INK,
        width: float = 1.5,
        dash: str | None = None,
        opacity: float = 1.0,
    ) -> None:
        dd = f' stroke-dasharray="{dash}"' if dash else ""
        o = f' fill-opacity="{opacity}"' if opacity < 1 else ""
        self.add(
            f'<path d="{d}" fill="{fill}"{o} stroke="{stroke}" stroke-width="{width}"'
            f'{dd} stroke-linejoin="round" stroke-linecap="round"/>'
        )

    def poly(self, pts: Sequence[Point], **kw: object) -> None:
        """A polyline through ``pts`` (closed when ``fill`` is given)."""
        d = "M " + " L ".join(f"{num(x)} {num(y)}" for x, y in pts)
        if kw.get("fill", "none") != "none":
            d += " Z"
        self.path(d, **kw)  # type: ignore[arg-type]

    def circle(
        self, x: float, y: float, r: float, fill: str, stroke: str = "none", sw: float = 1.5
    ) -> None:
        self.add(
            f'<circle cx="{num(x)}" cy="{num(y)}" r="{num(r)}" fill="{fill}" '
            f'stroke="{stroke}" stroke-width="{sw}"/>'
        )

    def box(
        self,
        x: float,
        y: float,
        w: float,
        h: float,
        role: str,
        *,
        tag: str | None = None,
        title: str | None = None,
        rows: Sequence[str] = (),
        size: float = 13,
        title_size: float = 15.5,
        icon: str | None = None,
        center: bool = False,
    ) -> None:
        """A box of one role: a small tag over a title over grey lines of text."""
        fill, stroke = ROLES[role]
        self.rect(x, y, w, h, fill, stroke, dash="6 4" if role == "planned" else None)
        tx = x + w / 2 if center else x + 14
        anchor = "middle" if center else "start"
        cy = y + 14
        if tag:
            self.text(tx, cy + 11, tag, size=11.5, weight=650, color=stroke, anchor=anchor)
            cy += 19
        if title:
            self.text(tx, cy + title_size, title, size=title_size, weight=700, anchor=anchor)
            cy += title_size + 7
        cy += size
        self.lines(tx, cy, rows, size=size, color=MUTED, anchor=anchor)
        if icon:
            self.icon(icon, x + w - 30, y + 12, stroke)

    def panel(self, x: float, y: float, w: float, h: float, heading: str) -> None:
        """A plain framed area with a heading, for notes."""
        self.rect(x, y, w, h, WASH, HAIR, rx=8, sw=1.2)
        self.text(x + 14, y + 24, heading, size=13, weight=700, color=MUTED)

    def frame(self, x: float, y: float, w: float, h: float, label: str, color: str = FAINT) -> None:
        """A dashed frame round what belongs together, named at its top left."""
        self.rect(x, y, w, h, WASH, color, rx=12, sw=1.3, dash="6 5")
        self.text(x + 14, y + 22, label, size=12.5, weight=700, color=MUTED)

    # -- connections ----------------------------------------------------------------------------

    def link(
        self,
        pts: Sequence[Point],
        kind: str = "local",
        *,
        label: Sequence[str] | str | None = None,
        at: float = 0.5,
        dx: float = 0,
        dy: float = -8,
        both: bool = False,
        head: bool = True,
        label_color: str | None = None,
        anchor: str = "middle",
        size: float = 12,
    ) -> None:
        color, dash, width = LINKS[kind]
        d = "M " + " L ".join(f"{num(px)} {num(py)}" for px, py in pts)
        dash_attr = f' stroke-dasharray="{dash}"' if dash else ""
        mid = f"ah-{kind}"
        end = f' marker-end="url(#{mid})"' if head else ""
        start = f' marker-start="url(#{mid})"' if both else ""
        self.add(
            f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width}"'
            f'{dash_attr} stroke-linecap="round" stroke-linejoin="round"{start}{end}/>'
        )
        if label:
            rows = [label] if isinstance(label, str) else list(label)
            lx, ly = point_along(pts, at)
            lx += dx
            ly += dy - (len(rows) - 1) * size * 1.3
            for row in rows:
                self.text(
                    lx,
                    ly,
                    row,
                    size=size,
                    weight=600,
                    color=label_color or color,
                    anchor=anchor,
                    halo=True,
                )
                ly += size * 1.3

    def badge(self, x: float, y: float, s: str, role: str = "neutral", size: float = 11.5) -> None:
        """A small rounded label, centred on ``x`` with its baseline at ``y``."""
        fill, stroke = ROLES[role]
        w = len(s) * size * 0.58 + 16
        self.rect(x - w / 2, y - size - 4, w, size + 9, fill, stroke, rx=(size + 9) / 2, sw=1.1)
        self.text(x, y, s, size=size, weight=650, color=stroke, anchor="middle")

    def number(self, x: float, y: float, n: str, color: str = INK, r: float = 11) -> None:
        """A numbered marker."""
        self.circle(x, y, r, color, PAPER, 2)
        self.text(x, y + r * 0.41, n, size=r * 1.14, weight=700, color=PAPER, anchor="middle")

    def tick(self, x: float, y: float, color: str = "#15803d") -> None:
        self.circle(x, y, 11, PAPER, color, 2)
        self.path(
            f"M {num(x - 5)} {num(y)} L {num(x - 1.5)} {num(y + 4)} L {num(x + 5.5)} "
            f"{num(y - 4.5)}",
            stroke=color,
            width=2.4,
        )

    def cross(self, x: float, y: float, color: str = "#b91c1c") -> None:
        self.circle(x, y, 11, PAPER, color, 2)
        self.path(
            f"M {num(x - 4.5)} {num(y - 4.5)} L {num(x + 4.5)} {num(y + 4.5)} "
            f"M {num(x + 4.5)} {num(y - 4.5)} L {num(x - 4.5)} {num(y + 4.5)}",
            stroke=color,
            width=2.4,
        )

    def hatch(self, ident: str, color: str) -> str:
        """A diagonal hatch fill, defined once; returns its url()."""
        self.defs.append(
            f'<pattern id="{ident}" width="7" height="7" patternUnits="userSpaceOnUse" '
            f'patternTransform="rotate(45)"><rect width="7" height="7" fill="{PAPER}"/>'
            f'<line x1="0" y1="0" x2="0" y2="7" stroke="{color}" stroke-width="2.2"/></pattern>'
        )
        return f"url(#{ident})"

    # -- icons ----------------------------------------------------------------------------------

    def icon(self, kind: str, x: float, y: float, color: str) -> None:
        """An 18-pixel line icon with its top-left corner at (x, y)."""
        s = f'fill="none" stroke="{color}" stroke-width="1.6" stroke-linecap="round"'
        if kind == "envelope":
            self.add(
                f'<g {s} stroke-linejoin="round"><rect x="{x}" y="{y + 3}" width="18" '
                f'height="13" rx="2"/><path d="M {x} {y + 4} L {x + 9} {y + 11} L {x + 18} '
                f'{y + 4}"/></g>'
            )
        elif kind == "antenna":
            cx = x + 9
            self.add(
                f'<g {s}><path d="M {cx} {y + 6} L {cx} {y + 18} M {cx - 4} {y + 18} L {cx + 4}'
                f' {y + 18}"/><circle cx="{cx}" cy="{y + 5}" r="1.6" fill="{color}"/>'
                f'<path d="M {cx - 4} {y + 1.5} Q {cx - 6.5} {y + 5} {cx - 4} {y + 8.5}"/>'
                f'<path d="M {cx + 4} {y + 1.5} Q {cx + 6.5} {y + 5} {cx + 4} {y + 8.5}"/>'
                f'<path d="M {cx - 7.5} {y - 1} Q {cx - 11} {y + 5} {cx - 7.5} {y + 11}"/>'
                f'<path d="M {cx + 7.5} {y - 1} Q {cx + 11} {y + 5} {cx + 7.5} {y + 11}"/></g>'
            )
        elif kind == "person":
            self.add(
                f'<g {s}><circle cx="{x + 9}" cy="{y + 5.5}" r="4"/>'
                f'<path d="M {x + 1.5} {y + 18} Q {x + 2} {y + 11} {x + 9} {y + 11} Q {x + 16}'
                f' {y + 11} {x + 16.5} {y + 18}"/></g>'
            )
        elif kind == "server":
            self.add(
                f'<g {s} stroke-linejoin="round"><rect x="{x}" y="{y + 1}" width="18" '
                f'height="7" rx="1.5"/><rect x="{x}" y="{y + 10}" width="18" height="7" '
                f'rx="1.5"/><circle cx="{x + 4}" cy="{y + 4.5}" r="0.9" fill="{color}"/>'
                f'<circle cx="{x + 4}" cy="{y + 13.5}" r="0.9" fill="{color}"/></g>'
            )
        elif kind == "globe":
            self.add(
                f'<g {s}><circle cx="{x + 9}" cy="{y + 9}" r="8"/><ellipse cx="{x + 9}" '
                f'cy="{y + 9}" rx="3.5" ry="8"/><path d="M {x + 1} {y + 9} L {x + 17} '
                f'{y + 9}"/></g>'
            )
        elif kind == "window":
            self.add(
                f'<g {s} stroke-linejoin="round"><rect x="{x}" y="{y + 1}" width="18" '
                f'height="15" rx="2"/><path d="M {x} {y + 5.5} L {x + 18} {y + 5.5}"/>'
                f'<path d="M {x + 3} {y + 12} L {x + 6} {y + 9} L {x + 9} {y + 11} L {x + 15}'
                f' {y + 8}"/></g>'
            )
        elif kind == "wave":
            self.add(
                f'<g {s}><path d="M {x} {y + 9} Q {x + 2.25} {y + 2} {x + 4.5} {y + 9} T {x + 9}'
                f' {y + 9} T {x + 13.5} {y + 9} T {x + 18} {y + 9}"/></g>'
            )
        elif kind == "radio":
            self.add(
                f'<g {s} stroke-linejoin="round"><rect x="{x}" y="{y + 5}" width="18" '
                f'height="12" rx="2"/><circle cx="{x + 12.5}" cy="{y + 11}" r="3"/>'
                f'<path d="M {x + 3} {y + 9} L {x + 7} {y + 9} M {x + 3} {y + 12} L {x + 7} '
                f'{y + 12} M {x + 4} {y + 5} L {x + 9} {y}"/></g>'
            )
        elif kind == "plug":
            self.add(
                f'<g {s} stroke-linejoin="round"><rect x="{x + 3}" y="{y + 6}" width="12" '
                f'height="8" rx="2"/><path d="M {x + 6.5} {y + 6} L {x + 6.5} {y + 1} M '
                f"{x + 11.5} {y + 6} L {x + 11.5} {y + 1} M {x + 9} {y + 14} L {x + 9} "
                f'{y + 18}"/></g>'
            )

    # -- charts ---------------------------------------------------------------------------------

    def axes(
        self,
        box: tuple[float, float, float, float],
        xs: Sequence[tuple[float, str]],
        ys: Sequence[tuple[float, str]],
        *,
        xlabel: str = "",
        ylabel: str = "",
    ) -> None:
        """Gridlines and labels for a plot area ``box`` = (x, y, w, h); ticks are pixels."""
        x, y, w, h = box
        for px, label in xs:
            self.line(px, y, px, y + h, HAIR, 0.8)
            self.text(px, y + h + 18, label, size=11.5, color=MUTED, anchor="middle")
        for py, label in ys:
            self.line(x, py, x + w, py, HAIR, 0.8)
            self.text(x - 8, py + 4, label, size=11.5, color=MUTED, anchor="end")
        self.rect(x, y, w, h, "none", FAINT, rx=0, sw=1)
        if xlabel:
            self.text(
                x + w / 2, y + h + 40, xlabel, size=12.5, weight=600, color=INK, anchor="middle"
            )
        if ylabel:
            cx, cy = x - 58, y + h / 2
            self.add(
                f'<text x="{num(cx)}" y="{num(cy)}" font-family="{FONT}" font-size="12.5" '
                f'font-weight="600" fill="{INK}" text-anchor="middle" '
                f'transform="rotate(-90 {num(cx)} {num(cy)})">{escape(ylabel)}</text>'
            )

    # -- legend and output ----------------------------------------------------------------------

    def legend(self, y: float, kinds: Sequence[tuple[str, str]], x: float = 24) -> None:
        for kind, label in kinds:
            self.link([(x, y - 4), (x + 38, y - 4)], kind=kind, head=False)
            self.text(x + 46, y, label, size=12, color=MUTED)
            x += 46 + len(label) * 6.6 + 30

    def svg(self) -> str:
        markers = "".join(
            f'<marker id="ah-{kind}" viewBox="0 0 10 10" refX="8.5" refY="5" '
            'markerWidth="7" markerHeight="7" orient="auto-start-reverse">'
            f'<path d="M 0 0 L 10 5 L 0 10 z" fill="{color}"/></marker>'
            for kind, (color, _dash, _w) in LINKS.items()
        )
        head = [
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.w} {self.h}" '
            f'width="{self.w}" height="{self.h}" role="img" '
            f'aria-label="{escape(self.description)}">',
            f"<defs>{markers}{''.join(self.defs)}</defs>",
            f'<rect x="0" y="0" width="{self.w}" height="{self.h}" fill="{PAPER}"/>',
        ]
        return "\n".join([*head, *self.body, "</svg>", ""])


def point_along(pts: Sequence[Point], t: float) -> Point:
    """The point a fraction ``t`` of the way along a polyline."""
    segs = [(a, b, math.dist(a, b)) for a, b in pairwise(pts)]
    total = sum(s[2] for s in segs)
    goal = total * t
    for a, b, n in segs:
        if goal <= n and n > 0:
            f = goal / n
            return (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f)
        goal -= n
    return pts[-1]
