"""Frame receiver: CFO correction, channel estimation, equalization, LLR weighting (P1-6).

Given a :class:`~aether_model.phy.sync.FrameSync`, the receiver

1. removes the estimated CFO from the whole frame (phase-continuous from the frame start),
   then measures the residual CFO from the pilot-to-pilot phase progression across all
   symbols (the channel cancels between consecutive symbols) and corrects again — this
   data-aided step brings the offset error from the preamble's ≈ 0.7 Hz at 0 dB down to
   well under 0.1 Hz, which 64-QAM needs;
2. takes the FFT of every symbol (pilot symbols, data symbols);
3. for DATA frames, reads the mode and redundancy version: the data carriers of the full
   pilot symbols carry the (rv, mode) PN chips, which are correlated coherently (using the
   comb-pilot channel estimate of those symbols) against all 56 sequences; the
   best/runner-up ratio is reported so
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

Noise variance is measured from the pilot residuals per OFDM symbol, shrunk toward the
frame-wide value (P2-5) so that a symbol hit by an impulse becomes an erasure rather than
confident nonsense. SNR is reported
both per carrier and referenced to 3 kHz for the metrics bus.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.modes import LONG, SHORT, FrameLayout, air_interface
from aether_model.phy.ofdm import OfdmDemodulator
from aether_model.phy.preamble import (
    FrameType,
    preamble,
)
from aether_model.phy.sync import FrameSync
from aether_model.phy.wiener import (
    estimate_channel_statistics,
    frequency_filter,
    smooth_time,
    time_filter,
)
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
    rv: int = 0
    """Redundancy version read from the same chips (0 for CONTROL frames)."""
    chip_runner_up: int = 0
    """Second-best chip hypothesis (:func:`~aether_model.phy.preamble.chip_hypothesis`)."""
    mode_confidence: float = 1.0
    """Best chip metric divided by the runner-up; below ~1.3 the caller may retry with
    ``chip_runner_up`` if the CRC fails."""


def layout_for(header_type: FrameType, params: WaveformParams = WIDE_2300) -> FrameLayout:
    """The layout an OFDM frame of this type has on this waveform."""
    if params is WIDE_2300:
        return LONG if header_type is FrameType.DATA else SHORT
    return air_interface(params).layout_for(header_type is FrameType.DATA)


class FrameReceiver:
    noise_shrinkage: float = 0.5
    """How far each per-symbol noise estimate is pulled back toward the frame-wide one.
    0 trusts 15 pilots completely, 1 ignores them; 0.5 flags a damaged symbol without
    letting pilot noise alone condemn a clean one (P2-5)."""
    noise_floor_fraction: float = 0.25
    """A symbol's variance is never allowed below this multiple of the frame value, so an
    unluckily quiet pilot set cannot make the decoder over-trust a symbol."""
    wiener_doppler_max_hz: float = 1.5
    wiener_time_half_width: int = 1
    wiener_tau_max_s: float | None = None
    wiener_design_snr_db: float | None = None
    """Wiener design parameters (:mod:`aether_model.phy.wiener`). Delay spread and SNR are
    ``None`` = measured from the pilots on every frame, which is what makes the estimator
    worth having: matched to the channel it beats linear interpolation everywhere, while a
    fixed worst-case design loses to it nearly everywhere (P2-6)."""
    wiener_pilot_passthrough: bool = True
    """Use the raw (time-smoothed) LS values at the *pilot* carriers when measuring noise.
    The noise estimate is ``raw - h*known`` at the pilots, which measures noise only if ``h``
    reproduces them — true of linear interpolation, false of any filter that denoises. Without
    this the Wiener filter's own smoothing is charged to the noise estimate and every LLR in
    the frame is scaled wrong."""
    channel_estimator: str = "linear"
    """``"linear"`` (default) is linear-in-frequency with a 3-tap time average; ``"wiener"``
    is the MMSE interpolator of :mod:`aether_model.phy.wiener`.

    P2-6 asked for MMSE estimation *if benchmarks justify it*. They did not, and the default
    stays linear. Matched to the delay spread actually present, Wiener interpolation beats
    linear on raw interpolation error by 1.3 dB (ITU Poor) to 5.3 dB (AWGN) — but that gain
    does not survive into frame error rate here, and on the fading channels the receiver ends
    up worse. The most likely reason is that the two smoothers overlap: the ±1-symbol time
    average has already removed most of the pilot noise by the time the frequency filter
    runs, so its noise-averaging buys little while its design mismatch still costs. Both
    implementations and the measurements are kept so the question can be reopened with a
    joint 2-D design rather than two separable ones bolted together."""

    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.dem = OfdmDemodulator(params)
        self.cmap = self.dem.cmap
        self.pre = preamble(params)
        self.pilot_seq = self.cmap.pilot_sequence
        self.pilot_c = self.cmap.pilot_carriers
        self.data_c = self.cmap.data_carriers

    def frame_span(self, sync: FrameSync) -> tuple[int, int]:
        layout = layout_for(sync.header.frame_type, self.p)
        return sync.start, sync.start + layout.samples

    def receive(
        self, x: ComplexArray, sync: FrameSync, hypothesis: int | None = None
    ) -> ReceivedFrame:
        """Demodulate and equalize one frame. ``hypothesis`` (a chip-sequence index)
        overrides chip-based (mode, rv) detection — used for the runner-up retry."""
        layout = layout_for(sync.header.frame_type, self.p)
        start, end = self.frame_span(sync)
        if end + self.dem.fft_offset > len(x):
            raise ValueError("frame runs past the end of the buffer")
        per = self.p.symbol_samples
        fs = self.p.fs_baseband
        n_sym = layout.total_symbols
        pre = layout.preamble_symbols
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
        radius = layout.pilot_smoothing
        comb = np.zeros((n_sym, len(pc)), dtype=np.complex128)
        comb[pre:] = raw[pre:, pc] / ref
        carriers = np.arange(self.cmap.n_carriers)
        if self.channel_estimator == "wiener":
            tau, snr_design = estimate_channel_statistics(comb[pre:], self.p)
            if self.wiener_tau_max_s is not None:
                tau = self.wiener_tau_max_s
            if self.wiener_design_snr_db is not None:
                snr_design = self.wiener_design_snr_db
            sm = smooth_time(
                comb,
                pre,
                time_filter(
                    self.p,
                    self.wiener_doppler_max_hz,
                    snr_design,
                    self.wiener_time_half_width,
                ),
            )
            weights = frequency_filter(self.p, tau, snr_design)

            def interp(row: ComplexArray) -> ComplexArray:
                return np.asarray(weights @ row, dtype=np.complex128)
        else:
            sm = comb.copy()
            for s_i in range(pre, n_sym):
                lo, hi = max(pre, s_i - radius), min(n_sym - 1, s_i + radius)
                sm[s_i] = comb[lo : hi + 1].mean(axis=0)

            def interp(row: ComplexArray) -> ComplexArray:
                return np.interp(carriers, pc, row.real) + 1j * np.interp(carriers, pc, row.imag)

        # 3. mode and redundancy version from the pilot-symbol chips (DATA frames)
        mode_idx, rv, runner_up, confidence = 0, 0, 0, 1.0
        if sync.header.frame_type is FrameType.DATA:
            dc = self.data_c
            z_parts = []
            for s_i in pilot_syms:
                h_est = interp(sm[s_i])[dc]
                z_parts.append(raw[s_i, dc] * np.conj(h_est))
            z = np.concatenate(z_parts)
            z /= max(float(np.linalg.norm(z)), 1e-12)
            seqs = self.pre.sequences_for(layout)
            n_used = len(z)
            metrics = np.array([abs(np.vdot(seq[:n_used], z)) for seq in seqs]) / np.sqrt(n_used)
            order = np.argsort(metrics)[::-1]
            best, runner_up = int(order[0]), int(order[1])
            confidence = float(metrics[order[0]] / max(metrics[order[1]], 1e-12))
            mode_idx, rv = self.pre.chip_hypothesis(best if hypothesis is None else hypothesis)

        # 4. known carrier values per symbol, then channel estimates
        known = np.zeros((n_sym, self.cmap.n_carriers), dtype=np.complex128)
        full = np.zeros(n_sym, dtype=bool)
        for pilot_no, s_i in enumerate(pilot_syms):
            known[s_i] = self.pilot_seq
            if sync.header.frame_type is FrameType.DATA:
                known[s_i, self.data_c] = self.pre.mode_chips(mode_idx, pilot_no, rv, layout)
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
        # Noise variance from pilot residuals on data symbols (smoothed estimate vs raw),
        # estimated *per symbol* (P2-5). Impulsive noise is concentrated in time: one hot
        # sample damages every carrier of the symbol it lands in and none of the others. A
        # single frame-wide variance would average that damage over clean symbols, which both
        # overstates the noise on the clean ones and — far worse — understates it on the
        # damaged ones, so the decoder trusts exactly the LLRs it should be discarding. Per
        # symbol, a hit symbol's LLRs shrink on their own and it becomes an erasure.
        if self.channel_estimator == "wiener" and self.wiener_pilot_passthrough:
            h[pre:, pc] = sm[pre:]
        resid = raw[data_syms][:, pc] - h[data_syms][:, pc] * known[data_syms][:, pc]
        bias = (2 * radius + 1) / (2 * radius)  # undo the (2r+1)-tap averaging bias
        sigma2 = float(np.mean(np.abs(resid) ** 2)) * bias
        per_symbol = np.mean(np.abs(resid) ** 2, axis=1) * bias
        # Only 15 pilots back each per-symbol estimate, so shrink it toward the frame value:
        # enough freedom to flag a damaged symbol, not enough for pilot noise alone to
        # condemn a clean one.
        sigma2_sym = np.maximum(
            self.noise_shrinkage * sigma2 + (1.0 - self.noise_shrinkage) * per_symbol,
            sigma2 * self.noise_floor_fraction,
        )
        sig_power = float(np.mean(np.abs(h[data_syms]) ** 2))
        snr_carrier = sig_power / max(sigma2, 1e-12)
        bw_ratio = self.p.occupied_bandwidth_hz / 3000.0
        # 5. equalize data carriers of payload symbols, time-major
        eq = raw[data_syms][:, self.data_c] / h[data_syms][:, self.data_c]
        nv = sigma2_sym[:, None] / np.maximum(np.abs(h[data_syms][:, self.data_c]) ** 2, 1e-9)
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
            rv=rv,
            chip_runner_up=runner_up,
            mode_confidence=confidence,
        )
