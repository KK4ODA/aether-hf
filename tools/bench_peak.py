"""Peak-to-average ratio of every frame as the modem transmits it (P9-6).

    python tools/bench_peak.py [--frames 24] [--out bench/baselines/peak_to_average.csv]
    python tools/bench_peak.py --table     # the mode tables at equal peak power

An SSB transmitter is driven to a fixed *peak*: the operator raises the drive until the ALC
just starts to act, so the average power a frame puts on the air — and every decibel of the
SNR the far end measures — is that peak less the frame's peak-to-average ratio. The benches
compare modes at equal *average* power, which credits nothing to a waveform with a steadier
envelope; this measures what each frame gives away, so the benches can also be read at equal
peak power (``bench_link.py --peak``, and ``--table`` here for the mode tables).

For every OFDM mode on either air's ladder, the ordinary control frame, and the tone
floor's frames (ADR-0013), ``--frames`` frames with random payloads are rendered by the
modem's own transmitter — ADR-0004's peak reduction included — and the envelope power of
the complex baseband, which is the RF envelope of the SSB signal, is ranked against its
mean. Three statistics: the highest sample of all (the
ALC's worst case), and the levels exceeded by one sample in ten thousand and one in a
thousand (a limiter's and a slow ALC's view). References: a steady tone is 0 dB, two equal
tones 3 dB.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import TONE_CONTROL, air_interface
from aether_model.phy import tone
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.waveform import NARROW_500, WIDE_2300


def envelope_stats(bursts: list[np.ndarray]) -> tuple[float, float, float]:
    """(max, 99.99 %, 99.9 %) of instantaneous envelope power over mean power, in dB, each
    frame referred to its own mean so a level difference between frames cannot pass for a
    peak."""
    rel = np.concatenate([np.abs(b) ** 2 / np.mean(np.abs(b) ** 2) for b in bursts])
    q = np.quantile(rel, [1.0, 0.9999, 0.999])
    return tuple(float(10 * np.log10(v)) for v in q)  # type: ignore[return-value]


def table(csv_path: Path) -> int:
    """Every rung's 10 % FER threshold per channel, at equal average power (as measured) and
    at equal peak power (plus the frame's own peak-to-average ratio; for the tone floor,
    which goes out at the OFDM peak, plus its gain over the OFDM average)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from aether_model.frame.modes import NARROW, WIDE
    from bench_link import channel_thresholds, table_for

    ratio: dict[tuple[int, str], float] = {}
    with csv_path.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            ratio[(int(r["bandwidth_hz"]), r["frame"])] = float(r["papr_max_db"])
    for air, fer in ((WIDE, "phy_fer.csv"), (NARROW, "phy_fer_500.csv")):
        bw = air.params.bandwidth.value
        tables = channel_thresholds(Path("bench/baselines") / fer, awgn=table_for(air)[0], air=air)
        channels = [c for c in ("awgn", "good", "moderate", "poor") if c in tables]
        print(f"\n{bw} Hz: threshold at equal average / equal peak power (dB, 3 kHz)")
        print("  mode  papr  " + "  ".join(f"{c:>13s}" for c in channels))
        for m in sorted(table_for(air)[0]):
            rung = air.ladder[m]
            if rung.tone is not None:
                papr = tone.TONE_GAIN_DB
            else:
                assert rung.mode is not None
                papr = ratio[(bw, f"mode {rung.mode.index}")]
            cells = "  ".join(f"{tables[c][m]:6.1f}/{tables[c][m] + papr:6.1f}" for c in channels)
            print(f"  {m:4d}  {papr:4.1f}  {cells}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--frames", type=int, default=24)
    ap.add_argument("--seed", type=int, default=5)
    ap.add_argument("--out", default="")
    ap.add_argument(
        "--table",
        action="store_true",
        help="print the mode tables at equal peak power from the committed baselines",
    )
    args = ap.parse_args()
    if args.table:
        return table(Path("bench/baselines/peak_to_average.csv"))

    rng = np.random.default_rng(args.seed)
    rows: list[dict[str, object]] = []
    for params in (WIDE_2300, NARROW_500):
        air = air_interface(params)
        modem = Modem(params)
        cases = [
            (f"mode {r.mode.index}", r.mode, FrameHeader(FrameType.DATA, r.mode.index), air.long)
            for r in air.ladder
            if r.mode is not None
        ]
        cases.append(("control short", air.control_mode, FrameHeader(FrameType.CONTROL), air.short))
        for label, mode, header, layout in cases:
            codec = modem.codec(mode, layout)  # type: ignore[arg-type]
            bursts = []
            for _ in range(args.frames):
                payload = bytes(rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8))
                bursts.append(modem.tx.baseband(header, layout, codec.encode(payload, 0)))
            peak, p4, p3 = envelope_stats(bursts)
            row = {
                "bandwidth_hz": params.bandwidth.value,
                "frame": label,
                "modulation": mode.modulation.name,  # type: ignore[union-attr]
                "layout": layout.name,
                "papr_max_db": round(peak, 2),
                "papr_9999_db": round(p4, 2),
                "papr_999_db": round(p3, 2),
                "frames": args.frames,
            }
            rows.append(row)
            print(
                f"{params.bandwidth.value:5d} Hz  {label:14s} {row['modulation']:8s} "
                f"{layout.name:11s} max {peak:5.2f}  1e-4 {p4:5.2f}  1e-3 {p3:5.2f} dB",
                flush=True,
            )
        # the tone floor from a generator of its own, so the OFDM frames' random payloads
        # are the ones the table has always had
        tone_rng = np.random.default_rng(args.seed + params.bandwidth.value)
        for kind in (*air.tone_data, TONE_CONTROL):
            bursts = [
                tone.burst(
                    kind, bytes(tone_rng.integers(0, 256, kind.payload_bytes, dtype=np.uint8))
                )
                for _ in range(args.frames)
            ]
            edge = kind.num.edge_samples
            peak, p4, p3 = envelope_stats([b[edge:-edge] for b in bursts])
            rows.append(
                {
                    "bandwidth_hz": params.bandwidth.value,
                    "frame": kind.name,
                    "modulation": f"FSK{kind.num.tones}",
                    "layout": "tone",
                    "papr_max_db": round(peak, 2),
                    "papr_9999_db": round(p4, 2),
                    "papr_999_db": round(p3, 2),
                    "frames": args.frames,
                }
            )
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
            w.writeheader()
            w.writerows(rows)
        print(f"\nwrote {out} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
