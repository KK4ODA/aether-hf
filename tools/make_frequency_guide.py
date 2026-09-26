"""The Region 2 frequency companion, as a PDF an operator can print and take to the shack.

    python tools/make_frequency_guide.py [out.pdf]

The content is the reasoning in `docs/user/frequency-plan.md` narrowed to Region 2 and the
United States, where most early Aether stations are, and laid out for reading beside a
radio rather than on a screen. It is regenerated rather than edited, so the plan and the
guide cannot drift apart.

Nothing here is legal advice, and the document says so on its first page: segment edges
move, watering holes move, and the operator's licence is the thing that binds.
"""

from __future__ import annotations

import datetime as dt
import sys
from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.enums import TA_LEFT
from reportlab.lib.pagesizes import letter
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import inch
from reportlab.platypus import (
    HRFlowable,
    KeepTogether,
    PageBreak,
    Paragraph,
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)

# Aether's accent, darkened for ink: the panel's teal is a screen colour.
ACCENT = colors.HexColor("#0f8f80")
INK = colors.HexColor("#16202a")
INK_2 = colors.HexColor("#4d5c6b")
RULE = colors.HexColor("#c8d2dc")
BAND = colors.HexColor("#eef3f7")
WARN = colors.HexColor("#9a3412")

MARGIN = 0.75 * inch


def styles() -> dict[str, ParagraphStyle]:
    base = getSampleStyleSheet()
    return {
        "title": ParagraphStyle(
            "title",
            parent=base["Title"],
            fontName="Helvetica-Bold",
            fontSize=22,
            leading=26,
            textColor=INK,
            alignment=TA_LEFT,
            spaceAfter=2,
        ),
        "subtitle": ParagraphStyle(
            "subtitle",
            parent=base["Normal"],
            fontName="Helvetica",
            fontSize=11.5,
            leading=15,
            textColor=ACCENT,
            spaceAfter=10,
        ),
        "h1": ParagraphStyle(
            "h1",
            parent=base["Heading1"],
            fontName="Helvetica-Bold",
            fontSize=13.5,
            leading=17,
            textColor=INK,
            spaceBefore=13,
            spaceAfter=4,
        ),
        "h2": ParagraphStyle(
            "h2",
            parent=base["Heading2"],
            fontName="Helvetica-Bold",
            fontSize=11,
            leading=14,
            textColor=ACCENT,
            spaceBefore=11,
            spaceAfter=3,
        ),
        "body": ParagraphStyle(
            "body",
            parent=base["Normal"],
            fontName="Helvetica",
            fontSize=9.7,
            leading=13.0,
            textColor=INK,
            spaceAfter=5,
        ),
        "note": ParagraphStyle(
            "note",
            parent=base["Normal"],
            fontName="Helvetica-Oblique",
            fontSize=9.2,
            leading=12.2,
            textColor=INK_2,
            spaceAfter=5,
        ),
        "warn": ParagraphStyle(
            "warn",
            parent=base["Normal"],
            fontName="Helvetica-Bold",
            fontSize=9.7,
            leading=13.2,
            textColor=WARN,
            spaceAfter=6,
        ),
        "cell": ParagraphStyle(
            "cell",
            parent=base["Normal"],
            fontName="Helvetica",
            fontSize=8.6,
            leading=11,
            textColor=INK,
        ),
        "cellhead": ParagraphStyle(
            "cellhead",
            parent=base["Normal"],
            fontName="Helvetica-Bold",
            fontSize=8.6,
            leading=11,
            textColor=colors.white,
        ),
    }


def table(rows: list[list[str]], widths: list[float], s: dict) -> Table:
    """A table whose first row is its heading, sized to the text frame."""
    data = [[Paragraph(c, s["cellhead"]) for c in rows[0]]]
    data += [[Paragraph(c, s["cell"]) for c in row] for row in rows[1:]]
    t = Table(data, colWidths=widths, repeatRows=1, hAlign="LEFT")
    t.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), INK),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, BAND]),
                ("GRID", (0, 0), (-1, -1), 0.4, RULE),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("LEFTPADDING", (0, 0), (-1, -1), 5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 5),
                ("TOPPADDING", (0, 0), (-1, -1), 4),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
            ]
        )
    )
    return t


def furniture(canvas, doc) -> None:
    """The running foot: what this is, and that it is not the rule itself."""
    canvas.saveState()
    canvas.setStrokeColor(RULE)
    canvas.setLineWidth(0.5)
    canvas.line(MARGIN, 0.62 * inch, letter[0] - MARGIN, 0.62 * inch)
    canvas.setFont("Helvetica", 7.4)
    canvas.setFillColor(INK_2)
    canvas.drawString(
        MARGIN,
        0.46 * inch,
        "Aether HF frequency companion, Region 2. Guidance, not legal advice: "
        "verify against the current rules and your own band plan.",
    )
    canvas.drawRightString(letter[0] - MARGIN, 0.46 * inch, f"Page {doc.page}")
    canvas.restoreState()


def story(s: dict) -> list:
    """The document, in order."""
    width = letter[0] - 2 * MARGIN
    out: list = []
    p = lambda text, style="body": Paragraph(text, s[style])  # noqa: E731

    out += [
        p("Where to operate", "title"),
        p("An Aether HF frequency companion for Region 2 and the United States", "subtitle"),
        HRFlowable(width="100%", thickness=1, color=ACCENT, spaceAfter=10),
    ]

    out += [
        p("Read this first", "h1"),
        p(
            "This is <b>guidance, not law and not legal advice</b>. Segment edges, band plans "
            "and the frequencies other modes gather on all move. What binds you is your "
            "licence, the current text of the rules, and the band plan where you operate. "
            "Check them before you key up."
        ),
        p(
            "Aether runs at <b>2.3 kHz</b> or <b>500 Hz</b>, whichever the station is set to; "
            "measured by the rules' own 26 dB definition of bandwidth, the signals occupy up to "
            "<b>2.52 kHz</b> and <b>0.71 kHz</b>. That number decides most of what follows, "
            "because it decides what fits where. Aether's code is publicly documented, which is "
            "what 47 CFR 97.309(a)(4) requires of a digital code used on the amateur bands; the "
            "specification lives at docs/spec/air-interface.md in the project."
        ),
        p(
            "Aether checks every transmission against these rules before it keys the radio - "
            "the segment, your licence class, how the station is controlled - and refuses what "
            "they do not allow. Tell it once, in Setup step 1. It is a safeguard against "
            "mistakes, not a substitute for the control operator, who is still you."
        ),
        p(
            "Aether is new on crowded bands, so <b>Aether moves</b> - not the established mode "
            "that was there first. If you find yourself on top of somebody, change frequency "
            "rather than wait them out.",
            "warn",
        ),
    ]

    out += [
        p("The distinction that decides everything", "h1"),
        p(
            "Almost every question about where an Aether station may sit comes down to one "
            "thing: <b>is somebody there?</b>"
        ),
        p("Attended, under local or remote control", "h2"),
        p(
            "You are at the radio, or you are controlling it from somewhere with a means of "
            "shutting it down. 47 CFR 97.221, which governs automatic control, <b>does not "
            "apply</b>. You may work Aether anywhere your licence permits data on that band, "
            "at either bandwidth. This is the ordinary case: two operators arranging a contact."
        ),
        p("Automatically controlled, with nobody there", "h2"),
        p(
            "The station is left listening and answers calls on its own - a gateway, or a "
            "station you walked away from. Then 97.221 applies, and there are two ways to be "
            "legal:"
        ),
        p(
            "<b>Inside the 97.221(b) segments</b> (the third table below), and anywhere on "
            "6 m, the station may call, answer and start a session, at 2.3 kHz or 500 Hz. It "
            "will not beacon on a timer: 97.203(d) allows an automatically controlled beacon "
            "only in a few segments."
        ),
        p(
            "<b>Outside them</b>, 97.221(c) lets a station transmit only <b>in response</b> to "
            "a station under local or remote control, and only at 500 Hz or less. Aether's "
            "signals all measure wider than that - even the 500 Hz setting's - so Aether does "
            "not use this allowance: under automatic control it transmits inside the (b) "
            "segments and on 6 m, and nowhere else. <b>Answer only</b> in Setup, step 4 "
            "(<font face='Courier'>[radio] answer_only = true</font>) keeps an unattended "
            "station from calling, beaconing or probing on its own."
        ),
        p(
            "Only you know how the station is controlled: set <b>Control</b> in Setup, step 1, "
            "to Automatic whenever nobody is at the radio. The modem never works it out.",
            "note",
        ),
    ]

    out += [PageBreak()]

    out += [
        p("1. Where data is permitted in the United States", "h1"),
        p(
            "RTTY and data segments by licence class, from 47 CFR 97.301 and 97.305. An "
            "attended station may use any of this; an unattended one is further limited by "
            "table 3."
        ),
        table(
            [
                ["Band", "Extra", "General and Advanced", "Notes"],
                [
                    "160 m",
                    "1.800 - 2.000",
                    "1.800 - 2.000",
                    "Shared with a lot of weak-signal work at night.",
                ],
                [
                    "80 m",
                    "3.500 - 3.600",
                    "3.525 - 3.600",
                    "Allocations vary more here than on any other band.",
                ],
                [
                    "60 m",
                    "channelised",
                    "channelised",
                    "Five fixed channels, USB, 2.8 kHz and 100 W ERP, shared with primary users. Best avoided by a new mode.",
                ],
                [
                    "40 m",
                    "7.000 - 7.125",
                    "7.025 - 7.125",
                    "Phone begins at 7.125. Busy with gateways near the top.",
                ],
                [
                    "30 m",
                    "10.100 - 10.150",
                    "10.100 - 10.150",
                    "Data only, no phone, <b>200 W PEP maximum</b>. Narrow modes by band plan.",
                ],
                [
                    "20 m",
                    "14.000 - 14.150",
                    "14.025 - 14.150",
                    "The most likely band for a first long contact.",
                ],
                ["17 m", "18.068 - 18.110", "18.068 - 18.110", "Narrow band, fills quickly."],
                ["15 m", "21.000 - 21.200", "21.025 - 21.200", ""],
                ["12 m", "24.890 - 24.930", "24.890 - 24.930", ""],
                ["10 m", "28.000 - 28.300", "28.000 - 28.300", "Technicians too, at 200 W."],
                [
                    "6 m",
                    "50.100 - 54.000",
                    "50.100 - 54.000",
                    "Technicians too, all of it. 50.0 - 50.1 is CW only.",
                ],
            ],
            [0.58 * inch, 1.12 * inch, 1.42 * inch, width - 3.12 * inch],
            s,
        ),
        p("All figures in MHz. Verify against the current rules before relying on them.", "note"),
    ]

    out += [
        p("2. What actually fits, and where the signal lands", "h1"),
        p(
            "Aether is tuned by an upper-sideband dial frequency, and the signal sits "
            "<b>above</b> that dial. This catches people out when they pick a clear-sounding "
            "spot and then transmit somewhere else."
        ),
        table(
            [
                ["Bandwidth", "Sits above the dial", "Clear space needed", "Use it for"],
                [
                    "2.3 kHz",
                    "240 Hz to 2760 Hz (measured)",
                    "roughly 2.8 kHz above the dial",
                    "Winlink and Pat traffic, the most throughput, attended contacts with room to spare.",
                ],
                [
                    "500 Hz",
                    "1140 Hz to 1850 Hz (measured)",
                    "roughly 0.7 kHz, centred 1.5 kHz above the dial",
                    "Peer-to-peer work, VarAC, crowded bands, weak paths.",
                ],
            ],
            [0.88 * inch, 1.45 * inch, 1.72 * inch, width - 4.05 * inch],
            s,
        ),
        p(
            "So a 500 Hz station on a 7.056 dial is actually occupying about 7.0571 to 7.0579 "
            "- a kilohertz and a half up from where the dial reads. Listen across that, not "
            "just at the dial.",
            "note",
        ),
    ]

    out += [PageBreak()]

    out += [
        p("3. Automatic control segments, 47 CFR 97.221(b)", "h1"),
        p(
            "The only places on HF an <b>unattended</b> Aether station transmits at all - "
            "and all of 6 m besides. An attended station is not restricted to these."
        ),
        table(
            [
                ["Band", "97.221(b) segment", "Room at 2.3 kHz", "Note"],
                ["80 m", "3.585 - 3.600", "yes, comfortably", "Region 2."],
                [
                    "40 m",
                    "7.100 - 7.105",
                    "dials 7.0998 - 7.1022",
                    "5 kHz wide and shared with Winlink, VARA and ARDOP gateways. Expect it to be busy.",
                ],
                [
                    "30 m",
                    "10.140 - 10.150",
                    "not by band plan",
                    "Band plan marks 30 m for narrow modes. Use 500 Hz here.",
                ],
                [
                    "20 m",
                    "14.0950 - 14.0995 and 14.1005 - 14.112",
                    "yes, above the gap",
                    "The gap protects the International Beacon Project on 14.100. Stay above it.",
                ],
                ["17 m", "18.105 - 18.110", "tight", "Only 5 kHz."],
                ["15 m", "21.090 - 21.100", "yes", ""],
                ["12 m", "24.925 - 24.930", "tight", "Only 5 kHz."],
                ["10 m", "28.120 - 28.189", "plenty", "Wide, when the band is open at all."],
                [
                    "6 m",
                    "all of it (data from 50.1)",
                    "plenty",
                    "97.221(b) opens the 6 m and shorter bands throughout.",
                ],
            ],
            [0.58 * inch, 1.72 * inch, 1.12 * inch, width - 3.42 * inch],
            s,
        ),
        p(
            "Segment figures follow the ARRL's summary of 97.221. Verify against the current "
            "rule text before leaving a station unattended on any of them.",
            "note",
        ),
    ]

    out += [
        p("4. Aether's suggested calling dials", "h1"),
        p(
            "<b>Proposals, not standards.</b> Nobody can assign a frequency to a mode by "
            "writing it down; these become real only if operators agree to use them. Each one "
            "is chosen so the whole signal sits inside the 97.221(b) segment above, which "
            "means a station may legally be left on it."
        ),
        table(
            [
                ["Band", "Dial (USB)", "Bandwidth", "Signal occupies", "Clear of"],
                ["80 m", "3.590", "2.3 kHz", "3.5902 - 3.5928", "FT8 at 3.573, JS8 at 3.578"],
                ["40 m", "7.101", "2.3 kHz", "7.1012 - 7.1038", "FT8 at 7.074, JS8 at 7.078"],
                ["30 m", "10.141", "500 Hz", "10.1421 - 10.1429", "see the caution below"],
                ["20 m", "14.107", "2.3 kHz", "14.1072 - 14.1098", "the beacon project on 14.100"],
                ["17 m", "18.107", "2.3 kHz", "18.1072 - 18.1098", "JS8 at 18.104"],
                ["15 m", "21.094", "2.3 kHz", "21.0942 - 21.0968", ""],
                ["12 m", "24.926", "2.3 kHz", "24.9262 - 24.9288", "JS8 at 24.922"],
                ["10 m", "28.126", "2.3 kHz", "28.1262 - 28.1288", "FT4 at 28.180"],
                ["6 m", "50.690", "2.3 kHz", "50.6902 - 50.6928", "packet calling at 50.620"],
            ],
            [0.52 * inch, 0.72 * inch, 0.72 * inch, 1.28 * inch, width - 3.24 * inch],
            s,
        ),
        p(
            "Caution on 30 m: PSK31 has long gathered around 10.142 and FT4 around 10.140, "
            "both inside the same segment. The 10.141 dial puts Aether's 500 Hz signal very "
            "close to the PSK31 window. Listen carefully, and move up within 10.140 - 10.150 "
            "if anyone is there.",
            "warn",
        ),
    ]

    out += [PageBreak()]

    out += [
        p("5. Stay clear of these", "h1"),
        p(
            "Dial frequencies where other modes gather. They are narrow, busy, and their users "
            "are the least able to work around a signal sitting on top of them. These shift "
            "over time, so treat the table as a starting point and listen."
        ),
        table(
            [
                ["Band", "FT8", "FT4", "JS8", "WSPR", "PSK31"],
                ["160 m", "1.840", "-", "1.842", "1.8366", "1.838"],
                ["80 m", "3.573", "3.575", "3.578", "3.5686", "3.580"],
                ["40 m", "7.074", "7.0475", "7.078", "7.0386", "7.070"],
                ["30 m", "10.136", "10.140", "10.130", "10.1387", "10.142"],
                ["20 m", "14.074", "14.080", "14.078", "14.0956", "14.070"],
                ["17 m", "18.100", "18.104", "18.104", "18.1046", "18.100"],
                ["15 m", "21.074", "21.140", "21.078", "21.0946", "21.070"],
                ["12 m", "24.915", "24.919", "24.922", "24.9246", "24.920"],
                ["10 m", "28.074", "28.180", "28.078", "28.1246", "28.120"],
            ],
            [0.62 * inch] + [(width - 0.62 * inch) / 5] * 5,
            s,
        ),
        p(
            "Also stay off the <b>Winlink RMS channel lists</b> and the published VARA and "
            "ARDOP calling frequencies. Those are busy, and a station parked on one is heard "
            "as interference by people with no way to decode what it is."
        ),
        p(
            "Two of these sit inside 97.221(b) segments: WSPR at 14.0956 is within 14.0950 - "
            "14.0995, and PSK31 at 28.120 is on the bottom edge of 28.120 - 28.189. Being "
            "legal and being welcome are not the same thing. On 6 m the weak-signal modes "
            "(FT8, FT4, WSPR, MSK144) gather around 50.26 - 50.33, well below the 50.690 dial.",
            "note",
        ),
    ]

    out += [
        p("6. Finding a quiet spot for an attended contact", "h1"),
        p(
            "When both operators are present, you have the whole data segment from table 1 and "
            "no obligation to sit in the gateway sub-bands. At 500 Hz you need almost no room, "
            "so there is usually somewhere clear. Sensible places to start looking, remembering "
            "that the signal lands about 1.5 kHz above the dial:"
        ),
        table(
            [
                ["Band", "Try around", "Why"],
                [
                    "40 m",
                    "7.052 - 7.065",
                    "Below the gateway sub-band, above FT4 at 7.0475, below PSK at 7.070.",
                ],
                [
                    "30 m",
                    "10.144 - 10.149",
                    "Above the PSK31 and FT4 windows, still inside the segment.",
                ],
                [
                    "20 m",
                    "14.105 - 14.112",
                    "Inside the upper 97.221(b) block, clear of the beacon gap.",
                ],
                ["80 m", "3.585 - 3.600", "The segment itself is usually quiet enough at night."],
            ],
            [0.62 * inch, 1.25 * inch, width - 1.87 * inch],
            s,
        ),
        p(
            "These are starting points, not assignments. <b>Listen first, across the whole "
            "window the signal will occupy.</b> Aether's busy detector will refuse to start a "
            "session on an occupied channel, but it cannot know that the frequency is "
            "somebody's net every Tuesday evening.",
            "warn",
        ),
    ]

    out += [
        p("7. Before you transmit", "h1"),
        p(
            "<b>Tell the modem the rules.</b> Setup, step 1: the rules, how the station is "
            "controlled, and your licence class. Until they are set nothing is transmitted; "
            "then the badge in the header says LEGAL, WARNING or TX BLOCKED for the dial the "
            "radio is on, and why."
        ),
        p(
            "<b>Set the bandwidth on both stations the same.</b> A call in the other bandwidth "
            "is simply not heard. Setup, step 4."
        ),
        p(
            "<b>Narrow the receiver, and mind the AGC.</b> On a crowded band a wide-open "
            "receive filter lets strong adjacent signals pump the radio's AGC, which disturbs "
            "the noise floor Aether's busy detector learns and leaves it reading busy. A filter "
            "of about 500 Hz centred on 1500 Hz audio, with AGC fast or off, fixes it."
        ),
        p(
            "<b>Listen across the occupied window</b>, not just at the dial frequency, and give "
            "way to whoever is already there."
        ),
    ]

    generated = dt.datetime.now(dt.UTC).strftime("%Y-%m-%d")
    out += [
        Spacer(1, 6),
        HRFlowable(width="100%", thickness=0.5, color=RULE, spaceAfter=6),
        p(
            f"Generated {generated} by tools/make_frequency_guide.py from the reasoning in "
            "docs/user/frequency-plan.md. Corrections belong in the project, not in a copy of "
            "this file: github.com/KK4ODA/aether-hf",
            "note",
        ),
    ]
    return [KeepTogether(f) if isinstance(f, Table) else f for f in out]


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("aether-frequency-guide-region2.pdf")
    s = styles()
    doc = SimpleDocTemplate(
        str(out),
        pagesize=letter,
        leftMargin=MARGIN,
        rightMargin=MARGIN,
        topMargin=MARGIN,
        bottomMargin=0.85 * inch,
        title="Aether HF - Where to Operate (Region 2)",
        author="Aether HF",
        subject="Frequency guidance for Region 2 and the United States",
    )
    doc.build(story(s), onFirstPage=furniture, onLaterPages=furniture)
    print(f"wrote {out} ({out.stat().st_size // 1024} kB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
