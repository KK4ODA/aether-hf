"""Frame transmitter: header + coded symbols → complex baseband → audio (roadmap P1-4).

A frame is ``[SC, SC] + data symbols`` (the SC sequence encodes the frame type); data
symbol ``i`` is a full pilot symbol when ``i`` is in ``layout.pilot_symbol_indices`` — in
DATA frames its data carriers carry the mode's PN chips — and otherwise carries the next
``n_data_carriers`` constellation symbols (time-major) on its data carriers with comb
pilots. Frames are separated by ``guard_symbols`` of silence so the windowed edges decay
cleanly; the trailing taper of one frame overlap-adds into the guard.
"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.modes import FrameLayout
from aether_model.phy.ofdm import OfdmModulator
from aether_model.phy.passband import BasebandToAudio
from aether_model.phy.preamble import FrameHeader, FrameType, preamble
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]


class FrameTransmitter:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.mod = OfdmModulator(params)
        self.pre = preamble(params)
        self.n_data = len(self.mod.cmap.data_carriers)

    def symbol_values(
        self, header: FrameHeader, layout: FrameLayout, qam: ComplexArray
    ) -> list[ComplexArray]:
        """Carrier-value vectors for every OFDM symbol of the frame, preamble first."""
        qam = np.asarray(qam, dtype=np.complex128)
        if qam.shape != (layout.qam_symbols,):
            raise ValueError(
                f"expected {layout.qam_symbols} constellation symbols, got {qam.shape}"
            )
        symbols = self.pre.symbols(header)
        pilots = set(layout.pilot_symbol_indices)
        pos = 0
        pilot_no = 0
        for i in range(layout.data_symbols):
            if i in pilots:
                chips = None
                if header.frame_type is FrameType.DATA:
                    chips = self.pre.mode_chips(header.mode, pilot_no)
                pilot_no += 1
                symbols.append(self.mod.symbol_values(None, full_pilot=True, chips=chips))
            else:
                symbols.append(self.mod.symbol_values(qam[pos : pos + self.n_data]))
                pos += self.n_data
        assert pos == layout.qam_symbols
        return symbols

    def baseband(self, header: FrameHeader, layout: FrameLayout, qam: ComplexArray) -> ComplexArray:
        """Windowed complex-baseband waveform of one frame (``layout.samples + taper`` long)."""
        return self.mod.modulate(self.symbol_values(header, layout, qam))

    def burst(
        self,
        header: FrameHeader,
        layout: FrameLayout,
        qam: ComplexArray,
        lead_silence_s: float = 0.0,
        tail_silence_s: float = 0.0,
    ) -> ComplexArray:
        """Frame with silence padding at the baseband rate (for PTT lead/tail or tests)."""
        fs = self.p.fs_baseband
        lead = np.zeros(round(lead_silence_s * fs), dtype=np.complex128)
        tail = np.zeros(round(tail_silence_s * fs), dtype=np.complex128)
        return np.concatenate((lead, self.baseband(header, layout, qam), tail))

    def audio(self, baseband: ComplexArray) -> NDArray[np.float32]:
        """Convert a complete baseband burst to 48 kHz audio (fresh converter per burst)."""
        conv = BasebandToAudio(self.p)
        # flush the converter's group delay so the burst is complete
        pad = np.zeros(2 * conv.tx_delay_samples // self.p.resample_factor + 8, dtype=np.complex128)
        return conv.process(np.concatenate((baseband, pad)))
