"""Draw the figures of docs/research/radio-mail/00-concept.md.

Run from anywhere: ``python docs/research/radio-mail/fig/make_figures.py``. It writes one SVG per
figure beside this file. Standard library only, so the figures can be redrawn without the
project's environment. Every figure shares one style: a role colour per kind of station (radio
user, gateway, post office, ordinary email) and a line style per kind of link (over the air, the
gateway API over the Internet, ordinary email), so a reader learns them once.
"""

from __future__ import annotations

from collections.abc import Iterable, Sequence
from itertools import pairwise
from pathlib import Path
from xml.sax.saxutils import escape

HERE = Path(__file__).parent

FONT = "Inter, 'Segoe UI', 'Helvetica Neue', Helvetica, Arial, sans-serif"
MONO = "'JetBrains Mono', 'Cascadia Mono', Consolas, 'DejaVu Sans Mono', Menlo, monospace"

INK = "#1e293b"
MUTED = "#475569"
FAINT = "#94a3b8"
HAIR = "#cbd5e1"
PAPER = "#ffffff"

# fill, stroke
ROLES = {
    "client": ("#eff6ff", "#1d4ed8"),
    "gateway": ("#fff7ed", "#c2410c"),
    "po": ("#ecfdf5", "#047857"),
    "mail": ("#f5f3ff", "#6d28d9"),
    "neutral": ("#f8fafc", "#475569"),
    "warn": ("#fef2f2", "#b91c1c"),
    "paper": ("#ffffff", "#94a3b8"),
}

# colour, dash, width
LINKS = {
    "radio": ("#c2410c", "8 5", 2.2),
    "internet": ("#334155", None, 2.0),
    "smtp": ("#6d28d9", None, 2.0),
    "local": ("#64748b", "2 4", 1.8),
    "fail": ("#b91c1c", "5 4", 2.0),
}

Point = tuple[float, float]


class Fig:
    """An SVG drawing with the shared style."""

    def __init__(self, width: int, height: int, title: str, subtitle: str = "") -> None:
        self.w = width
        self.h = height
        self.body: list[str] = []
        self.title = title
        self.subtitle = subtitle

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
    ) -> None:
        family = MONO if mono else FONT
        extra = ""
        if halo:
            extra += (
                f' paint-order="stroke" stroke="{PAPER}" stroke-width="5" stroke-linejoin="round"'
            )
        if italic:
            extra += ' font-style="italic"'
        self.add(
            f'<text xml:space="preserve" x="{x:.1f}" y="{y:.1f}" font-family="{family}" '
            f'font-size="{size}" '
            f'font-weight="{weight}" fill="{color}" text-anchor="{anchor}"{extra}>'
            f"{escape(s)}</text>"
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

    def rect(  # noqa: PLR0917 - a rectangle and its two colours, as SVG orders them
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
            f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" '
            f'fill="{fill}"{o} stroke="{stroke}" stroke-width="{sw}"{d}/>'
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
        mono_rows: Sequence[str] = (),
        size: float = 13.5,
        title_size: float = 15.5,
        icon: str | None = None,
        center: bool = False,
    ) -> None:
        fill, stroke = ROLES[role]
        self.rect(x, y, w, h, fill, stroke)
        tx = x + w / 2 if center else x + 14
        anchor = "middle" if center else "start"
        cy = y + 14
        if tag:
            self.text(tx, cy + 11, tag, size=11.5, weight=650, color=stroke, anchor=anchor)
            cy += 19
        if title:
            self.text(tx, cy + title_size, title, size=title_size, weight=700, anchor=anchor)
            cy += title_size + 9
        cy += size
        cy = self.lines(tx, cy, rows, size=size, color=MUTED, anchor=anchor)
        if mono_rows:
            self.lines(tx, cy + 2, mono_rows, size=size - 1.5, color=INK, anchor=anchor, mono=True)
        if icon:
            self.icon(icon, x + w - 30, y + 12, stroke)

    def panel(self, x: float, y: float, w: float, h: float, heading: str) -> None:
        """A plain framed area with a heading, for examples and notes."""
        self.rect(x, y, w, h, "#f8fafc", HAIR, rx=8, sw=1.2)
        self.text(x + 14, y + 24, heading, size=13, weight=700, color=MUTED)

    # -- links ----------------------------------------------------------------------------------

    def link(
        self,
        pts: Sequence[Point],
        kind: str = "internet",
        *,
        label: Sequence[str] | str | None = None,
        at: float = 0.5,
        dx: float = 0,
        dy: float = -8,
        both: bool = False,
        head: bool = True,
        label_color: str | None = None,
        anchor: str = "middle",
        size: float = 12.5,
    ) -> None:
        color, dash, width = LINKS[kind]
        d = "M " + " L ".join(f"{px:.1f} {py:.1f}" for px, py in pts)
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
        """A small rounded label, centred on ``x``."""
        fill, stroke = ROLES[role]
        w = len(s) * size * 0.58 + 16
        self.rect(x - w / 2, y - size - 4, w, size + 9, fill, stroke, rx=(size + 9) / 2, sw=1.1)
        self.text(x, y, s, size=size, weight=650, color=stroke, anchor="middle")

    def number(self, x: float, y: float, n: str, color: str = INK) -> None:
        """A numbered step marker."""
        self.add(
            f'<circle cx="{x:.1f}" cy="{y:.1f}" r="11" fill="{color}" stroke="{PAPER}" '
            'stroke-width="2"/>'
        )
        self.text(x, y + 4.5, n, size=12.5, weight=700, color=PAPER, anchor="middle")

    def cross(self, x: float, y: float, color: str = "#b91c1c") -> None:
        self.add(
            f'<circle cx="{x:.1f}" cy="{y:.1f}" r="12" fill="{PAPER}" stroke="{color}" '
            'stroke-width="2"/>'
            f'<path d="M {x - 5:.1f} {y - 5:.1f} L {x + 5:.1f} {y + 5:.1f} M {x + 5:.1f} '
            f'{y - 5:.1f} L {x - 5:.1f} {y + 5:.1f}" stroke="{color}" stroke-width="2.4" '
            'stroke-linecap="round"/>'
        )

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
        elif kind == "key":
            self.add(
                f'<g {s}><circle cx="{x + 5}" cy="{y + 9}" r="4"/><path d="M {x + 9} {y + 9} '
                f"L {x + 18} {y + 9} M {x + 15} {y + 9} L {x + 15} {y + 13} M {x + 18} {y + 9}"
                f' L {x + 18} {y + 12}"/></g>'
            )
        elif kind == "doc":
            self.add(
                f'<g {s} stroke-linejoin="round"><path d="M {x + 2} {y} L {x + 12} {y} L '
                f'{x + 16} {y + 4} L {x + 16} {y + 18} L {x + 2} {y + 18} Z"/><path d="M '
                f"{x + 5} {y + 8} L {x + 13} {y + 8} M {x + 5} {y + 11.5} L {x + 13} {y + 11.5}"
                f' M {x + 5} {y + 15} L {x + 10} {y + 15}"/></g>'
            )

    # -- legend and output ----------------------------------------------------------------------

    def legend(self, y: float, kinds: Sequence[tuple[str, str]], x: float = 24) -> None:
        for kind, label in kinds:
            self.link([(x, y - 4), (x + 38, y - 4)], kind=kind, head=False)
            self.text(x + 46, y, label, size=12.5, color=MUTED)
            x += 46 + len(label) * 6.9 + 34

    def svg(self) -> str:
        defs = []
        for kind, (color, _dash, _w) in LINKS.items():
            defs.append(
                f'<marker id="ah-{kind}" viewBox="0 0 10 10" refX="8.5" refY="5" '
                'markerWidth="7" markerHeight="7" orient="auto-start-reverse">'
                f'<path d="M 0 0 L 10 5 L 0 10 z" fill="{color}"/></marker>'
            )
        head = [
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.w} {self.h}" '
            f'width="{self.w}" height="{self.h}" role="img" aria-label="{escape(self.title)}">',
            f"<defs>{''.join(defs)}</defs>",
            f'<rect x="0" y="0" width="{self.w}" height="{self.h}" fill="{PAPER}"/>',
        ]
        head.append(
            f'<text x="24" y="34" font-family="{FONT}" font-size="18" font-weight="700" '
            f'fill="{INK}">{escape(self.title)}</text>'
        )
        if self.subtitle:
            head.append(
                f'<text x="24" y="56" font-family="{FONT}" font-size="13.5" fill="{MUTED}">'
                f"{escape(self.subtitle)}</text>"
            )
        return "\n".join([*head, *self.body, "</svg>", ""])

    def save(self, name: str) -> None:
        (HERE / name).write_text(self.svg(), encoding="utf-8")


def point_along(pts: Sequence[Point], t: float) -> Point:
    """The point a fraction ``t`` of the way along a polyline."""
    segs = [(a, b, ((b[0] - a[0]) ** 2 + (b[1] - a[1]) ** 2) ** 0.5) for a, b in pairwise(pts)]
    total = sum(s[2] for s in segs)
    goal = total * t
    for a, b, n in segs:
        if goal <= n and n > 0:
            f = goal / n
            return (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f)
        goal -= n
    return pts[-1]


LEGEND = [
    ("radio", "Over the air (HF or VHF)"),
    ("internet", "Gateway and post office (HTTPS)"),
    ("smtp", "Ordinary email (SMTP, IMAP)"),
]


# ------------------------------------------------------------------------------------------------
# Figure 1: the network at a glance
# ------------------------------------------------------------------------------------------------


def fig_overview() -> None:
    f = Fig(1000, 720, "Figure 1. The network at a glance")
    rows = [
        (80, "Ordinary email", "mail", ["Anyone, any provider"]),
        (
            196,
            "Post offices",
            "po",
            ["Mailboxes and email.", "Run by groups;", "one or more domains."],
        ),
        (
            364,
            "Gateways",
            "gateway",
            ["Radio stations that", "pass sessions on. Run", "by anyone; no domain."],
        ),
        (552, "Radio users", "client", ["The new client,", "on HF or VHF."]),
    ]
    for y, name, role, desc in rows:
        f.text(24, y + 20, name, size=15, weight=700, color=ROLES[role][1])
        f.lines(24, y + 42, desc, size=12.5, color=MUTED)

    # ordinary email
    f.box(
        200,
        72,
        760,
        66,
        "mail",
        title="Ordinary email: Gmail, Outlook, any provider",
        rows=["maria@example.com, a relative of Facundo's, uses her usual mail program"],
        icon="globe",
    )
    # post offices
    f.box(
        200,
        190,
        320,
        108,
        "po",
        tag="Post office",
        title="po.hfpost.org",
        rows=["Serves hfpost.org", "Run by a Georgia radio group"],
        icon="envelope",
    )
    f.box(
        640,
        190,
        320,
        108,
        "po",
        tag="Post office",
        title="po.radiopost.org",
        rows=["Serves radiopost.org", "Run by a Texas radio group"],
        icon="envelope",
    )
    # gateways
    f.box(
        200,
        364,
        230,
        104,
        "gateway",
        tag="Gateway",
        title="N2CALL-5",
        rows=["County EOC, Atlanta", "2 m packet (Direwolf)"],
        icon="antenna",
    )
    f.box(
        465,
        364,
        230,
        104,
        "gateway",
        tag="Gateway",
        title="KK4ODA-1",
        rows=["Home station, Atlanta", "HF 80/40 m (Aether)"],
        icon="antenna",
    )
    f.box(
        730,
        364,
        230,
        104,
        "gateway",
        tag="Gateway",
        title="N1CALL-10",
        rows=["Austin, Texas", "HF 40/20 m (Aether)"],
        icon="antenna",
    )
    # users
    f.box(
        200,
        566,
        230,
        96,
        "client",
        tag="Role mailbox",
        title="shelter3@hfpost.org",
        rows=["Volunteers at a shelter"],
        icon="person",
        title_size=14.5,
    )
    f.box(
        505,
        566,
        215,
        96,
        "client",
        tag="Facundo, KK4ODA",
        title="facundo@hfpost.org",
        rows=["Mobile station"],
        icon="person",
        title_size=14.5,
    )
    f.box(
        760,
        566,
        200,
        96,
        "client",
        tag="Ana, N0CALL",
        title="ana@radiopost.org",
        rows=["Home station"],
        icon="person",
        title_size=14.5,
    )

    # email
    f.link([(360, 190), (360, 140)], "smtp", both=True)
    f.link([(800, 190), (800, 140)], "smtp", both=True)
    f.link([(522, 236), (638, 236)], "smtp", both=True, label="SMTP", dy=-9)
    # gateway API
    f.link([(315, 364), (315, 300)], "internet", both=True)
    f.link([(560, 364), (470, 300)], "internet", both=True)
    f.link([(850, 364), (850, 300)], "internet", both=True)
    f.link(
        [(770, 364), (505, 300)],
        "internet",
        both=True,
        label=["Facundo's session, passed", "on to his post office"],
        at=0.5,
        dy=4,
        size=12,
    )
    # radio
    f.link([(315, 566), (315, 470)], "radio", both=True, label="2 m", at=0.5, dx=8, anchor="start")
    f.link(
        [(575, 566), (575, 470)],
        "radio",
        both=True,
        label="HF, at home",
        at=0.5,
        dx=-8,
        anchor="end",
    )
    f.link(
        [(680, 566), (790, 470)],
        "radio",
        both=True,
        label=["HF, on the", "road in Texas"],
        at=0.5,
        dx=8,
        dy=4,
        anchor="start",
        size=12,
    )
    f.link([(895, 566), (895, 470)], "radio", both=True, label="HF", at=0.5, dx=8, anchor="start")
    f.legend(700, LEGEND)
    f.save("fig1-overview.svg")


# ------------------------------------------------------------------------------------------------
# Figure 2: inside a post office and a gateway
# ------------------------------------------------------------------------------------------------


def fig_inside() -> None:
    f = Fig(1000, 660, "Figure 2. Inside a post office and a gateway")
    # externals on the left
    f.box(
        20,
        96,
        150,
        86,
        "mail",
        title="Internet email",
        rows=["Gmail, Outlook,", "other post offices"],
        size=12.5,
        title_size=14,
    )
    f.box(
        20,
        214,
        150,
        86,
        "client",
        title="Users' mail apps",
        rows=["Phone or laptop,", "when online"],
        size=12.5,
        title_size=14,
    )
    f.box(
        20,
        410,
        150,
        150,
        "neutral",
        title="DNS for hfpost.org",
        mono_rows=["MX", "SPF  DKIM", "DMARC", "_radiomail", "  (post office key)"],
        size=12.5,
        title_size=14,
    )

    # post office container
    f.rect(200, 72, 330, 556, "#f7fdf9", ROLES["po"][1], rx=14, sw=2)
    f.text(220, 100, "Post office: po.hfpost.org", size=15.5, weight=700, color=ROLES["po"][1])
    f.text(220, 120, "A rented server with a fixed address", size=12.5, color=MUTED)
    f.box(
        220,
        138,
        290,
        132,
        "paper",
        tag="Mail server (Stalwart)",
        title="Ordinary email",
        rows=[
            "SMTP in and out; DKIM, SPF, DMARC",
            "IMAP and JMAP; storage",
            "Spam filter; Sieve rules",
        ],
        size=12.5,
        title_size=14.5,
        icon="server",
    )
    f.box(
        220,
        388,
        290,
        222,
        "po",
        tag="New code",
        title="Radio post office service",
        rows=[
            "Accounts, keys, credentials",
            "Accept lists",
            "Preparing mail for the air",
            "Gateway API (HTTPS)",
            "Copies to usual gateways",
            "Radio routes (Internet down)",
        ],
        size=12.5,
        title_size=14.5,
        icon="key",
    )
    for x, label in [
        (250, ["MTA hook:", "accept list"]),
        (345, ["JMAP:", "mailboxes"]),
        (440, ["SMTP:", "sending"]),
    ]:
        f.link([(x, 272), (x, 386)], "local", both=True)
        f.lines(x + 7, 316, label, size=11.5, weight=600, color=MUTED)

    f.link([(172, 140), (218, 170)], "smtp", both=True)
    f.link([(172, 256), (218, 236)], "smtp", both=True)
    f.link([(172, 486), (218, 486)], "local", both=True)

    # gateway container
    f.rect(630, 72, 350, 556, "#fffaf5", ROLES["gateway"][1], rx=14, sw=2)
    f.text(650, 100, "Gateway: KK4ODA-1", size=15.5, weight=700, color=ROLES["gateway"][1])
    f.text(650, 120, "A PC or Raspberry Pi at the radio", size=12.5, color=MUTED)
    f.box(
        650,
        138,
        310,
        196,
        "gateway",
        tag="New code",
        title="Gateway program",
        rows=[
            "Sessions with radio users",
            "Credential check (keys kept)",
            "Relay to any post office",
            "Store: outgoing queue, copies held",
            "Forwarding schedule (Internet down)",
        ],
        size=12.5,
        title_size=14.5,
        icon="antenna",
    )
    f.box(650, 366, 310, 50, "paper", title="Modem adapter", size=12.5, title_size=14)
    f.box(
        650,
        440,
        310,
        68,
        "paper",
        title="Modem",
        rows=["Aether, VARA, ARDOP or Direwolf"],
        size=12.5,
        title_size=14,
    )
    f.box(650, 532, 150, 50, "paper", title="Radio", size=12.5, title_size=14)
    f.link([(805, 336), (805, 364)], "local", head=False)
    f.link([(805, 418), (805, 438)], "local", head=False)
    f.link([(725, 510), (725, 530)], "local", head=False)
    f.link(
        [(802, 557), (958, 557)],
        "radio",
        both=True,
        label=["to radio users and", "other gateways"],
        at=0.5,
        dy=-12,
        size=11.5,
    )

    # between them
    f.link(
        [(512, 470), (648, 230)],
        "internet",
        both=True,
        label=["HTTPS", "sessions, waiting", "mail, copies"],
        at=0.5,
        dx=-6,
        dy=-4,
        size=12,
    )
    f.legend(652, LEGEND)
    f.save("fig2-inside.svg")


# ------------------------------------------------------------------------------------------------
# Figure 3: addresses, keys and credentials
# ------------------------------------------------------------------------------------------------


def fig_identity() -> None:
    f = Fig(1000, 624, "Figure 3. How a gateway knows who is calling, with no Internet")
    f.box(
        20,
        76,
        420,
        92,
        "neutral",
        tag="DNS record for hfpost.org (public)",
        mono_rows=["_radiomail.hfpost.org  TXT", '  "v=1; k=ed25519; p=Hk3sW…9Q="'],
        size=13,
    )
    f.box(
        20,
        236,
        260,
        110,
        "po",
        tag="Post office",
        title="po.hfpost.org",
        rows=["Keeps its private key", "Issues credentials"],
        icon="envelope",
    )
    f.box(
        350,
        206,
        300,
        206,
        "paper",
        tag="Credential (about 150 bytes)",
        mono_rows=[
            "address   facundo@hfpost.org",
            "operator  KK4ODA",
            "device    ed25519:7Q4f…aZ",
            "issued    2026-11-09",
            "expires   2027-02-07",
            "signed    hfpost.org (po2026)",
        ],
        size=14,
        icon="doc",
    )
    f.box(
        350,
        482,
        300,
        96,
        "client",
        tag="Facundo's laptop or phone",
        title="Radio mail client",
        rows=["Private device key never leaves it"],
        icon="person",
    )
    f.box(
        720,
        206,
        260,
        250,
        "gateway",
        tag="Any gateway",
        title="N1CALL-10 checks:",
        rows=[
            "1. Credential signed by a",
            "    post office key it knows",
            "2. Session signed by the",
            "    device key named in it",
            "3. Callsign on the air is",
            "    the operator's",
            "4. Not expired, not revoked",
        ],
        icon="antenna",
    )
    f.badge(850, 440, "Needs no Internet", "gateway")
    f.box(
        20,
        420,
        260,
        158,
        "client",
        tag="Role mailbox",
        title="shelter3@hfpost.org",
        rows=[
            "One credential per volunteer,",
            "each under that volunteer's",
            "own callsign (N3CALL,",
            "N4CALL, ...)",
        ],
        size=12.5,
        title_size=14.5,
    )

    f.link([(150, 234), (150, 170)], "internet", label="publishes its key", dx=8, anchor="start")
    f.link([(282, 300), (348, 300)], "internet", label="signs", dy=-9)
    f.link([(500, 414), (500, 480)], "local", label="kept by the client", dx=8, anchor="start")
    f.link(
        [(442, 122), (850, 122), (850, 204)],
        "internet",
        label="key fetched when online and kept",
        at=0.45,
        dy=-9,
    )
    f.link(
        [(652, 530), (850, 530), (850, 458)],
        "radio",
        label=["credential (first visit only)", "and a signature on each session"],
        at=0.32,
        dy=-12,
        size=12,
    )
    f.legend(
        612,
        [
            ("radio", "Over the air"),
            ("internet", "Internet, when there is one"),
            ("local", "Stored on the device"),
        ],
    )
    f.save("fig3-identity.svg")


# ------------------------------------------------------------------------------------------------
# Figure 4: a session over the air
# ------------------------------------------------------------------------------------------------


def fig_session() -> None:
    f = Fig(
        1000,
        760,
        "Figure 4. A session: Facundo calls N1CALL-10 while travelling in Texas",
        "Five transmissions on the air. The gateway asks his post office over the Internet "
        "while the radio waits.",
    )
    lanes = {"client": 150, "gateway": 520, "po": 860}
    f.box(
        40,
        80,
        220,
        60,
        "client",
        title="Facundo's client",
        rows=["KK4ODA, mobile"],
        center=True,
        size=12.5,
        title_size=14.5,
    )
    f.box(
        410,
        80,
        220,
        60,
        "gateway",
        title="Gateway N1CALL-10",
        rows=["Austin, HF"],
        center=True,
        size=12.5,
        title_size=14.5,
    )
    f.box(
        750,
        80,
        220,
        60,
        "po",
        title="po.hfpost.org",
        rows=["his post office"],
        center=True,
        size=12.5,
        title_size=14.5,
    )
    for x in lanes.values():
        f.add(
            f'<path d="M {x} 142 L {x} 712" stroke="{HAIR}" stroke-width="1.6" '
            'stroke-dasharray="4 5"/>'
        )

    c, g, p = lanes["client"], lanes["gateway"], lanes["po"]

    def step(  # noqa: PLR0917 - one sequence-diagram row
        y: float, a: float, b: float, kind: str, rows: list[str], n: str | None
    ) -> None:
        f.link([(a, y), (b, y)], kind, label=rows, at=0.5, dy=-9, size=12.5)
        if n:
            f.number(a + (12 if b > a else -12), y, n, LINKS[kind][0])

    step(
        206,
        c,
        g,
        "radio",
        ["Hello: facundo@hfpost.org, operator KK4ODA.", "2 messages to send; last pickup was #418"],
        "1",
    )
    step(244, g, p, "internet", ["Anything after #418?"], None)
    step(282, p, g, "internet", ["#419, #420, #421 (sizes)"], None)
    step(
        366,
        g,
        c,
        "radio",
        [
            "Challenge: 7c41e9. I will take both.",
            "Waiting: #419 1 kB, #420 2 kB, #421 photo 310 kB",
            "#419 follows",
        ],
        "2",
    )
    step(
        448,
        c,
        g,
        "radio",
        ["Signature over the challenge and my IDs.", "My 2 messages. Send #420; keep #421."],
        "3",
    )
    step(486, g, p, "internet", ["2 messages and the signature"], None)
    f.text(p - 12, 512, "checks the signature again,", size=12, color=MUTED, anchor="end")
    f.text(p - 12, 528, "sends them on", size=12, color=MUTED, anchor="end")
    step(586, g, c, "radio", ["Received both. #420 follows.", "Nothing more for you."], "4")
    step(644, c, g, "radio", ["Have #419 and #420. Done."], "5")
    step(694, g, p, "internet", ["Picked up #419, #420; #421 still waiting"], None)
    f.legend(746, [("radio", "One transmission on the air"), ("internet", "Over the Internet")])
    f.save("fig4-session.svg")


# ------------------------------------------------------------------------------------------------
# Figure 5: radio to Internet
# ------------------------------------------------------------------------------------------------


def fig_outbound() -> None:
    f = Fig(
        1000,
        602,
        "Figure 6. From the radio to ordinary email",
        "Facundo answers Maria from Texas. His mail still leaves from his own post office.",
    )
    y, h = 84, 112
    f.box(
        20,
        y,
        200,
        h,
        "client",
        tag="1. Client",
        title="Facundo writes",
        rows=["A plain-text reply", "Signed with his key"],
        icon="person",
    )
    f.box(
        270,
        y,
        200,
        h,
        "gateway",
        tag="2. Any gateway",
        title="N1CALL-10",
        rows=["Checks the credential", "Passes the message on"],
        icon="antenna",
    )
    f.box(
        520,
        y,
        220,
        h,
        "po",
        tag="3. His post office",
        title="po.hfpost.org",
        rows=["Checks the signature", "Rebuilds a normal email"],
        icon="envelope",
    )
    f.box(
        790,
        y,
        190,
        h,
        "mail",
        tag="4. Maria's provider",
        title="example.com",
        rows=["SPF, DKIM, DMARC pass", "A reply in her thread"],
        icon="globe",
    )
    f.link([(222, 140), (268, 140)], "radio", label="HF", dy=-10)
    f.link([(472, 140), (518, 140)], "internet", label="HTTPS", dy=-10)
    f.link([(742, 140), (788, 140)], "smtp", label="SMTP", dy=-10)

    f.panel(20, 236, 420, 196, "On the air: the compact form")
    f.lines(
        36,
        286,
        [
            "to       maria@example.com",
            "reply    #a3f9 (her message)",
            "subject  Re: Are you safe?",
            "text     1 214 bytes",
            "         640 bytes compressed",
            "proof    covered by the session",
            "         signature (Figure 4)",
        ],
        size=13,
        mono=True,
    )
    f.panel(470, 236, 510, 316, "What Maria receives")
    f.lines(
        486,
        286,
        [
            "From: Facundo <facundo@hfpost.org>",
            "To: Maria <maria@example.com>",
            "Subject: Re: Are you safe?",
            "In-Reply-To: <CAF7x9q@mail.example.com>",
            "Message-ID: <a3f9c27e@hfpost.org>",
            "Radio-Path: KK4ODA via N1CALL-10 (HF)",
            "DKIM-Signature: d=hfpost.org; s=po2026; ...",
            "",
            "We are fine. Power is out, but ...",
            "--",
            "Sent by amateur radio. Please reply in",
            "plain text, under 20 kB. No business.",
        ],
        size=12.5,
        mono=True,
    )
    f.link([(120, 198), (120, 234)], "local")
    f.link([(630, 198), (630, 234)], "local")
    f.panel(20, 452, 420, 100, "Why it leaves from hfpost.org")
    f.lines(
        36,
        502,
        [
            "Only hfpost.org's own server can sign for",
            "hfpost.org, so Maria's provider trusts it.",
        ],
        size=13,
        color=MUTED,
    )
    f.legend(586, LEGEND)
    f.save("fig6-outbound.svg")


# ------------------------------------------------------------------------------------------------
# Figure 6: Internet to radio
# ------------------------------------------------------------------------------------------------


def fig_inbound() -> None:
    f = Fig(
        1000,
        700,
        "Figure 5. From ordinary email to the radio",
        "Maria writes to facundo@hfpost.org. The post office decides at the door, then makes "
        "the mail fit the air.",
    )
    f.box(
        20,
        84,
        170,
        104,
        "mail",
        tag="Sender",
        title="Maria",
        rows=["maria@example.com"],
        icon="globe",
        size=12.5,
    )
    f.box(
        240,
        84,
        220,
        104,
        "po",
        tag="1. The SMTP door",
        title="On Facundo's list?",
        rows=["Asked while Maria's", "server is still talking"],
        size=12.5,
    )
    f.box(
        520,
        84,
        190,
        104,
        "po",
        tag="2. Mailbox",
        title="Stored",
        rows=["His mail app sees it", "at once (IMAP)"],
        size=12.5,
    )
    f.box(
        770,
        84,
        210,
        160,
        "po",
        tag="3. Ready for the air",
        title="Made small",
        rows=[
            "HTML to plain text",
            "Quoted history trimmed",
            "Photo shrunk, held",
            "14 kB becomes 1.1 kB",
        ],
        size=12.5,
    )
    f.link([(192, 136), (238, 136)], "smtp")
    f.link([(462, 136), (518, 136)], "local", label="yes", dy=-9)
    f.link([(712, 136), (768, 136)], "local")

    f.link([(350, 190), (350, 262)], "fail", label="no", dx=8, anchor="start", dy=4)
    f.box(
        170,
        264,
        480,
        150,
        "warn",
        tag="Refused at the door, with a reason",
        mono_rows=[
            "550 5.7.1 facundo@hfpost.org reads mail by",
            "  amateur radio and takes it only from senders",
            "  he has listed. Ask him to add you. Amateur",
            "  radio carries no business content.",
        ],
        size=13.5,
    )
    f.link(
        [(168, 340), (60, 340), (60, 190)],
        "fail",
        label=["her own provider", "tells her why"],
        at=0.78,
        dx=8,
        anchor="start",
        size=12,
    )
    f.panel(20, 436, 630, 124, "The accept list")
    f.lines(
        36,
        486,
        [
            "Grows by itself: writing to someone adds them.",
            "Mail from other radio mail post offices is accepted by default.",
            "Nothing is sent back to a forged sender: the refusal is an SMTP reply.",
        ],
        size=13,
        color=MUTED,
    )

    f.box(
        770,
        290,
        210,
        126,
        "gateway",
        tag="4. Copies placed ahead",
        title="Usual gateways",
        rows=["KK4ODA-1, N2CALL-5", "hold a copy, in case", "the Internet fails"],
        size=12.5,
        icon="antenna",
    )
    f.link([(875, 246), (875, 288)], "internet")
    f.box(
        670,
        448,
        310,
        200,
        "client",
        tag="5. Facundo connects, from any gateway",
        title="A list comes first",
        mono_rows=[
            "#419 Maria   Are you safe?   1.1 kB",
            "#420 EOC     Shelter roster  2.0 kB",
            "#421 Maria   photo           310 kB",
            "",
            "He asks for #421 only if",
            "he wants it.",
        ],
        size=12.5,
        icon="person",
    )
    f.link([(875, 418), (875, 446)], "radio")
    f.legend(686, [*LEGEND, ("fail", "Refused")])
    f.save("fig5-inbound.svg")


# ------------------------------------------------------------------------------------------------
# Figure 7: the Internet goes down at a gateway
# ------------------------------------------------------------------------------------------------


def fig_outage() -> None:
    f = Fig(
        1000,
        666,
        "Figure 7. When a gateway loses the Internet",
        "Copies placed ahead let Facundo get his mail; what he sends waits in the gateway's queue.",
    )
    xs = [20, 347, 674]
    heads = ["1. Before: copies placed", "2. During the outage", "3. The Internet is back"]
    for x, head in zip(xs, heads, strict=True):
        f.rect(x, 74, 306, 520, "#fbfcfe", HAIR, rx=12, sw=1.2)
        f.text(x + 16, 102, head, size=14.5, weight=700)

    def nodes(x: float) -> tuple[Point, Point, Point, Point]:
        f.box(
            x + 63,
            124,
            180,
            62,
            "po",
            title="po.hfpost.org",
            rows=["his post office"],
            center=True,
            size=12,
            title_size=14,
        )
        f.box(
            x + 16,
            290,
            130,
            68,
            "gateway",
            title="KK4ODA-1",
            rows=["Atlanta"],
            center=True,
            size=12,
            title_size=14,
        )
        f.box(
            x + 160,
            290,
            130,
            68,
            "gateway",
            title="N2CALL-5",
            rows=["County EOC"],
            center=True,
            size=12,
            title_size=14,
        )
        f.box(
            x + 16,
            478,
            130,
            62,
            "client",
            title="Facundo",
            rows=["mobile, HF"],
            center=True,
            size=12,
            title_size=14,
        )
        return (x + 153, 188), (x + 81, 288), (x + 225, 288), (x + 81, 476)

    po, kk, n2, _fc = nodes(xs[0])
    f.link([(po[0] - 30, po[1]), (kk[0] + 10, kk[1])], "internet")
    f.link([(po[0] + 30, po[1]), (n2[0] - 10, n2[1])], "internet")
    f.text(
        po[0],
        226,
        "copies of #419-#421",
        size=12,
        weight=600,
        color=INK,
        anchor="middle",
        halo=True,
    )
    f.badge(kk[0], 388, "3 copies held", "gateway")
    f.badge(n2[0], 388, "3 copies held", "gateway")

    po, kk, n2, _fc = nodes(xs[1])
    f.link([(po[0] - 30, po[1]), (kk[0] + 10, kk[1])], "fail", head=False)
    f.cross(po[0] - 52, 238)
    f.link([(po[0] + 30, po[1]), (n2[0] - 10, n2[1])], "internet", both=True)
    f.link(
        [(kk[0] - 40, 476), (kk[0] - 40, 360)],
        "radio",
        both=True,
        label=["gets #419-#421", "sends 2 messages"],
        at=0.4,
        dx=10,
        dy=8,
        anchor="start",
        size=12,
    )
    f.badge(kk[0] + 34, 388, "2 queued", "gateway")
    f.badge(n2[0], 388, "3 copies held", "gateway")

    po, kk, n2, _fc = nodes(xs[2])
    f.link([(kk[0] + 10, kk[1]), (po[0] - 30, po[1])], "internet")
    f.link([(po[0] + 30, po[1]), (n2[0] - 10, n2[1])], "internet")
    f.text(kk[0] - 4, 214, "picked up #419-#421;", size=12, weight=600, anchor="start", halo=True)
    f.text(kk[0] - 4, 230, "2 messages to send", size=12, weight=600, anchor="start", halo=True)
    f.text(n2[0] + 22, 258, "drop the copies", size=12, weight=600, anchor="middle", halo=True)
    f.badge(kk[0], 388, "queue empty", "gateway")
    f.badge(n2[0], 388, "copies dropped", "gateway")

    f.text(
        24,
        620,
        "If two gateways both hand him a copy, his client keeps one: a message's ID comes "
        "from its content.",
        size=13,
        color=MUTED,
    )
    f.legend(
        650,
        [("radio", "Over the air"), ("internet", "Internet"), ("fail", "Internet down")],
    )
    f.save("fig7-outage.svg")


# ------------------------------------------------------------------------------------------------
# Figure 8: forwarding by radio
# ------------------------------------------------------------------------------------------------


def fig_forwarding() -> None:
    f = Fig(
        1000,
        700,
        "Figure 8. Forwarding by radio when a whole area loses the Internet",
        "Shelter 3 writes to Ana in Texas and Ana answers. HF carries the mail out of and back "
        "into Atlanta.",
    )
    f.rect(20, 74, 450, 540, "#fef2f2", "#fca5a5", rx=14, sw=1.4, dash="6 5")
    f.text(40, 102, "Atlanta: no Internet", size=14.5, weight=700, color="#b91c1c")
    f.rect(500, 74, 480, 540, "#f8fafc", HAIR, rx=14, sw=1.4)
    f.text(520, 102, "Elsewhere: the Internet works", size=14.5, weight=700, color=MUTED)

    f.box(
        50,
        470,
        220,
        104,
        "client",
        tag="Role mailbox",
        title="Shelter 3",
        rows=["shelter3@hfpost.org", "2 m radio"],
        icon="person",
    )
    f.box(
        50,
        250,
        250,
        118,
        "gateway",
        tag="Gateway",
        title="N2CALL-5",
        rows=["County EOC", "2 m for users", "HF for forwarding"],
        icon="antenna",
    )
    f.box(
        300,
        126,
        150,
        74,
        "gateway",
        title="KK4ODA-1",
        rows=["offline too"],
        size=12.5,
        title_size=14,
    )

    f.box(
        530,
        126,
        200,
        92,
        "po",
        tag="Post office",
        title="po.hfpost.org",
        rows=["rented server, still up"],
        size=12.5,
        title_size=14.5,
    )
    f.box(
        760,
        126,
        200,
        92,
        "po",
        tag="Post office",
        title="po.radiopost.org",
        rows=["Ana's post office"],
        size=12.5,
        title_size=14.5,
    )
    f.box(
        530,
        262,
        210,
        118,
        "gateway",
        tag="Gateway",
        title="N1CALL-10",
        rows=["Austin, HF, Internet up", "Radio route into", "Atlanta for hfpost.org"],
        icon="antenna",
        size=12.5,
    )
    f.box(
        760,
        470,
        200,
        104,
        "client",
        tag="Ana, N0CALL",
        title="ana@radiopost.org",
        rows=["Reads by IMAP"],
        icon="person",
        size=12.5,
        title_size=14.5,
    )

    # out: shelter -> N2 -> N1 -> radiopost -> Ana; back: Ana -> radiopost -> hfpost -> N1 -> ...
    f.link([(120, 468), (120, 370)], "radio", both=True)
    f.number(104, 420, "1", LINKS["radio"][0])
    f.number(136, 420, "9", LINKS["radio"][0])
    f.link(
        [(302, 320), (528, 320)],
        "radio",
        both=True,
        label=["HF 7.101 MHz, every hour at :15"],
        at=0.5,
        dy=-10,
        size=12.5,
    )
    f.number(400, 340, "2", LINKS["radio"][0])
    f.number(424, 340, "8", LINKS["radio"][0])
    f.link([(712, 260), (800, 221)], "internet")
    f.number(770, 253, "3", LINKS["internet"][0])
    f.link([(860, 221), (860, 468)], "smtp", both=True)
    f.number(846, 345, "4", LINKS["smtp"][0])
    f.number(874, 345, "5", LINKS["smtp"][0])
    f.link([(758, 160), (732, 160)], "smtp")
    f.number(745, 138, "6", LINKS["smtp"][0])
    f.link([(600, 221), (600, 260)], "internet")
    f.number(584, 241, "7", LINKS["internet"][0])

    f.panel(20, 628, 960, 56, "")
    f.text(
        36,
        650,
        "1-4 Shelter 3 to Ana: by 2 m to the EOC, by HF to Austin, then the Internet.   "
        "5-9 Ana's answer: her post office to hfpost.org,",
        size=12.5,
        color=INK,
    )
    f.text(
        36,
        670,
        "which cannot reach Atlanta's gateways and uses its radio route: N1CALL-10 by HF to "
        "N2CALL-5, then by 2 m to the shelter.",
        size=12.5,
        color=INK,
    )
    f.box(
        530,
        420,
        210,
        170,
        "paper",
        tag="Radio route of hfpost.org",
        mono_rows=[
            "users at N2CALL-5, KK4ODA-1",
            "  via N1CALL-10",
            "  HF 7.101 MHz",
            "  hourly at :15",
        ],
        size=13,
    )
    f.save("fig8-forwarding.svg")


# ------------------------------------------------------------------------------------------------
# Figure 9: one protocol over many modems
# ------------------------------------------------------------------------------------------------


def fig_modems() -> None:
    f = Fig(
        1000,
        560,
        "Figure 9. One session protocol over many modems",
        "The client and the gateway program never talk to a modem directly; an adapter does.",
    )
    f.box(
        150,
        76,
        320,
        58,
        "client",
        title="Radio mail client",
        center=True,
        icon="person",
        size=12.5,
        title_size=15,
    )
    f.box(
        530,
        76,
        320,
        58,
        "gateway",
        title="Gateway program",
        center=True,
        icon="antenna",
        size=12.5,
        title_size=15,
    )
    f.box(
        20,
        158,
        960,
        66,
        "neutral",
        title="Session protocol: the same over every modem",
        rows=["hello, challenge, signature, messages, done; resumable; its own acknowledgements"],
        center=True,
        size=13,
        title_size=14.5,
    )
    f.box(
        20,
        248,
        960,
        66,
        "neutral",
        title="Modem adapter interface",
        rows=[
            "listen, call, send bytes, receive bytes, close; "
            "optional: link quality, delivery receipts, broadcasts"
        ],
        center=True,
        size=13,
        title_size=14.5,
    )
    for x in (310, 690):
        f.link([(x, 136), (x, 156)], "local", head=False)
    f.link([(500, 226), (500, 246)], "local", head=False)

    adapters = [
        ("Aether", ["Control API", "Delivery receipts", "Rules check on", "every keying"]),
        ("VARA HF or FM", ["Published TNC", "commands", "A byte stream"]),
        ("ARDOP", ["Its host", "interface", "A byte stream"]),
        ("AX.25 by KISS", ["Direwolf or a TNC", "VHF and UHF", "packet"]),
        ("TCP with TLS", ["The Internet", "AREDN mesh", "No radio"]),
    ]
    w = (960 - 4 * 16) / 5
    for i, (name, rows) in enumerate(adapters):
        x = 20 + i * (w + 16)
        role = "gateway" if i < 4 else "mail"
        f.box(x, 340, w, 128, role, title=name, rows=rows, size=12.5, title_size=14.5)
        f.link([(x + w / 2, 316), (x + w / 2, 338)], "local", head=False)
    f.rect(20, 492, 4 * w + 3 * 16, 44, "#fff7ed", ROLES["gateway"][1], rx=8, sw=1.2)
    f.text(
        20 + (4 * w + 3 * 16) / 2,
        520,
        "Radio: HF, VHF, UHF",
        size=14,
        weight=700,
        color=ROLES["gateway"][1],
        anchor="middle",
    )
    f.rect(20 + 4 * (w + 16), 492, w, 44, "#f5f3ff", ROLES["mail"][1], rx=8, sw=1.2)
    f.text(
        20 + 4 * (w + 16) + w / 2,
        520,
        "Internet",
        size=14,
        weight=700,
        color=ROLES["mail"][1],
        anchor="middle",
    )
    f.save("fig9-modems.svg")


def main() -> None:
    fig_overview()
    fig_inside()
    fig_identity()
    fig_session()
    fig_inbound()
    fig_outbound()
    fig_outage()
    fig_forwarding()
    fig_modems()


if __name__ == "__main__":
    main()
