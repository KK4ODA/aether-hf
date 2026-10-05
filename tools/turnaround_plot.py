"""What the receiver hears as the radio comes back from transmitting: every key release in a
recording lined up at t = 0.

A modem that answers fast meets the radio at its worst moment: the transceiver switching back
to receive, its AGC recovering from its own RF, the USB codec unmuting — or, at the other end,
a VOX-keyed station still on the air (ADR-0036). Whether any of that reaches the audio the
modem decodes, how long it lasts, and what the station made of it is what this tool shows.
For each release it measures the received audio in the passband (300–2 700 Hz by default) in
short steps from just before to a few seconds after, against the same recording's level well
after the release, and prints and plots the lot with t = 0 at the release.

Where the releases come from:

* an Aether recording's sidecar (`ptt` `released` events; the `.json` beside the WAV is read
  by default). A sidecar from beta.76 on also has `rx_trace` lines — the busy detector's
  level, floor, busy and deafness after each release — `preamble` announcements and frames'
  `start_s`, which are drawn too;
* or ``--detect``: the end of every stretch of digital silence, which is what a transceiver's
  USB codec delivers while it transmits. That reads a recording of any program's session —
  VARA's, made with a recorder on the radio's USB audio — so the two can be compared on one
  plot.

    python tools/turnaround_plot.py recordings/20261005-010244_KK4ODA-1_WC4Y.wav
    python tools/turnaround_plot.py vara.wav --detect --png vara.png
    python tools/turnaround_plot.py aether.wav vara.wav --detect-for vara.wav --png both.png

A rise above the reference just after t = 0 that fades within a second is the receiver
recovering; a flat line from the first non-silent step on is a radio that comes back clean.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

from tx_envelope import read_wav

SILENCE_DBFS = -70.0
"""Below this a step is the codec's silence: no receiver delivers noise that quiet."""
MIN_SILENCE_S = 0.2
"""``--detect``: a silent stretch at least this long is a transmission."""
REFERENCE_S = (1.5, 3.0)
"""Where, after a release, the recording's settled level is taken from."""


@dataclass
class Release:
    """One key release and what the recording heard around it."""

    t_s: float
    levels: np.ndarray
    """Passband level per step, dBFS, from ``-before`` to ``after``."""
    first_sound_s: float | None
    """The first step after t = 0 that is not the codec's silence."""
    trace: list[dict[str, float]] = field(default_factory=list)
    frames: list[tuple[float, bool]] = field(default_factory=list)
    """Frames starting in the window: start relative to the release, decoded."""
    preambles: list[float] = field(default_factory=list)


def band_levels(x: np.ndarray, rate: int, step_s: float, band: tuple[float, float]) -> np.ndarray:
    """Passband power per step, in dBFS (a full-scale sine reads −3)."""
    n = round(step_s * rate)
    blocks = len(x) // n
    frames = x[: blocks * n].reshape(blocks, n) * np.hanning(n)
    spectrum = np.abs(np.fft.rfft(frames, axis=1)) ** 2
    freqs = np.fft.rfftfreq(n, 1.0 / rate)
    inside = (freqs >= band[0]) & (freqs <= band[1])
    # Parseval with the window's power: the mean square of the band-limited block
    power = 2.0 * spectrum[:, inside].sum(axis=1) / (n * np.sum(np.hanning(n) ** 2))
    return np.asarray(10.0 * np.log10(np.maximum(power, 1e-12)))


def detect_releases(levels: np.ndarray, step_s: float) -> list[float]:
    """The ends of the codec's silent stretches long enough to be transmissions."""
    silent = levels < SILENCE_DBFS
    releases = []
    run = 0
    for i, quiet in enumerate(silent):
        if quiet:
            run += 1
            continue
        if run * step_s >= MIN_SILENCE_S:
            releases.append(i * step_s)
        run = 0
    return releases


def number(text: str, key: str) -> float | None:
    found = re.search(rf"\b{key} (-?[0-9.]+|-?inf)", text)
    if not found:
        return None
    return float(found.group(1))


def releases_of(
    wav: Path, detect: bool, step_s: float, before: float, after: float, band: tuple[float, float]
) -> tuple[list[Release], float]:
    x, rate = read_wav(wav)
    levels = band_levels(x, rate, step_s, band)
    sidecar_path = wav.with_suffix(".json")
    sidecar = None
    if sidecar_path.exists():
        sidecar = json.loads(sidecar_path.read_text(encoding="utf-8"))
    if detect or sidecar is None:
        times = detect_releases(levels, step_s)
    else:
        times = [
            float(e["t_s"])
            for e in sidecar.get("events", [])
            if e.get("event") == "ptt" and e.get("detail") == "released"
        ]
    lo, hi = round(before / step_s), round(after / step_s)
    out = []
    for t in times:
        at = round(t / step_s)
        if at - lo < 0 or at + hi > len(levels):
            continue
        window = levels[at - lo : at + hi]
        sound = np.nonzero(window[lo:] >= SILENCE_DBFS)[0]
        release = Release(t, window, float(sound[0] * step_s) if len(sound) else None)
        if sidecar is not None and not detect:
            for e in sidecar.get("events", []):
                rel = float(e["t_s"]) - t
                if e.get("event") == "rx_trace" and 0.0 <= rel <= after:
                    line = {"t": rel}
                    for key in ("power", "level", "floor", "busy", "deaf"):
                        value = number(str(e.get("detail", "")), key)
                        if value is not None:
                            line[key] = value
                    release.trace.append(line)
                elif e.get("event") == "preamble" and -before <= rel <= after:
                    ago = number(str(e.get("detail", "")), "ago") or 0.0
                    release.preambles.append(rel - ago)
            for f in sidecar.get("frames", []):
                start = f.get("start_s")
                if start is not None and -before <= float(start) - t <= after:
                    release.frames.append((float(start) - t, bool(f.get("decoded"))))
        out.append(release)
    return out, before


def summarise(name: str, releases: list[Release], step_s: float, before: float) -> np.ndarray:
    """Print the release-by-release numbers and the median curve; return the curve, relative
    to the reference level."""
    lo = round(before / step_s)
    ref_lo, ref_hi = (round(s / step_s) + lo for s in REFERENCE_S)
    rel = []
    print(f"== {name}: {len(releases)} releases")
    for r in releases:
        reference = float(np.median(r.levels[ref_lo:ref_hi]))
        curve = r.levels - reference
        rel.append(curve)
        sound = "never" if r.first_sound_s is None else f"{r.first_sound_s * 1000:.0f} ms"
        early = curve[lo : lo + round(0.3 / step_s)]
        audible = early[r.levels[lo : lo + len(early)] >= SILENCE_DBFS]
        rise = f"{float(np.max(audible)):+.1f} dB" if len(audible) else "-"
        frames = " ".join(f"{s:+.2f}{'*' if d else ''}" for s, d in r.frames)
        print(
            f"  t={r.t_s:8.2f}  sound after {sound:>6}  reference {reference:6.1f} dBFS"
            f"  most in 0-0.3 s {rise:>8}" + (f"  frames {frames}" if frames else "")
        )
    curves = np.array(rel)
    median = np.median(curves, axis=0)
    shown = median[lo : lo + round(1.0 / step_s)]
    print(f"  median, dB against the settled level, {step_s * 1000:.0f} ms steps from t = 0:")
    print("   " + " ".join(f"{v:+.1f}" for v in shown))
    return np.asarray(median)


def plot(
    curves: list[tuple[str, list[Release], np.ndarray]],
    step_s: float,
    before: float,
    png: Path,
) -> None:
    try:
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        print("matplotlib is not installed; no plot (uv run --with matplotlib …)", file=sys.stderr)
        return
    traced = any(r.trace for _, releases, _ in curves for r in releases)
    fig, axes = plt.subplots(
        2 if traced else 1, 1, figsize=(11, 7 if traced else 4.5), sharex=True, squeeze=False
    )
    ax = axes[0][0]
    for name, releases, median in curves:
        t = np.arange(len(median)) * step_s - before
        lo = round(before / step_s)
        ref_lo, ref_hi = (round(s / step_s) + lo for s in REFERENCE_S)
        for r in releases:
            ref = float(np.median(r.levels[ref_lo:ref_hi]))
            ax.plot(t, r.levels - ref, color="0.8", linewidth=0.5)
        (line,) = ax.plot(t, median, linewidth=2, label=f"{name} (median of {len(releases)})")
        for r in releases:
            for start, decoded in r.frames:
                ax.axvline(start, color=line.get_color(), alpha=0.5 if decoded else 0.2, lw=0.8)
    ax.axvline(0.0, color="k", linestyle="--", linewidth=1)
    ax.set_ylim(-20, 12)
    ax.set_ylabel("passband level, dB vs settled")
    ax.set_title("Received audio around each key release (t = 0); thin lines: frame starts")
    ax.legend(loc="upper right")
    ax.grid(alpha=0.3)
    if traced:
        bx = axes[1][0]
        for name, releases, _ in curves:
            points = [p for r in releases for p in r.trace]
            if not points:
                continue
            ts = np.array([p["t"] for p in points])
            for key, style in (("power", "."), ("level", "-"), ("floor", "--")):
                ys = np.array([p.get(key, np.nan) for p in points])
                order = np.argsort(ts)
                bx.plot(ts[order], ys[order], style, markersize=2, label=f"{name} {key}")
            busy = [p["t"] for p in points if p.get("busy") == 1]
            if busy:
                bx.plot(busy, [bx.get_ylim()[1]] * len(busy), "r|", label=f"{name} busy")
        bx.axvline(0.0, color="k", linestyle="--", linewidth=1)
        bx.set_ylabel("busy detector, dBFS (baseband)")
        bx.set_xlabel("seconds after the key came up")
        bx.legend(loc="lower right", fontsize=7)
        bx.grid(alpha=0.3)
    else:
        ax.set_xlabel("seconds after the key came up")
    fig.tight_layout()
    fig.savefig(png, dpi=120)
    print(f"plot written to {png}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("wav", nargs="+", type=Path)
    ap.add_argument("--detect", action="store_true", help="find releases in every file's audio")
    ap.add_argument(
        "--detect-for", action="append", default=[], type=Path, help="find releases in this file"
    )
    ap.add_argument("--step-ms", type=float, default=25.0)
    ap.add_argument("--before", type=float, default=0.5)
    ap.add_argument("--after", type=float, default=3.0)
    ap.add_argument("--band", default="300,2700", help="passband in Hz, low,high")
    ap.add_argument("--png", type=Path)
    ap.add_argument("--csv", type=Path)
    args = ap.parse_args()
    step_s = args.step_ms / 1000.0
    low, high = (float(v) for v in args.band.split(","))
    curves = []
    for wav in args.wav:
        detect = args.detect or wav in args.detect_for
        releases, before = releases_of(wav, detect, step_s, args.before, args.after, (low, high))
        if not releases:
            print(f"== {wav.name}: no releases found", file=sys.stderr)
            continue
        curves.append((wav.name, releases, summarise(wav.name, releases, step_s, before)))
    if args.csv and curves:
        with args.csv.open("w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(["t_s", *(name for name, _, _ in curves)])
            length = min(len(m) for _, _, m in curves)
            for i in range(length):
                w.writerow(
                    [
                        round(i * step_s - args.before, 4),
                        *(round(float(m[i]), 2) for _, _, m in curves),
                    ]
                )
        print(f"csv written to {args.csv}")
    if args.png and curves:
        plot(curves, step_s, args.before, args.png)
    return 0 if curves else 1


if __name__ == "__main__":
    raise SystemExit(main())
