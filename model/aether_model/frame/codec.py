"""Payload ↔ constellation-symbol codec for one (mode, layout) pair (roadmap P1-3).

    bytes ─► bits ─► CRC-24 ─► fillers ─► LDPC (BG per §7.2.2) ─► rate matching (RV) ─►
    coprime-stride interleaver ─► Gray-labelled constellation ─► QAM symbols (time-major)

and the soft inverse, with HARQ-IR combining across redundancy versions. :class:`ToneCodec` is
the same chain for the tone floor (ADR-0013), ending in four Gray-labelled bits a tone
instead of a constellation point.

The interleaver is ``π(k) = k·p mod E`` with ``p`` the integer nearest ``E/φ`` that is coprime
with ``E``: consecutive coded bits land ≈ 0.618·E apart in the time-major symbol grid, so any
burst in time (a fade) or frequency (a notch) is scattered over the whole codeword without
the structured-interleaver failure mode of parking a run of coded bits on one carrier.
"""

from __future__ import annotations

import math
from functools import cache

import numpy as np
from numpy.typing import NDArray

from aether_model.fec.nr_ldpc import RateMatcher, nr_ldpc_code
from aether_model.frame.modes import LONG, PAYLOAD_CRC, FrameLayout, Mode, ToneKind
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
        # The all-zero word is a codeword of every linear code and its CRC is zero, so a
        # decoder fed noise (a false detection, a frame read at the wrong start) converges
        # to it and "passes". No frame of ours is all zeros — the link layer never assigns
        # session 0 and its other frames have a non-zero kind — so the block is refused.
        if not block.any():
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


# ── the tone floor (ADR-0013) ─────────────────────────────────────────


@cache
def gray_labels(bits: int) -> NDArray[np.uint8]:
    """Row ``t``: the bits of tone ``t``'s Gray label, most significant first — neighbouring
    tones, the ones a carrier offset or a Doppler smear confuses, differ in one bit."""
    t = np.arange(1 << bits)
    g = t ^ (t >> 1)
    return ((g[:, None] >> np.arange(bits - 1, -1, -1)[None, :]) & 1).astype(np.uint8)


class ToneCodec:
    """Payload ↔ data tones for one tone-floor kind: the OFDM codec's chain — CRC-24, LDPC,
    rate matching (RV), the golden-ratio interleaver — ending in ``log2 M`` bits a tone."""

    def __init__(self, kind: ToneKind) -> None:
        self.kind = kind
        self.bg = kind.base_graph
        self.z = kind.lifting_size
        self.code = nr_ldpc_code(self.bg, self.z)
        if kind.info_bits > self.code.k:
            raise ValueError("information block does not fit the selected lifting size")
        n = kind.coded_bits
        self._perm = (np.arange(n, dtype=np.int64) * coprime_stride(n)) % n
        m = kind.data.bits_per_symbol
        self._weights = 1 << np.arange(m - 1, -1, -1)
        self._tone_of_label = np.argsort(gray_labels(m) @ self._weights)

    def _matcher(self, rv: int) -> RateMatcher:
        return RateMatcher(self.code, self.kind.info_bits, self.kind.coded_bits, rv=rv)

    def encode(self, payload: bytes, rv: int = 0) -> NDArray[np.int64]:
        """The data symbols' tones."""
        if len(payload) != self.kind.payload_bytes:
            raise ValueError(f"payload must be {self.kind.payload_bytes} bytes, got {len(payload)}")
        info = np.zeros(self.code.k, dtype=np.uint8)
        info[: self.kind.info_bits] = PAYLOAD_CRC.attach(
            np.unpackbits(np.frombuffer(payload, dtype=np.uint8))
        )
        e = self._matcher(rv).match(self.code.encode(info))
        bits = np.empty_like(e)
        bits[self._perm] = e
        label = bits.reshape(-1, self.kind.data.bits_per_symbol).astype(np.int64) @ self._weights
        return np.asarray(self._tone_of_label[label], dtype=np.int64)

    def decode(
        self,
        llr_bits: FloatArray,
        rv: int = 0,
        buffer: FloatArray | None = None,
        max_iter: int = 30,
    ) -> tuple[bytes | None, FloatArray]:
        """``(payload or None, the codeword's LLR buffer)`` from the data symbols' bit LLRs
        (positive = 0) in transmission order; the buffer combines retransmissions (HARQ-IR)
        as :meth:`FrameCodec.decode`'s does."""
        llr_e = np.asarray(llr_bits, dtype=np.float64)[self._perm]
        full = self._matcher(rv).recover(llr_e, buffer=buffer)
        hard, _converged, _ = self.code.decode(full, max_iter=max_iter)
        block = hard[: self.kind.info_bits]
        # the all-zero word passes every linear code and its CRC: refused, as FrameCodec does
        if not PAYLOAD_CRC.check(block) or not block.any():
            return None, full
        return np.packbits(block[: -PAYLOAD_CRC.width]).tobytes(), full


@cache
def tone_codec(kind: ToneKind) -> ToneCodec:
    return ToneCodec(kind)
