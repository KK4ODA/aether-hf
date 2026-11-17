"""What a gateway that arrives partway through a caller's try notices, and what noise fakes.

For a scan that stays on a dial only when it notices a call under way (the author's question,
2026-11-17, after S4): the caller's first try, as the model's engine sends it to a station not
heard lately (the OFDM call frame, then tone-24 with the same request), passes through the
model's channel; a fresh receiver (the gateway just moved there) starts listening at a chosen
point of it, half a second after its arrival (the move's deafness), and the bench records the
first thing it notices and when:

* ``ofdm``: an OFDM candidate the stream detected (what the daemon announces as arriving);
* ``tone``: a tone frame announced as arriving (`ToneStream`, its first sync block in);
* ``report``: a frame handed over, decoded or not, with its acquisition confidence.

Each with its confidence (OFDM: ``detect_confidence``; tone arrival: statistic over the
announcement threshold; a tone report: statistic over the detection threshold). ``noise`` runs
the same receiver on noise alone and records the same events, for how often a quiet dial fakes a
notice.

    python -I notice.py tries --out tries.csv [--jobs 4] [--trials 6]
    python -I notice.py noise --out noise.csv [--minutes 10]
    python -I notice.py summary tries.csv noise.csv [--dwell 3.0]
"""

from __future__ import annotations

import argparse
import csv
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "model"))

from aether_model.channel import make_channel  # noqa: E402
from aether_model.frame.modes import NARROW, WIDE  # noqa: E402
from aether_model.link.phy import Container  # noqa: E402
from aether_model.link.granted import GrantConfig, GrantedEngine, Transmit  # noqa: E402
from aether_model.link.harness import bandwidth_capabilities, phy_timing  # noqa: E402
from aether_model.phy.pipeline import Modem  # noqa: E402
from aether_model.phy.stream import StreamingReceiver  # noqa: E402

DEAF_S = 0.5
"""The move's deafness: the card's latency and the radio settling (``move_deaf_s``)."""
LISTEN_S = 7.0
"""How long the receiver is fed after it starts listening."""
BLOCK_S = 0.02
"""The daemon's capture block."""
PRE_S = 2.0
"""Silence before the try, so the channel's fading has run."""
FIELDS = ["air", "profile", "snr", "offset", "trial", "event", "at", "confidence", "ok"]


def first_try(air: object) -> list:
    """The frames of a caller's first try to a station not heard lately (ADR-0126: H)."""
    params = air.params  # type: ignore[attr-defined]
    cfg = GrantConfig(
        max_mode=air.n_rungs - 1,  # type: ignore[attr-defined]
        capabilities=bandwidth_capabilities(params),
    )
    eng = GrantedEngine("W4ODA", phy_timing(params), cfg, seed=11)
    eng.connect("KK4XYZ-10")
    t = 0.0
    while t < 30.0:
        eng.tick(t)
        for action in eng.drain():
            if isinstance(action, Transmit):
                return action.frames
        t += 0.05
    raise RuntimeError("no try")


def render(modem: Modem, frames: list) -> np.ndarray:
    parts = []
    for f in frames:
        if f.container is Container.DATA:
            parts.append(modem.rung_burst(f.payload, f.mode, f.rv, f.follows))
        else:
            parts.append(modem.control_burst(f.payload, f.rv, floor=f.floor))
    return np.concatenate(parts)


def events(rx: StreamingReceiver, modem: Modem, x: np.ndarray, fs: float) -> list[tuple]:
    """Feed ``x`` block by block; every first sighting: (event, at_s, confidence, ok)."""
    out: list[tuple] = []
    block = int(round(BLOCK_S * fs))
    seen_ofdm: set[int] = set()
    seen_tone: set[int] = set()
    det_threshold = modem.tone_detector.threshold
    for i in range(0, len(x), block):
        reports = rx.feed(x[i : i + block])
        t = (i + block) / fs
        for p in rx._pending:  # noqa: SLF001 - what the daemon announces as an OFDM arrival
            if p.sync.start not in seen_ofdm:
                seen_ofdm.add(p.sync.start)
                out.append(("ofdm", t, p.sync.detect_confidence(modem.air), ""))
        for a in rx.arriving:
            if a.start not in seen_tone:
                seen_tone.add(a.start)
                out.append(("tone", t, a.statistic / rx.tone.announce_threshold, ""))
        for d in reports:
            if d.tone_sync is not None:
                conf = d.tone_sync.statistic / det_threshold
            elif d.frame is not None:
                conf = d.frame.sync.detect_confidence(modem.air)
            else:
                conf = 0.0
            out.append(("report", t, conf, "ok" if d.ok else "failed"))
    return out


def one_point(job: tuple) -> list[dict]:
    bw, profile, snr, offsets, trials = job
    air = WIDE if bw == 2300 else NARROW
    params = air.params
    fs = params.fs_baseband
    modem = Modem(params)
    tx = render(modem, first_try(air))
    rows = []
    for trial in range(trials):
        seed = 1000 * trial + int(snr * 10) % 1000 + (7 if bw == 500 else 0)
        audio = np.concatenate(
            (np.zeros(int(PRE_S * fs)), tx, np.zeros(int((LISTEN_S + 8.0) * fs)))
        ).astype(np.complex128)
        ch = make_channel(profile, snr_db=float(snr), fs=fs, seed=seed, signal_power=1.0)
        heard = ch.process(audio)
        for off in offsets:
            start = int((PRE_S + off + DEAF_S) * fs)
            rx = StreamingReceiver(params)
            seen = events(rx, modem, heard[start : start + int(LISTEN_S * fs)], fs)
            if not seen:
                rows.append(dict(zip(FIELDS, (bw, profile, snr, off, trial, "none", "", "", ""))))
            for ev, at, conf, ok in seen:
                rows.append(
                    dict(zip(FIELDS, (bw, profile, snr, off, trial, ev, f"{at:.2f}", f"{conf:.3f}", ok)))
                )
    return rows


def tries(args: argparse.Namespace) -> None:
    offsets = [round(-1.0 + 0.4 * k, 2) for k in range(19)]  # -1.0 … 6.2 s into the try
    jobs = [
        (bw, profile, snr, offsets, args.trials)
        for bw in (2300, 500)
        for profile in ("awgn", "moderate")
        for snr in (10.0, -10.0, -16.0)
    ]
    with open(args.out, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        w.writeheader()
        with ProcessPoolExecutor(args.jobs) as pool:
            for rows in pool.map(one_point, jobs):
                w.writerows(rows)
                f.flush()
                print(f"{rows[0]['air']} {rows[0]['profile']} {rows[0]['snr']}: done", flush=True)


def noise_chunk(job: tuple) -> list[dict]:
    bw, seconds, seed = job
    air = WIDE if bw == 2300 else NARROW
    params = air.params
    fs = params.fs_baseband
    modem = Modem(params)
    ch = make_channel("awgn", snr_db=-16.0, fs=fs, seed=seed, signal_power=1.0)
    x = ch.process(np.zeros(int(seconds * fs), dtype=np.complex128))
    rx = StreamingReceiver(params)
    return [
        dict(zip(FIELDS, (bw, "noise", "", seconds, seed, ev, f"{at:.2f}", f"{conf:.3f}", ok)))
        for ev, at, conf, ok in events(rx, modem, x, fs)
    ]


def noise(args: argparse.Namespace) -> None:
    chunk = 60.0
    jobs = [
        (bw, chunk, 100 + k) for bw in (2300, 500) for k in range(int(args.minutes * 60 / chunk))
    ]
    with open(args.out, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        w.writeheader()
        with ProcessPoolExecutor(args.jobs) as pool:
            for rows in pool.map(noise_chunk, jobs):
                w.writerows(rows)
                f.flush()
    print(f"{args.minutes} min of noise per air: {args.out}")


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("tries")
    t.add_argument("--out", required=True)
    t.add_argument("--jobs", type=int, default=4)
    t.add_argument("--trials", type=int, default=6)
    n = sub.add_parser("noise")
    n.add_argument("--out", required=True)
    n.add_argument("--jobs", type=int, default=4)
    n.add_argument("--minutes", type=float, default=10.0)
    args = ap.parse_args()
    {"tries": tries, "noise": noise}[args.cmd](args)


if __name__ == "__main__":
    main()
