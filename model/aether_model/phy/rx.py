"""Frame receiver: CFO correction, channel estimation, equalization, LLR weighting (P1-6).

Given a :class:`~aether_model.phy.sync.FrameSync`, the receiver

1. removes the estimated CFO from the whole frame (phase-continuous from the frame start),
   then measures the residual CFO from the pilot-to-pilot phase progression across all
   symbols (the channel cancels between consecutive symbols) and corrects again — this
   data-aided step brings the offset error from the preamble's ≈ 0.7 Hz at 0 dB down to
   well under 0.1 Hz, which 64-QAM needs;
2. takes the FFT of every symbol (unique word, pilot symbols, data symbols);
3. estimates the channel on every symbol:
   * full estimates (all carriers) on the unique word and the full pilot symbols;
   * comb estimates (every 4th carrier) on data symbols, smoothed over the neighbouring
     symbols (3-tap, ±1 symbol — well inside the coherence time of a 1 Hz Doppler channel)
     and linearly interpolated across carriers (both edges are pilots, so nothing is ever
     extrapolated);
4. equalizes the data carriers by division and returns the constellation symbols in
   time-major order together with a per-symbol noise variance ``σ²/|Ĥ|²`` — the LLR
   weighting that lets the LDPC decoder discount faded carriers instead of trusting them.

Noise variance is measured from the pilot residuals over the whole frame. SNR is reported
both per carrier and referenced to 3 kHz for the metrics bus.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.modes import LONG, SHORT, FrameLayout
from aether_model.phy.ofdm import OfdmDemodulator
from aether_model.phy.preamble import FrameType, preamble
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

    def receive(self, x: ComplexArray, sync: FrameSync) -> ReceivedFrame:
        layout = layout_for(sync.header.frame_type)
        start, end = self.frame_span(sync)
        if end + self.dem.fft_offset > len(x):
            raise ValueError("frame runs past the end of the buffer")
        per = self.p.symbol_samples
        fs = self.p.fs_baseband
        n_sym = layout.total_symbols
        # known carrier values per symbol (comb pilots everywhere from the UW onward)
        known = np.zeros((n_sym, self.cmap.n_carriers), dtype=np.complex128)
        full = np.zeros(n_sym, dtype=bool)
        known[2] = self.pre.uw_values(sync.header)
        full[2] = True
        pilot_syms = {3 + i for i in layout.pilot_symbol_indices}
        for s in range(3, n_sym):
            if s in pilot_syms:
                known[s] = self.pilot_seq
                full[s] = True
            else:
                known[s, self.pilot_c] = self.pilot_seq[self.pilot_c]
        raw_in = np.asarray(x[start : end + per], dtype=np.complex128)
        t = np.arange(len(raw_in)) / fs

        def demod(cfo: float) -> ComplexArray:
            seg = raw_in * np.exp(-2j * np.pi * cfo * t)
            return np.stack([self.dem.carriers(seg, i * per) for i in range(n_sym)])

        # 1. CFO removal, then data-aided residual from pilot phase progression
        raw = demod(sync.cfo_hz)
        pc = self.pilot_c
        prod = raw[3:, pc] * np.conj(raw[2:-1, pc]) * known[2:-1, pc] * np.conj(known[3:, pc])
        residual = float(np.angle(prod.sum()) / (2 * np.pi * self.p.symbol_period_s))
        cfo_total = sync.cfo_hz + residual
        # 2. raw carriers of every symbol with the refined offset
        raw = demod(cfo_total)
        # 3. channel estimates
        # comb LS estimates on every symbol from 2 onward (pilot carriers exist on all)
        comb = np.zeros((n_sym, len(self.pilot_c)), dtype=np.complex128)
        comb[2:] = raw[2:, self.pilot_c] / known[2:, self.pilot_c]
        # 3-tap time smoothing of the comb estimates (symbols 2 … n_sym−1)
        sm = comb.copy()
        for s in range(2, n_sym):
            lo, hi = max(2, s - 1), min(n_sym - 1, s + 1)
            sm[s] = comb[lo : hi + 1].mean(axis=0)
        # frequency interpolation to all carriers
        h = np.zeros((n_sym, self.cmap.n_carriers), dtype=np.complex128)
        carriers = np.arange(self.cmap.n_carriers)
        for s in range(2, n_sym):
            if full[s]:
                # full symbols: use the direct LS estimate, lightly smoothed across carriers
                direct = raw[s] / known[s]
                h[s] = np.convolve(direct, np.array([0.25, 0.5, 0.25]), mode="same")
                h[s, 0], h[s, -1] = direct[0], direct[-1]
            else:
                h[s] = np.interp(carriers, self.pilot_c, sm[s].real) + 1j * np.interp(
                    carriers, self.pilot_c, sm[s].imag
                )
        # noise variance from pilot residuals on data symbols (smoothed estimate vs raw)
        data_syms = [s for s in range(3, n_sym) if s not in pilot_syms]
        resid = (
            raw[data_syms][:, self.pilot_c]
            - h[data_syms][:, self.pilot_c] * known[data_syms][:, self.pilot_c]
        )
        sigma2 = float(np.mean(np.abs(resid) ** 2)) * (3.0 / 2.0)  # undo the 3-tap averaging bias
        sig_power = float(np.mean(np.abs(h[data_syms]) ** 2))
        snr_carrier = sig_power / max(sigma2, 1e-12)
        bw_ratio = self.p.occupied_bandwidth_hz / 3000.0
        # 4. equalize data carriers of payload symbols, time-major
        eq = raw[data_syms][:, self.data_c] / h[data_syms][:, self.data_c]
        nv = sigma2 / np.maximum(np.abs(h[data_syms][:, self.data_c]) ** 2, 1e-9)
        return ReceivedFrame(
            sync=sync,
            layout=layout,
            symbols=eq.reshape(-1),
            noise_var=nv.reshape(-1),
            snr_carrier_db=10 * np.log10(snr_carrier),
            snr_3k_db=10 * np.log10(snr_carrier * bw_ratio),
            channel=h[3:],
            cfo_hz=cfo_total,
        )
