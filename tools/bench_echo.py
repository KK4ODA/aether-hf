"""What a path's echo does to each rung, and what the tone floor reads of it (ADR-0087).

    python tools/bench_echo.py [--air 500,2300] [--paths ...] [--snr 10,20] [--frames 16]
                               [--jobs 3] [--out bench/baselines/echo.csv]
    python tools/bench_echo.py --limits [--out bench/baselines/echo.csv]

For every air, path and SNR, ``--frames`` frames of each rung measured here go through the
channel and the air's own receiver — the tone detector or the OFDM one, the frame's start,
carrier offset and kind as they find them — and are counted when they decode; the tone frames,
the control frame among them, also give their echo share (``tone.echo_share``), kept as its
10th, 50th and 90th percentiles. A path is a channel profile by name (``nvis``, ``flutter``,
``high-moderate`` …) or ``D@F``: two equal Rayleigh rays ``D`` ms apart, each with a Doppler
spread of ``F`` Hz (ITU-R F.1487's model). Each frame has its own fade, a carrier offset
uniform in ±50 Hz and a random start. The SNR is the 3 kHz OFDM reference, the tone frames at
equal peak power, as in ``bench_tone.py``.

``--limits`` reads the file back and prints the echo share above which each rung measured is
kept off the ladder — the rule and its reasons are :func:`limits`'s — which is what
``ECHO_LIMIT`` and ``NARROW_ECHO_LIMIT`` in ``aether_model/link/rate.py`` hold (a test holds
them to this file). Rows of airs not run are kept from ``--out``.
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
import time
from collections import defaultdict
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import ChannelProfile, make_channel
from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import DATA_HEADER
from aether_model.phy import tone as T
from aether_model.phy.pipeline import Modem

AIRS: dict[int, AirInterface] = {500: NARROW, 2300: WIDE}
RUNGS: dict[int, tuple[int, ...]] = {500: (0, 1, 2, 3, 4, 5, 7, 9), 2300: (0, 2, 3, 4, 5, 6, 8, 10)}
"""The rungs measured: every tone rung that can break (the floor's own two are there to show
they do not), and the OFDM family's slowest and two above it."""
PATHS = (
    "awgn,good,moderate,poor,flutter,nvis,low-moderate,low-disturbed,high-moderate,"
    "high-disturbed,1@1,2@1,3@1,4@1,5@1,6@1,7@1,8@1,4@0.1,7@0.1,4@10,7@10"
)
OUT = "bench/baselines/echo.csv"
KEEP = 0.7
"""A path that decodes less than this share of the floor's slowest rung's frames carries
nothing (:func:`limits`)."""
MARGIN = 1.25
"""How far above the highest echo a path that carries a rung reads the rung's limit must sit."""


def profile(path: str) -> ChannelProfile | str:
    if "@" not in path:
        return path
    d, f = path.split("@")
    return ChannelProfile(
        f"two rays {d} ms, {f} Hz", (0.0, float(d) * 1e-3), (float(f),) * 2, (0.0, 0.0)
    )


def run(job: tuple[int, int, str, float, int, int]) -> dict[str, object]:
    bw, rung, path, snr, frames, seed = job
    air = AIRS[bw]
    step = air.ladder[rung]
    modem = Modem(air.params)
    det = T.ToneDetector((air.tone_control, *air.tone_data))
    fs = air.params.fs_baseband
    rng = np.random.default_rng(seed)
    decoded = 0
    echoes: list[float] = []
    kind = step.tone
    for _ in range(frames):
        payload = rng.integers(0, 256, step.payload_bytes, dtype=np.uint8).tobytes()
        x = T.burst(kind, payload, 0) if kind is not None else modem.rung_burst(payload, rung, 0)
        lead = int(rng.integers(1500, 2500))
        buf = np.concatenate((np.zeros(lead, complex), x, np.zeros(2500, complex)))
        ch = make_channel(
            profile(path),
            snr_db=snr,
            fs=fs,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-50, 50)),
        )
        y = ch.process(buf)
        if kind is not None:
            found = [s for s in det.detect(y, max_frames=2) if s.kind == kind]
            if not found:
                continue
            s = found[0]
            frame = T.demodulate(y, kind, s.rv, s.start, s.cfo_hz, middle=s.middle)
            if frame.echo is not None:
                echoes.append(frame.echo)
            out, _ = frame.decode()
            decoded += out == payload
        else:
            got = [d for d in modem.decode_buffer(y, max_frames=2) if d.frame is not None][:1]
            decoded += bool(got) and got[0].payload == payload
    q = np.percentile(echoes, [10, 50, 90]) if echoes else [math.nan] * 3
    return {
        "bandwidth_hz": bw,
        "rung": rung,
        "frame": step.name,
        "path": path,
        "snr_3k_db": snr,
        "frames": frames,
        "decoded": decoded,
        "echo_p10": "" if math.isnan(q[0]) else round(float(q[0]), 5),
        "echo_p50": "" if math.isnan(q[1]) else round(float(q[1]), 5),
        "echo_p90": "" if math.isnan(q[2]) else round(float(q[2]), 5),
    }


def worth(bw: int, rung: int) -> float:
    """What a frame of ``rung`` carries a second of air: its data after the DATA header, over
    its frame's length — the measure :func:`~aether_model.link.rate.usable_modes` orders a
    ladder by."""
    air = AIRS[bw]
    step = air.ladder[rung]
    seconds = step.tone.duration_s if step.tone is not None else air.long.duration_s
    return max(0, step.payload_bytes - DATA_HEADER) / seconds


def probe(path: str) -> bool:
    """A two-ray path off ITU-R F.1487's grid of conditions — a Doppler spread other than the
    mid-latitude disturbed 1 Hz the delay sweep is run at: measured to show where the echo
    reading stops telling the rungs apart, not drawn on (:func:`limits`)."""
    return "@" in path and float(path.split("@")[1]) != 1.0


def limits(rows: list[dict[str, str]], bw: int) -> dict[int, float]:
    """The echo share above which each rung is kept off the ladder on air ``bw``, from the
    measured rows of ITU-R F.1487's conditions and the delay sweep at its 1 Hz (not the
    :func:`probe` paths).

    On a path and at an SNR a rung is *beaten* when a slower rung measured there delivers more
    — its decode rate times :func:`worth` — and a path *refuses* the rung when it is beaten at
    every SNR measured, *carries* it otherwise. A path that does not carry the floor's slowest
    rung (decodes under :data:`KEEP` of its frames) carries nothing the ladder could be fitted
    to — ITU's high-latitude disturbed, 7 ms at 30 Hz — and is left out; rung 0 has no limit,
    there is nothing below it. A rung's limit is the lowest median echo, read on rung 0's frames
    (the floor's own kind every call, acceptance and floor control frame shares), of a path
    that refuses it, less a tenth — provided it stands :data:`MARGIN` above the highest median
    echo of every path that carries it. Where it does not, the measurement gives the rung no
    limit of its own: a path the echo reading cannot tell from one that refuses it is not
    refused on that reading. Then within each family — the floor's rungs, the OFDM ones — a
    rung is held to the lowest limit of a slower rung of it, measured or not: a faster rung of a
    family never carries more echo than a slower one (the same symbols at a higher code rate,
    or a denser constellation behind the same prefix)."""
    echo: dict[tuple[str, float], float] = {}
    rate: dict[tuple[str, float], dict[int, float]] = defaultdict(dict)
    for r in rows:
        if int(r["bandwidth_hz"]) != bw or probe(r["path"]):
            continue
        key = (r["path"], float(r["snr_3k_db"]))
        rate[key][int(r["rung"])] = int(r["decoded"]) / int(r["frames"])
        if int(r["rung"]) == 0 and r["echo_p50"] != "":
            echo[key] = float(r["echo_p50"])
    paths = {p for p, _ in rate}
    usable = {p for p in paths if max(rate[k].get(0, 0.0) for k in rate if k[0] == p) >= KEEP}
    measured: dict[int, float] = {}
    rungs = sorted({m for by_rung in rate.values() for m in by_rung} - {0})
    for rung in rungs:
        refused, carried = [], []
        for path in usable:
            keys = [k for k in rate if k[0] == path and rung in rate[k] and k in echo]
            if not keys:
                continue
            beaten = [
                any(
                    q < rung and v * worth(bw, q) > rate[k][rung] * worth(bw, rung)
                    for q, v in rate[k].items()
                )
                for k in keys
            ]
            reads = [echo[k] for k in keys]
            if all(beaten):
                refused.append(min(reads))
            else:
                carried.append(max(reads))
        if not refused:
            continue
        limit = min(refused)
        if carried and limit < MARGIN * max(carried):
            continue
        measured[rung] = round(limit * 0.9, 4)
    air = AIRS[bw]
    out: dict[int, float] = {}
    for floor in (True, False):
        held: float | None = None
        for rung in range(air.n_rungs):
            if air.is_floor(rung) != floor:
                continue
            if rung in measured:
                held = measured[rung] if held is None else min(held, measured[rung])
            if held is not None:
                out[rung] = held
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--air", default="500,2300")
    ap.add_argument("--paths", default=PATHS)
    ap.add_argument("--snr", default="10,20")
    ap.add_argument("--frames", type=int, default=16)
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--limits", action="store_true")
    ap.add_argument("--out", default=OUT)
    args = ap.parse_args()
    out = Path(args.out)
    if args.limits:
        with out.open(encoding="utf-8") as f:
            rows = list(csv.DictReader(f))
        for bw in sorted({int(r["bandwidth_hz"]) for r in rows}):
            print(bw, limits(rows, bw))
        return 0

    airs = [int(a) for a in args.air.split(",")]
    paths = [p for p in args.paths.split(",") if p]
    snrs = [float(s) for s in args.snr.split(",")]
    jobs = [
        (bw, rung, path, snr, args.frames, args.seed * 1_000_003 + 7919 * i)
        for i, (bw, rung, path, snr) in enumerate(
            (bw, rung, path, snr)
            for bw in airs
            for rung in RUNGS[bw]
            for path in paths
            for snr in snrs
        )
    ]
    kept: list[dict[str, object]] = []
    if out.exists():
        with out.open(encoding="utf-8") as f:
            kept = [r for r in csv.DictReader(f) if int(r["bandwidth_hz"]) not in airs]
    rows: list[dict[str, object]] = []
    t0 = time.time()
    with Pool(args.jobs) as pool:
        for row in pool.imap(run, jobs):
            rows.append(row)
            print(
                f"{row['bandwidth_hz']:4d} {row['frame']:14s} {row['path']:14s} "
                f"{row['snr_3k_db']:+5.1f} dB  {row['decoded']:2d}/{row['frames']}  "
                f"echo {row['echo_p50']}   [{time.time() - t0:5.0f} s]",
                flush=True,
            )
    rows.sort(
        key=lambda r: (
            int(r["bandwidth_hz"]),
            int(r["rung"]),
            paths.index(str(r["path"])),
            float(r["snr_3k_db"]),
        )
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        w.writeheader()
        w.writerows([*kept, *rows])
    print("wrote", out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
