"""A real-time HF channel between virtual audio cables, for the A/B bench (roadmap P9-1).

    python tools/channel_cable.py --a-tx "CABLE Output" --b-rx "CABLE-A Input"
                                  --b-tx "CABLE-B Output" --a-rx "CABLE-C Input"
                                  --channel good --snr 10 [--signal-dbfs -15] [--auto-level]
                                  [--cfo-hz 0] [--seed 1] [--duration 0] [--log ab.csv]
    python tools/channel_cable.py --list-devices

Two modems on one machine, each with a transmit cable and a receive cable, and this
between them: what modem A plays into its transmit cable is captured here, run through the
reference model's channel simulator — the ITU-R F.1487 fading profile and the noise at a
3 kHz-referenced SNR, exactly as every benchmark curve in ``bench/baselines/`` defines them
— and played into modem B's receive cable; the other direction the same, through its own
independent fading. Either modem, VARA or Aether, then sees the identical impairment at the
audio level, which is the one comparison that settles "comparable or better" before the air
does. The repository holds this tool, the protocol (``bench/ab/README.md``) and the
results; it never holds a byte of VARA.

The signal level the SNR is relative to is the modem's transmit audio: ``--signal-dbfs``
says what RMS to expect (set each modem's drive to it, watching the level this prints), or
``--auto-level`` follows each burst's measured power, so the SNR holds whatever the drive.
Noise is present whether or not anything is being sent, as it is on the air.

The processing is ``CableChannel``, plain arrays in and out, testable without a sound
card; ``main`` wraps two of them in PortAudio streams through ``sounddevice`` (the
``audio`` extra: ``uv sync --extra audio``).
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
import threading
import time
from pathlib import Path

import numpy as np
from numpy.typing import NDArray
from scipy import signal

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import ChannelConfig, HfChannel

FloatArray = NDArray[np.float64]
ComplexArray = NDArray[np.complex128]

AUDIO_RATE = 48_000
BASEBAND_RATE = 8_000
DECIMATION = AUDIO_RATE // BASEBAND_RATE
CENTRE_HZ = 1500.0
"""Where the modems' signals sit in the audio passband: the middle of a 3 kHz SSB channel."""
PASSBAND_HALF_HZ = 1900.0
"""Half the width kept either side of the centre — 0 to 3.4 kHz of audio — which is what a
transceiver's SSB filter passes. Wider than either modem's signal, so it shapes nothing."""
CHANNEL_NAMES = ("awgn", "good", "moderate", "poor")


class CableChannel:
    """Audio in, impaired audio out, block by block, with every state carried across.

    A block of transmit audio is mixed down to complex baseband at 8 kHz, run through the
    model's :class:`~aether_model.channel.HfChannel` normalised to the reference signal
    power (so the noise is what the SNR says relative to the modem's level), and mixed back
    up. The fading has unit mean power, so the output level is the input level plus noise.
    """

    def __init__(
        self,
        channel: str = "awgn",
        snr_db: float | None = 10.0,
        signal_dbfs: float = -15.0,
        auto_level: bool = False,
        cfo_hz: float = 0.0,
        seed: int = 0,
        rate: int = AUDIO_RATE,
    ) -> None:
        if rate != AUDIO_RATE:
            raise ValueError(f"the cable runs at {AUDIO_RATE} Hz; set the devices to that")
        self.channel_name = channel
        self.snr_db = snr_db
        self.auto_level = auto_level
        # a real signal of RMS r mixed down and low-passed has power r²/2 at baseband
        self.reference_power = 10.0 ** (signal_dbfs / 10.0) / 2.0
        self._configured_power = self.reference_power
        self.hf = HfChannel(
            ChannelConfig(
                profile=channel,
                snr_db=snr_db,
                fs=float(BASEBAND_RATE),
                signal_power=1.0,
                cfo_hz=cfo_hz,
                seed=seed,
            )
        )
        taps = signal.firwin(241, PASSBAND_HALF_HZ, fs=rate)
        self._taps = taps.astype(np.float64)
        self._down_state = np.zeros(len(taps) - 1, dtype=np.complex128)
        self._up_state = np.zeros(len(taps) - 1, dtype=np.complex128)
        self._phase_down = 0
        self._phase_up = 0
        self._decimation_offset = 0
        self.blocks = 0
        self.active_blocks = 0
        self.last_input_power = 0.0
        self.last_output_peak = 0.0

    # ── levels ────────────────────────────────────────────────────────

    @property
    def reference_dbfs(self) -> float:
        """The signal RMS the noise is set against, in dBFS of the audio."""
        return 10.0 * math.log10(max(self.reference_power * 2.0, 1e-20))

    @property
    def noise_dbfs_3k(self) -> float | None:
        """The noise this channel adds, as RMS in a 3 kHz audio band, dBFS."""
        if self.snr_db is None:
            return None
        return self.reference_dbfs - self.snr_db

    def _track_level(self, power: float) -> None:
        """Follow the bursts: a block well above the noise counts as signal, and the
        reference moves a step of the way towards its power."""
        if not self.auto_level:
            return
        gate = self.reference_power / 10.0
        if power > gate:
            self.active_blocks += 1
            self.reference_power += (power - self.reference_power) * 0.05

    # ── the passband ↔ baseband conversion ────────────────────────────

    def _mix_down(self, audio: FloatArray) -> ComplexArray:
        n = len(audio)
        k = np.arange(self._phase_down, self._phase_down + n)
        self._phase_down = (self._phase_down + n) % AUDIO_RATE
        lo = np.exp(-2j * np.pi * CENTRE_HZ * k / AUDIO_RATE)
        mixed, self._down_state = signal.lfilter(
            self._taps, [1.0], audio.astype(np.float64) * lo, zi=self._down_state
        )
        # decimate on a grid that continues across blocks
        start = (-self._decimation_offset) % DECIMATION
        out = mixed[start::DECIMATION]
        self._decimation_offset = (self._decimation_offset + n) % DECIMATION
        return np.asarray(out, dtype=np.complex128)

    def _mix_up(self, baseband: ComplexArray, n_audio: int) -> FloatArray:
        stuffed = np.zeros(n_audio, dtype=np.complex128)
        stuffed[: len(baseband) * DECIMATION : DECIMATION] = baseband
        # the interpolation filter passes one image; the gain makes up for the zeros
        upsampled, self._up_state = signal.lfilter(
            self._taps * DECIMATION, [1.0], stuffed, zi=self._up_state
        )
        k = np.arange(self._phase_up, self._phase_up + n_audio)
        self._phase_up = (self._phase_up + n_audio) % AUDIO_RATE
        lo = np.exp(2j * np.pi * CENTRE_HZ * k / AUDIO_RATE)
        # the mix-down halved the amplitude (the other sideband went); this doubles it back
        return np.asarray(2.0 * np.real(upsampled * lo), dtype=np.float64)

    # ── one block ─────────────────────────────────────────────────────

    def process(self, audio: NDArray) -> NDArray[np.float32]:
        """One block of transmit audio in, the same length of channel out."""
        audio = np.asarray(audio, dtype=np.float64)
        if audio.ndim != 1 or len(audio) % DECIMATION:
            raise ValueError(f"blocks must be mono and a multiple of {DECIMATION} samples")
        baseband = self._mix_down(audio)
        power = float(np.mean(np.abs(baseband) ** 2)) if len(baseband) else 0.0
        self.last_input_power = power
        self._track_level(power)
        scale = math.sqrt(self.reference_power)
        impaired = self.hf.process(baseband / scale) * scale
        out = self._mix_up(impaired, len(audio))
        # a fade that piles two paths in phase, or a hot drive, must not wrap the cable
        peak = float(np.max(np.abs(out))) if len(out) else 0.0
        self.last_output_peak = peak
        if peak > 0.98:
            out = np.tanh(out / 0.98) * 0.98
        self.blocks += 1
        return out.astype(np.float32)


def find_device(name: str | int, kind: str) -> int:
    """The PortAudio index of a device by name substring (or index), input or output."""
    import sounddevice as sd

    devices = sd.query_devices()
    if isinstance(name, int) or (isinstance(name, str) and name.isdigit()):
        return int(name)
    key = "max_input_channels" if kind == "input" else "max_output_channels"
    apis = sd.query_hostapis()
    candidates = [
        (index, device)
        for index, device in enumerate(devices)
        if device[key] > 0 and name.lower() in str(device["name"]).lower()
    ]
    if not candidates:
        sys.exit(f"no {kind} device matches {name!r}; --list-devices shows what there is")
    # the same device is listed once per host API on Windows; WASAPI has the least latency
    for index, device in candidates:
        if "WASAPI" in str(apis[device["hostapi"]]["name"]):
            return index
    return candidates[0][0]


def list_devices() -> None:
    import sounddevice as sd

    apis = sd.query_hostapis()
    for index, device in enumerate(sd.query_devices()):
        kind = []
        if device["max_input_channels"] > 0:
            kind.append("in")
        if device["max_output_channels"] > 0:
            kind.append("out")
        api = str(apis[device["hostapi"]]["name"])
        print(f"{index:3d} {'/'.join(kind):6s} {api:12s} {device['name']}")


class Direction:
    """One modem's transmit cable to the other's receive cable, through a channel."""

    def __init__(self, label: str, tx_device: int, rx_device: int, channel: CableChannel) -> None:
        self.label = label
        self.channel = channel
        self.devices = (tx_device, rx_device)
        self.overflows = 0
        self.underflows = 0
        self.stream: object | None = None

    def start(self, blocksize: int) -> None:
        import sounddevice as sd

        def callback(indata, outdata, frames, time_info, status):  # type: ignore[no-untyped-def]
            if status:
                if status.input_overflow:
                    self.overflows += 1
                if status.output_underflow:
                    self.underflows += 1
            mono = np.ascontiguousarray(indata[:, 0], dtype=np.float64)
            outdata[:, 0] = self.channel.process(mono)

        self.stream = sd.Stream(
            device=self.devices,
            samplerate=AUDIO_RATE,
            blocksize=blocksize,
            channels=1,
            dtype="float32",
            latency="low",
            callback=callback,
        )
        self.stream.start()  # type: ignore[attr-defined]

    def stop(self) -> None:
        if self.stream is not None:
            self.stream.stop()  # type: ignore[attr-defined]
            self.stream.close()  # type: ignore[attr-defined]

    def status(self) -> str:
        c = self.channel
        level = 10.0 * math.log10(max(c.last_input_power * 2.0, 1e-20))
        noise = "off" if c.noise_dbfs_3k is None else f"{c.noise_dbfs_3k:6.1f}"
        return (
            f"{self.label}: in {level:6.1f} dBFS  ref {c.reference_dbfs:6.1f}  "
            f"noise(3k) {noise}  peak {c.last_output_peak:4.2f}  "
            f"over/under {self.overflows}/{self.underflows}"
        )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--list-devices", action="store_true")
    ap.add_argument("--a-tx", help="capture device: what modem A transmits into")
    ap.add_argument("--a-rx", help="playback device: what modem A receives from")
    ap.add_argument("--b-tx", help="capture device: what modem B transmits into")
    ap.add_argument("--b-rx", help="playback device: what modem B receives from")
    ap.add_argument("--channel", choices=CHANNEL_NAMES, default="awgn")
    ap.add_argument("--snr", type=float, default=10.0, help="dB in 3 kHz; 'none' for no noise")
    ap.add_argument("--no-noise", action="store_true")
    ap.add_argument("--signal-dbfs", type=float, default=-15.0, help="the modems' transmit RMS")
    ap.add_argument("--auto-level", action="store_true", help="follow each burst's power")
    ap.add_argument("--cfo-hz", type=float, default=0.0, help="carrier offset, both ways")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--block-ms", type=float, default=20.0)
    ap.add_argument("--duration", type=float, default=0.0, help="seconds; 0 runs until Ctrl-C")
    ap.add_argument("--log", type=Path, help="a CSV of the levels, once a second")
    args = ap.parse_args()

    if args.list_devices:
        list_devices()
        return 0
    missing = [k for k in ("a_tx", "a_rx", "b_tx", "b_rx") if getattr(args, k) is None]
    if missing:
        sys.exit("all four devices are needed: --a-tx --a-rx --b-tx --b-rx (--list-devices)")

    snr = None if args.no_noise else args.snr
    blocksize = round(AUDIO_RATE * args.block_ms / 1000.0 / DECIMATION) * DECIMATION
    make = lambda seed: CableChannel(  # noqa: E731
        args.channel, snr, args.signal_dbfs, args.auto_level, args.cfo_hz, seed
    )
    a_to_b = Direction(
        "A→B",
        find_device(args.a_tx, "input"),
        find_device(args.b_rx, "output"),
        make(args.seed),
    )
    b_to_a = Direction(
        "B→A",
        find_device(args.b_tx, "input"),
        find_device(args.a_rx, "output"),
        make(args.seed + 1000),
    )
    print(
        f"channel {args.channel}, SNR {'off' if snr is None else f'{snr:+.1f} dB'} in 3 kHz, "
        f"reference {args.signal_dbfs:.1f} dBFS{' (auto)' if args.auto_level else ''}, "
        f"{blocksize / AUDIO_RATE * 1000:.0f} ms blocks"
    )
    log = None
    writer = None
    if args.log:
        args.log.parent.mkdir(parents=True, exist_ok=True)
        log = args.log.open("w", newline="", encoding="utf-8")
        writer = csv.writer(log)
        writer.writerow(
            ["t_s", "direction", "in_dbfs", "ref_dbfs", "noise_dbfs_3k", "peak", "over", "under"]
        )
    stop = threading.Event()
    started = time.monotonic()
    a_to_b.start(blocksize)
    b_to_a.start(blocksize)
    try:
        while not stop.is_set():
            time.sleep(1.0)
            elapsed = time.monotonic() - started
            for direction in (a_to_b, b_to_a):
                print(f"{elapsed:6.0f} s  {direction.status()}", flush=True)
                if writer is not None:
                    c = direction.channel
                    writer.writerow(
                        [
                            f"{elapsed:.0f}",
                            direction.label,
                            f"{10.0 * math.log10(max(c.last_input_power * 2.0, 1e-20)):.1f}",
                            f"{c.reference_dbfs:.1f}",
                            "" if c.noise_dbfs_3k is None else f"{c.noise_dbfs_3k:.1f}",
                            f"{c.last_output_peak:.2f}",
                            direction.overflows,
                            direction.underflows,
                        ]
                    )
            if args.duration and elapsed >= args.duration:
                break
    except KeyboardInterrupt:
        pass
    finally:
        a_to_b.stop()
        b_to_a.stop()
        if log is not None:
            log.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
