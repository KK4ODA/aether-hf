"""The amplitude envelope of a transmission, burst by burst, from any recording of it.

A burst that looks unsteady on a receiver's scope can have gone wrong at four places —
in the modem, at the sound card, in the transmitter, or only in the receiver's AGC — and
the way to tell them apart is to hold the same burst up at each point. This prints the
same numbers for any of them: the exact audio `aetherd` handed the sound card
(`[record] tx_audio`, under `tx/` in the recordings folder), a session recording made at
the other end, a recording of the rig's monitor, or of another modem altogether. For
every burst it finds: level, crest factor, how the level settled over the first half
second, and every hole — a stretch inside the burst with no signal in it. A rendered
burst has no holes and settles in under twenty milliseconds; what appears downstream was
put there downstream.

    python tools/tx_envelope.py recordings/tx/20260919-090658_tx.wav
    python tools/tx_envelope.py truck.wav --first 3 --png bursts.png
    python tools/tx_envelope.py aether.wav vara.wav --csv envelopes.csv

Reads 16-bit, 24-bit and 32-bit float WAV, mono or stereo (the first channel), at any rate.
"""

from __future__ import annotations

import argparse
import csv
import struct
import sys
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

STEP_S = 0.010
"""Envelope resolution: the transmitter's ALC, a keying relay and a receiver's AGC all act
within tens of milliseconds, so ten is fine enough to see them and coarse enough to read."""
ONSET_S = 0.5
"""How much of each burst's start is reported step by step."""
BURST_MARGIN_DB = 10.0
"""A step this far above the recording's quiet level is inside a burst."""
BRIDGE_S = 0.3
"""Gaps inside a burst shorter than this join the two halves: a hole is inside a burst, a
gap between bursts is longer than any hole worth calling one."""
HOLE_DB = 15.0
"""A step this far under the burst's steady level, inside it, is a hole."""
HOLE_MIN_S = 0.02
"""Shorter than this is a zero crossing, not a hole."""
SETTLED_DB = 2.0
"""The envelope has settled when it stays within this of the burst's steady level."""
MIN_BURST_S = 0.3
"""Anything shorter is a click, not a burst."""


def read_wav(path: Path) -> tuple[np.ndarray, int]:
    """Samples in ±1.0 and the rate; the first channel of a multi-channel file."""
    data = path.read_bytes()
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError(f"{path} is not a WAV file")
    at = 12
    fmt: tuple[int, int, int, int] | None = None
    samples: np.ndarray | None = None
    rate = 0
    while at + 8 <= len(data):
        chunk_id = data[at : at + 4]
        size = struct.unpack("<I", data[at + 4 : at + 8])[0]
        body = data[at + 8 : at + 8 + size]
        if chunk_id == b"fmt ":
            code, channels, rate, _, _, bits = struct.unpack("<HHIIHH", body[:16])
            if code == 0xFFFE and len(body) >= 26:  # extensible: the real code is inside
                code = struct.unpack("<H", body[24:26])[0]
            fmt = (code, channels, rate, bits)
        elif chunk_id == b"data":
            if fmt is None:
                raise ValueError(f"{path}: data before fmt")
            code, channels, _, bits = fmt
            if code == 3 and bits == 32:
                raw = np.frombuffer(body[: len(body) - len(body) % 4], dtype="<f4").astype(
                    np.float64
                )
            elif code == 1 and bits == 16:
                raw = np.frombuffer(body[: len(body) - len(body) % 2], dtype="<i2") / 32768.0
            elif code == 1 and bits == 24:
                trimmed = body[: len(body) - len(body) % 3]
                as_bytes = np.frombuffer(trimmed, dtype=np.uint8).reshape(-1, 3)
                ints = (
                    as_bytes[:, 0].astype(np.int32)
                    | (as_bytes[:, 1].astype(np.int32) << 8)
                    | (as_bytes[:, 2].astype(np.int32) << 16)
                )
                ints = np.where(ints >= 1 << 23, ints - (1 << 24), ints)
                raw = ints / float(1 << 23)
            else:
                raise ValueError(f"{path}: unsupported format code {code} at {bits} bits")
            samples = raw[::channels] if channels > 1 else raw
            break
        at += 8 + size + (size & 1)
    if samples is None or rate == 0:
        raise ValueError(f"{path}: no audio")
    return samples, rate


def db(x: np.ndarray | float) -> np.ndarray | float:
    return 20.0 * np.log10(np.maximum(x, 1e-9))


@dataclass
class Burst:
    start_s: float
    length_s: float
    steady_rms_dbfs: float
    peak_dbfs: float
    crest_db: float
    settle_s: float | None
    """When the smoothed level last left the settled band, from the burst's start; `None`
    when it never settled."""
    onset_rms_db: list[float] = field(default_factory=list)
    """Level of each step of the onset relative to the steady level."""
    onset_peak_db: list[float] = field(default_factory=list)
    holes: list[tuple[float, float]] = field(default_factory=list)
    """(offset from the burst start, length), in seconds."""


def envelope(samples: np.ndarray, rate: int, step_s: float) -> tuple[np.ndarray, np.ndarray]:
    """RMS and peak per step, in dBFS."""
    step = max(1, round(step_s * rate))
    n = len(samples) // step
    blocks = samples[: n * step].reshape(n, step)
    rms = np.sqrt(np.mean(blocks * blocks, axis=1))
    peak = np.max(np.abs(blocks), axis=1)
    return np.asarray(db(rms)), np.asarray(db(peak))


def find_bursts(rms_db: np.ndarray, step_s: float) -> list[tuple[int, int]]:
    """Step ranges [start, end) that carry a burst: above the quiet level by a margin,
    with gaps shorter than `BRIDGE_S` bridged."""
    quiet = float(np.percentile(rms_db, 10))
    loud = rms_db > quiet + BURST_MARGIN_DB
    bridge = round(BRIDGE_S / step_s)
    bursts: list[tuple[int, int]] = []
    i = 0
    while i < len(loud):
        if not loud[i]:
            i += 1
            continue
        j = i
        last = i
        while j < len(loud) and j - last <= bridge:
            if loud[j]:
                last = j
            j += 1
        if (last + 1 - i) * step_s >= MIN_BURST_S:
            bursts.append((i, last + 1))
        i = last + 1
    return bursts


def analyse(
    samples: np.ndarray, rate: int, step_s: float = STEP_S, onset_s: float = ONSET_S
) -> list[Burst]:
    rms_db, peak_db = envelope(samples, rate, step_s)
    out: list[Burst] = []
    for start, end in find_bursts(rms_db, step_s):
        seg = rms_db[start:end]
        # the steady level: the median of the burst past its first half second, or of all
        # of it when it is shorter — what the onset is judged against
        after = seg[int(onset_s / step_s) :] if len(seg) > 2 * int(onset_s / step_s) else seg
        steady = float(np.median(after))
        peak = float(np.max(peak_db[start:end]))
        block = max(1, round(0.1 / step_s))
        smoothed = np.convolve(seg, np.ones(block) / block, mode="same")
        outside = np.flatnonzero(np.abs(smoothed - steady) > SETTLED_DB)
        settle = None if len(outside) == 0 else float((outside[-1] + 1) * step_s)
        if settle is not None and settle > (end - start) * step_s - 0.1:
            settle = None  # never settled: the deviation runs to the end
        holes: list[tuple[float, float]] = []
        hole_since: int | None = None
        for k, level in enumerate(seg):
            low = level < steady - HOLE_DB
            if low and hole_since is None:
                hole_since = k
            elif not low and hole_since is not None:
                length = (k - hole_since) * step_s
                if length >= HOLE_MIN_S and hole_since > 0:
                    holes.append((round(hole_since * step_s, 3), round(length, 3)))
                hole_since = None
        onset = min(len(seg), round(onset_s / step_s))
        out.append(
            Burst(
                start_s=round(start * step_s, 3),
                length_s=round((end - start) * step_s, 3),
                steady_rms_dbfs=round(steady, 1),
                peak_dbfs=round(peak, 1),
                crest_db=round(peak - steady, 1),
                settle_s=None if settle is None else round(settle, 3),
                onset_rms_db=[round(float(v - steady), 1) for v in seg[:onset]],
                onset_peak_db=[round(float(v - steady), 1) for v in peak_db[start : start + onset]],
                holes=holes,
            )
        )
    return out


def describe(burst: Burst, index: int) -> str:
    settle = (
        "never settled" if burst.settle_s is None else f"settled by {burst.settle_s * 1000:.0f} ms"
    )
    holes = (
        "no holes"
        if not burst.holes
        else "holes at "
        + ", ".join(f"{at:.2f} s ({length * 1000:.0f} ms)" for at, length in burst.holes)
    )
    onset = " ".join(f"{v:+.0f}" for v in burst.onset_rms_db[: int(0.2 / STEP_S)])
    return (
        f"burst {index}: {burst.start_s:8.2f} s, {burst.length_s:6.2f} s long, steady "
        f"{burst.steady_rms_dbfs:6.1f} dBFS rms, peak {burst.peak_dbfs:6.1f} dBFS (crest "
        f"{burst.crest_db:4.1f} dB), {settle}, {holes}\n"
        f"         first 200 ms, rms per {STEP_S * 1000:.0f} ms re steady (dB): {onset}"
    )


def plot(files: list[tuple[Path, np.ndarray, int, list[Burst]]], png: Path, first: int) -> None:
    try:
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:  # pragma: no cover - a plot is a convenience
        print("matplotlib is not installed; no plot", file=sys.stderr)
        return
    rows = sum(min(first, len(bursts)) for _, _, _, bursts in files)
    if rows == 0:
        return
    fig, axes = plt.subplots(rows, 1, figsize=(10, 2.2 * rows), squeeze=False)
    row = 0
    for path, samples, rate, bursts in files:
        rms_db, _ = envelope(samples, rate, STEP_S)
        for index, burst in enumerate(bursts[:first]):
            ax = axes[row][0]
            start = int(burst.start_s / STEP_S)
            end = start + int(burst.length_s / STEP_S)
            t = (np.arange(start, end) - start) * STEP_S
            ax.plot(t, rms_db[start:end] - burst.steady_rms_dbfs, lw=0.8)
            ax.axhline(0, color="k", lw=0.5, alpha=0.5)
            for at, length in burst.holes:
                ax.axvspan(at, at + length, color="red", alpha=0.3)
            ax.set_xlim(0, min(3.0, burst.length_s))
            ax.set_ylim(-30, 10)
            ax.set_ylabel("dB re steady")
            ax.set_title(f"{path.name} burst {index} at {burst.start_s:.1f} s", fontsize=9)
            row += 1
    axes[-1][0].set_xlabel("seconds from the burst's start")
    fig.tight_layout()
    fig.savefig(png, dpi=120)
    print(f"plot written to {png}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("wav", type=Path, nargs="+", help="recordings to analyse")
    parser.add_argument(
        "--first", type=int, default=20, help="report at most this many bursts per file"
    )
    parser.add_argument("--csv", type=Path, help="write every burst's numbers here")
    parser.add_argument("--png", type=Path, help="plot the first bursts' envelopes here")
    args = parser.parse_args(argv)

    analysed: list[tuple[Path, np.ndarray, int, list[Burst]]] = []
    for path in args.wav:
        samples, rate = read_wav(path)
        bursts = analyse(samples, rate)
        analysed.append((path, samples, rate, bursts))
        print(f"{path}: {len(samples) / rate:.1f} s at {rate} Hz, {len(bursts)} burst(s)")
        for index, burst in enumerate(bursts[: args.first]):
            print(describe(burst, index))
    if args.csv:
        with args.csv.open("w", newline="") as handle:
            writer = csv.writer(handle)
            writer.writerow(
                [
                    "file",
                    "burst",
                    "start_s",
                    "length_s",
                    "steady_rms_dbfs",
                    "peak_dbfs",
                    "crest_db",
                    "settle_s",
                    "holes",
                ]
            )
            for path, _, _, bursts in analysed:
                for index, burst in enumerate(bursts):
                    writer.writerow(
                        [
                            path.name,
                            index,
                            burst.start_s,
                            burst.length_s,
                            burst.steady_rms_dbfs,
                            burst.peak_dbfs,
                            burst.crest_db,
                            "" if burst.settle_s is None else burst.settle_s,
                            ";".join(f"{at}+{length}" for at, length in burst.holes),
                        ]
                    )
        print(f"csv written to {args.csv}")
    if args.png:
        plot(analysed, args.png, args.first)
    return 0


if __name__ == "__main__":
    sys.exit(main())
