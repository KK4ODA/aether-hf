"""Payload ↔ constellation-symbol codec for one (mode, layout) pair (roadmap P1-3).

    bytes ─► bits ─► CRC-24 ─► fillers ─► LDPC (BG per §7.2.2) ─► rate matching (RV) ─►
    coprime-stride interleaver ─► Gray-labelled constellation ─► QAM symbols (time-major)

and the soft inverse, with HARQ-IR combining across redundancy versions.

The interleaver is ``π(k) = k·p mod E`` with ``p`` the integer nearest ``E/φ`` that is coprime
with ``E``: consecutive coded bits land ≈ 0.618·E apart in the time-major symbol grid, so any
burst in time (a fade) or frequency (a notch) is scattered over the whole codeword without
the structured-interleaver failure mode of parking a run of coded bits on one carrier.
"""

from __future__ import annotations

import math

import numpy as np
from numpy.typing import NDArray

from aether_model.fec.nr_ldpc import RateMatcher, nr_ldpc_code
from aether_model.frame.modes import LONG, PAYLOAD_CRC, FrameLayout, Mode
from aether_model.phy.constellation import constellation

BitArray = NDArray[np.uint8]
FloatArray = NDArray[np.float64]
ComplexArray = NDArray[np.complex128]

_PHI = (1 + 5**0.5) / 2


def coprime_stride(e: int) -> int:
    """Stride nearest E/φ that is coprime with E (search outward from the ideal)."""
    ideal = max(1, round(e / _PHI))
    for d in range(e):
        for cand in (ideal - d, ideal + d):
            if 1 <= cand < e and math.gcd(cand, e) == 1:
                return cand
    return 1


class FrameCodec:
    def __init__(self, mode: Mode, layout: FrameLayout = LONG) -> None:
        self.mode = mode
        self.layout = layout
        self.payload_bytes = mode.payload_bytes(layout)
        self.info_bits = mode.info_bits(layout)
        self.coded_bits = mode.coded_bits(layout)
        self.bg = mode.base_graph(layout)
        self.z = mode.lifting_size(layout)
        self.code = nr_ldpc_code(self.bg, self.z)
        if self.info_bits > self.code.k:
            raise ValueError("information block does not fit the selected lifting size")
        self.constellation = constellation(mode.modulation)
        self._stride = coprime_stride(self.coded_bits)
        k = np.arange(self.coded_bits, dtype=np.int64)
        self._perm = (k * self._stride) % self.coded_bits  # coded bit k → position π(k)
        self._inv = np.empty_like(self._perm)
        self._inv[self._perm] = k

    def _rate_matcher(self, rv: int) -> RateMatcher:
        return RateMatcher(self.code, self.info_bits, self.coded_bits, rv=rv)

    # ── transmit ──────────────────────────────────────────────────────

    def encode(self, payload: bytes, rv: int = 0) -> ComplexArray:
        if len(payload) != self.payload_bytes:
            raise ValueError(
                f"payload must be exactly {self.payload_bytes} bytes, got {len(payload)}"
            )
        bits = np.unpackbits(np.frombuffer(payload, dtype=np.uint8))
        info = np.zeros(self.code.k, dtype=np.uint8)
        info[: self.info_bits] = PAYLOAD_CRC.attach(bits)
        codeword = self.code.encode(info)
        e = self._rate_matcher(rv).match(codeword)
        interleaved = np.empty_like(e)
        interleaved[self._perm] = e
        return self.constellation.map(interleaved)

    # ── receive ───────────────────────────────────────────────────────

    def decode(
        self,
        symbols: ComplexArray,
        noise_var: float | FloatArray,
        rv: int = 0,
        buffer: FloatArray | None = None,
        max_iter: int = 25,
    ) -> tuple[bytes | None, FloatArray]:
        """Returns ``(payload or None, full-codeword LLR buffer)``.

        Pass the returned buffer back as ``buffer`` with the next redundancy version to
        combine transmissions (HARQ-IR). ``None`` means the CRC failed.
        """
        y = np.asarray(symbols, dtype=np.complex128)
        if y.shape != (self.layout.qam_symbols,):
            raise ValueError(f"expected {self.layout.qam_symbols} symbols, got {y.shape}")
        llr_interleaved = self.constellation.llr(y, noise_var)
        llr_e = llr_interleaved[self._perm]
        full = self._rate_matcher(rv).recover(llr_e, buffer=buffer)
        hard, _converged, _ = self.code.decode(full, max_iter=max_iter)
        block = hard[: self.info_bits]
        if not PAYLOAD_CRC.check(block):
            return None, full
        payload = np.packbits(block[: -PAYLOAD_CRC.width]).tobytes()
        return payload, full

    # ── introspection ─────────────────────────────────────────────────

    def describe(self) -> dict[str, float | int | str]:
        return {
            "mode": self.mode.name,
            "layout": self.layout.name,
            "payload_bytes": self.payload_bytes,
            "info_bits": self.info_bits,
            "coded_bits": self.coded_bits,
            "effective_rate": round(self.info_bits / self.coded_bits, 4),
            "base_graph": self.bg,
            "z": self.z,
            "fillers": self.code.k - self.info_bits,
            "interleaver_stride": self._stride,
        }
