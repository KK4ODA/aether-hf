"""Each air's ladder as the link runs it: what every rung carries, and where it starts to work.

    python tools/ladder_table.py [--bandwidth 500|2300] [--markdown]

Every number is read, never typed: the rungs, their payloads and air times from the reference
model's ladder (``aether_model.frame.modes``); the data a frame carries after its DATA header
(``aether_model.link.frames.data_capacity``); the AWGN 10 % frame-error points the rate
controller steps by (``aether_model.link.rate``); and each rung's 10 % point on the ITU-R
F.1487 Good, Moderate and Poor channels as the calibrated fading pipe has it
(``bench/baselines/fading_pipe.csv``, ``fitted_db``: the measured class waterfall the pipe
reproduces). For each channel it marks the rungs on the rate/threshold Pareto front — a rung
that another beats on both counts is never worth asking for on that channel — and lists the
gaps between neighbouring rungs on the front, in decibels and in data rate: where a path whose
SNR falls between two rungs is left carrying the lower one's rate (12-two-paths-80m.md §Q2).
"""

from __future__ import annotations

import argparse
import csv
import itertools
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "model"))

from aether_model.frame.modes import NARROW, WIDE, AirInterface  # noqa: E402
from aether_model.link.frames import data_capacity  # noqa: E402
from aether_model.link.rate import AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB  # noqa: E402

FADING_CSV = ROOT / "bench" / "baselines" / "fading_pipe.csv"
CHANNELS = ("awgn", "good", "moderate", "poor")


@dataclass(frozen=True)
class Rung:
    index: int
    name: str
    """What the frame is: the tone kind's name, or the OFDM mode's."""
    key: str
    """The name the fading calibration knows it by (``fading.frame_key``)."""
    payload_bytes: int
    data_bytes: int
    """What a DATA frame carries after its header."""
    frame_s: float
    threshold_db: dict[str, float]
    """The 10 % frame-error point per channel; AWGN from the rate table."""

    @property
    def net_bps(self) -> float:
        return 8.0 * self.payload_bytes / self.frame_s

    @property
    def data_bps(self) -> float:
        return 8.0 * self.data_bytes / self.frame_s


def fading_points(air: AirInterface, path: Path = FADING_CSV) -> dict[tuple[str, str], float]:
    """(frame key, channel) -> the class's 10 % point, for this air's bandwidth."""
    out: dict[tuple[str, str], float] = {}
    with path.open(encoding="utf-8") as f:
        for row in csv.DictReader(f):
            if int(row["bandwidth_hz"]) == air.params.bandwidth.value:
                out[(row["frame"], row["channel"])] = float(row["fitted_db"])
    return out


def ladder(air: AirInterface, path: Path = FADING_CSV) -> list[Rung]:
    awgn = NARROW_AWGN_THRESHOLD_DB if air is NARROW else AWGN_THRESHOLD_DB
    fading = fading_points(air, path)
    rungs = []
    for r in air.ladder:
        if r.tone is not None:
            key = r.tone.name
        else:
            assert r.mode is not None
            key = f"mode {r.mode.index}"
        points = {"awgn": awgn[r.index]}
        for channel in CHANNELS[1:]:
            if (key, channel) in fading:
                points[channel] = fading[(key, channel)]
        rungs.append(
            Rung(
                index=r.index,
                name=r.name,
                key=key,
                payload_bytes=r.payload_bytes,
                data_bytes=data_capacity(r.payload_bytes),
                frame_s=r.duration_s,
                threshold_db=points,
            )
        )
    return rungs


def front(rungs: list[Rung], channel: str) -> list[int]:
    """The rungs no other beats on both data rate and threshold on ``channel``, ascending —
    the rule ``rate.usable_modes`` applies to the AWGN table."""
    have = [r for r in rungs if channel in r.threshold_db]
    out = []
    for r in have:
        beaten = any(
            o.index != r.index
            and o.data_bps >= r.data_bps
            and o.threshold_db[channel] <= r.threshold_db[channel]
            for o in have
        )
        if not beaten:
            out.append(r.index)
    return sorted(out, key=lambda i: rungs[i].threshold_db[channel])


@dataclass(frozen=True)
class Gap:
    low: int
    high: int
    db: float
    """How much more SNR the higher rung needs."""
    ratio: float
    """How much faster it carries data."""


def gaps(rungs: list[Rung], channel: str) -> list[Gap]:
    """Between neighbours on the front, ordered by threshold."""
    on = front(rungs, channel)
    return [
        Gap(
            a,
            b,
            rungs[b].threshold_db[channel] - rungs[a].threshold_db[channel],
            rungs[b].data_bps / rungs[a].data_bps,
        )
        for a, b in itertools.pairwise(on)
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bandwidth", type=int, choices=(500, 2300), default=500)
    ap.add_argument("--markdown", action="store_true", help="a table for a report")
    args = ap.parse_args()
    air = NARROW if args.bandwidth == 500 else WIDE
    rungs = ladder(air)
    fronts = {c: set(front(rungs, c)) for c in CHANNELS}

    def cell(r: Rung, c: str) -> str:
        if c not in r.threshold_db:
            return "—"
        mark = "" if r.index in fronts[c] else " (beaten)"
        return f"{r.threshold_db[c]:+.1f}{mark}"

    head = ["rung", "frame", "bytes", "data B", "s", "bit/s", "data bit/s"] + [
        f"{c} dB" for c in CHANNELS
    ]
    rows = [
        [
            str(r.index),
            r.name,
            str(r.payload_bytes),
            str(r.data_bytes),
            f"{r.frame_s:.3f}",
            f"{r.net_bps:.0f}",
            f"{r.data_bps:.0f}",
        ]
        + [cell(r, c) for c in CHANNELS]
        for r in rungs
    ]
    if args.markdown:
        print("| " + " | ".join(head) + " |")
        print("|" + "---|" * len(head))
        for row in rows:
            print("| " + " | ".join(row) + " |")
    else:
        widths = [max(len(x) for x in col) for col in zip(head, *rows, strict=True)]
        for row in [head, *rows]:
            print("  ".join(x.rjust(w) for x, w in zip(row, widths, strict=True)))
    print(f"\ngaps between neighbouring rungs on each channel's front ({args.bandwidth} Hz):")
    for c in CHANNELS:
        line = ", ".join(f"{g.low}→{g.high} {g.db:.1f} dB ×{g.ratio:.2f}" for g in gaps(rungs, c))
        print(f"  {c:<9} {line}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
