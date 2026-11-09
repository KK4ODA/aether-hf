"""Each rung's frames through a receive cut, decoded by the model (the receive-filter review, Gate 1).

    python -I tools/bench_cut.py OUT.csv [--frames 30] [--jobs 4] [--margin 3] [--model DIR]
        [--receiver KEY=VALUE ...] [--only CUT] [--family ofdm|tone]

A rung's data frames, each at its own power (a tone frame 5.5 dB over the OFDM average), go
through AWGN at the rung's AWGN threshold plus ``--margin``, then through a receive response
applied to signal and noise alike (a radio's filter: an eighth-order cut 10 or 20 dB down at an
edge of the signal, or Saturday's low-pass at 1600 Hz), then the audio chain's floor 50 dB under
the band's noise. The model's ``Modem.decode_buffer`` finds and decodes them; the CSV counts the
frames detected and decoded and the SNR the receiver read. ``--model`` runs another tree's model
(a scratch copy with a receiver change), the same frames and noise; ``--receiver`` sets one of
``FrameReceiver``'s settings for the run instead (``noise_shape=false``), or the tone module's
(``tone.NOISE_SHAPE=false``).
"""

from __future__ import annotations

import argparse
import csv
import sys
from multiprocessing import Pool
from pathlib import Path

import numpy as np

MODEL = Path(__file__).resolve().parents[1] / "model"
if "--model" in sys.argv:
    MODEL = Path(sys.argv[sys.argv.index("--model") + 1])
sys.path.insert(0, str(MODEL))

from aether_model.channel import make_channel  # noqa: E402
from aether_model.frame.modes import air_interface  # noqa: E402
from aether_model.link.rate import AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB  # noqa: E402
from aether_model.phy.pipeline import Modem  # noqa: E402
from aether_model.waveform import NARROW_500, WIDE_2300  # noqa: E402

ORDER = 8
EDGES = {2300: (360.0, 2640.0), 500: (1260.0, 1740.0)}


def corner(edge, depth, side):
    r = (10 ** (depth / 10) - 1) ** (1 / (2 * ORDER))
    return edge / r if side == "high" else edge * r


def cuts(hz):
    lo, hi = EDGES[hz]
    out = {"uncut": None}
    for d in (10.0, 20.0):
        out[f"top{int(d)}"] = ("low", corner(hi, d, "high"))
        out[f"bottom{int(d)}"] = ("high", corner(lo, d, "low"))
    out["saturday"] = ("low", 1600.0)
    return out


def gain(f_audio, cut):
    if cut is None:
        return np.ones_like(f_audio)
    kind, fc = cut
    f = np.maximum(np.abs(f_audio), 1e-3)
    # a low-pass at fc, or a high-pass
    ratio = f / fc if kind == "low" else fc / f
    return np.sqrt(1 / (1 + ratio ** (2 * ORDER)))


def configure(settings: tuple[str, ...]) -> None:
    """Set the receivers' settings for this process: ``KEY=VALUE`` a ``FrameReceiver`` class
    setting, ``tone.KEY=VALUE`` a constant of the tone module."""
    from aether_model.phy import tone
    from aether_model.phy.rx import FrameReceiver

    for setting in settings:
        key, value = setting.split("=", 1)
        target: object = FrameReceiver
        if key.startswith("tone."):
            target, key = tone, key.removeprefix("tone.")
        old = getattr(target, key)
        new = type(old)(value) if not isinstance(old, bool) else value.lower() in ("1", "true")
        setattr(target, key, new)


def family(hz: int, rung: int) -> str:
    air = air_interface(WIDE_2300 if hz == 2300 else NARROW_500)
    return "ofdm" if air.ladder[rung].mode is not None else "tone"


def run(job):
    hz, cut_name, rung, frames, margin, seed = job[:6]
    configure(job[6] if len(job) > 6 else ())
    params = WIDE_2300 if hz == 2300 else NARROW_500
    fs = params.fs_baseband
    modem = Modem(params)
    air = air_interface(params)
    thresholds = AWGN_THRESHOLD_DB if hz == 2300 else NARROW_AWGN_THRESHOLD_DB
    snr = thresholds[rung] + margin
    cut = cuts(hz)[cut_name]
    rng = np.random.default_rng(seed)
    detected = decoded = 0
    readings = []
    for _ in range(frames):
        payload = rng.integers(0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8).tobytes()
        burst = modem.rung_burst(payload, rung)
        # each burst at its own power: a tone frame goes 5.5 dB over the OFDM average
        lead = int(rng.uniform(0.3, 0.6) * fs)
        x = np.concatenate((np.zeros(lead, complex), burst, np.zeros(int(0.8 * fs), complex)))
        y = make_channel(
            "awgn", snr_db=snr, fs=fs, seed=int(rng.integers(0, 2**31)), signal_power=1.0
        ).process(x)
        if cut is not None:
            Y = np.fft.fft(y)
            f_bb = np.fft.fftfreq(len(y), 1 / fs)
            y = np.fft.ifft(Y * gain(f_bb + params.centre_hz, cut))
        noise_3k = 10 ** (-snr / 10)
        floor = noise_3k * 1e-5 * fs / 3000
        y = y + (rng.standard_normal(len(y)) + 1j * rng.standard_normal(len(y))) * np.sqrt(
            floor / 2
        )
        out = modem.decode_buffer(y, max_frames=3)
        near = [d for d in out if abs(d.start - lead) < params.symbol_samples * 4 + 200]
        if near:
            detected += 1
        good = [d for d in near if d.ok and d.payload[: len(payload)] == payload]
        if good:
            decoded += 1
            d = good[0]
            if d.frame is not None:
                readings.append(d.frame.snr_carrier_db)
            elif d.tone is not None and hasattr(d.tone, "snr_db"):
                readings.append(d.tone.snr_db)
    kind = "ofdm" if air.ladder[rung].mode is not None else "tone"
    return {
        "hz": hz,
        "cut": cut_name,
        "rung": rung,
        "kind": kind,
        "snr": round(snr, 1),
        "frames": frames,
        "detected": detected,
        "decoded": decoded,
        "snr_read": round(float(np.median(readings)), 2) if readings else "",
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--margin", type=float, default=3.0)
    ap.add_argument("--model", type=Path, help="another tree's model directory")
    ap.add_argument(
        "--receiver",
        action="append",
        default=[],
        metavar="KEY=VALUE",
        help="a FrameReceiver setting (noise_shape=false) or a tone one (tone.NOISE_SHAPE=false)",
    )
    ap.add_argument("--only", help="run only the cuts whose name contains this")
    ap.add_argument("--family", choices=("ofdm", "tone"), help="run only this family's rungs")
    args = ap.parse_args()
    rungs = {2300: [1, 3, 5, 7, 10, 13, 16, 19], 500: [1, 2, 3, 4, 5, 8, 11, 14]}
    # the same seed for a rung across cuts: the same frames and noise
    jobs = [
        (hz, cut_name, r, args.frames, args.margin, 1000 * hz + r, tuple(args.receiver))
        for hz, rs in rungs.items()
        for cut_name in cuts(hz)
        for r in rs
        if not args.only or args.only in cut_name
        if not args.family or family(hz, r) == args.family
    ]
    with open(args.out, "w", newline="") as f, Pool(args.jobs) as pool:
        w = None
        for row in pool.imap_unordered(run, jobs):
            if w is None:
                w = csv.DictWriter(f, fieldnames=list(row))
                w.writeheader()
            w.writerow(row)
            f.flush()
            print(row, flush=True)


if __name__ == "__main__":
    main()
