"""Generate FEC cross-validation vectors for the Rust core (roadmap P3-1).

    python tools/make_fec_vectors.py [--out core/aether-fec/tests/data/fec_vectors.json]

ADR-0001 makes the Python model the specification and the Rust core the shipped
implementation, and requires the two to stay bit-exact. This writes what "bit-exact" means
for the FEC: for every (base graph, lifting size) the mode table actually uses, the exact
codeword the model produces, the exact rate-matched output for each redundancy version, and
the exact CRC remainders. `core/aether-fec/tests/model_vectors.rs` fails if the core
disagrees with any of it.

Bit arrays are packed MSB-first into hex so the file stays small enough to read in a diff.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.fec.crc import CRC6, CRC11, CRC16, CRC24A, CRC24B, CRC24C
from aether_model.fec.nr_ldpc import FILLER_LLR, RateMatcher, nr_ldpc_code
from aether_model.frame.modes import LONG, MODES


def pack(bits: np.ndarray) -> str:
    """Bits, MSB-first, as hex. The length is carried separately: the last byte may be padded."""
    return np.packbits(np.asarray(bits, dtype=np.uint8)).tobytes().hex()


def pattern(n: int, seed: int) -> np.ndarray:
    """A deterministic, non-trivial bit pattern — a fixed RNG rather than something
    structured, so a transcription error cannot accidentally satisfy the test."""
    return np.random.default_rng(seed).integers(0, 2, n).astype(np.uint8)


def crc_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for crc in (CRC24A, CRC24B, CRC24C, CRC16, CRC11, CRC6):
        for length in (0, 1, 7, 64, 200, 1023):
            bits = pattern(length, 1000 + length)
            out.append(
                {
                    "crc": crc.name,
                    "poly": crc.poly,
                    "width": crc.width,
                    "input_len": length,
                    "input": pack(bits),
                    "remainder": pack(crc.remainder(bits)),
                }
            )
    return out


def code_parameters() -> list[tuple[int, int, int]]:
    """(base graph, Z, K') actually used by the mode table, plus a couple of extremes."""
    seen: dict[tuple[int, int, int], None] = {}
    for mode in MODES:
        seen[(mode.base_graph(LONG), mode.lifting_size(LONG), mode.info_bits(LONG))] = None
    for extra in ((1, 32, 22 * 32), (2, 2, 10 * 2), (1, 384, 22 * 384), (2, 384, 10 * 384)):
        seen[extra] = None
    return list(seen)


def ldpc_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for bg, z, info_len in code_parameters():
        code = nr_ldpc_code(bg, z)
        info = np.zeros(code.k, dtype=np.uint8)
        info[:info_len] = pattern(info_len, bg * 100003 + z)
        codeword = code.encode(info)
        assert code.syndrome_ok(codeword)
        out.append(
            {
                "bg": bg,
                "z": z,
                "k": int(code.k),
                "n_full": int(code.n_full),
                "n_cb": int(code.n_cb),
                "info_len": info_len,
                "info": pack(info),
                "codeword": pack(codeword),
            }
        )
    return out


def rate_match_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for mode in MODES:
        bg = mode.base_graph(LONG)
        z = mode.lifting_size(LONG)
        info_len = mode.info_bits(LONG)
        e = mode.coded_bits(LONG)
        code = nr_ldpc_code(bg, z)
        info = np.zeros(code.k, dtype=np.uint8)
        info[:info_len] = pattern(info_len, mode.index * 7919 + 11)
        codeword = code.encode(info)
        for rv in range(4):
            matcher = RateMatcher(code, info_len, e, rv=rv)
            out.append(
                {
                    "mode": mode.index,
                    "mode_name": mode.name,
                    "bg": bg,
                    "z": z,
                    "info_len": info_len,
                    "e": e,
                    "rv": rv,
                    "info": pack(info),
                    "matched": pack(matcher.match(codeword)),
                    "positions_head": [int(p) for p in matcher.positions[:16]],
                }
            )
    return out


def decode_cases() -> list[dict]:  # type: ignore[type-arg]
    """Hard-error cases. Float arithmetic is not required to be bit-identical across
    languages, so what is pinned is the outcome: the decoder must recover exactly these
    information bits from exactly these sign-flipped inputs."""
    out = []
    for bg, z, info_len in [(2, 30, 232), (2, 52, 392), (1, 176, 3528)]:
        code = nr_ldpc_code(bg, z)
        info = np.zeros(code.k, dtype=np.uint8)
        info[:info_len] = pattern(info_len, bg * 31 + z)
        codeword = code.encode(info)
        magnitude = 2.0
        llr = np.where(codeword == 1, -magnitude, magnitude).astype(np.float64)
        llr[info_len : code.k] = FILLER_LLR
        flipped = list(range(0, code.n_full, 41))
        for position in flipped:
            if position < info_len or position >= code.k:  # never corrupt a filler
                llr[position] = -llr[position]
        _hard, converged, iters = code.decode(llr, max_iter=40, alpha=0.8)
        out.append(
            {
                "bg": bg,
                "z": z,
                "info_len": info_len,
                "magnitude": magnitude,
                "flipped": [p for p in flipped if p < info_len or p >= code.k],
                "info": pack(info),
                "codeword": pack(codeword),
                "converged": bool(converged[0]),
                "model_iterations": int(iters[0]),
            }
        )
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-fec/tests/data/fec_vectors.json")
    args = ap.parse_args()

    document = {
        "note": "Generated by tools/make_fec_vectors.py from the Python reference model. "
        "Bit arrays are packed MSB-first into hex; use the accompanying length.",
        "crc": crc_cases(),
        "ldpc": ldpc_cases(),
        "rate_match": rate_match_cases(),
        "decode": decode_cases(),
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    counts = {k: len(v) for k, v in document.items() if isinstance(v, list)}
    print(f"wrote {out} ({counts})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
