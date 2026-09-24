"""Streaming receiver: feed baseband blocks of any size, get decoded frames out (P1-8).

Wraps the offline detectors and receivers with a rolling buffer so live audio (sound card,
WAV replay) can be processed block by block:

* blocks are appended to a buffer that keeps at most ``max_buffer_s`` seconds;
* OFDM detection runs over the part of the buffer that has not been searched yet, extended
  backwards by the detector's ``stream_lookback`` so a preamble straddling a block boundary
  is still found once a symbol of what follows it has arrived;
* a detected OFDM frame is decoded only when its last sample (plus the FFT window margin) is
  in the buffer; until then it stays pending;
* the tone floor (ADR-0013) runs alongside on the same band-limited stream: a
  :class:`~aether_model.phy.tone.ToneStream` keeps its spectrogram rows by absolute hop,
  hands over each tone frame a symbol after it ends, and announces one as arriving once its
  first sync block is in (:attr:`StreamingReceiver.arriving`);
* absolute sample indices are kept so ``FrameSync.start`` values remain meaningful across
  buffer trimming (they refer to the band-limited stream, which lags the raw input by the
  filter's group delay, ``delay_samples``).

The decoded output is identical to :meth:`Modem.decode_buffer` on the concatenated input
(checked by tests), so real-time behaviour never diverges from the offline model.
"""

from __future__ import annotations

from dataclasses import dataclass, replace

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.phy.blanker import NoiseBlanker, StreamingBlanker
from aether_model.phy.passband import band_limit_taps
from aether_model.phy.pipeline import DecodedFrame, Modem
from aether_model.phy.sync import FrameSync
from aether_model.phy.tone import ToneArrival, ToneStream
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]


@dataclass
class _Pending:
    sync: FrameSync  # start is an absolute sample index
    end_abs: int


class StreamingReceiver:
    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        max_buffer_s: float = 7.0,
        blank_impulses: bool = True,
        blanker: NoiseBlanker | None = None,
    ) -> None:
        self.p = params
        self.modem = Modem(params, blank_impulses=False)  # blanking happens here, once
        self.blanker = StreamingBlanker(blanker or NoiseBlanker()) if blank_impulses else None
        """Impulse blanker, applied before the band-limiting FIR (P2-5) — it has to run
        before the filter smears an impulse into a long tail. The *streaming* wrapper is not
        an optimisation: blanking block by block judges the start of every burst against the
        silence in front of it and removes it, measured at 20 dB of SNR on a clean channel.
        The wrapper holds samples back so each window can be centred, which is why the
        indexed stream lags the input by :attr:`blanker_latency` as well as the filter's own
        group delay."""
        self._buf = np.zeros(0, dtype=np.complex128)
        self._buf_abs0 = 0  # absolute index of buf[0]
        self._searched_abs = 0  # everything before this absolute index has been searched
        self._pending: list[_Pending] = []
        self._done: list[tuple[int, int]] = []  # spans of frames already handed out
        self._tone_keep = max(k.samples for k in self.modem.air.tone_data) + 4000
        """Samples always kept for the tone floor: its longest frame and half a second — a
        tone frame is taken a symbol after it ends and refined against its own samples."""
        self._max_buf = max(int(max_buffer_s * params.fs_baseband), self._tone_keep)
        self._lookback = self.modem.detector.stream_lookback
        self.tone = ToneStream(self.modem.tone_detector)
        self.frames_decoded = 0
        self._taps = band_limit_taps(params)
        self.blanker_latency = self.blanker.latency_samples if self.blanker else 0
        """Samples the blanker holds back; the indexed stream runs this far behind."""
        self._zi = np.zeros(len(self._taps) - 1, dtype=np.complex128)
        self.delay_samples = (len(self._taps) - 1) // 2

    @property
    def samples_seen(self) -> int:
        return self._buf_abs0 + len(self._buf)

    @property
    def arriving(self) -> list[ToneArrival]:
        """Tone frames whose first sync block is in and which are not final yet: what the
        station hands the link layer so it holds its acknowledgement (ADR-0013)."""
        return list(self.tone.arriving)

    def feed(self, block: ComplexArray) -> list[DecodedFrame]:
        if self.blanker is not None:
            block = self.blanker.process(np.asarray(block, dtype=np.complex128))
        if len(block) == 0:
            return []  # the blanker is still holding everything back
        block, self._zi = signal.lfilter(
            self._taps, [1.0], np.asarray(block, dtype=np.complex128), zi=self._zi
        )
        self._buf = np.concatenate((self._buf, block))
        out: list[DecodedFrame] = []
        det = self.modem.detector
        margin = det.dem.fft_offset + self.p.symbol_samples

        # 1. search the unsearched region (with lookback) for new preambles
        search_abs0 = max(self._buf_abs0, self._searched_abs - self._lookback)
        rel0 = search_abs0 - self._buf_abs0
        region = self._buf[rel0:]
        if len(region) >= 4 * self.p.symbol_samples:
            for sync in det.detect(region, max_frames=8):
                span = self.modem.rx.frame_span(replace(sync, start=0))[1]
                new = _Pending(replace(sync, start=sync.start + search_abs0), 0)
                new.end_abs = new.sync.start + span
                spans = [(p.sync.start, p.end_abs) for p in self._pending] + self._done
                if any(a - self.p.symbol_samples < new.sync.start < b for a, b in spans):
                    continue  # duplicate, or inside a frame we already know about
                self._pending.append(new)
            self._searched_abs = max(self._searched_abs, self.samples_seen - self._lookback)

        # 2. decode every pending frame that is complete
        still: list[_Pending] = []
        for pend in sorted(self._pending, key=lambda q: q.sync.start):
            if pend.end_abs + margin <= self.samples_seen:
                rel = replace(pend.sync, start=pend.sync.start - self._buf_abs0)
                self._done = [*self._done, (pend.sync.start, pend.end_abs)][-20:]
                try:
                    decoded = self.modem.decode_sync(self._buf, rel)
                except (ValueError, IndexError):
                    continue
                assert decoded.frame is not None
                decoded.frame.sync = pend.sync  # report absolute positions
                out.append(decoded)
                self.frames_decoded += 1
            else:
                still.append(pend)
        self._pending = still

        # 3. the tone floor, on the same stream
        for ts in self.tone.feed(self._buf, self._buf_abs0):
            local = replace(ts, start=ts.start - self._buf_abs0)
            decoded = self.modem.decode_tone(self._buf, local)
            decoded.tone_sync = ts  # report absolute positions
            out.append(decoded)
            self.frames_decoded += 1

        # 4. trim the buffer, never discarding a pending frame's start or the lookback
        keep_from_abs = min(
            [self._searched_abs - self._lookback, self.samples_seen - self._tone_keep]
            + [q.sync.start for q in self._pending]
        )
        keep_from_abs = max(keep_from_abs, self.samples_seen - self._max_buf, self._buf_abs0)
        drop = keep_from_abs - self._buf_abs0
        if drop > 0:
            self._buf = self._buf[drop:]
            self._buf_abs0 += drop
        return sorted(out, key=lambda d: d.start)
