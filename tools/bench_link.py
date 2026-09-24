"""Link-layer throughput and rate-control benchmark (roadmap P2-2).

    python tools/bench_link.py [--backend sim|phy] [--channels awgn,good,moderate,poor]
                               [--snr -4,0,4,8,12,16,20] [--bytes 8000] [--trials 3]
                               [--ramp] [--out bench/baselines/link_throughput.csv]

For every (channel, SNR) it runs a complete session — connect, transfer ``bytes``, orderly
disconnect — and reports the goodput the *link layer* actually achieved, which is the number
a user sees: PHY air time plus ACK turnarounds, retransmissions, HARQ combining and whatever
the rate controller chose along the way.

Two backends:

* ``sim`` (default) — the DSP-free lossy pipe, calibrated per channel from the measured PHY
  sweep (``bench/baselines/phy_fer.csv``): a whole grid in seconds, good for rate-control
  behaviour and regressions in the protocol itself.
* ``phy`` — the real modem and channel simulator, frame by frame. Minutes per point, but the
  numbers are end-to-end truth.

``--ramp`` replaces the fixed-SNR runs with a triangular fade (``--ramp-span`` dB peak to
trough over ``--ramp-period`` s) and reports how the controller tracked it: the modes it
used, how often it changed them, and what that cost in retransmissions.

``--replay <sidecar.json>`` (P6-7) runs the engines against what a recorded session did: the
SNR the receiver measured, frame by frame, becomes the schedule the pipe follows, and the
per-mode thresholds are the AWGN table shifted by the penalty that best explains the
session's own decodes — the Test session's ladder when there is one (a burst pinned at each
mode, so the frame error rate per mode is measured directly), the data frames otherwise.
The transfer is the size the session carried. What comes out is the goodput the model's
engines would have got on that path beside what the air delivered, and the fitted penalty,
which is the on-air equivalent of ``channel_thresholds``.

Output columns: backend, channel, snr_db (or ramp description), bytes, seconds, goodput_bps,
ideal_bps, efficiency, mode_min/mode_max/mode_final, mode_changes, bursts, frames_sent,
frames_resent, harq_rescues, ack_timeouts, ok.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
import sys
import time
from collections import defaultdict
from collections.abc import Callable
from itertools import pairwise
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import NARROW, TONE_CONTROL, WIDE, AirInterface
from aether_model.link.engine import LinkConfig, LinkEngine
from aether_model.link.fading import FadingPipe, SharedFading, frame_key, shapes_for
from aether_model.link.harness import phy_timing, two_modem_sim
from aether_model.link.phy import Container, TxFrame
from aether_model.link.rate import (
    AWGN_THRESHOLD_DB,
    NARROW_AWGN_THRESHOLD_DB,
    NARROW_PAYLOAD_BYTES,
    PAYLOAD_BYTES,
    usable_modes,
)
from aether_model.link.sim import TwoStationSim, control_thresholds_for
from aether_model.phy.tone import TONE_GAIN_DB

CHANNEL_ORDER = ("awgn", "good", "moderate", "poor")


# ── channel calibration for the fast backend ──────────────────────────


TONE_CSV = "bench/baselines/tone_floor.csv"
"""``tools/bench_tone.py``'s measurements of the tone floor's frames (ADR-0013), per channel —
the same frames on both airs."""


def tone_points(
    csv_path: Path, target: str = "decoded"
) -> dict[tuple[str, str], list[tuple[float, float]]]:
    """(channel, kind name) → [(SNR, FER)] from ``bench_tone.py``'s rows."""
    points: dict[tuple[str, str], list[tuple[float, float]]] = defaultdict(list)
    if not csv_path.exists():
        return points
    with csv_path.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            fer = 1.0 - int(r[target]) / max(int(r["frames"]), 1)
            points[(r["channel"], r["frame"])].append((float(r["snr_3k_db"]), fer))
    return points


def channel_thresholds(
    csv_path: Path,
    target_fer: float = 0.10,
    awgn: dict[int, float] = AWGN_THRESHOLD_DB,
    air: AirInterface = WIDE,
    tone_csv: Path = Path(TONE_CSV),
) -> dict[str, dict[int, float]]:
    """Per-channel, per-rung minimum usable SNR interpolated from the PHY sweeps: the OFDM
    modes' rows of ``csv_path`` (``bench_phy.py``, which names OFDM modes) mapped onto the
    air's ladder, and the tone floor's from ``tone_csv``.

    Rungs the sweeps did not cover are filled by shifting the AWGN table by the mean measured
    penalty of that channel — crude, but it keeps the whole ladder available to the rate
    controller instead of leaving holes it would have to skip.
    """
    if not csv_path.exists():
        return {}
    with csv_path.open(encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    points: dict[tuple[str, int], list[tuple[float, float]]] = defaultdict(list)
    for r in rows:
        try:
            rung = air.rung_of(int(r["mode"]))
        except ValueError:
            continue  # an OFDM mode on no rung: the 500 Hz air's retired floor
        points[(r["channel"], rung)].append((float(r["snr_3k_db"]), float(r["fer"])))
    names = {k.name: i for i, k in enumerate(air.tone_data)}
    for (channel, name), pts in tone_points(tone_csv).items():
        if name in names:
            points[(channel, names[name])] += pts

    def crossing(pts: list[tuple[float, float]]) -> float | None:
        prev: tuple[float, float] | None = None
        for snr, fer in sorted(pts):
            if fer <= target_fer:
                if prev is None or prev[1] == fer:
                    return snr
                s0, f0 = prev
                return s0 + (f0 - target_fer) / (f0 - fer) * (snr - s0)
            prev = (snr, fer)
        return None

    measured: dict[str, dict[int, float]] = defaultdict(dict)
    for (channel, mode), pts in points.items():
        x = crossing(pts)
        if x is not None:
            measured[channel][mode] = x

    out: dict[str, dict[int, float]] = {}
    for channel, table in measured.items():
        penalties = [table[m] - awgn[m] for m in table if m in awgn]
        shift = statistics.fmean(penalties) if penalties else 0.0
        out[channel] = {m: table.get(m, awgn[m] + shift) for m in awgn}
    return out


FADING_CSV = "bench/baselines/fading_pipe.csv"
"""``tools/calibrate_fading.py``: the fading pipe's effective-SNR constant per frame type and
channel class."""


def fading_pipe(csv_path: Path, air: AirInterface, channel: str, seed: int) -> FadingPipe:
    """A fading channel for one run: the class's two-ray process, shared by both stations,
    and each frame's calibrated effective-SNR mapping (P9-6). AWGN is the static channel,
    judged on the same steep waterfall."""
    betas: dict[str, float] = {}
    if csv_path.exists():
        with csv_path.open(encoding="utf-8") as f:
            for r in csv.DictReader(f):
                if int(r["bandwidth_hz"]) == air.params.bandwidth.value and r["channel"] == channel:
                    betas[r["frame"]] = float(r["beta"])
    if channel != "awgn" and not betas:
        raise SystemExit(f"{csv_path}: no calibration for {channel}; run tools/calibrate_fading.py")
    return FadingPipe(
        SharedFading(channel, seed),
        shapes_for(air),
        lambda frame: betas.get(frame_key(frame, air), 1.0),
        np.random.default_rng(seed + 99),
    )


PEAK_CSV = "bench/baselines/peak_to_average.csv"
"""``tools/bench_peak.py``: each frame's peak-to-average ratio as the modem transmits it."""


def peak_offsets(csv_path: Path, air: AirInterface) -> Callable[[TxFrame], float]:
    """Each frame's SNR at equal *peak* power relative to the SNR the axis states: minus its
    own measured peak-to-average ratio (the highest sample), so a waveform with a steadier
    envelope gets the average power it really puts on the air (P9-6)."""
    ratio: dict[str, float] = {}
    with csv_path.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            if int(r["bandwidth_hz"]) == air.params.bandwidth.value:
                ratio[r["frame"]] = float(r["papr_max_db"])

    def offset(frame: TxFrame) -> float:
        # the tone floor goes out at the OFDM frames' peak: its thresholds are stated at
        # that level already, TONE_GAIN_DB over the OFDM average (ADR-0013)
        if frame.container is Container.CONTROL:
            return -TONE_GAIN_DB if frame.floor else -ratio["control short"]
        rung = air.ladder[frame.mode]
        if rung.tone is not None:
            return -TONE_GAIN_DB
        assert rung.mode is not None
        return -ratio[f"mode {rung.mode.index}"]

    return offset


CONTROL_CSV = {
    WIDE: "bench/baselines/floor_2300.csv",
    NARROW: "bench/baselines/floor_500.csv",
}
"""``tools/bench_floor.py``'s measurements of each air's control frames, per channel."""


def control_thresholds(
    csv_path: Path,
    awgn: dict[bool, float],
    tables: dict[str, dict[int, float]],
    awgn_data: dict[int, float],
    target_fer: float = 0.10,
    tone_csv: Path = Path(TONE_CSV),
) -> dict[str, dict[bool, float]]:
    """Per-channel thresholds of the two control frames (keyed by family), interpolated from
    ``bench_floor.py``'s rows ``control short`` and ``bench_tone.py``'s ``tone-control``. A
    channel or frame the sweeps did not cover gets its AWGN value shifted by that channel's
    mean data penalty, as :func:`channel_thresholds` fills a rung it did not measure."""
    rows: list[dict[str, str]] = []
    if csv_path.exists():
        with csv_path.open(encoding="utf-8") as f:
            rows = list(csv.DictReader(f))
    points: dict[tuple[str, bool], list[tuple[float, float]]] = defaultdict(list)
    for r in rows:
        if r["frame"] != "control short":
            continue
        fer = 1.0 - int(r["decoded"]) / max(int(r["frames"]), 1)
        points[(r["channel"], False)].append((float(r["snr_3k_db"]), fer))
    for (channel, name), pts in tone_points(tone_csv).items():
        if name == TONE_CONTROL.name:
            points[(channel, True)] += pts

    def crossing(pts: list[tuple[float, float]]) -> float | None:
        prev: tuple[float, float] | None = None
        for snr, fer in sorted(pts):
            if fer <= target_fer:
                if prev is None or prev[1] == fer:
                    return snr
                s0, f0 = prev
                return s0 + (f0 - target_fer) / (f0 - fer) * (snr - s0)
            prev = (snr, fer)
        return None

    out: dict[str, dict[bool, float]] = {}
    for channel in sorted(set(tables) | {c for c, _ in points}):
        table = tables.get(channel, {})
        penalties = [table[m] - awgn_data[m] for m in table if m in awgn_data]
        shift = statistics.fmean(penalties) if penalties else 0.0
        short = crossing(points.get((channel, False), []))
        short = short if short is not None else awgn[False] + shift
        floor = crossing(points.get((channel, True), []))
        out[channel] = {False: short, True: floor if floor is not None else awgn[True] + shift}
    return out


def ideal_bps(thresholds: dict[int, float], snr_db: float, air: AirInterface = WIDE) -> float:
    """Payload rate of the fastest mode the channel supports at this SNR, ignoring every
    protocol cost — the ceiling the link layer is measured against."""
    awgn, payload = table_for(air)
    frame_s = {r.index: r.duration_s for r in air.ladder}
    best = 0.0
    for m in usable_modes(awgn, payload, frame_s):
        if thresholds.get(m, awgn[m]) <= snr_db:
            best = max(best, air.ladder[m].net_bps)
    return best


def table_for(air: AirInterface) -> tuple[dict[int, float], dict[int, float]]:
    """The measured AWGN thresholds and payloads of an air interface."""
    if air is NARROW:
        return NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES
    return AWGN_THRESHOLD_DB, PAYLOAD_BYTES


# ── one run ───────────────────────────────────────────────────────────


def run_point(
    backend: str,
    channel: str,
    snr_db: float,
    payload: bytes,
    seed: int,
    thresholds: dict[int, float] | None,
    ramp: tuple[float, float] | None = None,
    air: AirInterface = WIDE,
    rate: dict[str, float | int] | None = None,
    schedule: Callable[[float], float] | None = None,
    controls: dict[bool, float] | None = None,
    peak: bool = False,
    fading: bool = False,
    continuous: bool = False,
    link: dict[str, float | int] | None = None,
) -> dict[str, object]:
    timing = phy_timing(air.params)
    cfg = LinkConfig(max_mode=air.n_rungs - 1, rate=dict(rate or {}), **(link or {}))  # type: ignore[arg-type]
    a = LinkEngine("W4ODA", timing, cfg, seed=seed)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=seed + 1)
    if ramp is not None:
        span, period = ramp
        top, half = snr_db + span / 2, period / 2

        def schedule(t: float, top: float = top, span: float = span, half: float = half) -> float:
            phase = t % (2 * half)
            return top - span * phase / half if phase < half else top - span * (2 - phase / half)

    if backend == "phy":
        sim = two_modem_sim(
            a,
            b,
            channel=channel,
            snr_db=snr_db,
            seed=seed,
            params=air.params,
            continuous=continuous,
        )
        sim.snr_schedule = schedule
    elif fading:
        # every frame through its own stretch of the class's fade, judged at the AWGN
        # thresholds on the modem's waterfall at its effective SNR
        sim = TwoStationSim(
            a,
            b,
            snr_db=snr_db,
            seed=seed,
            thresholds=table_for(air)[0],
            snr_schedule=schedule,
            control_thresholds=control_thresholds_for(timing),
            frame_snr_offset=peak_offsets(Path(PEAK_CSV), air) if peak else None,
            fading=fading_pipe(Path(FADING_CSV), air, channel, seed),
        )
    else:
        sim = TwoStationSim(
            a,
            b,
            snr_db=snr_db,
            seed=seed,
            thresholds=thresholds or table_for(air)[0],
            snr_schedule=schedule,
            control_thresholds=controls,
            frame_snr_offset=peak_offsets(Path(PEAK_CSV), air) if peak else None,
        )

    modes: list[int] = []
    original = a._send_burst

    def wrapped() -> None:
        modes.append(min(a._recommended, a.cfg.max_mode))
        original()

    a._send_burst = wrapped  # type: ignore[method-assign]

    wall = time.perf_counter()
    a.connect("KK4XYZ")
    a.send(payload)
    a.disconnect()
    # long enough for the tone floor's 25-36 bit/s (ADR-0013): a session that is still
    # carrying data slowly is not a failed one
    seconds = sim.run(until=0.4 * len(payload) + 600.0)
    ok = sim.delivered(1) == payload
    changes = sum(1 for x, y in pairwise(modes) if x != y)
    goodput = 8 * len(sim.delivered(1)) / seconds if seconds > 0 else 0.0
    table = thresholds or table_for(air)[0]
    if peak:
        # at equal peak power a mode needs its own peak-to-average ratio on top
        offset = peak_offsets(Path(PEAK_CSV), air)
        table = {m: t - offset(TxFrame(Container.DATA, b"", mode=m)) for m, t in table.items()}
    ceiling = ideal_bps(table, snr_db, air)
    return {
        "backend": backend,
        "bandwidth_hz": air.params.bandwidth.value,
        "rate": ",".join(f"{k}={v}" for k, v in sorted((rate or {}).items())),
        "link": ",".join(f"{k}={v}" for k, v in sorted((link or {}).items())),
        "channel": channel,
        "snr_db": round(snr_db, 1),
        "snr_reference": "peak" if peak else "average",
        "pipe": (
            ("phy-continuous" if continuous else "phy")
            if backend == "phy"
            else ("fading" if fading else "logistic")
        ),
        "ramp": "" if ramp is None else f"+/-{ramp[0] / 2:.0f}dB/{ramp[1]:.0f}s",
        "bytes": len(payload),
        "seconds": round(seconds, 1),
        "goodput_bps": round(goodput, 1),
        "ideal_bps": round(ceiling, 1),
        "efficiency": round(goodput / ceiling, 3) if ceiling else "",
        "mode_min": min(modes) if modes else "",
        "mode_max": max(modes) if modes else "",
        "mode_final": modes[-1] if modes else "",
        "mode_changes": changes,
        "bursts": a.stats.bursts,
        "frames_sent": a.stats.frames_sent,
        "frames_resent": a.stats.frames_resent,
        "harq_rescues": b.stats.harq_rescues,
        "ack_timeouts": a.stats.ack_timeouts,
        "ok": int(ok),
        "wall_s": round(time.perf_counter() - wall, 1),
    }


# ── a recorded session, replayed (P6-7) ──────────────────────────────

_REPLAY_STEEP = 1.2
"""The pipe's logistic steepness, as ``aether_model.link.sim`` has it."""


def sidecar_schedule(document: dict[str, object]) -> Callable[[float], float] | None:
    """The SNR the receiver measured, frame by frame, as a function of session time:
    linear between frames, held at the ends. ``None`` when the sidecar has fewer than two
    frames with an SNR."""
    frames = document.get("frames") or []
    assert isinstance(frames, list)
    points = sorted(
        (float(f["t_s"]), float(f["snr_3k_db"]))
        for f in frames
        if isinstance(f, dict) and isinstance(f.get("snr_3k_db"), (int, float))
    )
    if len(points) < 2:
        return None
    t0 = points[0][0]
    points = [(t - t0, s) for t, s in points]

    def schedule(t: float) -> float:
        if t <= points[0][0]:
            return points[0][1]
        if t >= points[-1][0]:
            return points[-1][1]
        for (x0, y0), (x1, y1) in pairwise(points):
            if x0 <= t <= x1:
                return y0 if x1 == x0 else y0 + (y1 - y0) * (t - x0) / (x1 - x0)
        return points[-1][1]

    return schedule


def fit_penalty(observations: list[tuple[int, int, int, float]], awgn: dict[int, float]) -> float:
    """The shift of the AWGN thresholds, dB, that best explains ``(mode, frames, decoded,
    snr_db)`` observations under the pipe's logistic model: least squares over the frame
    error rates, weighted by frames, searched at a quarter of a decibel. Zero when there is
    nothing to fit."""
    usable = [(m, n, d, s) for m, n, d, s in observations if n > 0 and m in awgn]
    if not usable:
        return 0.0

    def cost(shift: float) -> float:
        total = 0.0
        for mode, frames, decoded, snr in usable:
            predicted = 1.0 - 1.0 / (1.0 + math.exp(-_REPLAY_STEEP * (snr - awgn[mode] - shift)))
            total += frames * (predicted - (1.0 - decoded / frames)) ** 2
        return total

    grid = [x / 4.0 for x in range(-24, 121)]  # −6 … +30 dB
    return min(grid, key=cost)


def sidecar_observations(document: dict[str, object]) -> list[tuple[int, int, int, float]]:
    """What the session says about each rung: the ladder's rungs when there was a Test
    session, else the data frames the receiver found, one observation each — a pre-ladder
    sidecar's OFDM mode numbers mapped onto the ladder (:func:`field_ingest.sidecar_rung`)."""
    from field_ingest import sidecar_rung

    session = document.get("session") or {}
    assert isinstance(session, dict)
    test = session.get("test") or {}
    assert isinstance(test, dict)
    rungs = [
        (rung, int(r["frames"]), int(r["decoded"]), float(r["snr_db"]))
        for r in test.get("ladder") or []
        if isinstance(r, dict)
        and isinstance(r.get("snr_db"), (int, float))
        and (rung := sidecar_rung(document, int(r["mode"]))) is not None
    ]
    if rungs:
        return rungs
    frames = document.get("frames") or []
    assert isinstance(frames, list)
    return [
        (rung, 1, int(bool(f.get("decoded"))), float(f["snr_3k_db"]))
        for f in frames
        if isinstance(f, dict)
        and f.get("kind") == "data"
        and isinstance(f.get("snr_3k_db"), (int, float))
        and (rung := sidecar_rung(document, int(f["mode"]))) is not None
    ]


def replay_sidecar(path: Path, seed: int) -> dict[str, object]:
    """One run of the engines against a recorded session."""
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("format") not in ("aether-hf-session/1", "aether-hf-session/2"):
        raise ValueError(f"{path}: not a session sidecar")
    session = document.get("session") or {}
    test = session.get("test") or {}
    air = NARROW if session.get("bandwidth_hz") == 500 else WIDE
    awgn = table_for(air)[0]
    observations = sidecar_observations(document)
    penalty = fit_penalty(observations, awgn)
    thresholds = {m: t + penalty for m, t in awgn.items()}
    controls = {k: t + penalty for k, t in control_thresholds_for(phy_timing(air.params)).items()}
    schedule = sidecar_schedule(document)
    snrs = [s for *_, s in observations] or [
        float(f["snr_3k_db"]) for f in document.get("frames") or [] if "snr_3k_db" in f
    ]
    mean_snr = statistics.mean(snrs) if snrs else 10.0
    transfers = [test.get("message") or {}, test.get("file") or {}]
    size = sum(int(t.get("bytes") or 0) for t in transfers if isinstance(t, dict))
    measured = next(
        (float(t["bps"]) for t in reversed(transfers) if isinstance(t, dict) and t.get("bps")),
        None,
    )
    counters = document.get("counters") or {}
    if size == 0:
        size = int(counters.get("bytes_delivered") or 0)
        seconds = float((document.get("audio") or {}).get("seconds") or 0.0)
        measured = 8.0 * size / seconds if size and seconds > 0 else None
    size = max(size, 512)
    payload = bytes((i * 37) % 256 for i in range(size))
    row = run_point(
        "sim", "replay", mean_snr, payload, seed, thresholds, None, air, None, schedule, controls
    )
    row["replay_of"] = path.stem
    row["penalty_db"] = penalty
    row["observations"] = len(observations)
    row["measured_bps"] = "" if measured is None else round(measured, 1)
    return row


def overrides(text: str) -> dict[str, float | int]:
    """``key=value,key=value`` as a dict of numbers."""
    out: dict[str, float | int] = {}
    for item in text.split(","):
        if item.strip():
            key, value = item.split("=", 1)
            out[key.strip()] = int(value) if value.strip().lstrip("-").isdigit() else float(value)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--backend", choices=("sim", "phy"), default="sim")
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--snr", default="-4,0,4,8,12,16,20")
    ap.add_argument("--bytes", type=int, default=8000)
    ap.add_argument("--trials", type=int, default=3)
    ap.add_argument("--ramp", action="store_true", help="triangular fade instead of a fixed SNR")
    ap.add_argument("--ramp-span", type=float, default=16.0)
    ap.add_argument("--ramp-period", type=float, default=60.0)
    ap.add_argument("--fer-csv", default="")
    ap.add_argument(
        "--bandwidth", type=int, choices=(2300, 500), default=2300, help="the air interface"
    )
    ap.add_argument(
        "--rate",
        default="",
        help="rate-controller overrides, key=value pairs separated by commas "
        "(RateController fields), to compare one controller against another",
    )
    ap.add_argument(
        "--link",
        default="",
        help="link-engine overrides, key=value pairs separated by commas (LinkConfig "
        "fields), to compare one engine against another",
    )
    ap.add_argument(
        "--replay",
        nargs="+",
        type=Path,
        default=[],
        help="session sidecars to run the engines against instead of a grid",
    )
    ap.add_argument(
        "--peak",
        action="store_true",
        help="read the SNR axis at equal transmitter peak power: each frame's SNR is the "
        "axis less its own peak-to-average ratio (bench/baselines/peak_to_average.csv)",
    )
    ap.add_argument(
        "--fading",
        action="store_true",
        help="the fading pipe: each frame through its own stretch of the class's two-ray "
        "fade, judged at its calibrated effective SNR (bench/baselines/fading_pipe.csv)",
    )
    ap.add_argument(
        "--continuous",
        action="store_true",
        help="with --backend phy: one fade for the whole session, shared by both "
        "directions, instead of a fresh channel per frame",
    )
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    if args.replay:
        return replay_main(args.replay, args.trials, args.out)

    air = NARROW if args.bandwidth == 500 else WIDE
    fer_csv = args.fer_csv or (
        "bench/baselines/phy_fer_500.csv" if air is NARROW else "bench/baselines/phy_fer.csv"
    )
    rate = overrides(args.rate)
    link = overrides(args.link)
    tables = channel_thresholds(Path(fer_csv), awgn=table_for(air)[0], air=air)
    if args.backend == "sim" and not tables:
        print(f"warning: {fer_csv} not found; every channel modelled as AWGN", flush=True)
    awgn_controls = control_thresholds_for(phy_timing(air.params))
    controls = control_thresholds(Path(CONTROL_CSV[air]), awgn_controls, tables, table_for(air)[0])
    channels = [c.strip() for c in args.channels.split(",") if c.strip()]
    snrs = [float(s) for s in args.snr.split(",") if s.strip()]
    payload = bytes((i * 37) % 256 for i in range(args.bytes))
    ramp = (args.ramp_span, args.ramp_period) if args.ramp else None

    rows: list[dict[str, object]] = []
    for channel in sorted(
        channels, key=lambda c: CHANNEL_ORDER.index(c) if c in CHANNEL_ORDER else 9
    ):
        thresholds = tables.get(channel)
        for snr in snrs:
            for trial in range(args.trials):
                row = run_point(
                    args.backend,
                    channel,
                    snr,
                    payload,
                    100 + 7 * trial,
                    thresholds,
                    ramp,
                    air,
                    rate,
                    controls=controls.get(channel, awgn_controls),
                    peak=args.peak,
                    fading=args.fading,
                    continuous=args.continuous,
                    link=link,
                )
                rows.append(row)
                print(
                    f"{channel:9s} SNR {snr:+5.1f} dB  {row['goodput_bps']:7.1f} bps "
                    f"(eff {row['efficiency']})  modes {row['mode_min']}-{row['mode_max']} "
                    f"({row['mode_changes']} changes)  resent {row['frames_resent']}  "
                    f"ok={row['ok']}  [{row['wall_s']} s wall]",
                    flush=True,
                )

    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
        print(f"\nwrote {out} ({len(rows)} rows)")
    return 0


def replay_main(sidecars: list[Path], trials: int, out: str) -> int:
    rows: list[dict[str, object]] = []
    for sidecar in sidecars:
        for trial in range(trials):
            row = replay_sidecar(sidecar, 100 + 7 * trial)
            rows.append(row)
            measured = row["measured_bps"]
            against = f"measured {measured} bps" if measured != "" else "no measured goodput"
            print(
                f"{row['replay_of']}: replayed {row['goodput_bps']} bps ({against}), "
                f"penalty {row['penalty_db']:+.2f} dB over AWGN from {row['observations']} "
                f"observations, modes {row['mode_min']}-{row['mode_max']}, ok={row['ok']}",
                flush=True,
            )
    if out and rows:
        target = Path(out)
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
        print(f"\nwrote {target} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
