"""Audio backends with one contract: 48 kHz mono float32 in and out, pulled in blocks.

* :class:`WavFileBackend` — replays a recording as the receive side and writes what the
  modem transmits to another WAV (the regression-test workhorse: every field recording
  becomes a test).
* :class:`SimulatorBackend` — full-duplex loopback through :class:`~aether_model.channel.HfChannel`
  (transmitted audio → baseband → channel → audio), so two modems can talk with no radio.
* :class:`SoundDeviceBackend` — a real full-duplex sound-card stream via ``sounddevice``
  (PortAudio), with ring buffers between the callback and the modem thread and a
  peak/RMS/clipping meter on the input. Imported lazily; absent on CI.

Nothing here knows about frames: backends move samples and report levels, period.
"""

from __future__ import annotations

import queue
import threading
import wave
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Protocol

import numpy as np
from numpy.typing import NDArray

from aether_model.channel import ChannelConfig, HfChannel
from aether_model.phy.passband import AudioToBaseband, BasebandToAudio
from aether_model.waveform import WIDE_2300, WaveformParams

Audio = NDArray[np.float32]

TX_AUDIO_LEVEL = 0.25
"""RMS level (full scale = 1.0) at which the modem emits audio: −12 dBFS, leaving headroom
for the ≈ 10 dB PAPR of OFDM."""


@dataclass
class LevelMeter:
    """Peak / RMS / clipping statistics over the most recent audio (for the wizard)."""

    window_s: float = 1.0
    rate: int = 48000
    peak: float = 0.0
    rms: float = 0.0
    clipped: int = 0
    _hist: list[Audio] = field(default_factory=list)

    def update(self, block: Audio) -> None:
        self._hist.append(np.asarray(block, dtype=np.float32))
        total = sum(len(b) for b in self._hist)
        limit = int(self.window_s * self.rate)
        while total > limit and len(self._hist) > 1:
            total -= len(self._hist.pop(0))
        x = np.concatenate(self._hist) if self._hist else np.zeros(1, dtype=np.float32)
        self.peak = float(np.max(np.abs(x)))
        self.rms = float(np.sqrt(np.mean(x.astype(np.float64) ** 2)))
        self.clipped = int(np.sum(np.abs(x) >= 0.999))

    @property
    def peak_dbfs(self) -> float:
        return 20 * np.log10(max(self.peak, 1e-9))

    @property
    def rms_dbfs(self) -> float:
        return 20 * np.log10(max(self.rms, 1e-9))


class AudioBackend(Protocol):
    rate: int

    def start(self) -> None: ...
    def stop(self) -> None: ...
    def read(self, n: int) -> Audio:
        """Next ``n`` input samples (zero-padded at end of stream); never blocks forever."""
        ...

    def write(self, block: Audio) -> None: ...


# ── WAV replay / capture ──────────────────────────────────────────────


def read_wav(path: str | Path) -> tuple[int, Audio]:
    with wave.open(str(path), "rb") as w:
        rate, width, channels, n = (
            w.getframerate(),
            w.getsampwidth(),
            w.getnchannels(),
            w.getnframes(),
        )
        raw = w.readframes(n)
    if width == 2:
        x = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif width == 4:
        x = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"unsupported sample width {width}")
    if channels > 1:
        x = x.reshape(-1, channels)[:, 0]
    return rate, x


def write_wav(path: str | Path, rate: int, audio: NDArray) -> None:
    pcm = np.clip(np.asarray(audio, dtype=np.float64), -1.0, 1.0)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes((pcm * 32767.0).astype("<i2").tobytes())


class WavFileBackend:
    def __init__(
        self,
        input_path: str | Path | None,
        output_path: str | Path | None = None,
        rate: int = 48000,
    ) -> None:
        self.rate = rate
        self._in = np.zeros(0, dtype=np.float32)
        if input_path is not None:
            r, x = read_wav(input_path)
            if r != rate:
                raise ValueError(f"{input_path}: {r} Hz, expected {rate}")
            self._in = x
        self._pos = 0
        self._out_path = Path(output_path) if output_path else None
        self._out: list[Audio] = []
        self.meter = LevelMeter(rate=rate)

    def start(self) -> None:
        self._pos = 0

    def stop(self) -> None:
        if self._out_path is not None:
            write_wav(
                self._out_path, self.rate, np.concatenate(self._out) if self._out else np.zeros(0)
            )

    @property
    def exhausted(self) -> bool:
        return self._pos >= len(self._in)

    def read(self, n: int) -> Audio:
        block = self._in[self._pos : self._pos + n]
        self._pos += n
        if len(block) < n:
            block = np.concatenate((block, np.zeros(n - len(block), dtype=np.float32)))
        self.meter.update(block)
        return block

    def write(self, block: Audio) -> None:
        self._out.append(np.asarray(block, dtype=np.float32))


# ── Channel-simulator loopback ────────────────────────────────────────


class SimulatorBackend:
    """What is written comes back on ``read`` after passing through the HF channel model.

    Two of these can be cross-connected (``a.peer = b``) to simulate two stations. The
    channel's SNR is referenced to ``tx_level`` (the RMS of the transmitted audio, which
    :class:`~aether_model.phy.pipeline.Modem` also uses), so a configured ``snr_db`` means
    exactly that at the receiver."""

    def __init__(
        self,
        config: ChannelConfig | None = None,
        params: WaveformParams = WIDE_2300,
        tx_level: float = TX_AUDIO_LEVEL,
    ) -> None:
        self.rate = params.audio_rate
        self.p = params
        cfg = config or ChannelConfig(profile="awgn", snr_db=20.0, fs=params.fs_baseband)
        self.channel = HfChannel(replace(cfg, fs=params.fs_baseband, signal_power=tx_level**2))
        self._to_bb = AudioToBaseband(params)
        self._to_audio = BasebandToAudio(params)
        self._rx = np.zeros(0, dtype=np.float32)
        self.peer: SimulatorBackend | None = None
        self.meter = LevelMeter(rate=self.rate)

    def start(self) -> None:
        pass

    def stop(self) -> None:
        pass

    def _deliver(self, baseband: NDArray[np.complex128], target: SimulatorBackend) -> None:
        audio = target._to_audio.process(self.channel.process(baseband))
        target._rx = np.concatenate((target._rx, audio))

    def write(self, block: Audio) -> None:
        bb = self._to_bb.process(np.asarray(block, dtype=np.float64))
        if len(bb):
            self._deliver(bb, self.peer if self.peer is not None else self)

    def read(self, n: int) -> Audio:
        if len(self._rx) < n:  # silence between transmissions still carries the noise floor
            missing = n - len(self._rx)
            zeros = np.zeros(-(-missing // self.p.resample_factor) + 1, dtype=np.complex128)
            self._deliver(zeros, self)
        block, self._rx = self._rx[:n], self._rx[n:]
        self.meter.update(block)
        return block


# ── Sound card ────────────────────────────────────────────────────────


class SoundDeviceBackend:
    """Full-duplex PortAudio stream. ``device`` may be a name substring or an index pair."""

    def __init__(
        self,
        device_in: str | int | None = None,
        device_out: str | int | None = None,
        rate: int = 48000,
        blocksize: int = 2048,
        queue_blocks: int = 64,
    ) -> None:
        self.rate = rate
        self.blocksize = blocksize
        self._dev = (device_in, device_out)
        self._rx: queue.Queue[Audio] = queue.Queue(maxsize=queue_blocks)
        self._tx: queue.Queue[Audio] = queue.Queue(maxsize=queue_blocks)
        self._tx_partial = np.zeros(0, dtype=np.float32)
        self._stream: object | None = None
        self._lock = threading.Lock()
        self.meter = LevelMeter(rate=rate)
        self.overruns = 0
        self.underruns = 0

    @staticmethod
    def list_devices() -> list[dict[str, object]]:
        import sounddevice as sd  # noqa: PLC0415 - optional dependency

        return [dict(d, index=i) for i, d in enumerate(sd.query_devices())]

    def start(self) -> None:
        import sounddevice as sd  # noqa: PLC0415 - optional dependency

        def callback(indata, outdata, frames, time_info, status):  # type: ignore[no-untyped-def]
            if status and status.input_overflow:
                self.overruns += 1
            mono = np.ascontiguousarray(indata[:, 0], dtype=np.float32)
            try:
                self._rx.put_nowait(mono)
            except queue.Full:
                self.overruns += 1
            out = np.zeros(frames, dtype=np.float32)
            filled = 0
            with self._lock:
                while filled < frames:
                    if len(self._tx_partial) == 0:
                        try:
                            self._tx_partial = self._tx.get_nowait()
                        except queue.Empty:
                            break
                    take = min(frames - filled, len(self._tx_partial))
                    out[filled : filled + take] = self._tx_partial[:take]
                    self._tx_partial = self._tx_partial[take:]
                    filled += take
            outdata[:, 0] = out

        stream = sd.Stream(
            samplerate=self.rate,
            blocksize=self.blocksize,
            channels=1,
            dtype="float32",
            callback=callback,
            device=self._dev,
        )
        stream.start()
        self._stream = stream

    def stop(self) -> None:
        stream = self._stream
        if stream is not None:
            stream.stop()  # type: ignore[attr-defined]
            stream.close()  # type: ignore[attr-defined]
            self._stream = None

    def read(self, n: int) -> Audio:
        chunks: list[Audio] = []
        got = 0
        while got < n:
            try:
                b = self._rx.get(timeout=1.0)
            except queue.Empty:
                break
            chunks.append(b)
            got += len(b)
        x = np.concatenate(chunks) if chunks else np.zeros(0, dtype=np.float32)
        if len(x) > n:  # push the remainder back for the next read
            self._rx.queue.appendleft(x[n:])
            x = x[:n]
        if len(x) < n:
            x = np.concatenate((x, np.zeros(n - len(x), dtype=np.float32)))
        self.meter.update(x)
        return x

    def write(self, block: Audio) -> None:
        self._tx.put(np.asarray(block, dtype=np.float32))

    @property
    def tx_pending(self) -> bool:
        with self._lock:
            return not self._tx.empty() or len(self._tx_partial) > 0
