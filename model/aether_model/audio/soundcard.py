"""
aether_model/audio/soundcard.py

Sound card audio I/O with resampling between 48 kHz audio and 12 kHz baseband.

Uses sounddevice (PortAudio wrapper) for cross-platform audio.
Falls back to a file-based loopback for testing without hardware.
"""

import contextlib
import logging
import queue

import numpy as np

from aether_model.constants import AUDIO_BUFFER_SIZE, RESAMPLE_FACTOR, SAMPLE_RATE

log = logging.getLogger(__name__)
_RNG = np.random.default_rng()


class AudioInterface:
    """Cross-platform audio I/O with baseband resampling.

    Provides 12 kHz complex baseband samples to the modem from 48 kHz
    real audio, and vice versa for transmission.
    """

    def __init__(self, device_in: str | None = None, device_out: str | None = None):
        self._dev_in = device_in
        self._dev_out = device_out
        self._running = False

        self._rx_queue: queue.Queue[np.ndarray] = queue.Queue(maxsize=100)
        self._tx_queue: queue.Queue[np.ndarray] = queue.Queue(maxsize=100)

        self._stream = None

    def start(self):
        """Start audio I/O."""
        try:
            import sounddevice as sd

            self._start_sounddevice(sd)
        except ImportError:
            log.warning("sounddevice not available — using loopback mode")
            self._running = True

    def stop(self):
        """Stop audio I/O."""
        self._running = False
        if self._stream:
            self._stream.stop()
            self._stream.close()
            self._stream = None

    def read_baseband(self, n_samples: int, timeout: float = 1.0) -> np.ndarray | None:
        """Read baseband (12 kHz complex) samples from the receiver.

        Returns None on timeout.
        """
        try:
            return self._rx_queue.get(timeout=timeout)
        except queue.Empty:
            return None

    def write_baseband(self, samples: np.ndarray):
        """Queue baseband samples for transmission.

        Samples are upsampled to 48 kHz and sent to the sound card.
        """
        self._tx_queue.put(samples)

    def write_audio(self, audio_48k: np.ndarray):
        """Queue raw 48 kHz audio samples for playback (e.g. for testing)."""
        # Downsample to baseband
        baseband = self._downsample(audio_48k)
        self._rx_queue.put(baseband)

    # ── Resampling ────────────────────────────────────────────────────

    @staticmethod
    def _downsample(audio_48k: np.ndarray) -> np.ndarray:
        """Downsample 48 kHz real audio to 12 kHz complex baseband.

        Applies a low-pass filter and decimates by 4.
        The complex baseband is obtained by mixing to DC first.
        """
        from scipy.signal import decimate

        # Simple decimation (prototype — production would use a proper
        # analytic signal conversion with Hilbert transform)
        real_12k = decimate(audio_48k.astype(np.float64), RESAMPLE_FACTOR)
        return real_12k.astype(np.complex128)

    @staticmethod
    def _upsample(baseband_12k: np.ndarray) -> np.ndarray:
        """Upsample 12 kHz complex baseband to 48 kHz real audio.

        Interpolates by 4 and takes the real part (SSB-like).
        """
        from scipy.signal import resample_poly

        # Take real part for audio output
        real_12k = baseband_12k.real.astype(np.float64)
        audio_48k = resample_poly(real_12k, RESAMPLE_FACTOR, 1)
        # Normalize to [-1, 1] range for 16-bit audio
        peak = np.max(np.abs(audio_48k))
        if peak > 0:
            audio_48k /= peak * 1.1  # leave headroom
        return audio_48k.astype(np.float32)

    # ── Sounddevice integration ───────────────────────────────────────

    def _start_sounddevice(self, sd):
        """Start full-duplex audio stream via sounddevice."""
        self._running = True

        def callback(indata, outdata, frames, time_info, status):
            if status:
                log.warning(f"Audio status: {status}")

            # RX: downsample incoming audio and queue
            if indata is not None:
                mono = indata[:, 0] if indata.ndim > 1 else indata
                baseband = self._downsample(mono)
                with contextlib.suppress(queue.Full):  # drop the block if the consumer is behind
                    self._rx_queue.put_nowait(baseband)

            # TX: get baseband, upsample, output
            try:
                tx_bb = self._tx_queue.get_nowait()
                audio_out = self._upsample(tx_bb)
                n = min(len(audio_out), frames)
                outdata[:n, 0] = audio_out[:n]
                outdata[n:, 0] = 0
            except queue.Empty:
                outdata[:] = 0

        self._stream = sd.Stream(
            samplerate=SAMPLE_RATE,
            blocksize=AUDIO_BUFFER_SIZE,
            channels=1,
            dtype="float32",
            callback=callback,
            device=(self._dev_in, self._dev_out),
        )
        self._stream.start()
        log.info(f"Audio started: {SAMPLE_RATE} Hz, buffer {AUDIO_BUFFER_SIZE}")


class LoopbackAudio(AudioInterface):
    """Loopback audio for testing — TX output feeds directly to RX input."""

    def __init__(self, noise_db: float = -100):
        super().__init__()
        self._noise_db = noise_db

    def start(self):
        self._running = True

    def write_baseband(self, samples: np.ndarray):
        """In loopback mode, TX goes straight to RX with optional noise."""
        noise_power = 10 ** (self._noise_db / 10)
        noise = np.sqrt(noise_power / 2) * (
            _RNG.standard_normal(len(samples)) + 1j * _RNG.standard_normal(len(samples))
        )
        self._rx_queue.put(samples + noise)
