# SPDX-License-Identifier: MIT OR Apache-2.0
"""Draw the schematics of the Aether operator's manual (docs/manual/manual.md).

    python -m uv run python tools/manual/figures.py           # write docs/manual/fig/*.svg
    python -m uv run python tools/manual/figures.py --check   # fail if a file is not what it draws

One SVG per figure, with no number or title on it: the manual's caption names it, so a figure can
move without being redrawn. The chart of the rungs and the picture of the passband read their
numbers from the reference model (``aether_model``), never typed, so they move when the waveform
does. The screenshots beside these come from tools/manual/screens.mjs. CI holds the committed
files to this script (model/tests/test_manual.py).
"""

from __future__ import annotations

import math
import sys
from collections.abc import Callable
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path[:0] = [str(HERE), str(ROOT / "tools"), str(ROOT / "model")]

from kit import FAINT, GRAPHITE, HAIR, INK, MUTED, PAPER, ROLES, WASH, Plate  # noqa: E402

OUT = ROOT / "docs" / "manual" / "fig"

TEAL = ROLES["aether"][1]
BLUE = ROLES["you"][1]
ORANGE = ROLES["peer"][1]
AMBER = ROLES["caution"][1]
RED = ROLES["warn"][1]
GREEN = ROLES["ok"][1]
PURPLE = ROLES["program"][1]

FIGURES: dict[str, Callable[[], Plate]] = {}


def figure(name: str) -> Callable[[Callable[[], Plate]], Callable[[], Plate]]:
    def register(draw: Callable[[], Plate]) -> Callable[[], Plate]:
        FIGURES[name] = draw
        return draw

    return register


# ------------------------------------------------------------------------------------------------
# What Aether is: two stations, each with Aether between its programs and its radio
# ------------------------------------------------------------------------------------------------


def station_column(f: Plate, x: float, role: str, label: str, programs: list[str]) -> None:
    """One station drawn as a column: its programs, Aether, its radio."""
    f.frame(x, 16, 300, 384, label, ROLES[role][1])
    w = 260
    bx = x + 20
    f.box(
        bx,
        48,
        w,
        92,
        "program",
        tag=programs[0],
        title=programs[1],
        rows=programs[2:],
        icon="window",
    )
    f.box(
        bx,
        172,
        w,
        96,
        "aether",
        tag="The modem",
        title="Aether",
        rows=["Turns data into sound and back,", "keys the radio, checks the rules"],
        icon="wave",
    )
    f.box(
        bx,
        300,
        w,
        84,
        "neutral",
        tag="Through a sound card",
        title="HF radio",
        rows=["Upper sideband, set for data"],
        icon="radio",
    )
    mid = bx + w / 2
    f.link([(mid, 144), (mid, 168)], "local", both=True)
    f.text(mid + 10, 160, "on the same computer", size=11.5, color=MUTED)
    f.link([(mid - 40, 272), (mid - 40, 296)], "audio", both=True)
    f.text(mid - 50, 288, "audio", size=11.5, color=TEAL, anchor="end", weight=600)
    f.link([(mid + 40, 272), (mid + 40, 296)], "key")
    f.text(mid + 50, 288, "keying", size=11.5, color=AMBER, weight=600)


@figure("overview")
def fig_overview() -> Plate:
    f = Plate(1000, 416, "Two stations, each with Aether between its programs and its radio")
    station_column(
        f,
        24,
        "you",
        "Your station",
        [
            "Programs",
            "Your programs",
            "Winlink Express, VarAC, RadioPost,",
            "or Aether's own window",
        ],
    )
    station_column(
        f,
        676,
        "peer",
        "The other station",
        ["Programs", "Its programs", "The same kinds, or", "Aether's own window"],
    )
    f.link(
        [(308, 342), (692, 342)],
        "radio",
        both=True,
        label=["Aether's own signal on the air,", "2300 or 500 Hz wide"],
        dy=-14,
        size=12.5,
    )
    f.panel(344, 40, 312, 246, "What Aether does")
    rows = [
        ("Moves data without errors:", "a lost frame is sent again"),
        ("Starts robust, then climbs to the", "fastest speed the path carries"),
        ("Takes turns with the other station,", "so the two never key at once"),
        ("Checks every transmission against", "the band rules before it keys"),
    ]
    y = 92.0
    for first, second in rows:
        f.tick(366, y - 4, TEAL)
        f.text(386, y, first, size=12.5)
        f.text(386, y + 17, second, size=12.5, color=MUTED)
        y += 48
    f.text(
        500,
        404,
        "Both stations need Aether: it speaks only to Aether.",
        size=12.5,
        color=MUTED,
        anchor="middle",
        italic=True,
    )
    return f


# ------------------------------------------------------------------------------------------------
# Your station: computer, interface, radio
# ------------------------------------------------------------------------------------------------


@figure("station")
def fig_station() -> Plate:
    f = Plate(1000, 392, "Your computer runs Aether; a sound card interface joins it to the radio")
    f.frame(24, 16, 470, 262, "Your computer")
    # Aether: one program, two parts
    fill, stroke = ROLES["aether"]
    f.rect(270, 50, 204, 210, fill, stroke)
    f.text(284, 75, "Aether", size=15.5, weight=700)
    f.icon("wave", 444, 60, stroke)
    f.rect(284, 90, 176, 66, PAPER, stroke, rx=8, sw=1.2)
    f.text(298, 113, "The window", size=13.5, weight=700)
    f.lines(
        298,
        133,
        ["Setup, Session, Status…", "what you see and set"],
        size=12,
        color=MUTED,
        gap=1.35,
    )
    f.rect(284, 176, 176, 70, PAPER, stroke, rx=8, sw=1.2)
    f.text(298, 199, "The modem", size=13.5, weight=700)
    f.lines(
        298,
        219,
        ["sound, keying, the rules;", "runs alone on a gateway"],
        size=12,
        color=MUTED,
        gap=1.35,
    )
    f.link([(372, 158), (372, 174)], "local", both=True)
    f.box(
        44,
        152,
        196,
        100,
        "program",
        tag="Optional",
        title="Your programs",
        rows=["Winlink Express, Pat,", "VarAC, RadioPost"],
        icon="window",
    )
    f.link([(244, 211), (280, 211)], "local", both=True)
    f.lines(
        44,
        60,
        [
            "Aether's window opens with the",
            "modem. Your programs reach the",
            "modem through a port on the",
            "same computer.",
        ],
        size=12.5,
        color=MUTED,
    )
    # the USB cable, the interface and the radio
    f.box(
        560,
        104,
        168,
        174,
        "neutral",
        tag="Interface",
        title="Sound card",
        rows=["SignaLink, DigiRig,", "or the radio's own", "USB audio"],
        icon="plug",
    )
    f.link([(498, 196), (556, 196)], "cable", both=True, label="USB", dy=-10)
    f.box(
        836,
        104,
        140,
        174,
        "neutral",
        tag="Radio",
        title="Transceiver",
        rows=["USB or DATA-U", "mode"],
        icon="radio",
    )
    f.link([(732, 150), (832, 150)], "audio", label="to the radio", dy=-8, size=11.5)
    f.link([(832, 200), (732, 200)], "audio", label="from the radio", dy=-8, size=11.5)
    f.link([(732, 248), (832, 248)], "key", label="PTT or CAT", dy=-8, size=11.5)
    # the antenna
    f.line(906, 104, 906, 52, INK, 1.8)
    f.path("M 884 34 L 906 52 L 928 34", stroke=INK, width=1.8)
    f.text(940, 48, "antenna", size=11.5, color=MUTED)
    f.panel(24, 296, 952, 82, "What you set where")
    f.text(
        38,
        340,
        "Aether's Setup: the sound card and the keying (step 2), the receive level "
        "(step 3). The Session tab: the drive.",
        size=12.5,
        color=MUTED,
    )
    f.text(
        38,
        362,
        "The radio: USB or DATA mode, a filter at least as wide as the signal, and "
        "its data input and output levels.",
        size=12.5,
        color=MUTED,
    )
    return f


# ------------------------------------------------------------------------------------------------
# The header, drawn as the panel draws it, part by part
# ------------------------------------------------------------------------------------------------

HEAD_TEXT = "#e5e7eb"
HEAD_MUTED = "#9ca3af"
HEAD_BLUE = "#60a5fa"
HEAD_GREEN = "#4ade80"
HEAD_AMBER = "#fbbf24"


def head_text(f: Plate, x: float, y: float, s: str, **kw: object) -> None:
    kw.setdefault("color", HEAD_TEXT)
    kw.setdefault("size", 14.5)
    f.text(x, y, s, **kw)  # type: ignore[arg-type]


def lamp(f: Plate, x: float, y: float, label: str, color: str) -> None:
    f.circle(x, y - 5, 4.5, color)
    head_text(f, x + 10, y, label, color=color if color != "#4b5563" else HEAD_MUTED, weight=600)


@figure("header")
def fig_header() -> Plate:
    f = Plate(1000, 268, "The header in two halves, its parts numbered as the manual's list")
    # the left half: the name and the display
    f.rect(24, 52, 952, 64, GRAPHITE, GRAPHITE, rx=10, sw=1)
    head_text(f, 42, 90, "A", size=18, weight=800)
    head_text(f, 62, 90, "Aether", size=15, weight=700)
    head_text(f, 116, 90, "HF", size=15, color=HEAD_MUTED)
    f.rect(158, 60, 806, 48, "#0d0f11", "#30343a", rx=8, sw=1.2)
    head_text(f, 174, 90, "N2CALL", size=15, weight=700, color=HEAD_BLUE, mono=True)
    head_text(f, 246, 90, "Connected", size=15, weight=600, color=HEAD_BLUE)
    for x in (344, 512, 654, 790):
        f.line(x, 70, x, 98, "#30343a", 1.2)
    head_text(f, 360, 93, "7.101.000", size=24, weight=300)
    head_text(f, 528, 90, "Rung 11 ↑  12 ↓", color=HEAD_MUTED)
    head_text(f, 670, 90, "SNR +12.3 dB", color=HEAD_MUTED)
    lamp(f, 810, 90, "TX", "#4b5563")
    lamp(f, 856, 90, "RX", HEAD_GREEN)
    lamp(f, 904, 90, "Busy", HEAD_AMBER)
    for n, x in (("1", 290), ("2", 428), ("3", 584), ("4", 722), ("5", 876)):
        f.line(x, 44, x, 58, FAINT, 1.2)
        f.number(x, 30, n, TEAL)
    # the right half: the badge, the programs, the modem, the views, the callsign
    f.rect(24, 180, 952, 64, GRAPHITE, GRAPHITE, rx=10, sw=1)
    head_text(f, 40, 218, "…", color=HEAD_MUTED)
    f.rect(76, 194, 92, 36, "#0f1f14", HEAD_GREEN, rx=5, sw=1.3)
    f.path(
        "M 92 203 L 100 200 L 108 203 L 108 211 Q 108 218 100 222 Q 92 218 92 211 Z",
        stroke=HEAD_GREEN,
        width=1.5,
    )
    head_text(f, 116, 218, "Legal", color=HEAD_GREEN, weight=600)
    f.rect(186, 194, 174, 36, "#0f1f14", HEAD_GREEN, rx=5, sw=1.3)
    head_text(f, 200, 218, "✓ RadioPost client", color=HEAD_GREEN, weight=600)
    f.circle(388, 213, 4.5, "#4b5563")
    head_text(f, 398, 218, "Modem", color=HEAD_MUTED)
    f.rect(478, 194, 54, 36, "#2a2e33", "#2a2e33", rx=5, sw=1)
    head_text(f, 489, 218, "Full", weight=600)
    head_text(f, 550, 218, "Compact", color=HEAD_MUTED)
    head_text(f, 640, 218, "Mini", color=HEAD_MUTED)
    head_text(f, 720, 218, "Light", color=HEAD_MUTED)
    head_text(f, 852, 218, "N1CALL", size=15.5, weight=700, mono=True)
    for n, x in (("6", 122), ("7", 273), ("8", 418), ("9", 590), ("10", 740), ("11", 897)):
        f.line(x, 172, x, 186, FAINT, 1.2)
        f.number(x, 158, n, TEAL)
    f.text(
        500,
        262,
        "The header is one row in the window; it is drawn here in two halves.",
        size=12,
        color=MUTED,
        anchor="middle",
        italic=True,
    )
    return f


# ------------------------------------------------------------------------------------------------
# Six ways to key the radio
# ------------------------------------------------------------------------------------------------


def mini_chain(f: Plate, x: float, y: float, parts: list[tuple[str, str, str]]) -> None:
    """A tiny chain of boxes: (label, role, link kind to the next) left to right."""
    gap = 36.0
    widths = [max(58.0, len(label) * 7.2 + 18) for label, _role, _kind in parts]
    total = sum(widths) + gap * (len(parts) - 1)
    cx = x + (272 - total) / 2
    for i, (label, role, kind) in enumerate(parts):
        fill, stroke = ROLES[role]
        w = widths[i]
        f.rect(cx, y, w, 32, fill, stroke, rx=7, sw=1.3)
        f.text(cx + w / 2, y + 21, label, size=12, weight=650, anchor="middle")
        if i + 1 < len(parts):
            f.link([(cx + w + 3, y + 16), (cx + w + gap - 4, y + 16)], kind)
        cx += w + gap


@figure("keying")
def fig_keying() -> Plate:
    f = Plate(1000, 476, "The six ways Aether can key the radio, and whether each reads the dial")
    cards = [
        (
            "CAT command",
            [("Aether", "aether", "key"), ("Radio", "neutral", "")],
            [
                "The radio's own command on its",
                "CAT port: Yaesu, Kenwood, Icom.",
                "One USB cable can carry it all.",
            ],
            ("Reads the dial", "ok"),
        ),
        (
            "Serial line: RTS or DTR",
            [
                ("Aether", "aether", "key"),
                ("Interface", "neutral", "key"),
                ("Radio", "neutral", ""),
            ],
            ["A line of a serial port keys", "the radio, through the interface."],
            ("Dial typed by hand", "caution"),
        ),
        (
            "rigctld or FLRig",
            [("Aether", "aether", "local"), ("FLRig", "program", "key"), ("Radio", "neutral", "")],
            ["Shares the radio: another program", "can tune it while Aether keys."],
            ("Reads the dial", "ok"),
        ),
        (
            "CM108 interface",
            [
                ("Aether", "aether", "cable"),
                ("DRA, URI", "neutral", "key"),
                ("Radio", "neutral", ""),
            ],
            ["The sound card's own pin keys", "the radio (GPIO 3 on those boards)."],
            ("Dial typed by hand", "caution"),
        ),
        (
            "Host program",
            [("Aether", "aether", "local"), ("VarAC", "program", "key"), ("Radio", "neutral", "")],
            ["The program owns the radio and", "keys it when Aether says PTT ON."],
            ("You check the rules", "caution"),
        ),
        (
            "VOX (a SignaLink)",
            [
                ("Aether", "aether", "audio"),
                ("SignaLink", "neutral", "key"),
                ("Radio", "neutral", ""),
            ],
            ["Sound on the line keys the radio.", "Set Turnaround to its delay."],
            ("Dial typed by hand", "caution"),
        ),
    ]
    for i, (title, chain, rows, (chip, chip_role)) in enumerate(cards):
        x = 24 + (i % 3) * 324
        y = 16 + (i // 3) * 222
        f.rect(x, y, 304, 206, PAPER, HAIR, rx=10, sw=1.3)
        f.number(x + 22, y + 24, str(i + 1), TEAL, r=10)
        f.text(x + 40, y + 29, title, size=15, weight=700)
        mini_chain(f, x + 16, y + 50, chain)
        f.lines(x + 18, y + 118, rows, size=12.5, color=MUTED)
        f.badge(x + 152, y + 188, chip, chip_role, size=12)
    return f


# ------------------------------------------------------------------------------------------------
# The rungs: how fast each goes and how much signal it needs (from the model)
# ------------------------------------------------------------------------------------------------


@figure("ladder")
def fig_ladder() -> Plate:
    from aether_model.frame.modes import NARROW, WIDE
    from ladder_table import front, ladder

    f = Plate(1000, 516, "Each rung's data rate against the signal it needs, for both bandwidths")
    x0, y0, w, h = 112.0, 24.0, 852.0, 408.0
    lo_db, hi_db = -20.0, 20.0
    lo_r, hi_r = math.log10(20), math.log10(8000)

    def px(db: float) -> float:
        return x0 + (db - lo_db) / (hi_db - lo_db) * w

    def py(rate: float) -> float:
        return y0 + h - (math.log10(rate) - lo_r) / (hi_r - lo_r) * h

    f.rect(x0, y0, px(-8) - x0, h, "#fff7ed", "none", rx=0, sw=0, opacity=0.7)
    f.text(x0 + 10, y0 + 22, "Tone rungs", size=13, weight=700, color=ORANGE)
    f.lines(
        x0 + 10,
        y0 + 40,
        ["one tone at a time:", "they get through", "under the noise"],
        size=12,
        color=MUTED,
        gap=1.35,
    )
    f.text(px(-6) + 10, y0 + 22, "OFDM rungs", size=13, weight=700, color=TEAL)
    f.lines(
        px(-6) + 10,
        y0 + 40,
        ["many tones at once:", "fast on a good path"],
        size=12,
        color=MUTED,
        gap=1.35,
    )
    xs = [(px(d), f"{'−' if d < 0 else '+' if d > 0 else ''}{abs(d)}") for d in range(-20, 21, 5)]
    ys = [(py(r), f"{r:,}") for r in (20, 50, 100, 200, 500, 1000, 2000, 5000)]
    f.axes(
        (x0, y0, w, h),
        xs,
        ys,
        xlabel="Signal-to-noise ratio it needs, dB in 3 kHz (9 frames in 10 through, steady path)",
        ylabel="Data rate, bit/s",
    )
    f.line(px(0), y0, px(0), y0 + h, FAINT, 1.2, dash="4 4")
    f.text(px(0) + 6, y0 + h - 10, "0 dB: signal as strong as the noise", size=11.5, color=MUTED)
    for air, color, name in ((WIDE, TEAL, "2300 Hz"), (NARROW, AMBER, "500 Hz")):
        rungs = ladder(air)
        used = front(rungs, "awgn")
        pts = [(px(rungs[i].threshold_db["awgn"]), py(rungs[i].data_bps)) for i in used]
        f.poly(pts, stroke=color, width=2.0, opacity=1.0)
        for r in rungs:
            x, y = px(r.threshold_db["awgn"]), py(r.data_bps)
            if r.index in used:
                f.circle(x, y, 5.5, color, PAPER, 1.5)
            else:
                f.circle(x, y, 5, PAPER, color, 1.8)
            below = name == "500 Hz"
            f.text(
                x + (0 if below else -1),
                y + (17 if below else -10),
                str(r.index),
                size=10.5,
                weight=650,
                color=color,
                anchor="middle",
                halo=True,
            )
        top = rungs[-1]
        f.text(
            px(top.threshold_db["awgn"]) + 10,
            py(top.data_bps) + 4,
            name,
            size=13,
            weight=700,
            color=color,
            halo=True,
        )
    # legend under the plot
    ly = y0 + h + 72
    f.circle(130, ly - 4, 5.5, INK, PAPER, 1.5)
    f.text(142, ly, "a rung the link uses (its number beside it)", size=12, color=MUTED)
    f.circle(470, ly - 4, 5, PAPER, INK, 1.8)
    f.text(
        482,
        ly,
        "a rung never asked for: another carries more from less signal",
        size=12,
        color=MUTED,
    )
    return f


# ------------------------------------------------------------------------------------------------
# Where the signal sits in the radio's audio (from the model)
# ------------------------------------------------------------------------------------------------


def skirt_block(f: Plate, x1: float, x2: float, base: float, top: float, color: str) -> None:
    """A signal's spectrum: flat across its span, falling away steeply at its edges."""
    s = 7.0
    d = (
        f"M {x1 - s:.1f} {base:.1f} C {x1 - s / 2:.1f} {base:.1f} {x1 - 2:.1f} {top:.1f} "
        f"{x1 + 3:.1f} {top:.1f} L {x2 - 3:.1f} {top:.1f} C {x2 + 2:.1f} {top:.1f} "
        f"{x2 + s / 2:.1f} {base:.1f} {x2 + s:.1f} {base:.1f} Z"
    )
    f.path(d, fill=color, stroke=color, width=1.5, opacity=0.22)


@figure("bandwidth")
def fig_bandwidth() -> Plate:
    from aether_model.frame.modes import NARROW, WIDE

    f = Plate(1000, 352, "Where Aether's two signals sit in the radio's receive audio")
    x0, x1 = 120.0, 960.0

    def px(hz: float) -> float:
        return x0 + hz / 3500 * (x1 - x0)

    strips = [(WIDE, 26.0, TEAL), (NARROW, 136.0, AMBER)]
    for air, top, color in strips:
        p = air.params
        lo = p.centre_hz - p.occupied_bandwidth_hz / 2
        hi = p.centre_hz + p.occupied_bandwidth_hz / 2
        base = top + 92
        f.line(x0, base, x1, base, FAINT, 1)
        # a typical SSB receive filter, 300–2700 Hz
        f.poly(
            [(px(150), base), (px(300), top + 10), (px(2700), top + 10), (px(2900), base)],
            stroke=FAINT,
            width=1.4,
            dash="5 4",
        )
        skirt_block(f, px(lo), px(hi), base, top + 24, color)
        f.text(28, top + 50, f"{p.bandwidth.value} Hz", size=16, weight=700, color=color)
        f.text(28, top + 70, "signal", size=12.5, color=MUTED)
        f.text(
            (px(lo) + px(hi)) / 2,
            top + 52,
            f"{lo:,.0f}–{hi:,.0f} Hz",
            size=13,
            weight=700,
            color=color,
            anchor="middle",
            halo=True,
        )
    f.text(px(2720), 52, "a typical 2.4 kHz", size=11.5, color=MUTED)
    f.text(px(2720), 67, "SSB receive filter", size=11.5, color=MUTED)
    ticks = [(px(hz), f"{hz:,}") for hz in range(0, 3501, 500)]
    for x, label in ticks:
        f.line(x, 228, x, 234, FAINT, 1)
        f.text(x, 250, label, size=11.5, color=MUTED, anchor="middle")
    f.text(
        (x0 + x1) / 2,
        274,
        "Audio frequency, Hz above the dial (upper sideband)",
        size=12.5,
        weight=600,
        anchor="middle",
    )
    f.rect(24, 292, 952, 44, WASH, HAIR, rx=8, sw=1.2)
    centre = WIDE.params.centre_hz
    f.text(
        38,
        319,
        f"Both are centred {centre:,.0f} Hz above the dial: with the dial at "
        f"7.1010 MHz, the signal's centre is on {7.101 + centre / 1e6:.4f} MHz. The 500 Hz "
        "signal fits a 500–600 Hz filter centred there.",
        size=12.5,
        color=MUTED,
    )
    return f


# ------------------------------------------------------------------------------------------------
# A session, from the call to the goodbye
# ------------------------------------------------------------------------------------------------


@figure("session")
def fig_session() -> Plate:
    f = Plate(1000, 418, "A session between two stations, transmission by transmission")
    lanes = {"you": 70.0, "peer": 160.0}
    f.text(24, lanes["you"] + 4, "N1CALL", size=14, weight=700, color=BLUE)
    f.text(24, lanes["you"] + 22, "calls", size=12, color=MUTED)
    f.text(24, lanes["peer"] + 4, "N2CALL", size=14, weight=700, color=ORANGE)
    f.text(24, lanes["peer"] + 22, "is called", size=12, color=MUTED)
    for y in lanes.values():
        f.line(110, y + 6, 980, y + 6, HAIR, 1)
    blocks = [
        ("you", 120, 196, "Call", "floor"),
        ("peer", 204, 274, "OK", "floor"),
        ("you", 284, 452, "Header + data", "data"),
        ("peer", 462, 504, "Ack", "ctl"),
        ("you", 514, 626, "Data", "data"),
        ("peer", 636, 760, "Ack + its data", "data"),
        ("you", 770, 812, "Bye", "ctl"),
        ("peer", 822, 900, "Bye + ID", "id"),
        ("you", 910, 976, "ID", "id"),
    ]
    for who, a, b, label, kind in blocks:
        fill, stroke = ROLES[who]
        y = lanes[who] - 16
        f.rect(a, y, b - a, 28, fill, stroke, rx=6, sw=1.4, dash="3 3" if kind == "floor" else None)
        f.text((a + b) / 2, y + 19, label, size=12, weight=650, color=stroke, anchor="middle")
        if kind == "data":
            # a header (on the caller's) and the frames after it
            for k in range(1, 4):
                fx = a + (b - a) * k / 4
                f.line(fx, y + 3, fx, y + 7, stroke, 1.2)
                f.line(fx, y + 21, fx, y + 25, stroke, 1.2)
    # each of the caller's transmissions grants the other a slot
    for end, start in ((452, 462), (626, 636), (812, 822)):
        f.path(
            f"M {end - 6} {lanes['you'] + 14} C {end + 2} {lanes['you'] + 50} {start - 2} "
            f"{lanes['peer'] - 54} {start + 6} {lanes['peer'] - 18}",
            stroke=FAINT,
            width=1.3,
        )
    f.text(540, 117, "each grants a slot", size=11.5, color=MUTED, anchor="middle", halo=True)
    f.number(158, 30, "1", INK)
    f.number(368, 30, "2", INK)
    f.number(560, 30, "3", INK)
    f.number(791, 30, "4", INK)
    f.line(110, 206, 980, 206, FAINT, 1)
    f.link([(110, 206), (980, 206)], "local", head=True)
    f.text(980, 224, "time", size=11.5, color=MUTED, anchor="end")
    f.panel(24, 238, 952, 168, "What happens")
    notes = [
        (
            "1",
            "The call goes on the tone floor, which gets through the weakest paths. The called "
            "station accepts it.",
        ),
        (
            "2",
            "The calling station sends a header and its data. The header says how long it is "
            "and gives the other a slot.",
        ),
        (
            "3",
            "The called station keys only in its slot: an answer, and its own data when it "
            "has some. The two never key at once.",
        ),
        (
            "4",
            "Bye: what is queued goes first. Morse IDs come last, the called station's, "
            "then yours.",
        ),
    ]
    y = 286.0
    for n, text in notes:
        f.number(46, y - 4, n, INK, r=9.5)
        f.text(66, y, text, size=12.5, color=INK)
        y += 32
    return f


# ------------------------------------------------------------------------------------------------
# From the installer to the first contact
# ------------------------------------------------------------------------------------------------


@figure("firststeps")
def fig_firststeps() -> Plate:
    f = Plate(1000, 196, "Six steps from installing Aether to a first contact")
    steps = [
        ("Install", ["Download, run", "the installer"], "plug"),
        ("Setup", ["Six steps, then", "Save"], "window"),
        ("Set drive", ["ALC just moving,", "then back off"], "radio"),
        ("Listen", ["Is the channel", "clear?"], "antenna"),
        ("Probe", ["Do you hear each", "other? How well?"], "wave"),
        ("Connect", ["Call, type, send", "files, Disconnect"], "person"),
    ]
    w, gap = 146.0, 15.2
    for i, (title, rows, icon) in enumerate(steps):
        x = 24 + i * (w + gap)
        f.rect(x, 24, w, 148, PAPER, HAIR, rx=10, sw=1.3)
        f.number(x + 22, 48, str(i + 1), TEAL)
        f.icon(icon, x + w - 32, 38, TEAL)
        f.text(x + 16, 92, title, size=16, weight=700)
        f.lines(x + 16, 118, rows, size=12.5, color=MUTED)
        if i + 1 < len(steps):
            ax = x + w + 2
            f.path(f"M {ax:.1f} 92 L {ax + gap - 4:.1f} 98 L {ax:.1f} 104", stroke=FAINT, width=1.6)
    return f


# ------------------------------------------------------------------------------------------------
# The two levels: receive and drive
# ------------------------------------------------------------------------------------------------


@figure("levels")
def fig_levels() -> Plate:
    f = Plate(1000, 360, "Setting the receive level and the transmit drive")
    # receive level, the meter Setup step 3 shows
    f.panel(24, 16, 466, 330, "Receive level  ·  Setup step 3")

    def mx(db: float) -> float:
        return 56 + (db + 60) / 60 * 372

    bar_y = 92.0
    f.rect(mx(-60), bar_y, mx(0) - mx(-60), 26, PAPER, FAINT, rx=4, sw=1)
    f.rect(mx(-60), bar_y, mx(-40) - mx(-60), 26, "#e2e8f0", "none", rx=4, sw=0)
    f.rect(mx(-40), bar_y, mx(-3) - mx(-40), 26, "#dcfce7", "none", rx=0, sw=0)
    f.rect(mx(-3), bar_y, mx(0) - mx(-3), 26, "#fef3c7", "none", rx=0, sw=0)
    f.rect(mx(-60), bar_y + 8, mx(-24) - mx(-60), 10, GREEN, "none", rx=3, sw=0)
    f.line(mx(-9), bar_y - 4, mx(-9), bar_y + 30, INK, 2)
    f.text(mx(-24), bar_y - 10, "RMS", size=11.5, color=GREEN, anchor="middle", weight=650)
    f.text(mx(-9), bar_y - 10, "peak", size=11.5, color=INK, anchor="middle", weight=650)
    f.line(mx(0), bar_y - 6, mx(0), bar_y + 32, RED, 2.4)
    for db in (-60, -40, -20, -3, 0):
        f.text(
            mx(db),
            bar_y + 46,
            f"{'−' if db < 0 else ''}{abs(db)}",
            size=11.5,
            color=MUTED,
            anchor="middle",
        )
    f.text(mx(-30), bar_y + 64, "dBFS", size=11.5, color=MUTED, anchor="middle")
    f.text(mx(-50), bar_y + 88, "Very quiet", size=12.5, weight=650, color=MUTED, anchor="middle")
    f.text(mx(-21), bar_y + 88, "Good", size=12.5, weight=650, color=GREEN, anchor="middle")
    f.text(474, bar_y + 88, "Almost", size=12.5, weight=650, color=AMBER, anchor="end")
    f.text(474, bar_y + 104, "clipping", size=12.5, weight=650, color=AMBER, anchor="end")
    f.lines(
        42,
        236,
        [
            "With the radio on a quiet frequency, turn the radio's",
            "audio output or the sound card's input until the meter",
            "says Good: RMS above −40 dBFS, peaks under −3.",
            "Clipping is never acceptable.",
        ],
        size=12.5,
        color=MUTED,
    )
    # drive: the radio's output against the transmit level
    f.panel(510, 16, 466, 330, "Transmit drive  ·  Session tab, Set drive")
    x0, y0, w, h = 572.0, 60.0, 380.0, 170.0

    def dx(level_db: float) -> float:
        return x0 + (level_db + 30) / 30 * w

    def dy(out: float) -> float:
        return y0 + h - out * h

    knee = -12.0
    pts = []
    for i in range(61):
        level = -30 + i * 0.5
        lin = 10 ** ((level - knee) / 10)
        out = min(lin, 1.0) if level <= knee else 1.0
        pts.append((dx(level), dy(out * 0.92)))
    f.rect(dx(knee - 4), y0, dx(knee) - dx(knee - 4), h, "#dcfce7", "none", rx=0, sw=0)
    f.rect(dx(knee), y0, dx(0) - dx(knee), h, "#fef2f2", "none", rx=0, sw=0)
    f.axes(
        (x0, y0, w, h),
        [(dx(v), f"{'−' if v < 0 else ''}{abs(v)}") for v in (-30, -20, -10, 0)],
        [(dy(0), "0"), (dy(0.92), "full")],
        xlabel="Transmit level, dB below full scale",
    )
    f.poly(pts, stroke=TEAL, width=2.4)
    f.text(dx(knee - 2), y0 + 44, "set", size=12, weight=700, color=GREEN, anchor="middle")
    f.text(dx(knee - 2), y0 + 59, "here", size=12, weight=700, color=GREEN, anchor="middle")
    f.text(dx(knee) + 12, y0 + 44, "ALC limiting:", size=12, weight=700, color=RED)
    f.text(dx(knee) + 12, y0 + 59, "the signal distorts", size=12, color=RED)
    f.text(x0 - 10, y0 - 6, "Power out", size=11.5, color=MUTED)
    f.lines(
        526,
        302,
        [
            "Raise the level until the ALC just moves, then back off.",
            "For more power, turn up the radio's power, not the level.",
        ],
        size=12.5,
        color=MUTED,
    )
    return f


# ------------------------------------------------------------------------------------------------
# When the radio's receive filter cuts the signal
# ------------------------------------------------------------------------------------------------


@figure("filter")
def fig_filter() -> Plate:
    f = Plate(
        1000, 300, "The receive filter passing Aether's signal, cutting its top, cutting its bottom"
    )
    hatch = f.hatch("cut", "#fca5a5")
    cases = [
        ("Passes the signal", (300, 2700), "ok", "Nothing to do"),
        ("Cuts the top", (100, 2100), "warn", "Turn: IF shift, high cut"),
        ("Cuts the bottom", (700, 3100), "warn", "Turn: IF shift, low cut"),
    ]
    lo, hi = 360.0, 2640.0
    for i, (title, (f_lo, f_hi), role, advice) in enumerate(cases):
        x = 24 + i * 324
        f.rect(x, 16, 304, 268, PAPER, HAIR, rx=10, sw=1.3)
        f.text(x + 16, 44, title, size=15, weight=700, color=ROLES[role][1])
        ax0, ax1 = x + 18, x + 286

        def px(hz: float, ax0: float = ax0, ax1: float = ax1) -> float:
            return ax0 + hz / 3200 * (ax1 - ax0)

        base, top = 190.0, 84.0
        f.line(ax0, base, ax1, base, FAINT, 1)
        skirt_block(f, px(lo), px(hi), base, top + 22, TEAL)
        if f_hi < hi:
            f.rect(
                px(f_hi),
                top + 22,
                px(hi) - px(f_hi),
                base - top - 22,
                hatch,
                "none",
                rx=0,
                sw=0,
                opacity=0.9,
            )
        if f_lo > lo:
            f.rect(
                px(lo),
                top + 22,
                px(f_lo) - px(lo),
                base - top - 22,
                hatch,
                "none",
                rx=0,
                sw=0,
                opacity=0.9,
            )
        f.poly(
            [
                (px(max(f_lo - 150, 0)), base),
                (px(f_lo), top),
                (px(f_hi), top),
                (px(min(f_hi + 150, 3200)), base),
            ],
            stroke=INK,
            width=1.6,
        )
        f.text(px(1500), top - 8, "the radio's filter", size=11.5, color=MUTED, anchor="middle")
        for hz in (0, 1000, 2000, 3000):
            f.text(px(hz), base + 18, f"{hz:,}", size=11, color=MUTED, anchor="middle")
        f.text(px(1600), base + 34, "Hz", size=11, color=MUTED, anchor="middle")
        f.badge(x + 152, 266, advice, role, size=12)
    return f


# ------------------------------------------------------------------------------------------------
# Programs that use Aether, and the ports they reach it on
# ------------------------------------------------------------------------------------------------


@figure("programs")
def fig_programs() -> Plate:
    f = Plate(1000, 452, "The programs that use Aether and the ports they connect to")
    rows = [
        (40, "aether", "Aether's own window", ["The panel you work in"], "window"),
        (124, "radiopost", "RadioPost", ["The client and RadioPost", "Gateway"], "envelope"),
        (
            222,
            "program",
            "Winlink and VarAC",
            ["Winlink Express, Pat,", "VarAC, RMS Trimode"],
            "window",
        ),
        (326, "program", "Packet programs", ["APRS, packet, VarAC's", "broadcasts"], "antenna"),
    ]
    for y, role, title, lines, icon in rows:
        f.box(24, y, 256, 80, role, title=title, rows=lines, icon=icon, title_size=15)
    ports = [
        (82, "Control port", "8515", "the control API", 124),
        (218, "Host port", "8300 and 8301", "VARA-compatible", 88),
        (322, "KISS port", "8100, when on", "KISS over TCP", 88),
    ]
    for y, name, number, what, h in ports:
        f.rect(340, y, 190, h, WASH, FAINT, rx=8, sw=1.3)
        f.text(356, y + 28, name, size=14, weight=700)
        f.text(356, y + 50, number, size=13, color=MUTED, mono=True)
        f.text(356, y + 72, what, size=12, color=MUTED)
    f.link([(284, 80), (336, 118)], "local")
    f.link([(284, 164), (336, 160)], "local")
    f.link([(284, 262), (336, 262)], "local")
    f.link([(284, 366), (336, 366)], "local")
    fill, stroke = ROLES["aether"]
    f.rect(600, 82, 190, 328, fill, stroke)
    f.text(616, 112, "Aether", size=16, weight=700)
    f.text(616, 130, "the modem", size=12.5, color=MUTED)
    f.lines(
        616,
        166,
        [
            "One session at a",
            "time.",
            "",
            "Each program",
            "answers for its",
            "own callsigns.",
            "",
            "Every transmission",
            "checked against",
            "the rules.",
        ],
        size=12.5,
        color=INK,
        gap=1.4,
    )
    for y in (144, 262, 366):
        f.link([(534, y), (596, y)], "local")
    f.box(
        830,
        176,
        146,
        120,
        "neutral",
        tag="Via the interface",
        title="Radio",
        rows=["audio and", "keying"],
    )
    f.link([(794, 218), (826, 218)], "audio", both=True)
    f.link([(794, 258), (826, 258)], "key")
    f.legend(
        440,
        [
            ("local", "A port on the same computer (or your network)"),
            ("audio", "Audio"),
            ("key", "Keying"),
        ],
    )
    return f


# ------------------------------------------------------------------------------------------------
# Mail by radio: Winlink and RadioPost through Aether
# ------------------------------------------------------------------------------------------------


@figure("mail")
def fig_mail() -> Plate:
    f = Plate(
        1000,
        400,
        "Winlink and RadioPost mail carried by Aether, from a station to a gateway to the Internet",
    )
    rows = [
        (
            36,
            "Winlink",
            ("Winlink Express", "or Pat"),
            ("RMS Trimode", "the Winlink gateway"),
            ("Winlink", "the CMS servers"),
            "program",
        ),
        (
            214,
            "RadioPost",
            ("RadioPost", "the client"),
            ("RadioPost Gateway", ""),
            ("Your post office", "then any email"),
            "radiopost",
        ),
    ]
    for y, name, you, gw, far, role in rows:
        f.text(24, y - 10, name, size=15, weight=700, color=ROLES[role][1])
        f.rect(24, y, 250, 120, ROLES["you"][0], ROLES["you"][1])
        f.text(38, y + 24, "Your station", size=11.5, weight=650, color=BLUE)
        f.text(38, y + 46, you[0], size=15, weight=700)
        f.text(38, y + 64, you[1], size=12.5, color=MUTED)
        f.rect(38, y + 76, 222, 32, ROLES["aether"][0], TEAL, rx=7, sw=1.2)
        f.text(149, y + 97, "+ Aether + radio", size=12.5, weight=650, color=TEAL, anchor="middle")
        f.rect(420, y, 250, 120, ROLES["peer"][0], ROLES["peer"][1])
        f.text(434, y + 24, "A gateway", size=11.5, weight=650, color=ORANGE)
        f.text(434, y + 46, gw[0], size=15, weight=700)
        if gw[1]:
            f.text(434, y + 64, gw[1], size=12.5, color=MUTED)
        f.rect(434, y + 76, 222, 32, ROLES["aether"][0], TEAL, rx=7, sw=1.2)
        f.text(545, y + 97, "+ Aether + radio", size=12.5, weight=650, color=TEAL, anchor="middle")
        f.box(
            780,
            y + 10,
            196,
            100,
            "neutral",
            tag="On the Internet",
            title=far[0],
            rows=[far[1]],
            icon="globe",
        )
        f.link([(278, y + 60), (416, y + 60)], "radio", both=True, label="by radio")
        f.link([(674, y + 60), (776, y + 60)], "internet", both=True, label="Internet")
    f.text(
        500,
        384,
        "Station to station needs no gateway: the two stations' programs talk "
        "directly, through Aether.",
        size=12.5,
        color=MUTED,
        anchor="middle",
        italic=True,
    )
    return f


# ------------------------------------------------------------------------------------------------
# A Test session, step by step
# ------------------------------------------------------------------------------------------------


@figure("test")
def fig_test() -> Plate:
    f = Plate(1000, 244, "The six steps of a Test session and what each measures")
    steps = [
        ("Probe", ["Do you hear", "each other?"]),
        ("Call", ["Starts the", "session"]),
        ("Message", ["Its speed, for", "a short text"]),
        ("File", ["About two", "minutes of data"]),
        ("Ladder", ["Each rung tried", "once"]),
        ("Disconnect", ["The report", "is saved"]),
    ]
    w, gap = 146.0, 15.2
    for i, (title, rows) in enumerate(steps):
        x = 24 + i * (w + gap)
        f.rect(x, 20, w, 128, ROLES["aether"][0], TEAL, rx=10, sw=1.3)
        f.number(x + 22, 44, str(i + 1), TEAL)
        f.text(x + 16, 84, title, size=16, weight=700)
        f.lines(x + 16, 108, rows, size=12.5, color=MUTED)
        if i + 1 < len(steps):
            ax = x + w + 2
            f.path(f"M {ax:.1f} 78 L {ax + gap - 4:.1f} 84 L {ax:.1f} 90", stroke=FAINT, width=1.6)
    f.rect(24, 172, 952, 12, "#e2e8f0", "none", rx=6, sw=0)
    f.rect(24, 172, 952 / 2, 12, "#99f6e4", "none", rx=6, sw=0)
    for minute, label, anchor in (
        (0, "0", "start"),
        (5, "about 5 minutes", "middle"),
        (10, "10 minutes: it stops itself", "end"),
    ):
        x = 24 + 952 * minute / 10
        f.line(x, 166, x, 190, INK, 1.4)
        f.text(x, 206, label, size=12, color=MUTED, anchor=anchor)
    f.text(
        24,
        230,
        "The other station only has to be listening. The report is saved as a .json "
        "file ending in _test, with the audio beside it.",
        size=12.5,
        color=MUTED,
    )
    return f


# ------------------------------------------------------------------------------------------------
# Every transmission is judged against the rules first
# ------------------------------------------------------------------------------------------------


@figure("rules")
def fig_rules() -> Plate:
    f = Plate(
        1000, 250, "Every transmission is judged against the band rules before the radio is keyed"
    )
    f.box(
        24,
        40,
        220,
        150,
        "you",
        tag="Something to send",
        title="A transmission",
        rows=["a call, an answer, data,", "a beacon, a tuner tone,", "a Morse ID"],
    )
    fill, stroke = ROLES["aether"]
    f.rect(300, 24, 380, 182, fill, stroke)
    f.text(318, 52, "The rules check", size=16, weight=700)
    checks = [
        "Your licence class: is this frequency yours?",
        "The band segment at the dial: data allowed?",
        "Both edges of the signal inside the segment",
        "Local, remote or automatic control",
        "Its width, by the FCC's 26 dB measure",
    ]
    y = 82.0
    for c in checks:
        f.circle(326, y - 4, 3, stroke)
        f.text(338, y, c, size=12.5)
        y += 24
    f.link([(248, 115), (296, 115)], "local")
    f.box(
        740,
        24,
        236,
        84,
        "ok",
        title="Legal",
        rows=["The radio is keyed and", "the signal goes out"],
    )
    f.box(
        740,
        122,
        236,
        84,
        "warn",
        title="Not allowed",
        rows=["Nothing goes out. The badge", "in the header says why."],
    )
    f.link([(684, 80), (736, 66)], "audio", label="yes", dy=-6)
    f.link([(684, 150), (736, 164)], "fail", label="no", dy=16)
    f.text(
        24,
        236,
        "Unset, nothing transmits: choose the rules in Setup step 1. You remain the "
        "control operator.",
        size=12.5,
        color=MUTED,
        italic=True,
    )
    return f


# ------------------------------------------------------------------------------------------------
# Aether FM, as planned
# ------------------------------------------------------------------------------------------------


@figure("fm")
def fig_fm() -> Plate:
    f = Plate(1000, 404, "Aether FM as planned: simplex, through a repeater, and packet")
    label = "PLANNED: NOT IN A RELEASE YET"
    f.badge(24 + (len(label) * 12 * 0.58 + 16) / 2, 32, label, "caution", size=12)
    # simplex
    f.text(24, 74, "Station to station (simplex)", size=14, weight=700)
    f.box(
        24,
        88,
        200,
        96,
        "planned",
        tag="Same application",
        title="Aether FM",
        rows=["one modem per radio"],
    )
    f.box(
        300,
        88,
        170,
        96,
        "planned",
        tag="VHF or UHF",
        title="FM radio",
        rows=["voice path or", "9600 data port"],
        icon="radio",
    )
    f.link([(228, 136), (296, 136)], "plan", both=True)
    f.link([(474, 136), (606, 136)], "radio", both=True, label="simplex")
    f.box(
        610,
        88,
        170,
        96,
        "planned",
        tag="The other station",
        title="FM radio",
        rows=["and Aether FM"],
        icon="radio",
    )
    f.box(
        820,
        88,
        156,
        96,
        "planned",
        tag="Your programs",
        title="RadioPost",
        rows=["Winlink, packet"],
    )
    f.link([(816, 136), (784, 136)], "plan", both=True)
    f.path("M 695 190 C 680 236 400 236 388 196", stroke=GREEN, width=1.8, dash="4 3")
    f.path("M 382 204 L 388 194 L 396 202", stroke=GREEN, width=1.8)
    f.text(
        540,
        250,
        "it measures your level and tells you",
        size=12,
        color=GREEN,
        anchor="middle",
        weight=600,
        halo=True,
    )
    # repeater
    f.text(24, 294, "Through a repeater", size=14, weight=700)
    f.badge(222, 293, "off by default", "neutral")
    f.box(24, 306, 200, 72, "planned", title="Your station", rows=["Aether FM and radio"])
    f.box(
        400,
        306,
        200,
        72,
        "planned",
        title="Repeater",
        rows=["with its owner's leave"],
        icon="antenna",
    )
    f.box(776, 306, 200, 72, "planned", title="Other station", rows=["Aether FM and radio"])
    f.link([(228, 342), (396, 342)], "radio", both=True)
    f.link([(604, 342), (772, 342)], "radio", both=True)
    f.text(
        24,
        398,
        "Packet radio too: 1200 baud AFSK, 9600 baud G3RUH, FX.25, IL2P and AX.25, "
        "for APRS, BBSs and nodes.",
        size=12.5,
        color=MUTED,
        italic=True,
    )
    return f


# ------------------------------------------------------------------------------------------------


def draw_all() -> dict[str, str]:
    return {name: draw().svg() for name, draw in FIGURES.items()}


def main() -> int:
    check = "--check" in sys.argv[1:]
    drawn = draw_all()
    stale = []
    for name, svg in drawn.items():
        path = OUT / f"{name}.svg"
        if check:
            if not path.exists() or path.read_text(encoding="utf-8") != svg:
                stale.append(name)
        else:
            OUT.mkdir(parents=True, exist_ok=True)
            path.write_text(svg, encoding="utf-8", newline="\n")
            print(path.relative_to(ROOT))
    if check:
        extra = sorted(p.stem for p in OUT.glob("*.svg") if p.stem not in drawn)
        for name in stale:
            print(f"docs/manual/fig/{name}.svg is not what figures.py draws", file=sys.stderr)
        for name in extra:
            print(f"docs/manual/fig/{name}.svg is drawn by nothing", file=sys.stderr)
        return 1 if stale or extra else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
