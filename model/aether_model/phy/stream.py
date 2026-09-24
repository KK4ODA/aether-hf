"""Streaming receiver: feed baseband blocks of any size, get decoded frames out (P1-8).

Wraps the offline detector/receiver with a rolling buffer so live audio (sound card, WAV
replay) can be processed block by block:

* blocks are appended to a buffer that keeps at most ``max_buffer_s`` seconds;
* detection runs over the part of the buffer that has not been searched yet, extended
  backwards by the detector's ``stream_lookback`` so a preamble straddling a block boundary
  is still found once enough of what follows it has arrived — a symbol for an ordinary
  frame, eighteen for a floor one (ADR-0009 §8);
* a detected frame is decoded only when its last sample (plus the FFT window margin) is
  in the buffer; until then it stays pending;
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
from aether_model.phy.sync import FLOOR_OVER_ORDINARY, FrameSync
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]


@dataclass
class _Pending:
    sync: FrameSync  # start is an absolute sample index
    end_abs: int


def _overlap(a: _Pending, b: _Pending) -> bool:
    return a.sync.start < b.end_abs and b.sync.start < a.end_abs


class StreamingReceiver:
    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        max_buffer_s: float = 6.0,
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
        self._early: list[_Pending] = []  # floor frames arriving, not final yet (absolute)
        self._max_buf = int(max_buffer_s * params.fs_baseband)
        # the detector needs a symbol after an ordinary candidate, and far enough past a floor
        # one that nothing still to come could claim it (ADR-0009 §8): search again from that
        # far back, and hold that much unsearched
        self._lookback = self.modem.detector.stream_lookback
        self.frames_decoded = 0
        self._taps = band_limit_taps(params)
        self.blanker_latency = self.blanker.latency_samples if self.blanker else 0
        """Samples the blanker holds back; the indexed stream runs this far behind."""
        self._zi = np.zeros(len(self._taps) - 1, dtype=np.complex128)
        self.delay_samples = (len(self._taps) - 1) // 2

    @property
    def samples_seen(self) -> int:
        return self._buf_abs0 + len(self._buf)

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
            # a floor frame is taken once nothing still to come could claim it — long before
            # its end — so its span keeps its own body's phantoms out (ADR-0009 §8)
            found, early = det.detect_streaming(region, max_frames=8)
            self._early = [self._absolute(sync, search_abs0) for sync in early]
            for new in (self._absolute(sync, search_abs0) for sync in found):
                if new.sync.floor and not self._settle_floor(new):
                    continue  # an ordinary frame it clashes with is the stronger
                spans = [(p.sync.start, p.end_abs) for p in self._pending] + self._done
                if any(a - self.p.symbol_samples < new.sync.start < b for a, b in spans):
                    continue  # duplicate, or inside a frame we already know about
                self._pending.append(new)
            self._searched_abs = max(self._searched_abs, self.samples_seen - self._lookback)

        # 2. decode every pending frame that is complete
        still: list[_Pending] = []
        for pend in sorted(self._pending, key=lambda q: q.sync.start):
            if pend.end_abs + margin <= self.samples_seen and not self._held(pend):
                rel = replace(pend.sync, start=pend.sync.start - self._buf_abs0)
                self._done = [*self._done, (pend.sync.start, pend.end_abs)][-20:]
                try:
                    decoded = self.modem.decode_sync(self._buf, rel)
                except (ValueError, IndexError):
                    continue
                decoded.frame.sync = pend.sync  # report absolute positions
                out.append(decoded)
                self.frames_decoded += 1
            else:
                still.append(pend)
        self._pending = still

        # 3. trim the buffer, never discarding a pending frame's start or the lookback
        keep_from_abs = min(
            [self._searched_abs - self._lookback] + [q.sync.start for q in self._pending]
        )
        keep_from_abs = max(keep_from_abs, self.samples_seen - self._max_buf, self._buf_abs0)
        drop = keep_from_abs - self._buf_abs0
        if drop > 0:
            self._buf = self._buf[drop:]
            self._buf_abs0 += drop
        return out

    def _absolute(self, sync: FrameSync, base: int) -> _Pending:
        """A frame found ``base`` samples into the stream, with its span, in absolute indices."""
        span = self.modem.rx.frame_span(replace(sync, start=0))[1]
        return _Pending(replace(sync, start=sync.start + base), sync.start + base + span)

    def _held(self, pend: _Pending) -> bool:
        """An ordinary frame a floor frame still arriving would settle away — the phantom a
        floor preamble can raise on the ordinary references, complete long before the floor
        frame is final — waits for that decision instead of being decoded first."""
        return not pend.sync.floor and any(
            _overlap(f, pend) and f.sync.timing_peak >= FLOOR_OVER_ORDINARY * pend.sync.timing_peak
            for f in self._early
        )

    def _settle_floor(self, floor: _Pending) -> bool:
        """A floor candidate, taken while its frame is still arriving, against the pending
        ordinary frames it clashes with — above all the phantoms its own body produced before
        the floor candidate was final, which offline settles in the same pass. As offline
        (``FrameDetector._settle_families``): the floor frame stays, and they go, where its
        statistic is at least ``FLOOR_OVER_ORDINARY`` of theirs; otherwise it is dropped."""
        clash = [p for p in self._pending if not p.sync.floor and _overlap(p, floor)]
        if any(floor.sync.timing_peak < FLOOR_OVER_ORDINARY * p.sync.timing_peak for p in clash):
            return False
        self._pending = [p for p in self._pending if p not in clash]
        return True
