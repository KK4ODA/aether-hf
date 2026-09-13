"""Frame receiver: CFO correction, channel estimation, equalization, LLR weighting (P1-6).

Given a :class:`~aether_model.phy.sync.FrameSync`, the receiver

1. removes the estimated CFO from the whole frame (phase-continuous from the frame start),
   then measures the residual CFO from the pilot-to-pilot phase progression across all
   symbols (the channel cancels between consecutive symbols) and corrects again — this
   data-aided step brings the offset error from the preamble's ≈ 0.7 Hz at 0 dB down to
   well under 0.1 Hz, which 64-QAM needs;
2. takes the FFT of every symbol (pilot symbols, data symbols);
3. for DATA frames, reads the mode: the data carriers of the full pilot symbols carry the
   mode's PN chips, which are correlated coherently (using the comb-pilot channel estimate
   of those symbols) against all 14 mode sequences; the best/runner-up ratio is reported so
   the caller can fall back to the runner-up if the CRC fails;
4. estimates the channel on every symbol:
   * full estimates (all carriers) on the full pilot symbols, once their chips are known;
   * comb estimates (every 4th carrier) on data symbols, smoothed over the neighbouring
     symbols (3-tap, ±1 symbol — well inside the coherence time of a 1 Hz Doppler channel)
     and linearly interpolated across carriers (both edges are pilots, so nothing is ever
     extrapolated);
5. equalizes the data carriers by division and returns the constellation symbols in
   time-major order together with a per-symbol noise variance ``σ²/|Ĥ|²`` — the LLR
   weighting that lets the LDPC decoder discount faded carriers instead of trusting them.

Noise variance is measured from the pilot residuals over the whole frame. SNR is reported
both per carrier and referenced to 3 kHz for the metrics bus.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.modes import LONG, PREAMBLE_SYMBOLS, SHORT, FrameLayout
from aether_model.phy.ofdm import OfdmDemodulator
from aether_model.phy.preamble import FrameType, mode_chip_sequences, preamble
from aether_model.phy.sync import FrameSync
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]


@dataclass
class ReceivedFrame:
    sync: FrameSync
    layout: FrameLayout
    symbols: ComplexArray
    """Equalized constellation symbols, time-major (``layout.qam_symbols``)."""
    noise_var: FloatArray
    """Effective complex noise variance per equalized symbol (for LLRs)."""
    snr_carrier_db: float
    """Mean E_s/N_0 per carrier estimated from pilots."""
    snr_3k_db: float
    """The same SNR referenced to a 3 kHz noise bandwidth (the project convention)."""
    channel: ComplexArray
    """Channel estimate per data symbol × carrier (diagnostics: shape (n_symbols, n_carriers))."""
    cfo_hz: float = 0.0
    """Total carrier offset actually removed (preamble estimate + data-aided residual)."""
    mode: int = 0
    """Mode index read from the pilot-symbol chips (DATA) or the control mode (CONTROL)."""
    mode_runner_up: int = 0
    mode_confidence: float = 1.0
    """Best mode metric divided by the runner-up; below ~1.3 the caller may retry with
    ``mode_runner_up`` if the CRC fails."""


def layout_for(header_type: FrameType) -> FrameLayout:
    return LONG if header_type is FrameType.DATA else SHORT


class FrameReceiver:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.dem = OfdmDemodulator(params)
        self.cmap = self.dem.cmap
        self.pre = preamble(params)
        self.pilot_seq = self.cmap.pilot_sequence
        self.pilot_c = self.cmap.pilot_carriers
        self.data_c = self.cmap.data_carriers

    def frame_span(self, sync: FrameSync) -> tuple[int, int]:
        layout = layout_for(sync.header.frame_type)
        return sync.start, sync.start + layout.samples

    def receive(self, x: ComplexArray, sync: FrameSync, mode: int | None = None) -> ReceivedFrame:
        """Demodulate and equalize one frame. ``mode`` overrides chip-based detection (used
        for the runner-up retry)."""
        layout = layout_for(sync.header.frame_type)
        start, end = self.frame_span(sync)
        if end + self.dem.fft_offset > len(x):
            raise ValueError("frame runs past the end of the buffer")
        per = self.p.symbol_samples
        fs = self.p.fs_baseband
        n_sym = layout.total_symbols
        pre = PREAMBLE_SYMBOLS
        pilot_syms = [pre + i for i in layout.pilot_symbol_indices]
        data_syms = [s for s in range(pre, n_sym) if s not in pilot_syms]
        raw_in = np.asarray(x[start : end + per], dtype=np.complex128)
        t = np.arange(len(raw_in)) / fs

        def demod(cfo: float) -> ComplexArray:
            seg = raw_in * np.exp(-2j * np.pi * cfo * t)
            return np.stack([self.dem.carriers(seg, i * per) for i in range(n_sym)])

        # 1. CFO removal, then data-aided residual from comb-pilot phase progression
        raw = demod(sync.cfo_hz)
        pc = self.pilot_c
        ref = self.pilot_seq[pc]
        prod = raw[pre + 1 :, pc] * np.conj(raw[pre:-1, pc]) * ref * np.conj(ref)
        residual = float(np.angle(prod.sum()) / (2 * np.pi * self.p.symbol_period_s))
        cfo_total = sync.cfo_hz + residual
        raw = demod(cfo_total)

        # 2. comb LS estimates on every symbol after the preamble (pilot carriers are known
        #    on all of them), smoothed over ±1 symbol
        comb = np.zeros((n_sym, len(pc)), dtype=np.complex128)
        comb[pre:] = raw[pre:, pc] / ref
        sm = comb.copy()
        for s_i in range(pre, n_sym):
            lo, hi = max(pre, s_i - 1), min(n_sym - 1, s_i + 1)
            sm[s_i] = comb[lo : hi + 1].mean(axis=0)
        carriers = np.arange(self.cmap.n_carriers)

        def interp(row: ComplexArray) -> ComplexArray:
            return np.interp(carriers, pc, row.real) + 1j * np.interp(carriers, pc, row.imag)

        # 3. mode from the pilot-symbol chips (DATA frames)
        mode_idx, runner_up, confidence = 0, 0, 1.0
        if sync.header.frame_type is FrameType.DATA:
            dc = self.data_c
            z_parts = []
            for s_i in pilot_syms:
                h_est = interp(sm[s_i])[dc]
                z_parts.append(raw[s_i, dc] * np.conj(h_est))
            z = np.concatenate(z_parts)
            z /= max(float(np.linalg.norm(z)), 1e-12)
            seqs = mode_chip_sequences(self.pre.n_chips)
            n_used = len(z)
            metrics = np.array([abs(np.vdot(seq[:n_used], z)) for seq in seqs]) / np.sqrt(n_used)
            order = np.argsort(metrics)[::-1]
            mode_idx, runner_up = int(order[0]), int(order[1])
            confidence = float(metrics[order[0]] / max(metrics[order[1]], 1e-12))
            if mode is not None:
                mode_idx = mode

        # 4. known carrier values per symbol, then channel estimates
        known = np.zeros((n_sym, self.cmap.n_carriers), dtype=np.complex128)
        full = np.zeros(n_sym, dtype=bool)
        for pilot_no, s_i in enumerate(pilot_syms):
            known[s_i] = self.pilot_seq
            if sync.header.frame_type is FrameType.DATA:
                known[s_i, self.data_c] = self.pre.mode_chips(mode_idx, pilot_no)
            full[s_i] = True
        for s_i in data_syms:
            known[s_i, pc] = ref
        h = np.zeros((n_sym, self.cmap.n_carriers), dtype=np.complex128)
        for s_i in range(pre, n_sym):
            if full[s_i]:
                direct = raw[s_i] / known[s_i]
                h[s_i] = np.convolve(direct, np.array([0.25, 0.5, 0.25]), mode="same")
                h[s_i, 0], h[s_i, -1] = direct[0], direct[-1]
            else:
                h[s_i] = interp(sm[s_i])
        # noise variance from pilot residuals on data symbols (smoothed estimate vs raw)
        resid = raw[data_syms][:, pc] - h[data_syms][:, pc] * known[data_syms][:, pc]
        sigma2 = float(np.mean(np.abs(resid) ** 2)) * (3.0 / 2.0)  # undo the 3-tap averaging bias
        sig_power = float(np.mean(np.abs(h[data_syms]) ** 2))
        snr_carrier = sig_power / max(sigma2, 1e-12)
        bw_ratio = self.p.occupied_bandwidth_hz / 3000.0
        # 5. equalize data carriers of payload symbols, time-major
        eq = raw[data_syms][:, self.data_c] / h[data_syms][:, self.data_c]
        nv = sigma2 / np.maximum(np.abs(h[data_syms][:, self.data_c]) ** 2, 1e-9)
        return ReceivedFrame(
            sync=sync,
            layout=layout,
            symbols=eq.reshape(-1),
            noise_var=nv.reshape(-1),
            snr_carrier_db=10 * np.log10(snr_carrier),
            snr_3k_db=10 * np.log10(snr_carrier * bw_ratio),
            channel=h[pre:],
            cfo_hz=cfo_total,
            mode=mode_idx,
            mode_runner_up=runner_up,
            mode_confidence=confidence,
        )
