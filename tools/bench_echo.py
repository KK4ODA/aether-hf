"""What a path's echo does to each rung, and what the tone floor reads of it (ADR-0087).

    python tools/bench_echo.py [--air 500,2300] [--paths ...] [--snr 10,20] [--frames 16]
                               [--jobs 3] [--out bench/baselines/echo.csv]
    python tools/bench_echo.py --limits [--out bench/baselines/echo.csv]
    python tools/bench_echo.py --harq [--air 500,2300] [--blocks 16] [--harq-out FILE]
    python tools/bench_echo.py --family-limits [--out …echo.csv] [--harq-out …echo_harq.csv]

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

``--harq`` measures what each rung *delivers per second of an exchange* with HARQ (the second
rate review's D3, ``docs/research/rate-control-2/03-candidates.md`` §2.3): ``--blocks`` blocks of
each rung, each sent until it decodes or four times, at the engine's RVs (0, 0, 2, 3), every copy
through its own fade and combined with the copies before it as the receiver combines them.
``--family-limits`` reads it back with the echo readings of ``--out`` and prints the echo above
which the OFDM family as a whole is kept off each air (:func:`family_limits`).
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
from aether_model.link.granted import RV_SEQUENCE, GrantConfig, GrantedEngine
from aether_model.link.harness import phy_timing
from aether_model.phy import tone as T
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameType

AIRS: dict[int, AirInterface] = {500: NARROW, 2300: WIDE}
RUNGS: dict[int, tuple[int, ...]] = {500: (0, 1, 2, 3, 4, 5, 7, 9), 2300: (0, 2, 3, 4, 5, 6, 8, 10)}
"""The rungs measured: every tone rung that can break (the floor's own two are there to show
they do not), and the OFDM family's slowest and two above it."""
PATHS = (
    "awgn,good,moderate,poor,flutter,nvis,low-moderate,low-disturbed,high-moderate,"
    "high-disturbed,1@1,2@1,3@1,4@1,5@1,6@1,7@1,8@1,4@0.1,7@0.1,4@10,7@10"
)
OUT = "bench/baselines/echo.csv"
HARQ_OUT = "bench/baselines/echo_harq.csv"
HARQ_RUNGS: dict[int, tuple[int, ...]] = {
    500: (0, 1, 2, 3, 4, 5, 6, 7, 9),
    2300: (0, 1, 2, 3, 4, 5, 7, 8, 9, 10),
}
"""The rungs measured with HARQ: every floor rung, and the OFDM family's slowest few — on a path
where the family's slowest rung loses to the floor, its faster ones lose by more."""
COPIES = 4
"""Copies of a block before the sender gives it up at its rung (``GrantConfig.max_combines``)."""
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


def _copy(
    modem: Modem,
    det: T.ToneDetector,
    rng: np.random.Generator,
    bw: int,
    rung: int,
    path: str,
    snr: float,
    payload: bytes,
    rv: int,
    buffer: object | None,
) -> tuple[bool, object | None]:
    """One copy of a block through its own fade, combined with ``buffer``: whether the block
    decoded, and the buffer to combine the next copy with."""
    air = AIRS[bw]
    kind = air.ladder[rung].tone
    x = T.burst(kind, payload, rv) if kind is not None else modem.rung_burst(payload, rung, rv)
    lead = int(rng.integers(1500, 2500))
    buf = np.concatenate((np.zeros(lead, complex), x, np.zeros(2500, complex)))
    ch = make_channel(
        profile(path),
        snr_db=snr,
        fs=air.params.fs_baseband,
        seed=int(rng.integers(0, 2**31)),
        signal_power=1.0,
        cfo_hz=float(rng.uniform(-50, 50)),
    )
    y = modem.detector.condition(ch.process(buf))
    if kind is not None:
        found = [s for s in det.detect(y, max_frames=2) if s.kind == kind]
        if not found:
            return False, buffer
        s = found[0]
        frame = T.demodulate(y, kind, s.rv, s.start, s.cfo_hz, middle=s.middle)
        out, combined = frame.decode(buffer)  # type: ignore[arg-type]
        return out == payload, combined
    syncs = modem.detector.detect(y, max_frames=1)
    if not syncs:
        return False, buffer
    try:
        received = modem.demodulate(y, syncs[0])
    except ValueError:
        return False, buffer
    if received.sync.header.frame_type is not FrameType.DATA:
        return False, buffer
    try:
        if air.rung_of(received.mode) != rung:
            return False, buffer  # chips misread: the receiver would file it elsewhere
    except ValueError:
        return False, buffer
    out, combined = modem.decode_frame(received, buffer)  # type: ignore[arg-type]
    return out == payload, combined


def run_harq(job: tuple[int, int, str, float, int, int]) -> dict[str, object]:
    """``blocks`` blocks of a rung, each sent until it decodes or :data:`COPIES` times."""
    bw, rung, path, snr, blocks, seed = job
    air = AIRS[bw]
    step = air.ladder[rung]
    modem = Modem(air.params)
    det = T.ToneDetector((air.tone_control, *air.tone_data))
    rng = np.random.default_rng(seed)
    by_copy = [0] * COPIES
    copies = 0
    for _ in range(blocks):
        payload = rng.integers(0, 256, step.payload_bytes, dtype=np.uint8).tobytes()
        buffer: object | None = None
        for k in range(COPIES):
            copies += 1
            ok, buffer = _copy(
                modem, det, rng, bw, rung, path, snr, payload, RV_SEQUENCE[k], buffer
            )
            if ok:
                by_copy[k] += 1
                break
    return {
        "bandwidth_hz": bw,
        "rung": rung,
        "frame": step.name,
        "path": path,
        "snr_3k_db": snr,
        "blocks": blocks,
        "delivered": sum(by_copy),
        "copies": copies,
        "by_copy": "/".join(str(n) for n in by_copy),
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


def exchange_worth(bw: int) -> dict[int, float]:
    """Per rung, the data bytes a second of a whole exchange carries with nothing lost: a full
    transmission of its frames, a control frame each way in its own family and the two gaps —
    what the rate controller ranks rungs by (``RateController._priced``), from the protocol's own lengths.
    Pricing the answers in the data's own family is the kindest to OFDM: on an echo path its
    answers may go on the floor, at 3.2 s each."""
    air = AIRS[bw]
    engine = GrantedEngine("W4ODA", phy_timing(air.params), GrantConfig(max_mode=air.n_rungs - 1))
    c = engine._delivery_controller()
    return {m: c._priced(m, c.lossless(m)) for m in c.modes}


def family_limits(
    rows: list[dict[str, str]], harq: list[dict[str, str]], bw: int
) -> tuple[float | None, list[str]]:
    """The echo share above which the OFDM family is kept off air ``bw`` (D3), and the paths
    that set it.

    A rung's *delivery* on a path and at an SNR is the bytes its blocks delivered over the air
    their copies took, priced per second of an exchange (:func:`exchange_worth`). A path
    *refuses* the family when, at every SNR measured, the best floor rung the ladder offers there
    — a floor rung whose own echo limit (:func:`limits`) the path's reading is under — delivers
    more than the family's best; it *carries* the family otherwise. The paths and the reading are
    :func:`limits`'s (ITU-R F.1487's conditions and the 1 Hz delay sweep, rung 0's median echo),
    and so is the rule: the lowest refusing path's reading less a tenth, provided it stands
    :data:`MARGIN` above every carrying path's; else no limit."""
    air = AIRS[bw]
    per_rung = limits(rows, bw)
    echo: dict[tuple[str, float], float] = {}
    for r in rows:
        if int(r["bandwidth_hz"]) == bw and int(r["rung"]) == 0 and r["echo_p50"] != "":
            echo[(r["path"], float(r["snr_3k_db"]))] = float(r["echo_p50"])
    worth_ex = exchange_worth(bw)
    delivery: dict[tuple[str, float], dict[int, float]] = defaultdict(dict)
    floor0: dict[str, float] = defaultdict(float)
    for r in harq:
        if int(r["bandwidth_hz"]) != bw or probe(r["path"]):
            continue
        rung = int(r["rung"])
        key = (r["path"], float(r["snr_3k_db"]))
        share = int(r["delivered"]) / max(1, int(r["copies"]))
        delivery[key][rung] = share * worth_ex.get(rung, 0.0)
        if rung == 0:
            floor0[r["path"]] = max(floor0[r["path"]], int(r["delivered"]) / int(r["blocks"]))
    refused: list[tuple[float, str]] = []
    carried: list[tuple[float, str]] = []
    for path in sorted({p for p, _ in delivery}):
        if floor0[path] < KEEP:
            continue
        keys = [k for k in delivery if k[0] == path and k in echo]
        if not keys:
            continue
        beaten = []
        for k in keys:
            offered = [
                v
                for m, v in delivery[k].items()
                if air.is_floor(m) and not (m in per_rung and echo[k] > per_rung[m])
            ]
            ofdm = [v for m, v in delivery[k].items() if not air.is_floor(m)]
            beaten.append(max(offered, default=0.0) > max(ofdm, default=0.0))
        reads = [echo[k] for k in keys]
        if all(beaten):
            refused.append((min(reads), path))
        else:
            carried.append((max(reads), path))
    if not refused:
        return None, []
    limit, at = min(refused)
    top = max(carried, default=(0.0, ""))
    if top[0] and limit < MARGIN * top[0]:
        return None, [f"refused from {at} ({limit:.4f}), carried up to {top[1]} ({top[0]:.4f})"]
    return round(limit * 0.9, 4), [
        f"refused from {at} ({limit:.4f})",
        f"carried up to {top[1]} ({top[0]:.4f})",
    ]


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
    ap.add_argument("--harq", action="store_true", help="each rung's delivery with HARQ")
    ap.add_argument("--blocks", type=int, default=16)
    ap.add_argument("--harq-out", default=HARQ_OUT)
    ap.add_argument("--family-limits", action="store_true")
    args = ap.parse_args()
    out = Path(args.out)
    if args.family_limits:
        with out.open(encoding="utf-8") as f:
            measured = list(csv.DictReader(f))
        with Path(args.harq_out).open(encoding="utf-8") as f:
            harq = list(csv.DictReader(f))
        for bw in sorted({int(r["bandwidth_hz"]) for r in harq}):
            limit, why = family_limits(measured, harq, bw)
            print(bw, "OFDM family:", limit, "; ".join(why))
        return 0
    if args.harq:
        return run_harq_grid(args)
    if args.limits:
        with out.open(encoding="utf-8") as f:
            measured = list(csv.DictReader(f))
        for bw in sorted({int(r["bandwidth_hz"]) for r in measured}):
            print(bw, limits(measured, bw))
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


def run_harq_grid(args: argparse.Namespace) -> int:
    airs = [int(a) for a in args.air.split(",")]
    paths = [p for p in args.paths.split(",") if p]
    snrs = [float(s) for s in args.snr.split(",")]
    out = Path(args.harq_out)
    jobs = [
        (bw, rung, path, snr, args.blocks, args.seed * 1_000_003 + 7919 * i)
        for i, (bw, rung, path, snr) in enumerate(
            (bw, rung, path, snr)
            for bw in airs
            for rung in HARQ_RUNGS[bw]
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
        for row in pool.imap_unordered(run_harq, jobs):
            rows.append(row)
            print(
                f"{row['bandwidth_hz']:4d} {row['frame']:14s} {row['path']:14s} "
                f"{row['snr_3k_db']:+5.1f} dB  {row['delivered']:2d}/{row['blocks']} in "
                f"{row['copies']:2d} copies ({row['by_copy']})   [{time.time() - t0:5.0f} s]",
                flush=True,
            )
    rows.sort(
        key=lambda r: (
            int(r["bandwidth_hz"]),  # type: ignore[call-overload]
            int(r["rung"]),  # type: ignore[call-overload]
            paths.index(str(r["path"])),
            float(r["snr_3k_db"]),  # type: ignore[arg-type]
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
