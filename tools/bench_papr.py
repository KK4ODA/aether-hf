"""PAPR study: how much link margin peak reduction is actually worth (roadmap P2-4).

    python tools/bench_papr.py [--frames 8] [--modes 0,4,8,13]
                               [--targets 7,6,5,4] [--out bench/baselines/papr.csv]

An SSB transmitter is driven at a fixed *peak* — the operator raises drive until ALC just
starts acting — so average power, and therefore link margin, is peak power minus PAPR. The
naive conclusion is that every dB of PAPR removed is a dB of link gain. It is not, and this
benchmark measures the three things that decide it:

1. **PAPR and EVM** per technique. Clipping is itself distortion, so it buys peak reduction
   with in-band error that caps the SNR the highest modes can reach.
2. **Delivered SNR through a saturating PA, subject to a splatter limit.** Out-of-band
   regrowth, not EVM, is what really limits drive on a shared HF band: you must not splash
   into the neighbouring QSO. Clip-*and-filter* keeps the splatter in check, which is what
   lets it drive harder. Reported for a soft PA (p = 2) and a harder, ALC-like one (p = 5).
3. **End-to-end decoding**, which confirms where the EVM ceiling actually bites.

Columns: metric, technique, target_papr_db, pa_smoothness, splatter_limit_db, mode, snr_db,
papr_db, evm_db, oob_db, avg_power_rel_psat_db, delivered_snr_db, gain_db, decoded, frames.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import SaturatingPa, make_channel
from aether_model.frame.modes import MODES
from aether_model.phy.ofdm import OfdmModulator
from aether_model.phy.papr import (
    ClipAndFilter,
    ToneReservation,
    ccdf,
    evm_db,
    out_of_band_db,
    papr_db,
)
from aether_model.phy.pipeline import Modem
from aether_model.phy.tx import FrameTransmitter
from aether_model.waveform import WIDE_2300 as P

CCDF_LEVELS = [4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]


def make_burst(modem: Modem, mode_idx: int, frames: int, rng: np.random.Generator) -> np.ndarray:
    """Raw frames — the study supplies its own peak reduction, so the transmitter's must be
    off or every measurement would be of a twice-clipped waveform."""
    out = []
    for _ in range(frames):
        n = modem.payload_bytes(MODES[mode_idx])
        payload = rng.integers(0, 256, n, dtype=np.uint8).tobytes()
        out.append(modem.data_burst(payload, MODES[mode_idx]))
    return np.concatenate(out)


def best_delivered_snr(
    signal: np.ndarray, noise_power: float, splatter_limit_db: float, smoothness: float
) -> tuple[float, float, float] | None:
    """Maximise receiver SNR over drive level for a PA of fixed saturation amplitude,
    subject to the splatter limit. Returns (snr_db, avg_power_rel_psat_db, oob_db)."""
    x = signal / np.sqrt(np.mean(np.abs(signal) ** 2))
    best: tuple[float, float, float] | None = None
    for drive_db in np.arange(-16.0, 8.0, 0.25):
        g = 10 ** (drive_db / 20)
        pa = SaturatingPa(1.0, smoothness)
        y = pa.process(g * x)
        oob = out_of_band_db(y, P)
        if oob > splatter_limit_db:
            continue
        alpha = np.vdot(g * x, y) / np.vdot(g * x, g * x)
        distortion = float(np.mean(np.abs(y - alpha * g * x) ** 2))
        wanted = float(np.mean(np.abs(alpha * g * x) ** 2))
        snr = 10 * np.log10(wanted / (noise_power + distortion))
        if best is None or snr > best[0]:
            best = (float(snr), float(10 * np.log10(np.mean(np.abs(y) ** 2))), float(oob))
    return best


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--frames", type=int, default=8)
    ap.add_argument("--modes", default="0,4,8,13")
    ap.add_argument("--targets", default="7,6,5,4")
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    modem = Modem(P)
    modem.tx = FrameTransmitter(P, papr_reduction=False)
    rng = np.random.default_rng(7)
    modes = [int(m) for m in args.modes.split(",")]
    targets = [float(t) for t in args.targets.split(",")]
    rows: list[dict[str, object]] = []

    def add(**kw: object) -> None:
        rows.append(kw)

    # ── 1. PAPR / EVM / splatter per technique ────────────────────────
    burst = make_burst(modem, 4, args.frames, rng)
    reference = ClipAndFilter(P, target_papr_db=99.0, iterations=1).process(burst)
    print("technique          PAPR    EVM      splatter")
    print(
        f"  {'none':16s} {papr_db(reference):5.2f}      —     {out_of_band_db(reference, P):6.1f}"
    )
    add(
        metric="waveform",
        technique="none",
        papr_db=round(papr_db(reference), 2),
        oob_db=round(out_of_band_db(reference, P), 1),
    )
    variants: dict[str, np.ndarray] = {"none": reference}
    for t in targets:
        y = ClipAndFilter(P, target_papr_db=t, iterations=4).process(burst)
        variants[f"clip{t:g}"] = y
        print(
            f"  {'clip+filter t=' + format(t, 'g'):16s} {papr_db(y):5.2f}  "
            f"{evm_db(reference, y):6.1f}    {out_of_band_db(y, P):6.1f}"
        )
        add(
            metric="waveform",
            technique="clip_and_filter",
            target_papr_db=t,
            papr_db=round(papr_db(y), 2),
            evm_db=round(evm_db(reference, y), 1),
            oob_db=round(out_of_band_db(y, P), 1),
        )
    for level, prob in zip(CCDF_LEVELS, ccdf(reference, CCDF_LEVELS), strict=True):
        add(metric="ccdf", technique="none", snr_db=level, decoded=float(prob))

    # tone reservation, per symbol body (its cost is payload, not EVM)
    mod = OfdmModulator(P)
    bodies = []
    for _ in range(200):
        vals = mod.symbol_values(rng.standard_normal(42) + 1j * rng.standard_normal(42))
        bodies.append(mod.to_time(vals)[P.cp_samples : P.cp_samples + P.fft_size])
    plain = float(np.mean([papr_db(b) for b in bodies]))
    print(f"\ntone reservation (symbol bodies, plain = {plain:.2f} dB):")
    for n_res in (2, 4, 8):
        tr = ToneReservation(P, n_reserved=n_res, target_papr_db=5.0, iterations=10)
        got = float(np.mean([papr_db(tr.process_symbol(b)) for b in bodies]))
        print(f"  reserve {n_res}: {got:5.2f} dB ({plain - got:+.2f}), costs {tr.payload_cost:.1%}")
        add(
            metric="tone_reservation",
            technique=f"reserve{n_res}",
            papr_db=round(got, 2),
            gain_db=round(plain - got, 2),
            evm_db=0.0,
            avg_power_rel_psat_db=round(-100 * tr.payload_cost, 1),
        )

    # ── 2. delivered SNR through the PA under a splatter limit ────────
    for smoothness in (2.0, 5.0):
        for limit in (-45.0, -40.0, -35.0):
            base = best_delivered_snr(reference, 10**-2.0, limit, smoothness)
            if base is None:
                continue
            print(f"\nPA p={smoothness:g}, splatter <= {limit:.0f} dB   (noise -20 dB)")
            print(f"  {'none':16s} SNR {base[0]:6.2f}  avg {base[1]:+6.2f} dB")
            add(
                metric="delivered",
                technique="none",
                pa_smoothness=smoothness,
                splatter_limit_db=limit,
                delivered_snr_db=round(base[0], 2),
                avg_power_rel_psat_db=round(base[1], 2),
                oob_db=round(base[2], 1),
                gain_db=0.0,
            )
            for t in targets:
                got = best_delivered_snr(variants[f"clip{t:g}"], 10**-2.0, limit, smoothness)
                if got is None:
                    continue
                print(
                    f"  {'clip t=' + format(t, 'g'):16s} SNR {got[0]:6.2f}  "
                    f"avg {got[1]:+6.2f} dB  gain {got[0] - base[0]:+5.2f} dB"
                )
                add(
                    metric="delivered",
                    technique="clip_and_filter",
                    target_papr_db=t,
                    pa_smoothness=smoothness,
                    splatter_limit_db=limit,
                    delivered_snr_db=round(got[0], 2),
                    avg_power_rel_psat_db=round(got[1], 2),
                    oob_db=round(got[2], 1),
                    gain_db=round(got[0] - base[0], 2),
                )

    # ── 3. where the EVM ceiling bites, end to end ────────────────────
    print("\nend-to-end decoding (AWGN, no PA): does the clipping distortion cost frames?")
    checks = {0: -4.0, 4: 2.0, 8: 7.0, 13: 18.0}
    for mode_idx in modes:
        snr = checks.get(mode_idx, 10.0)
        for t in [None, *targets]:
            cf = None if t is None else ClipAndFilter(P, target_papr_db=t, iterations=4)
            r = np.random.default_rng(1000 + mode_idx)
            ok = 0
            trials = 10
            for k in range(trials):
                n = modem.payload_bytes(MODES[mode_idx])
                payload = r.integers(0, 256, n, dtype=np.uint8).tobytes()
                bb = modem.data_burst(payload, MODES[mode_idx])  # transmitter clipping off
                if cf is not None:
                    bb = cf.process(bb)
                y = make_channel(
                    "awgn",
                    snr_db=snr,
                    fs=P.fs_baseband,
                    seed=500 + k,
                    signal_power=1.0,
                    cfo_hz=float(r.uniform(-60, 60)),
                ).process(np.concatenate((np.zeros(900), bb, np.zeros(900))))
                res = modem.decode_buffer(y)
                ok += int(len(res) == 1 and res[0].payload == payload)
            label = "none" if t is None else f"t={t:g}"
            print(f"  mode {mode_idx:2d} @ {snr:+5.1f} dB  clip {label:6s}: {ok}/{trials}")
            add(
                metric="decode",
                technique="none" if t is None else "clip_and_filter",
                target_papr_db=t,
                mode=mode_idx,
                snr_db=snr,
                decoded=ok,
                frames=trials,
            )

    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        fields = [
            "metric",
            "technique",
            "target_papr_db",
            "pa_smoothness",
            "splatter_limit_db",
            "mode",
            "snr_db",
            "papr_db",
            "evm_db",
            "oob_db",
            "avg_power_rel_psat_db",
            "delivered_snr_db",
            "gain_db",
            "decoded",
            "frames",
        ]
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
            w.writeheader()
            w.writerows(rows)
        print(f"\nwrote {out} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
