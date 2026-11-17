"""Can a receiver started partway through a try read it, and what must it be given before?

A scanning gateway restarts its receiver when it moves (ADR-0129 §2.3), so nothing heard on the
dial it left is answered on the one it reached. A fresh receiver has no audio from before it
started, so a try that began on the new dial before the gateway arrived is only noticed, by its
middle or end sync block (`notice.py`), never read: the planted calls of `02-sense-and-stay.md` §5
were read at their first try when the restart was taken out, at their second with it.

This bench starts the receiver at a chosen point of the caller's real first try (the move's half
second of deafness after the arrival), as `notice.py` does, three ways:

* ``fresh``: as built, a new receiver from the arrival on;
* ``zeros``: a new receiver given ``--lead`` seconds of silence before the arrival, so a frame that
  began before it has somewhere to begin;
* ``noise``: a new receiver given ``--lead`` seconds of another dial's noise before the arrival
  (the channel's noise at the same power, another draw), as a receiver not restarted would have.

and records whether the try's tone frame decoded within the listening time.

    python -I prefix.py --out prefix.csv [--jobs 2] [--trials 4] [--lead 6.0]
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
sys.path.insert(0, str(Path(__file__).resolve().parent))

from aether_model.channel import make_channel  # noqa: E402
from aether_model.frame.modes import NARROW, WIDE  # noqa: E402
from aether_model.phy.pipeline import Modem  # noqa: E402
from aether_model.phy.stream import StreamingReceiver  # noqa: E402
from notice import DEAF_S, LISTEN_S, PRE_S, first_try, render  # noqa: E402

FIELDS = ["air", "profile", "snr", "offset", "trial", "variant", "decoded", "at"]
OFFSETS = [1.4, 1.8, 2.2, 2.6, 3.0, 3.4, 3.8, 4.2, 4.6]
"""Seconds into the try the gateway arrives: the tone frame starts at 1.05 s, so these begin
0.85 s to 4.05 s into it (after the deafness), where a fresh receiver can only notice it."""


def decodes(rx: StreamingReceiver, x: np.ndarray, fs: float, skip: int) -> float | None:
    """Feed ``x`` in the daemon's 20 ms blocks; when the first frame decoded, in seconds after
    the first ``skip`` samples (the lead), or None."""
    block = int(round(0.02 * fs))
    for i in range(0, len(x), block):
        for d in rx.feed(x[i : i + block]):
            if d.ok:
                return (i + block - skip) / fs
    return None


def one_point(job: tuple) -> list[dict]:
    bw, profile, snr, trials, lead = job
    air = WIDE if bw == 2300 else NARROW
    params = air.params
    fs = params.fs_baseband
    modem = Modem(params)
    tx = render(modem, first_try(air))
    rows = []
    n_lead = int(round(lead * fs))
    for trial in range(trials):
        seed = 1000 * trial + int(snr * 10) % 1000 + (7 if bw == 500 else 0)
        audio = np.concatenate(
            (np.zeros(int(PRE_S * fs)), tx, np.zeros(int((LISTEN_S + 8.0) * fs)))
        ).astype(np.complex128)
        heard = make_channel(profile, snr_db=float(snr), fs=fs, seed=seed, signal_power=1.0).process(
            audio
        )
        other = make_channel(
            profile, snr_db=float(snr), fs=fs, seed=seed + 50_000, signal_power=1.0
        ).process(np.zeros(n_lead, dtype=np.complex128))
        for off in OFFSETS:
            start = int((PRE_S + off + DEAF_S) * fs)
            after = heard[start : start + int(LISTEN_S * fs)]
            for variant, before in (
                ("fresh", np.zeros(0, dtype=np.complex128)),
                ("zeros", np.zeros(n_lead, dtype=np.complex128)),
                ("noise", other),
            ):
                at = decodes(
                    StreamingReceiver(params), np.concatenate((before, after)), fs, len(before)
                )
                rows.append(
                    dict(
                        zip(
                            FIELDS,
                            (bw, profile, snr, off, trial, variant, at is not None,
                             "" if at is None else f"{at:.2f}"),
                        )
                    )
                )
    return rows


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--trials", type=int, default=4)
    ap.add_argument("--lead", type=float, default=6.0)
    args = ap.parse_args()
    jobs = [
        (bw, profile, snr, args.trials, args.lead)
        for bw in (2300, 500)
        for profile in ("awgn", "moderate")
        for snr in (10.0, -10.0)
    ]
    with open(args.out, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        w.writeheader()
        with ProcessPoolExecutor(args.jobs) as pool:
            for rows in pool.map(one_point, jobs):
                w.writerows(rows)
                f.flush()
                print(f"{rows[0]['air']} {rows[0]['profile']} {rows[0]['snr']}: done", flush=True)


if __name__ == "__main__":
    main()
