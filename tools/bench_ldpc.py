"""BLER / BER vs E_s/N_0 for the TS 38.212 BG2 LDPC over AWGN (roadmap P1-1 baseline).

    python tools/bench_ldpc.py [--info 480] [--rates 1/5,1/3,1/2,2/3,3/4,5/6]
                               [--max-blocks 2000] [--target-errors 100] [--out bench/baselines/ldpc_bg2_awgn.csv]

Modulation is BPSK (E_s = E_b · R); noise is complex AWGN with variance 1/(E_s/N_0) so the
same LLR convention as the modem applies. Rates are realised with circular-buffer rate
matching (RV0), exactly as the frame codec will. Each SNR point stops after
``target-errors`` block errors or ``max-blocks`` blocks, whichever comes first, so the
low-BLER tail is a bound, not a measurement (the CSV records how many blocks ran).
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
import time
from fractions import Fraction
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import complex_normal
from aether_model.fec.nr_ldpc import RateMatcher, nr_ldpc_code, select_lifting_size
from aether_model.phy.constellation import constellation
from aether_model.waveform import Modulation


def run_point(
    info_len: int,
    rate: Fraction,
    es_n0_db: float,
    rng: np.random.Generator,
    max_blocks: int,
    target_errors: int,
    batch: int = 128,
    max_iter: int = 25,
) -> dict[str, float | int | str]:
    z = select_lifting_size(2, info_len)
    code = nr_ldpc_code(2, z)
    e = int(round(info_len / rate))
    rm = RateMatcher(code, info_len, e, rv=0)
    bpsk = constellation(Modulation.BPSK)
    noise_var = 1.0 / 10 ** (es_n0_db / 10)
    blocks = errors = bit_errors = iters_total = 0
    t0 = time.perf_counter()
    while blocks < max_blocks and errors < target_errors:
        infos = rng.integers(0, 2, (batch, code.k)).astype(np.uint8)
        infos[:, info_len:] = 0  # fillers
        llrs = np.empty((batch, code.n_full))
        for b in range(batch):
            cw = code.encode(infos[b])
            y = bpsk.map(rm.match(cw)) + math.sqrt(noise_var) * complex_normal(rng, e)
            llrs[b] = rm.recover(bpsk.llr(y, noise_var))
        hard, _, iters = code.decode(llrs, max_iter=max_iter)
        wrong = hard[:, :info_len] != infos[:, :info_len]
        bit_errors += int(wrong.sum())
        errors += int(wrong.any(axis=1).sum())
        iters_total += int(iters.sum())
        blocks += batch
    eb_n0_db = es_n0_db - 10 * math.log10(float(rate))
    return {
        "bg": 2,
        "info_bits": info_len,
        "z": z,
        "rate": str(rate),
        "e_bits": e,
        "es_n0_db": es_n0_db,
        "eb_n0_db": round(eb_n0_db, 2),
        "blocks": blocks,
        "block_errors": errors,
        "bler": errors / blocks,
        "ber": bit_errors / (blocks * info_len),
        "mean_iterations": round(iters_total / blocks, 2),
        "seconds": round(time.perf_counter() - t0, 1),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--info", type=int, default=480, help="information bits K' (incl. CRC if any)")
    ap.add_argument("--rates", default="1/5,1/3,1/2,2/3,3/4,5/6")
    ap.add_argument("--max-blocks", type=int, default=2000)
    ap.add_argument("--target-errors", type=int, default=100)
    ap.add_argument("--step", type=float, default=0.5, help="E_s/N_0 step in dB")
    ap.add_argument("--seed", type=int, default=2026)
    ap.add_argument("--out", default="bench/baselines/ldpc_bg2_awgn.csv")
    args = ap.parse_args()

    # Sweep from ~2 dB below the Shannon limit for the rate until BLER < 1e-3 or 6 dB above.
    rows: list[dict[str, float | int | str]] = []
    rng = np.random.default_rng(args.seed)
    for rate_s in args.rates.split(","):
        rate = Fraction(rate_s)
        shannon_eb = 10 * math.log10((2 ** (2 * float(rate)) - 1) / (2 * float(rate)))
        start = (
            math.floor((shannon_eb + 10 * math.log10(float(rate)) - 1.0) / args.step) * args.step
        )
        es = start
        while es < start + 8.0:
            r = run_point(args.info, rate, es, rng, args.max_blocks, args.target_errors)
            rows.append(r)
            print(
                f"K'={r['info_bits']} Z={r['z']} R={r['rate']:>3} Es/N0={es:+5.1f} dB "
                f"Eb/N0={r['eb_n0_db']:+5.2f} dB  BLER={r['bler']:.4f} BER={r['ber']:.2e} "
                f"iters={r['mean_iterations']:5.2f}  ({r['blocks']} blocks, {r['seconds']} s)",
                flush=True,
            )
            if r["bler"] < 1e-3 and r["blocks"] >= args.max_blocks:
                break
            es += args.step
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
