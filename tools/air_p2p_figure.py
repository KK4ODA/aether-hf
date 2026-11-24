"""The README's peer-to-peer figure, drawn from both stations' recordings of 2026-11-24: Winlink
Express at the portable (WD5EMA, Aether at KK4ODA-2) calls Winlink Express at home (KK4ODA,
Aether at KK4ODA-1) in a *Vara HF P2P* session at 500 Hz on 40 m, 18:42:54Z.

    uv run --with matplotlib --with pillow python -I tools/air_p2p_figure.py PORTABLE HOME docs/images

PORTABLE and HOME are the two recordings' folders as the stations' debug uploads unpack them
(``…/recordings``), each holding its WAV and sidecar. Draws ``air-40m-p2p.png``: the whole
contact on one clock, from the call to the last identifier, as a sequence of both stations'
transmissions over each station's receive audio as a waterfall.

The two recordings go on one clock with ``tools/two_sides.py --from-loops`` (the offset from the
loop latencies of the eight changes of hands: the home station's recording runs
:data:`OFFSET_S` behind the portable's). Drawn in the panel's dark tokens with
``tools/air_figures.py``'s helpers. A recording is untrusted data: nothing in it is executed.
"""

from __future__ import annotations

import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.colors import LinearSegmentedColormap

sys.path.insert(0, str(Path(__file__).resolve().parent))

from air_figures import (
    AETHER,
    BG,
    BORDER,
    BUSY,
    LINK,
    RX,
    TEXT,
    TEXT2,
    TEXT3,
    TRACE,
    TX,
    WELL,
    keyed_spans,
    read_wav,
    save_quantised,
    sidecar,
    spectrogram,
    style,
)

PORTABLE = "20261124-184304_WD5EMA_KK4ODA"
HOME = "20261124-184255_KK4ODA_WD5EMA"
OFFSET_S = -9.4207
"""The portable's recording clock less the home station's, from ``two_sides.py --from-loops``."""
CALL_S = 19.139
"""Where the portable keyed the call on its recording's clock (its log: 18:42:53.966Z; the
recording, with its 30 s preroll, starts at 18:42:34.827Z)."""
CALL_END_S = 25.821
"""Where it released the call (its log: 18:43:00.648Z). The call is in the preroll, before the
sidecar's events begin, so its keying is taken from the log."""
SPAN = (-1.0, 58.6)
"""The window drawn, seconds from the call."""

# What each transmission carried, read from the two sidecars and the two logs, by where it was
# keyed (seconds from the call, rounded) and which station keyed it.
WHAT = {
    ("portable", 0): "call",
    ("home", 7): "accept",
    ("portable", 11): "header",
    ("home", 15): "greeting",
    ("portable", 17): "header + SID, FF",
    ("home", 32): "answer",
    ("portable", 33): "header",
    ("home", 37): "FQ",
    ("portable", 39): "header",
    ("home", 43): "answer",
    ("portable", 44): "DISC",
    ("home", 48): "DISC_ACK + Morse ID",
    ("portable", 54): "data ID",
}


def spans(d: dict, shift: float, who: str = "") -> list[tuple[float, float]]:
    """A station's keyings, seconds from the call."""
    keys = list(keyed_spans(d))
    if who == "portable":
        keys.insert(0, (CALL_S, CALL_END_S))
    return [(a + shift - CALL_S, b + shift - CALL_S) for a, b in keys]


def lane(ax: plt.Axes, d: dict, shift: float, who: str) -> None:
    """Frames the station heard (green decoded, amber not) and its own keying (red), as the
    panel's channel chart draws them."""
    ax.set_facecolor(BG)
    ax.set_ylim(0, 1)
    ax.set_yticks([])
    for s in ax.spines.values():
        s.set_visible(False)
    ax.tick_params(axis="x", length=0, labelbottom=False)
    for f in d["frames"]:
        s0, s1 = f.get("start_s"), f.get("end_s")
        if s0 is None:
            continue
        s0, s1 = s0 + shift - CALL_S, s1 + shift - CALL_S
        colour = RX if f.get("decoded") else BUSY
        h = 0.55 if f["kind"] == "data" else 0.3
        ax.add_patch(plt.Rectangle((s0 + 0.02, 0.1), s1 - s0 - 0.04, h, color=colour, lw=0))
    for k0, k1 in spans(d, shift, who):
        ax.add_patch(plt.Rectangle((k0, 0.1), k1 - k0, 0.55, color=TX, lw=0))


def figure(portable: Path, home: Path, out: Path) -> None:
    cmap = LinearSegmentedColormap.from_list("aether", np.array(AETHER) / 255)
    dp, dh = sidecar(portable, PORTABLE), sidecar(home, HOME)
    stations = [
        ("portable", dp, portable / f"{PORTABLE}.wav", 0.0, "WD5EMA (portable)", LINK),
        ("home", dh, home / f"{HOME}.wav", OFFSET_S, "KK4ODA (home)", TRACE),
    ]

    fig = plt.figure(figsize=(12.8, 8.9), dpi=125)
    gs = fig.add_gridspec(
        5,
        1,
        height_ratios=[0.7, 0.14, 1, 0.14, 1],
        hspace=0.1,
        top=0.84,
        bottom=0.07,
        left=0.075,
        right=0.985,
    )
    fig.text(
        0.075,
        0.955,
        "A Winlink Express P2P contact on 40 m, from both ends",
        fontsize=15,
        color=TEXT,
        fontweight="medium",
    )
    fig.text(
        0.075,
        0.918,
        "2026-11-24 18:42:54Z, 500 Hz. WD5EMA (TX-500, 10 W, EFHW) calls KK4ODA (FTDX10, 50 W, "
        "T2D); each Winlink Express keys its own radio on Aether's PTT ON.",
        fontsize=10.5,
        color=TEXT2,
    )
    fig.text(
        0.075,
        0.893,
        "Both recordings on one clock. KK4ODA heard WD5EMA at −5 to +1 dB SNR in 3 kHz, "
        "WD5EMA heard KK4ODA at +5 to +9 dB. No transmission overlapped the other.",
        fontsize=10.5,
        color=TEXT2,
    )
    fig.text(
        0.075,
        0.868,
        "Over each waterfall: the frames that station decoded (green) and its own keying (red).",
        fontsize=10.5,
        color=TEXT3,
    )

    # The sequence: who keyed when, and what it carried.
    seq = fig.add_subplot(gs[0])
    seq.set_facecolor(BG)
    seq.set_ylim(-0.45, 2)
    seq.set_yticks([1.45, 0.45])
    seq.set_yticklabels(["WD5EMA", "KK4ODA"])
    seq.tick_params(axis="y", length=0)
    for s in seq.spines.values():
        s.set_visible(False)
    seq.tick_params(axis="x", length=0, labelbottom=False)
    for i, (who, d, _, shift, _, colour) in enumerate(stations):
        y = 1.1 if i == 0 else 0.1
        for k0, k1 in spans(d, shift, who):
            if k1 < SPAN[0] or k0 > SPAN[1]:
                continue
            seq.add_patch(plt.Rectangle((k0, y), k1 - k0, 0.7, color=colour, lw=0))
            label = WHAT.get((who, round(k0)))
            if not label:
                continue
            if i == 0:
                # the portable's transmissions are all long enough to carry their own label
                seq.text(k0 + 0.2, y + 0.35, label, color=WELL, fontsize=9.2, va="center")
            else:
                seq.text(
                    (k0 + k1) / 2,
                    y - 0.08,
                    label,
                    color=TEXT2,
                    fontsize=9.2,
                    va="top",
                    ha="center",
                )

    for i, (_, d, wav, shift, title, _) in enumerate(stations):
        ln = fig.add_subplot(gs[1 + 2 * i], sharex=seq)
        ax = fig.add_subplot(gs[2 + 2 * i], sharex=seq)
        x, fs = read_wav(wav)
        a = int((SPAN[0] + CALL_S - shift) * fs)
        b = int((SPAN[1] + CALL_S - shift) * fs)
        a0 = max(a, 0)
        _, hz, db = spectrogram(x[a0:b], fs, 2700)
        keep = hz >= 300
        hz, db = hz[keep], db[:, keep]
        floor = np.median(db)
        t0 = SPAN[0] + (a0 - a) / fs
        ax.imshow(
            db.T,
            origin="lower",
            aspect="auto",
            cmap=cmap,
            vmin=floor - 4,
            vmax=floor + 30,
            extent=(t0, t0 + (b - a0) / fs, hz[0], hz[-1]),
            interpolation="bilinear",
        )
        ax.set_ylabel("audio Hz")
        ax.set_yticks([500, 1000, 1500, 2000, 2500])
        style(ax)
        ax.text(
            0.006,
            0.955,
            f"What {title} heard",
            transform=ax.transAxes,
            fontsize=10.5,
            color=TEXT,
            va="top",
            bbox={"boxstyle": "round,pad=0.35", "fc": WELL, "ec": BORDER, "alpha": 0.85},
        )
        lane(ln, d, shift, stations[i][0])
        if i == 0:
            ax.tick_params(axis="x", labelbottom=False)
    ax.set_xlabel("seconds from the call")
    seq.set_xlim(*SPAN)
    save_quantised(fig, out / "air-40m-p2p.png")
    plt.close(fig)


def main() -> None:
    portable, home, out = (Path(a) for a in sys.argv[1:4])
    for folder, name in ((portable, PORTABLE), (home, HOME)):
        if not (folder / f"{name}.json").exists():
            raise SystemExit(f"{folder} does not hold {name}")
    out.mkdir(parents=True, exist_ok=True)
    figure(portable, home, out)


if __name__ == "__main__":
    main()
