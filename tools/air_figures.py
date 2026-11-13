"""The README's *On the air* figures, drawn from KK4ODA-1's recordings of 2026-11-12: RadioPost
mail from the mobile (KK4ODA) to the gateway (KK4ODA-1) on 3.590 MHz, the same mail at 500 Hz
(14:13Z) and at 2300 Hz (14:26Z), both sessions in ``field/LOG.md``.

    uv run --with matplotlib python -I tools/air_figures.py RECORDINGS_DIR docs/images [WHICH ...]

RECORDINGS_DIR holds the gateway's two recordings (the WAV and its sidecar each), as its upload
of 2026-11-12 23:54Z unpacks them (``tools/fetch_uploads.py get … --extract``). WHICH is any of
``waterfall``, ``delivery`` and ``constellation`` (all three by default):

* ``air-80m-waterfall.png``: the gateway's receive audio over three exchanges of each session,
  with the frames it heard and its own keying from the sidecar;
* ``air-80m-delivery.png``: the data the gateway decoded against time, new blocks only, and the
  SNR it read on each data frame;
* ``air-80m-constellation.png``: the six rung-11 frames of one of the mobile's transmissions in
  each session — the one whose median SNR is nearest the session's — decoded again from the
  recording by the reference model's receiver.

Drawn in the panel's dark tokens (``app/ui/style.css``) and its own waterfall palette. A
recording is untrusted data: nothing in it is executed.
"""

from __future__ import annotations

import json
import statistics
import sys
import wave
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from matplotlib import font_manager
from matplotlib.colors import LinearSegmentedColormap

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import Modem
from aether_model.waveform import NARROW_500, WIDE_2300

S500 = "20261112-141346_KK4ODA-1_KK4ODA"
S2300 = "20261112-142606_KK4ODA-1_KK4ODA"
CONNECT_S = 30.0
"""Where the session came up in each recording (its 30 s preroll)."""

# The panel's dark tokens (app/ui/style.css).
BG = "#16181b"
WELL = "#101214"
BORDER = "#2d3136"
GRID = "#22262a"
TEXT = "#dde0e3"
TEXT2 = "#969da4"
TEXT3 = "#666d74"
TRACE = "#c9d2da"
LINK = "#6aa9e9"
TX = "#e5544b"
RX = "#73cf8a"
BUSY = "#d9a441"

AETHER = [[2, 5, 18], [4, 22, 78], [14, 78, 186], [44, 160, 252], [176, 232, 255], [255, 255, 236]]
"""The panel's own waterfall palette (app/ui/scopes.js)."""

for name in ("Inter", "Inter Display"):
    if any(name == f.name for f in font_manager.fontManager.ttflist):
        plt.rcParams["font.family"] = name
        break
plt.rcParams.update(
    {
        "figure.facecolor": BG,
        "axes.facecolor": WELL,
        "axes.edgecolor": BORDER,
        "axes.labelcolor": TEXT2,
        "axes.titlecolor": TEXT,
        "xtick.color": TEXT3,
        "ytick.color": TEXT3,
        "xtick.labelcolor": TEXT2,
        "ytick.labelcolor": TEXT2,
        "grid.color": GRID,
        "text.color": TEXT,
        "font.size": 11,
        "axes.titlesize": 12.5,
        "axes.titleweight": "medium",
        "axes.titlelocation": "left",
        "axes.titlepad": 10,
        "legend.frameon": False,
        "legend.labelcolor": TEXT2,
        "savefig.facecolor": BG,
    }
)


def read_wav(path: Path) -> tuple[np.ndarray, int]:
    with wave.open(str(path)) as w:
        raw = w.readframes(w.getnframes())
        fs = w.getframerate()
        ch = w.getnchannels()
    x = np.frombuffer(raw, dtype=np.int16).astype(np.float64) / 32768
    return (x.reshape(-1, ch)[:, 0] if ch > 1 else x), fs


def sidecar(rec: Path, name: str) -> dict:
    return json.loads((rec / f"{name}.json").read_text(encoding="utf-8"))


def keyed_spans(d: dict) -> list[tuple[float, float]]:
    out, on = [], None
    for e in d["events"]:
        if e["event"] != "ptt":
            continue
        if e.get("detail") == "keyed":
            on = e["t_s"]
        elif e.get("detail") == "released" and on is not None:
            out.append((on, e["t_s"]))
            on = None
    return out


def save_quantised(fig: plt.Figure, path: Path) -> None:
    """A PNG with an adaptive 256-colour palette: the noise of a waterfall does not compress."""
    import io

    from PIL import Image

    buf = io.BytesIO()
    fig.savefig(buf, format="png")
    buf.seek(0)
    Image.open(buf).convert("RGB").quantize(
        colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE
    ).save(path, optimize=True)


def style(ax: plt.Axes) -> None:
    for s in ("top", "right"):
        ax.spines[s].set_visible(False)
    for s in ("left", "bottom"):
        ax.spines[s].set_color(BORDER)
    ax.tick_params(length=3, width=0.8)


# ---------------------------------------------------------------------------------------------
# The waterfall


def spectrogram(x: np.ndarray, fs: int, top_hz: float) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    n, hop = 4096, 480
    win = np.hanning(n)
    frames = np.lib.stride_tricks.sliding_window_view(x, n)[::hop] * win
    spec = np.abs(np.fft.rfft(frames, axis=1)) ** 2
    hz = np.fft.rfftfreq(n, 1 / fs)
    keep = hz <= top_hz
    t = (np.arange(len(frames)) * hop + n / 2) / fs
    return t, hz[keep], 10 * np.log10(spec[:, keep] + 1e-20)


def waterfall(rec: Path, out: Path) -> None:
    cmap = LinearSegmentedColormap.from_list("aether", np.array(AETHER) / 255)
    rows = [
        (S500, 301.45, "500 Hz · rung 11, 16-QAM ¾ · 78 bytes a frame"),
        (S2300, 50.0, "2300 Hz · rung 11, QPSK ⅔ · 190 bytes a frame"),
    ]
    span = 25.75
    fig = plt.figure(figsize=(12.8, 7.6), dpi=125)
    gs = fig.add_gridspec(
        4,
        1,
        height_ratios=[0.16, 1, 0.16, 1],
        hspace=0.08,
        top=0.86,
        bottom=0.08,
        left=0.065,
        right=0.985,
    )
    fig.text(
        0.065,
        0.955,
        "What the gateway heard on 3.590 MHz",
        fontsize=15,
        color=TEXT,
        fontweight="medium",
    )
    fig.text(
        0.065,
        0.918,
        "KK4ODA-1's receive audio during the same RadioPost mail from the mobile, sent twice: "
        "three exchanges of each. Each is the mobile's header and six data frames,",
        fontsize=10.5,
        color=TEXT2,
    )
    fig.text(
        0.065,
        0.893,
        "then the gateway's 0.7 s answer (dark: its own receiver is muted while it transmits). "
        "Signal about +5 dB SNR in 3 kHz on both.",
        fontsize=10.5,
        color=TEXT2,
    )
    for i, (name, t0, label) in enumerate(rows):
        d = sidecar(rec, name)
        x, fs = read_wav(rec / f"{name}.wav")
        a, b = int(t0 * fs), int((t0 + span) * fs)
        _, hz, db = spectrogram(x[a:b], fs, 3200)
        floor = np.median(db)
        lane = fig.add_subplot(gs[2 * i])
        ax = fig.add_subplot(gs[2 * i + 1], sharex=lane)
        ax.imshow(
            db.T,
            origin="lower",
            aspect="auto",
            cmap=cmap,
            vmin=floor - 4,
            vmax=floor + 30,
            extent=(0, span, hz[0], hz[-1]),
            interpolation="bilinear",
        )
        ax.set_ylabel("audio Hz")
        ax.set_yticks([0, 1000, 2000, 3000])
        style(ax)
        ax.text(
            0.006,
            0.955,
            label,
            transform=ax.transAxes,
            fontsize=10.5,
            color=TEXT,
            va="top",
            bbox={"boxstyle": "round,pad=0.35", "fc": WELL, "ec": BORDER, "alpha": 0.85},
        )
        # The lane above: frames heard (green) and the gateway's own keying (red), as the
        # panel's channel chart draws them.
        lane.set_facecolor(BG)
        lane.set_ylim(0, 1)
        lane.set_yticks([])
        for s in lane.spines.values():
            s.set_visible(False)
        lane.tick_params(axis="x", length=0, labelbottom=False)
        for f in d["frames"]:
            s0, s1 = f.get("start_s"), f.get("end_s")
            if s0 is None or s1 < t0 or s0 > t0 + span:
                continue
            colour = RX if f.get("decoded") else BUSY
            h = 0.55 if f["kind"] == "data" else 0.3
            lane.add_patch(
                plt.Rectangle((s0 - t0 + 0.02, 0.1), s1 - s0 - 0.04, h, color=colour, lw=0)
            )
        for k0, k1 in keyed_spans(d):
            if k1 < t0 or k0 > t0 + span:
                continue
            lane.add_patch(plt.Rectangle((k0 - t0, 0.1), k1 - k0, 0.55, color=TX, lw=0))
        if i == 0:
            ax.tick_params(axis="x", labelbottom=False)
            first = [
                f
                for f in d["frames"]
                if f.get("start_s") is not None and t0 < f["start_s"] < t0 + 8.6
            ]
            k0, k1 = next(k for k in keyed_spans(d) if k[1] > t0)
            lane.text(
                (k0 + k1) / 2 - t0,
                0.78,
                "gateway's answer",
                color=TX,
                fontsize=9.5,
                ha="center",
                va="bottom",
            )
            h0 = min(f["start_s"] for f in first) - t0
            h1 = max(f["end_s"] for f in first) - t0
            lane.text(
                (h0 + h1) / 2,
                0.78,
                "mobile: header, then six data frames",
                color=RX,
                fontsize=9.5,
                ha="center",
                va="bottom",
            )
    ax.set_xlabel("seconds")
    ax.set_xlim(0, span)
    save_quantised(fig, out / "air-80m-waterfall.png")
    plt.close(fig)


# ---------------------------------------------------------------------------------------------
# Delivery against time


def deliveries(d: dict) -> tuple[list[float], list[float], list[tuple[float, float, int]]]:
    seen: dict[int, float] = {}
    t, kb, snr = [0.0], [0.0], []
    total = 0
    for f in sorted(d["frames"], key=lambda f: f["t_s"]):
        if f["kind"] != "data" or f.get("seq") is None:
            continue
        if f.get("decoded") and f.get("snr_3k_db") is not None:
            snr.append((f["start_s"] - CONNECT_S, f["snr_3k_db"], f["mode"]))
        if not f.get("decoded"):
            continue
        if f["seq"] in seen and f["start_s"] - seen[f["seq"]] < 120:
            continue
        seen[f["seq"]] = f["start_s"]
        total += f["bytes"] - 3
        t.append(f["end_s"] - CONNECT_S)
        kb.append(total / 1000)
    return t, kb, snr


def delivery(rec: Path, out: Path) -> None:
    fig, (ax, bx) = plt.subplots(
        2,
        1,
        figsize=(8.4, 6.2),
        dpi=150,
        sharex=True,
        gridspec_kw={"height_ratios": [2.3, 1], "hspace": 0.22},
    )
    fig.subplots_adjust(top=0.84, left=0.1, right=0.97, bottom=0.1)
    fig.text(
        0.1,
        0.945,
        "The same mail, at 500 Hz and at 2300 Hz",
        fontsize=14.5,
        color=TEXT,
        fontweight="medium",
    )
    fig.text(
        0.1,
        0.9,
        "Data the gateway decoded, by seconds since the call connected. "
        "Same stations, same path, minutes apart.",
        fontsize=10.2,
        color=TEXT2,
    )
    series = [(S500, LINK, "500 Hz"), (S2300, TRACE, "2300 Hz")]
    for name, colour, label in series:
        d = sidecar(rec, name)
        t, kb, snr = deliveries(d)
        rungs = sorted({m for _, _, m in snr})
        ax.step(t, kb, where="post", color=colour, lw=2.2, label=label)
        ax.plot([t[-1]], [kb[-1]], "o", color=colour, ms=6)
        ax.annotate(
            f"{label} · last block at {t[-1]:.0f} s",
            (t[-1], kb[-1]),
            xytext=(8, 4) if label == "2300 Hz" else (-4, 12),
            textcoords="offset points",
            color=colour,
            fontsize=10.5,
            ha="left" if label == "2300 Hz" else "right",
        )
        st, sv = [s[0] for s in snr], [s[1] for s in snr]
        bx.plot(st, sv, ".", color=colour, ms=4.5, alpha=0.85, mec="none")
        med = statistics.median(sv)
        print(f"{label}: {kb[-1]:.2f} kB in {t[-1]:.1f} s, rungs {rungs}, SNR median {med:.1f}")
    ax.set_ylabel("kB decoded at the gateway")
    ax.set_ylim(0, 25)
    ax.grid(True, lw=0.8)
    style(ax)
    bx.set_ylabel("SNR, dB")
    bx.set_ylim(-2, 12)
    bx.set_yticks([0, 5, 10])
    bx.grid(True, lw=0.8)
    bx.set_xlabel("seconds since connected")
    bx.set_xlim(0, 480)
    bx.set_title(
        "SNR of each data frame as the gateway read it, in 3 kHz", fontsize=10, color=TEXT2, pad=6
    )
    style(bx)
    ax.text(
        0.995,
        0.05,
        "500 Hz: rungs 9–11 (16-QAM)\n2300 Hz: rung 11 (QPSK ⅔)",
        transform=ax.transAxes,
        ha="right",
        va="bottom",
        fontsize=9.5,
        color=TEXT3,
        linespacing=1.5,
    )
    fig.savefig(out / "air-80m-delivery.png")
    plt.close(fig)


# ---------------------------------------------------------------------------------------------
# Constellations


def transmission_frames(d: dict, rung: int) -> list[list[dict]]:
    """The rung's decoded data frames grouped by transmission (frames under 0.2 s apart)."""
    frames = sorted(
        (
            f
            for f in d["frames"]
            if f["kind"] == "data" and f.get("mode") == rung and f.get("decoded")
        ),
        key=lambda f: f["start_s"],
    )
    groups: list[list[dict]] = []
    for f in frames:
        if groups and f["start_s"] - groups[-1][-1]["end_s"] < 0.2:
            groups[-1].append(f)
        else:
            groups.append([f])
    return [g for g in groups if len(g) == 6]


def decode_symbols(rec: Path, name: str, rung: int) -> tuple[np.ndarray, str, float, float, float]:
    d = sidecar(rec, name)
    params = WIDE_2300 if d["session"]["bandwidth_hz"] == 2300 else NARROW_500
    groups = transmission_frames(d, rung)
    session_median = statistics.median(f["snr_3k_db"] for g in groups for f in g)
    # The transmission whose own median reading is nearest the session's: a typical one.
    group = min(
        groups, key=lambda g: abs(statistics.median(f["snr_3k_db"] for f in g) - session_median)
    )
    x, _ = read_wav(rec / f"{name}.wav")
    bb = AudioToBaseband(params).process(x)
    fs = params.fs_baseband
    modem = Modem(params)
    symbols, carrier, mode = [], [], ""
    for f in group:
        a = max(0, int((f["start_s"] - 0.3) * fs))
        b = min(len(bb), int((f["end_s"] + 0.3) * fs))
        for dec in modem.decode_buffer(bb[a:b], max_frames=2):
            if dec.ok and dec.frame is not None and dec.mode is not None:
                symbols.append(dec.frame.symbols)
                carrier.append(dec.frame.snr_carrier_db)
                mode = dec.mode.name
                break
    readings = [f["snr_3k_db"] for f in group]
    print(
        f"{name}: {mode}, {len(symbols)} frames at {group[0]['start_s']:.1f} s, "
        f"SNR {min(readings):.1f}–{max(readings):.1f} dB (session median {session_median:.1f})"
    )
    return (
        np.concatenate(symbols),
        mode,
        statistics.median(readings),
        statistics.median(carrier),
        group[0]["start_s"] - CONNECT_S,
    )


def ideal(mode: str) -> np.ndarray:
    if mode.startswith("QAM16"):
        lv = np.array([-3, -1, 1, 3]) / np.sqrt(10)
        return (lv[:, None] + 1j * lv[None, :]).ravel()
    if mode.startswith("QPSK"):
        return np.array([1 + 1j, 1 - 1j, -1 + 1j, -1 - 1j]) / np.sqrt(2)
    return np.array([])


def constellation(rec: Path, out: Path) -> None:
    fig, axes = plt.subplots(1, 2, figsize=(8.4, 5.0), dpi=150)
    fig.subplots_adjust(top=0.8, left=0.04, right=0.96, bottom=0.1, wspace=0.12)
    fig.text(
        0.04,
        0.935,
        "Frames off the air, decoded again",
        fontsize=14.5,
        color=TEXT,
        fontweight="medium",
    )
    fig.text(
        0.04,
        0.885,
        "Six frames of one of the mobile's transmissions, from the gateway's "
        "recording, through the reference model's receiver.",
        fontsize=10.2,
        color=TEXT2,
    )
    for ax, (name, title) in zip(axes, [(S500, "500 Hz"), (S2300, "2300 Hz")], strict=True):
        z, mode, snr3k, carrier, _ = decode_symbols(rec, name, 11)
        nice = mode.replace("QAM16", "16-QAM").replace("-3/4", " ¾").replace("-2/3", " ⅔")
        ax.plot(
            z.real,
            z.imag,
            "o",
            color=TEXT,
            ms=2.4 if len(z) > 3000 else 3.2,
            alpha=0.45 if len(z) > 3000 else 0.7,
            mec="none",
        )
        p = ideal(mode)
        ax.plot(p.real, p.imag, "+", color=LINK, ms=10, mew=1.4)
        lim = 1.45
        ax.set_xlim(-lim, lim)
        ax.set_ylim(-lim, lim)
        ax.set_aspect("equal")
        ax.axhline(0, color=GRID, lw=0.8, zorder=0)
        ax.axvline(0, color=GRID, lw=0.8, zorder=0)
        ax.set_xticks([])
        ax.set_yticks([])
        for s in ax.spines.values():
            s.set_color(BORDER)
        ax.set_title(f"{title} · {nice}", fontsize=12)
        ax.set_xlabel(
            f"{len(z):,} symbols · {snr3k:+.1f} dB in 3 kHz · {carrier:.1f} dB a carrier",
            fontsize=9.8,
            color=TEXT2,
            labelpad=8,
        )
    fig.savefig(out / "air-80m-constellation.png")
    plt.close(fig)


def main() -> None:
    rec, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    if not (rec / f"{S500}.json").exists() or not (rec / f"{S2300}.json").exists():
        raise SystemExit(f"{rec} does not hold {S500} and {S2300}")
    which = sys.argv[3:] or ["waterfall", "delivery", "constellation"]
    if "waterfall" in which:
        waterfall(rec, out)
    if "delivery" in which:
        delivery(rec, out)
    if "constellation" in which:
        constellation(rec, out)


if __name__ == "__main__":
    main()
