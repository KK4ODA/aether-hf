"""What the busy detector's noise-floor estimate did on a recording, block by block.

The panel plots the floor at two samples a second, which is enough to see that something
is wrong and not enough to see what: the detector measures forty blocks a second, and a
single odd block can hold the floor down for the whole five-second window. This replays a
48 kHz recording (`aetherd`'s own, or any mono/stereo 16-bit WAV of the radio's audio)
through the same front end and the same statistic, and prints every block that fell well
below its surroundings — how deep, how long, whether the audio around it was zeros (a
capture dropout) or followed a spike (a receiver's AGC recovering from a crash).

    python tools/floor_trace.py recording.wav [--csv out.csv]
"""

from __future__ import annotations

import argparse
import sys
import wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.phy.passband import AudioToBaseband
from aether_model.waveform import WIDE_2300

BLOCK_S = 0.025
WINDOW_S = 5.0
SMOOTHING = 2.0
FLOOR = 1e-20
# the steadiness gate of `aetherd`'s detector: a block counts toward the floor only if the
# raw block powers over the last STEADY_BLOCKS stayed within STEADY_RANGE_DB
STEADY_BLOCKS = 8
STEADY_RANGE_DB = 3.0


def read_wav(path: Path) -> tuple[np.ndarray, int]:
    with wave.open(str(path), "rb") as w:
        rate = w.getframerate()
        channels = w.getnchannels()
        width = w.getsampwidth()
        raw = w.readframes(w.getnframes())
    if width == 2:
        samples = np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768.0
    elif width == 4:
        samples = np.frombuffer(raw, dtype="<i4").astype(np.float64) / 2147483648.0
    else:
        raise SystemExit(f"{path}: {8 * width}-bit samples are not supported")
    if channels > 1:
        samples = samples.reshape(-1, channels)[:, 0]
    return samples, rate


def longest_run(flags: np.ndarray) -> int:
    """The longest run of consecutive True values: a quantised sample is zero by chance now
    and then, a lost buffer is hundreds of zeros in a row."""
    best = run = 0
    for flag in flags:
        run = run + 1 if flag else 0
        best = max(best, run)
    return best


def write_svg(
    path: Path,
    t: np.ndarray,
    level: np.ndarray,
    floor: np.ndarray,
    plain: np.ndarray,
    title: str,
) -> None:
    """The three traces on one axis, no plotting library needed."""
    width, height, left, top = 1200, 360, 60, 30
    low = float(np.floor(min(level.min(), floor.min(), plain.min()) / 3) * 3)
    high = float(np.ceil(max(level.max(), floor.max(), plain.max()) / 3) * 3)
    x = left + (width - left - 20) * t / t[-1]

    def y(v: np.ndarray) -> np.ndarray:
        return top + (height - top - 40) * (high - v) / (high - low)

    def polyline(v: np.ndarray, colour: str, dash: str = "") -> str:
        points = " ".join(f"{a:.1f},{b:.1f}" for a, b in zip(x, y(v), strict=True))
        return (
            f'<polyline fill="none" stroke="{colour}" stroke-width="1.2" {dash} points="{points}"/>'
        )

    grid = "".join(
        f'<line x1="{left}" y1="{y(np.array(v))[()]:.1f}" x2="{width - 20}" '
        f'y2="{y(np.array(v))[()]:.1f}" stroke="#334" stroke-width="0.5"/>'
        f'<text x="{left - 6}" y="{y(np.array(v))[()] + 4:.1f}" fill="#99a" '
        f'font-size="11" text-anchor="end">{v:.0f}</text>'
        for v in np.arange(low, high + 1, 3)
    )
    ticks = "".join(
        f'<text x="{left + (width - left - 20) * s / t[-1]:.1f}" y="{height - 14}" '
        f'fill="#99a" font-size="11" text-anchor="middle">{s:.0f} s</text>'
        for s in np.arange(0, t[-1], 15)
    )
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" font-family="system-ui, sans-serif">'
        f'<rect width="{width}" height="{height}" fill="#0f1117"/>'
        f'<text x="{left}" y="18" fill="#dde" font-size="13">{title} — level (green), '
        f"floor as shipped (white), floor before (red), dBFS</text>"
        + grid
        + ticks
        + polyline(plain, "#e05050", 'stroke-dasharray="4 3"')
        + polyline(level, "#3fd9b4")
        + polyline(floor, "#f0f0f0")
        + "</svg>\n"
    )
    path.write_text(svg, encoding="utf-8", newline="\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("wav", type=Path)
    parser.add_argument(
        "--csv", type=Path, help="write t_s, level_db, floor_db, plain_floor_db, raw_db per block"
    )
    parser.add_argument("--svg", type=Path, help="draw the level and both floors")
    parser.add_argument(
        "--dip-db", type=float, default=1.5, help="how far below the local median counts as a dip"
    )
    args = parser.parse_args()

    audio, rate = read_wav(args.wav)
    if rate != WIDE_2300.audio_rate:
        raise SystemExit(f"{args.wav}: {rate} Hz; the modem records at {WIDE_2300.audio_rate}")
    front = AudioToBaseband(WIDE_2300)
    baseband = front.process(audio)
    block = int(BLOCK_S * WIDE_2300.fs_baseband)
    n_blocks = len(baseband) // block
    power = np.abs(baseband[: n_blocks * block].reshape(n_blocks, block)) ** 2
    raw = power.mean(axis=1)
    # the raw audio behind each block, for the dropout check: 48 kHz samples that were
    # exactly zero (a card that lost a buffer delivers zeros, not noise)
    audio_block = block * WIDE_2300.resample_factor
    delay = front.rx_delay_samples
    zero_runs = np.zeros(n_blocks, dtype=int)
    for i in range(n_blocks):
        start = max(0, i * audio_block - delay)
        seg = audio[start : start + audio_block]
        zero_runs[i] = longest_run(seg == 0.0)

    smoothed = np.empty(n_blocks)
    floor = np.empty(n_blocks)  # what the detector does: the minimum over steady blocks
    plain = np.empty(n_blocks)  # what it did before: the minimum over every block
    window = int(WINDOW_S / BLOCK_S)
    ratio = 10 ** (STEADY_RANGE_DB / 10)
    s = 0.0
    history: list[tuple[float, bool]] = []
    smooth_history: list[float] = []
    held = FLOOR
    for i, p in enumerate(raw):
        s = p if i == 0 else s + (p - s) / SMOOTHING
        smoothed[i] = s
        recent = raw[max(0, i - STEADY_BLOCKS + 1) : i + 1]
        steady = i + 1 >= STEADY_BLOCKS and recent.max() <= max(recent.min(), FLOOR) * ratio
        history.append((p, steady))
        smooth_history.append(s)
        if len(history) > window:
            history.pop(0)
            smooth_history.pop(0)
        candidates = [q for q, ok in history if ok]
        if candidates:
            held = max(min(candidates), FLOOR)
        elif held == FLOOR:
            held = max(min(q for q, _ in history), FLOOR)
        floor[i] = held
        plain[i] = max(min(smooth_history), FLOOR)
    level_db = 10 * np.log10(np.maximum(smoothed, FLOOR))
    floor_db = 10 * np.log10(floor)
    plain_db = 10 * np.log10(plain)
    raw_db = 10 * np.log10(np.maximum(raw, FLOOR))
    t = np.arange(n_blocks) * BLOCK_S

    # dips: blocks well below the median of the floor window around them
    half = int(WINDOW_S / 2 / BLOCK_S)
    local = np.array(
        [np.median(level_db[max(0, i - half) : i + half + 1]) for i in range(n_blocks)]
    )
    below = level_db < local - args.dip_db
    print(
        f"{args.wav.name}: {n_blocks * BLOCK_S:.1f} s, {n_blocks} blocks of {BLOCK_S * 1e3:.0f} ms"
    )
    median = np.median(level_db)
    print(f"level median {median:.1f} dBFS, std {np.std(level_db):.2f} dB")
    for name, trace in (
        ("floor (steady-gated minimum, as shipped)", floor_db),
        ("floor (plain minimum, before)", plain_db),
    ):
        excess = level_db - trace
        print(
            f"{name}: median {np.median(trace):.1f} dBFS, min {trace.min():.1f}; "
            f"more than 3 dB below the level's median {np.mean(trace < median - 3) * 100:.0f} % of the time; "
            f"worst level-over-floor {excess.max():.1f} dB, busy (>= 6 dB) {np.mean(excess >= 6) * 100:.1f} % of the time"
        )
    print(
        f"blocks whose audio holds a run of 48+ zero samples (a millisecond, a lost buffer): "
        f"{int(np.sum(zero_runs >= 48))}"
    )
    print()
    print(
        "dips (start, length, depth below the local median, zeros in the audio, spike just before):"
    )
    i = 0
    count = 0
    while i < n_blocks:
        if not below[i]:
            i += 1
            continue
        j = i
        while j < n_blocks and below[j]:
            j += 1
        depth = float(np.min(level_db[i:j] - local[i:j]))
        zeros = int(np.max(zero_runs[i:j]))
        before = level_db[max(0, i - 20) : i]
        spike = float(np.max(before) - local[i]) if len(before) else 0.0
        print(
            f"  {t[i]:7.2f} s  {(j - i) * BLOCK_S * 1e3:5.0f} ms  {depth:6.1f} dB   "
            f"zeros {zeros:5d}/{audio_block}   spike before {spike:+5.1f} dB"
        )
        count += 1
        i = j
    if count == 0:
        print("  none")

    if args.svg:
        write_svg(args.svg, t, level_db, floor_db, plain_db, args.wav.name)
        print(f"drew {args.svg}")
    if args.csv:
        with args.csv.open("w", encoding="utf-8", newline="\n") as f:
            f.write("t_s,level_db,floor_db,plain_floor_db,raw_db,zeros\n")
            for i in range(n_blocks):
                f.write(
                    f"{t[i]:.3f},{level_db[i]:.2f},{floor_db[i]:.2f},{plain_db[i]:.2f},"
                    f"{raw_db[i]:.2f},{zero_runs[i]}\n"
                )
        print(f"\nwrote {args.csv}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
