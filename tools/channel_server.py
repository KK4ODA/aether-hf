"""The channel between two daemons in a scenario, in lockstep (ADR-0042).

    python tools/channel_server.py --scenario bench/scenarios/80m-nvis-evening-500.toml --port 0
                                   --port-file port.txt --report channel.json
                                   [--max-seconds S] [--progress-file F]

``tools/session_matrix.py`` starts it for each scenario; ``bench/scenarios/README.md`` lists the
scenario's keys.

Two ``aetherd --channel HOST:PORT`` daemons connect here. Every tick (20 ms of audio) each
station's queue plays on the shared sample clock; what it radiates goes through the
scenario's channel to the other — the reference model's fading and noise at the stated SNR
(:class:`channel_cable.CableChannel`, the A/B bench's own), plus the scenario's other
stations and static crashes (:mod:`aether_model.qrm`) — and each station is sent the block it
heard. A tick waits for both stations to ask for it, so the session runs as fast as the two
daemons can process audio and its clock is the audio's, as a station's is.

The radios are modelled at both ends, from what the field recordings showed (ADR-0036…0040):

* what a station plays reaches the air a sound card's latency later (0.25 s,
  ``DEVICE_LATENCY_S`` in the daemon), and its key is on the air from its first sample to its
  last — the daemon's own keyed lead and tail are in what it plays;
* ``tx_delay_ms`` — the first part of each transmission never radiates (a transmitter, or an
  amplifier, still switching);
* ``vox_hold_ms`` — the radio is keyed by VOX: by the audio itself, so the daemon's silent
  lead and tail key nothing, and it stays keyed that long after the audio stops (a SignaLink's
  DLY): nothing radiates, and the station hears nothing;
* ``rx_recovery_ms`` — the receiver delivers silence that long after the key comes up (a
  rig's T/R recovery, a codec unmuting);
* while a station transmits, its receive audio is silence, as a transceiver's is;
* ``agc`` — ``off``, ``fast``, ``auto`` or ``slow``: the receiver's AGC (:class:`Agc`), with
  ``agc_threshold_db`` above the noise it hears (12) and the hang and decay overridable.

And the computer: ``[stations.<who>.machine]`` makes the station's run loop stall, as a slow or
busy PC's does (KE4QCM's fell 0.3–27 s behind, 2026-10-09). A stall of ``S`` seconds is ``S``
of air the station neither hears nor answers in: what it queued before plays on (a sound card
plays what it was handed), what it hears piles up — the newest ``backlog_s`` (2 s, the
daemon's ``max_backlog_s``) kept and the rest dropped, as its capture queue does — and when it
ends the station is handed the pile and processes it before the clock moves on, as a loop that
has fallen behind catches up. A daemon that greets with ``AEC2`` is told how much was dropped
ahead of the pile (``MSG_GAP``), as a sound card's queue tells the run loop, and moves its clock
past it. ``stall_every_s`` is the mean time between stalls (drawn,
seeded), ``stall_s`` a length or a ``[shortest, longest]`` range; the command file's
``stall <who> S`` stalls one now.

The report says when each station was keyed on the air, and every stretch both were — a
collision, which no recording at either end can show. It splits that time three ways
(ADR-0069 §5): **radiated** — both transmitters putting out signal at once, a collision of the
protocol's —, **vox** — one of them only held keyed by its VOX — and **guard** — the rest, a
station's keyed lead or tail of silence against the other's transmission, which costs nobody a
frame. It also says, for each station, how much of its peer's signal reached it while it could
not hear (``deaf_hits_s``: transmitting, keyed in its own silence, held by its VOX, or its
receiver recovering), and where each transmission went on the air with the account the daemon
gave of it (``transmissions``: why it was keyed, what it carried, what the engine believed),
which ``tools/air_account.py`` turns into a collision's mechanism.
"""

from __future__ import annotations

import argparse
import faulthandler
import json
import math
import socket
import struct
import sys
import threading
import time
import tomllib
from dataclasses import dataclass, field, replace
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import HfChannel, SampleRateOffset, noise_power_for_snr
from aether_model.qrm import (
    AtmosphericCrashes,
    Ft8Station,
    OfdmArqStation,
    PactorStation,
    RttyStation,
)
from channel_cable import AUDIO_RATE, BASEBAND_RATE, CENTRE_HZ, CableChannel

TICK = 960
"""Samples a tick: 20 ms at 48 kHz, a sound card's block."""
DEVICE_LATENCY_S = 0.25
"""From the daemon (``audio::DEVICE_LATENCY_S``): played to on the air."""

MSG_BLOCK, MSG_PLAY, MSG_READY, MSG_CLEAR, MSG_TAG, MSG_GAP = 1, 2, 3, 4, 5, 6
RADIATED_BLOCK = 48
"""Samples a radiated block: 1 ms. A block with any signal in it radiates — a waveform crosses
zero, a keyed lead or tail of silence is all zeros."""
STALL_REPORT_S = 30.0
"""Wall-clock seconds without a tick before the server says where it stands (:func:`watchdog`).
A night's run once stood still at 277 s of air with both daemons waiting for their next block,
and nothing said what the server was doing (nightly 34, 2026-10-16)."""


# ── the channel, one direction ────────────────────────────────────────


SRO_CUSHION = BASEBAND_RATE // 4
"""Baseband samples held back when the clocks differ (0.25 s): a receiver whose card runs fast
takes the stream a little quicker than it comes, and this is what it eats into — 100 ppm for
40 minutes."""


def level_at(schedule: list[list[float]], t: np.ndarray) -> np.ndarray:
    """The path's level in dB at times ``t``: straight lines between ``[seconds, dB]`` points,
    the first and last held — QSB, a band closing, a skip zone moving through."""
    if not schedule:
        return np.zeros(len(t))
    times = np.array([float(p[0]) for p in schedule])
    levels = np.array([float(p[1]) for p in schedule])
    return np.asarray(np.interp(t, times, levels), dtype=np.float64)


class ScenarioChannel(CableChannel):
    """:class:`CableChannel` with the scenario's other signals and crashes added at the
    receiver, after the path's fading and noise — they reach the receiver by paths of their
    own — and, from the path's keys:

    * ``level`` — ``[[seconds, dB], …]``: the signal's level against ``snr_db`` over the
      session (the noise and the other signals stay where they are);
    * ``cfo_drift_hz_per_s`` — the offset moving, as a rig warming up does;
    * ``sro_ppm`` — the receiving card's clock against the sender's: the whole stream it
      hears is resampled, so a frame's timing walks across a long frame;
    * ``delay_ms`` — the path's travel time: 10–60 ms on a DX path (ten times a local one),
      added to every turnaround;
    * ``echo`` — ``{delay_ms, db}``: a second arrival of the same signal that much later and
      weaker, long path behind short path (20 m), far past the OFDM prefix; it fades with the
      first."""

    def __init__(
        self,
        extras: list[object],
        level: list[list[float]] | None = None,
        cfo_drift_hz_per_s: float = 0.0,
        sro_ppm: float = 0.0,
        delay_ms: float = 0.0,
        echo: dict | None = None,
        **kwargs: object,
    ) -> None:
        super().__init__(**kwargs)  # type: ignore[arg-type]
        self.extras = extras
        self.level = level or []
        self._delay = np.zeros(round(delay_ms * BASEBAND_RATE / 1000), dtype=np.complex128)
        echo = echo or {}
        self._echo_gain = 10.0 ** (float(echo.get("db", -6.0)) / 20.0) if echo else 0.0
        self._echo = np.zeros(
            round(float(echo.get("delay_ms", 0.0)) * BASEBAND_RATE / 1000) if echo else 0,
            dtype=np.complex128,
        )
        self._n = 0  # baseband samples processed
        if cfo_drift_hz_per_s or sro_ppm:
            cfg = self.hf.config
            self.hf = HfChannel(replace(cfg, cfo_drift_hz_per_s=cfo_drift_hz_per_s))
        self.sro = SampleRateOffset(float(BASEBAND_RATE), sro_ppm) if sro_ppm else None
        self._sro_fifo = np.zeros(SRO_CUSHION if sro_ppm else 0, dtype=np.complex128)

    def process(self, audio: np.ndarray) -> np.ndarray:  # type: ignore[override]
        audio = np.asarray(audio, dtype=np.float64)
        baseband = self._mix_down(audio)
        n = len(baseband)
        scale = math.sqrt(self.reference_power)
        if self.level:
            t = (self._n + np.arange(n)) / BASEBAND_RATE
            baseband = baseband * 10.0 ** (level_at(self.level, t) / 20.0)
        self._n += n
        if len(self._delay):
            line = np.concatenate((self._delay, baseband))
            baseband, self._delay = line[:n], line[n:]
        if len(self._echo):
            line = np.concatenate((self._echo, baseband))
            late, self._echo = line[:n], line[n:]
            baseband = baseband + self._echo_gain * late
        impaired = self.hf.process(baseband / scale)
        for extra in self.extras:
            impaired = impaired + extra.next(len(impaired))  # type: ignore[attr-defined]
        if self.sro is not None:
            self._sro_fifo = np.concatenate((self._sro_fifo, self.sro.process(impaired)))
            if len(self._sro_fifo) < n:  # the cushion spent: a gap, as a card that ran dry
                self._sro_fifo = np.concatenate(
                    (self._sro_fifo, np.zeros(n - len(self._sro_fifo), dtype=np.complex128))
                )
            impaired, self._sro_fifo = self._sro_fifo[:n], self._sro_fifo[n:]
        out = self._mix_up(impaired * scale, len(audio))
        peak = float(np.max(np.abs(out))) if len(out) else 0.0
        if peak > 0.98:
            out = np.tanh(out / 0.98) * 0.98
        return out.astype(np.float32)


def extras_for(scenario: dict, at: str, snr_db: float | None, seed: int) -> list[object]:
    """The other stations and the crashes the station ``at`` hears."""
    out: list[object] = []
    for i, q in enumerate(scenario.get("qrm", [])):
        if q.get("at", "both") not in (at, "both"):
            continue
        offset = float(q.get("audio_hz", CENTRE_HZ)) - CENTRE_HZ
        common = {
            "seed": seed * 1000 + i + STATION_SEED.get(at, 0),
            "profile": q.get("profile"),
        }
        power = float(q.get("power_db", 0.0))
        kind = q["kind"]
        if kind == "ofdm-arq":
            out.append(
                OfdmArqStation(
                    BASEBAND_RATE, offset, power, partner_db=q.get("partner_db", -6.0), **common
                )
            )
        elif kind == "pactor":
            out.append(
                PactorStation(
                    BASEBAND_RATE, offset, power, partner_db=q.get("partner_db", -6.0), **common
                )
            )
        elif kind == "rtty":
            out.append(RttyStation(BASEBAND_RATE, offset, power, **common))
        elif kind == "ft8":
            # a band of FT8 stations: `count` of them spread over `spread_hz` around the
            # centre, each at its own level within `power_db` ± 6 dB and its own duty
            rng = np.random.default_rng(common["seed"])
            count = int(q.get("count", 10))
            spread = float(q.get("spread_hz", 400.0))
            for k in range(count):
                out.append(
                    Ft8Station(
                        BASEBAND_RATE,
                        offset + float(rng.uniform(-spread / 2, spread / 2)),
                        power + float(rng.uniform(-6.0, 6.0)),
                        duty=float(rng.uniform(0.4, 0.9)),
                        seed=int(common["seed"]) * 100 + k,  # type: ignore[call-overload]
                        profile=common["profile"],  # type: ignore[arg-type]
                    )
                )
        else:
            raise SystemExit(f"unknown qrm kind {kind!r}: ofdm-arq, pactor, rtty or ft8")
    crashes = scenario.get("crashes")
    if crashes and crashes.get("at", "both") in (at, "both") and snr_db is not None:
        out.append(
            AtmosphericCrashes(
                BASEBAND_RATE,
                noise_power_for_snr(snr_db, 1.0, BASEBAND_RATE),
                rate_per_s=float(crashes.get("rate_per_s", 0.5)),
                peak_db=float(crashes.get("peak_db", 25.0)),
                spread_db=float(crashes.get("spread_db", 6.0)),
                seed=seed * 1000 + 900 + CRASH_SEED.get(at, 0),
            )
        )
    return out


STATION_SEED = {"a": 0, "b": 500, "c": 200, "d": 300, "e": 400, "f": 600}
"""Each station's offset into a scenario's seeds: ``a`` and ``b`` keep the two-station
numbers, so a two-station scenario draws exactly what it did before more were possible."""
CRASH_SEED = {"a": 0, "b": 1, "c": 2, "d": 3, "e": 4, "f": 5}
"""Each station's offset into the static's seeds, ``a`` and ``b`` as before."""


def stations_of(scenario: dict) -> list[str]:
    """The stations on the air: ``a`` and ``b``, then any more the scenario names (``c``,
    ``d``, …: other clients of one gateway, say)."""
    named = scenario.get("stations", {})
    return ["a", "b", *sorted(k for k in named if k not in ("a", "b") and len(k) == 1)]


def main_peer(who: str) -> str:
    """The station whose path to ``who`` carries the noise ``who`` hears: the gateway ``b``
    for every other station, and ``a`` for ``b``. A receiver has one noise floor; the other
    stations reach it through noise-free paths of their own on top of it."""
    return "a" if who == "b" else "b"


def path_for(scenario: dict, direction: str) -> dict:
    """The path ``a_to_b``, ``b_to_a``, ``c_to_b``, …: ``[path]`` with the direction's
    overrides."""
    base = {k: v for k, v in scenario.get("path", {}).items() if not isinstance(v, dict)}
    base.update(scenario.get("path", {}).get(direction, {}))
    return base


def signal_dbfs(scenario: dict, who: str) -> float:
    """The RMS a station's waveform plays at: ``tx_level / √2`` (a sine-amplitude level)."""
    level = float(scenario.get("stations", {}).get(who, {}).get("tx_level", 0.25))
    return 20.0 * math.log10(level / math.sqrt(2.0))


class Agc:
    """A receiver's AGC as the FTDX10 recordings showed it (``busy.rs``): the gain steps down
    within a millisecond when the envelope passes the threshold, holds ``hang_ms``, then ramps
    back at ``decay_db_s``. It acts on the whole passband — a crash or a strong station
    nearby takes this session's signal down with it — and it runs on 1 ms envelope steps."""

    STEP = AUDIO_RATE // 1000

    def __init__(self, threshold: float, hang_ms: float, decay_db_s: float) -> None:
        self.threshold = threshold
        self.hang = hang_ms / 1000.0
        self.decay = decay_db_s / 1000.0  # dB a step
        self.gain_db = 0.0
        self.held = 0.0  # seconds of hang left

    def process(self, audio: np.ndarray) -> np.ndarray:
        steps = audio.reshape(-1, self.STEP)
        peaks = np.max(np.abs(steps), axis=1)
        gains = np.empty(len(peaks))
        for i, peak in enumerate(peaks):
            limit = 20.0 * math.log10(self.threshold / peak) if peak > self.threshold else 0.0
            if limit < self.gain_db:
                self.gain_db, self.held = limit, self.hang
            elif self.held > 0.0:
                self.held -= 0.001
            else:
                self.gain_db = min(0.0, limit, self.gain_db + self.decay)
            gains[i] = self.gain_db
        out = steps * (10.0 ** (gains / 20.0))[:, None]
        return out.reshape(-1).astype(np.float32)


# ── one station ───────────────────────────────────────────────────────


@dataclass
class Station:
    name: str
    sock: socket.socket
    tx_delay: int
    vox_hold: int
    recovery: int
    queue: list[np.ndarray] = field(default_factory=list)
    queued: int = 0
    consumed: int = 0
    gone: bool = False
    # on the air, delayed by the card's latency: samples, and whether each radiated
    air: np.ndarray = field(default_factory=lambda: np.zeros(0, dtype=np.float32))
    air_keyed: np.ndarray = field(default_factory=lambda: np.zeros(0, dtype=bool))
    keyed_since: int | None = None
    last_keyed: int = -(10**12)
    intervals: list[list[float]] = field(default_factory=list)
    agc: Agc | None = None
    # the computer (``machine``): blocks sent and not yet answered by a READY, the air sample
    # a stall lasts until, the blocks heard during it, and what was dropped from them
    owed: int = 1  # the first READY comes before the first block
    stall_from: int = 0
    stall_until: int = 0
    backlog: list[np.ndarray] = field(default_factory=list)
    backlog_limit: int = 2 * AUDIO_RATE
    stalls: list[list[float]] = field(default_factory=list)
    stall_dropped: int = 0
    # the daemon reads MSG_GAP (it said ``AEC2``): told what a stall dropped, its clock moves
    # on by it, as a sound card's queue tells the run loop
    reads_gaps: bool = False
    next_stall: int | None = None
    stall_every: float = 0.0
    stall_range: tuple[float, float] = (0.0, 0.0)
    rng: np.random.Generator | None = None
    # the account of the air (ADR-0069 §5): samples ever queued, the daemon's tags waiting for
    # their first sample to be played, the transmissions placed on the air, where the station
    # radiated, and how much of its peer it missed while deaf, by why
    queued_total: int = 0
    tags_waiting: list[tuple[int, dict]] = field(default_factory=list)
    transmissions: list[dict] = field(default_factory=list)
    radiated: list[list[float]] = field(default_factory=list)
    radiating_since: int | None = None
    deaf_hits: dict[str, int] = field(
        default_factory=lambda: {"transmitting": 0, "own_silence": 0, "vox": 0, "recovering": 0}
    )
    # what the server is reading from the station, for the watchdog
    reading: str = "nothing yet"

    def read_exact(self, n: int) -> bytes:
        data = b""
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise ConnectionError(self.name)
            data += chunk
        return data

    def until_ready(self) -> None:
        """Take what the station sends until it asks for its next block."""
        while True:
            self.reading = "the next message"
            kind = self.read_exact(1)[0]
            if kind == MSG_READY:
                return
            self.reading = f"message {kind}"
            if kind == MSG_PLAY:
                (n,) = struct.unpack("<I", self.read_exact(4))
                self.reading = f"{n} samples to play"
                samples = np.frombuffer(self.read_exact(4 * n), dtype="<f4").astype(np.float32)
                self.queue.append(samples)
                self.queued += n
                self.queued_total += n
            elif kind == MSG_CLEAR:
                self.queue.clear()
                self.queued = 0
                # what was queued and not played never goes: the next sample queued is the
                # next one played, and the accounts of what was dropped go with it
                self.queued_total = self.consumed
                self.tags_waiting.clear()
            elif kind == MSG_TAG:
                (n,) = struct.unpack("<I", self.read_exact(4))
                raw = self.read_exact(n).decode("utf-8", errors="replace")
                try:
                    tag = json.loads(raw)
                except json.JSONDecodeError:
                    tag = {"raw": raw}
                # it describes the samples queued next
                self.tags_waiting.append((self.queued_total, tag))
            else:
                raise ConnectionError(f"{self.name} sent message {kind}")

    def stall(self, t: int, samples: int) -> None:
        """Stall from air sample ``t`` for ``samples``: a loop that stopped for that long."""
        until = t + samples
        if until > self.stall_until:
            if self.stall_from <= t < self.stall_until:
                self.stalls[-1][1] = until / AUDIO_RATE
            else:
                self.stalls.append([t / AUDIO_RATE, until / AUDIO_RATE])
                self.stall_from = t
            self.stall_until = until

    def stalled(self, t: int) -> bool:
        """Whether the station's loop stands still at air sample ``t``, drawing the next
        stall from its ``machine`` when the last has passed."""
        if self.rng is not None and self.stall_every > 0:
            if self.next_stall is None:
                self.next_stall = t + round(self.rng.exponential(self.stall_every) * AUDIO_RATE)
            if t >= self.next_stall:
                lo, hi = self.stall_range
                self.stall(t, round(self.rng.uniform(lo, hi) * AUDIO_RATE))
                self.next_stall = self.stall_until + round(
                    self.rng.exponential(self.stall_every) * AUDIO_RATE
                )
        return self.stall_from <= t < self.stall_until

    def deliver(self, clock: int, samples: np.ndarray, stalled: bool) -> None:
        """Send the block heard — or, stalled, keep it; the first block after a stall carries
        the pile, the newest ``backlog_limit`` of it, as the capture queue keeps it."""
        if stalled:
            self.backlog.append(samples)
            return
        if self.backlog:
            pile = np.concatenate([*self.backlog, samples])
            self.backlog.clear()
            if len(pile) > self.backlog_limit:
                lost = len(pile) - self.backlog_limit
                self.stall_dropped += lost
                pile = pile[-self.backlog_limit :]
                if self.reads_gaps:
                    self.sock.sendall(struct.pack("<BQ", MSG_GAP, lost))
            for i in range(0, len(pile), TICK):
                self.send_block(clock, pile[i : i + TICK])
                self.owed += 1
            return
        self.send_block(clock, samples)
        self.owed += 1

    def take(self, n: int) -> tuple[np.ndarray, np.ndarray]:
        """``n`` samples of what it plays, and which of them were its own (not idle)."""
        out = np.zeros(n, dtype=np.float32)
        real = np.zeros(n, dtype=bool)
        filled = 0
        while filled < n and self.queue:
            head = self.queue[0]
            k = min(n - filled, len(head))
            out[filled : filled + k] = head[:k]
            real[filled : filled + k] = True
            filled += k
            if k == len(head):
                self.queue.pop(0)
            else:
                self.queue[0] = head[k:]
        self.queued -= filled
        self.consumed += filled
        return out, real

    def send_block(self, clock: int, samples: np.ndarray) -> None:
        """The block heard, with the playback clock — what the station's device has played,
        silence included, as a sound card counts it — and how much of its queue that was."""
        head = struct.pack("<BQQI", MSG_BLOCK, clock, self.consumed, len(samples))
        self.sock.sendall(head + samples.astype("<f4").tobytes())


# ── the server ────────────────────────────────────────────────────────


class Server:
    """The air between the stations. Each receiver hears its main peer (:func:`main_peer`)
    through a path with the noise, the other signals and the static it hears; with more than
    two stations, every other station reaches it through a fading path of its own with no
    noise, its level set by that path's ``snr_db`` against the same floor — so the noise is
    counted once, as a receiver has one floor however many stations it hears."""

    def __init__(self, scenario: dict, stations: dict[str, Station]) -> None:
        self.scenario = scenario
        self.stations = stations
        self.doing = "starting"  # for the watchdog
        self.a, self.b = stations["a"], stations["b"]
        seed = int(scenario.get("seed", 1))
        # (sender, receiver) → the path, and its gain against the receiver's floor
        self.paths: dict[tuple[str, str], ScenarioChannel] = {}
        self.gains: dict[tuple[str, str], float] = {}
        names = list(stations)
        for rx in names:
            main = main_peer(rx)
            main_snr = path_for(scenario, f"{main}_to_{rx}").get("snr_db", 10.0)
            for tx in names:
                if tx == rx:
                    continue
                path = path_for(scenario, f"{tx}_to_{rx}")
                snr = path.get("snr_db", 10.0)
                noisy = tx == main
                pair_seed = (
                    seed * 10 + (1 if tx == "a" else 2)
                    if noisy and rx in ("a", "b") and tx in ("a", "b")
                    else seed * 10 + 100 * (names.index(tx) + 1) + names.index(rx)
                )
                self.paths[(tx, rx)] = ScenarioChannel(
                    extras_for(scenario, rx, snr, seed) if noisy else [],
                    channel=path.get("profile", "awgn"),
                    snr_db=snr if noisy else None,
                    signal_dbfs=signal_dbfs(scenario, tx),
                    cfo_hz=float(path.get("cfo_hz", 0.0)),
                    cfo_drift_hz_per_s=float(path.get("cfo_drift_hz_per_s", 0.0)),
                    sro_ppm=float(path.get("sro_ppm", 0.0)),
                    level=path.get("level"),
                    delay_ms=float(path.get("delay_ms", 0.0)),
                    echo=path.get("echo"),
                    seed=pair_seed,
                )
                self.gains[(tx, rx)] = 1.0 if noisy else 10.0 ** ((snr - main_snr) / 20.0)
        for who, st in stations.items():
            radio = scenario.get("stations", {}).get(who, {}).get("radio", {})
            if radio.get("agc", "off") == "off":
                continue
            # the threshold sits above the noise the station hears, as a rig's does
            floor = self.paths[(main_peer(who), who)].noise_dbfs_3k
            noise_rms = 10.0 ** ((floor or -60.0) / 20.0)
            hang, decay = {"fast": (20.0, 200.0), "auto": (100.0, 50.0), "slow": (400.0, 15.0)}[
                radio["agc"]
            ]
            st.agc = Agc(
                noise_rms * 10.0 ** (float(radio.get("agc_threshold_db", 12.0)) / 20.0),
                float(radio.get("agc_hang_ms", hang)),
                float(radio.get("agc_decay_db_s", decay)),
            )
        self.latency = round(DEVICE_LATENCY_S * AUDIO_RATE)
        for st in stations.values():
            st.air = np.zeros(self.latency, dtype=np.float32)
            st.air_keyed = np.zeros(self.latency, dtype=bool)
        self.t = 0  # the air clock: samples since the start
        # an outage: until this sample neither station's signal reaches the other — a station
        # switched off, or a band gone dead — while both keep running (``outage S`` commands)
        self.outage_until = 0
        self.outages: list[list[float]] = []
        self.both_keyed = 0
        self.collisions: list[list[float]] = []
        self._colliding_since: int | None = None
        # the keyed overlap split (radiated, vox, guard), in samples, in all and per collision
        self.overlap = {"radiated": 0, "vox": 0, "guard": 0}
        self.collision_split: list[dict[str, int]] = []
        self.rf_overlaps: list[list[float]] = []
        self._rf_since: int | None = None
        # each path's travel time, so a receiver's deafness is set against the signal as it
        # arrives, not as it leaves
        self.delays = {
            (tx, rx): round(
                float(path_for(scenario, f"{tx}_to_{rx}").get("delay_ms", 0.0)) * AUDIO_RATE / 1000
            )
            for tx in names
            for rx in names
            if tx != rx
        }
        self._in_flight = {key: np.zeros(n, dtype=bool) for key, n in self.delays.items()}

    def _radiate(self, st: Station) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
        """This tick of what the station puts on the air; when it is keyed; when it radiates
        signal; and when it is keyed only by its VOX's hold."""
        before = st.consumed
        played, real = st.take(TICK)
        if st.vox_hold > 0:
            # a VOX interface keys on audio: the daemon's silent lead and tail trip nothing, and
            # its hold starts where the frames end — where an operator's delay time counts from
            real = real & (np.abs(played) > 1e-9)
        # a tag goes on the air with the first sample it describes, a card's latency after it
        # is played
        while st.tags_waiting and st.tags_waiting[0][0] < st.consumed:
            offset, tag = st.tags_waiting.pop(0)
            at = self.t + self.latency + max(0, offset - before)
            st.transmissions.append({"air_s": round(at / AUDIO_RATE, 3), **tag})
        st.air = np.concatenate((st.air, played))
        st.air_keyed = np.concatenate((st.air_keyed, real))
        audio, keyed = st.air[:TICK].copy(), st.air_keyed[:TICK].copy()
        st.air, st.air_keyed = st.air[TICK:], st.air_keyed[TICK:]
        played_keyed = keyed.copy()
        radiating = keyed.copy()
        for i in range(TICK):
            t = self.t + i
            if keyed[i]:
                if st.keyed_since is None:
                    st.keyed_since = t
                    st.intervals.append([t / AUDIO_RATE, t / AUDIO_RATE])
                st.intervals[-1][1] = (t + 1) / AUDIO_RATE
                st.last_keyed = t
                if t - st.keyed_since < st.tx_delay:
                    radiating[i] = False
            elif st.keyed_since is not None and t - st.last_keyed > st.vox_hold:
                st.keyed_since = None
            # a VOX hold keeps the transmitter on, radiating nothing
            if not keyed[i] and t - st.last_keyed <= st.vox_hold:
                keyed[i] = True
        audio[~radiating] = 0.0
        signal = np.repeat(
            (np.abs(audio) > 1e-9).reshape(-1, RADIATED_BLOCK).any(axis=1), RADIATED_BLOCK
        )
        vox = keyed & ~played_keyed
        return audio, keyed, signal, vox

    def _deaf(self, st: Station, keyed: np.ndarray) -> np.ndarray:
        """Where the station hears nothing: keyed, held, or recovering."""
        t = self.t + np.arange(TICK)
        recovering = (t - st.last_keyed) <= st.vox_hold + st.recovery
        return keyed | recovering

    def tick(self) -> None:
        # every block sent has been processed — the pile after a stall included, so a station
        # that fell behind catches up before the air moves on — unless it is stalled now
        stalled = {who: st.stalled(self.t) for who, st in self.stations.items()}
        for st in self.stations.values():
            self.doing = f"waiting on {st.name}"
            while st.owed > 0:
                st.until_ready()
                st.owed -= 1
        self.doing = "making the air"
        audio: dict[str, np.ndarray] = {}
        keyed: dict[str, np.ndarray] = {}
        signal: dict[str, np.ndarray] = {}
        vox: dict[str, np.ndarray] = {}
        for who, st in self.stations.items():
            audio[who], keyed[who], signal[who], vox[who] = self._radiate(st)
            if self.t < self.outage_until:
                audio[who] = np.zeros_like(audio[who])
            self._note_radiated(st, signal[who])
        heard: dict[str, np.ndarray] = {}
        for rx, st in self.stations.items():
            main = main_peer(rx)
            out = self.paths[(main, rx)].process(audio[main])
            for tx in self.stations:
                if tx not in (rx, main):
                    out = out + self.paths[(tx, rx)].process(audio[tx] * self.gains[(tx, rx)])
            deaf = self._deaf(st, keyed[rx])
            out[deaf] = 0.0
            if self.t >= self.outage_until:
                self._note_deaf_hits(st, rx, main, signal, keyed, vox, deaf)
            if st.agc is not None:
                out = st.agc.process(out)
            heard[rx] = out
        # two or more transmitters keyed at once
        both = np.sum(np.stack(list(keyed.values())), axis=0) >= 2
        if self.t < self.outage_until:
            both[:] = False  # neither hears the other: not a collision on this path
        self.both_keyed += int(both.sum())
        # what the keyed overlap was: signal from two at once, a VOX's hold, or a keyed silence
        radiated = np.sum(np.stack(list(signal.values())), axis=0) >= 2
        held = np.any(np.stack(list(vox.values())), axis=0)
        split = np.where(radiated, 0, np.where(held, 1, 2))
        for i in np.flatnonzero(both):
            t = self.t + int(i)
            if self._colliding_since is None or t > self._colliding_since + 1:
                self.collisions.append([t / AUDIO_RATE, (t + 1) / AUDIO_RATE])
                self.collision_split.append({"radiated": 0, "vox": 0, "guard": 0})
            self.collisions[-1][1] = (t + 1) / AUDIO_RATE
            self._colliding_since = t
            what = ("radiated", "vox", "guard")[int(split[i])]
            self.overlap[what] += 1
            self.collision_split[-1][what] += 1
            if what == "radiated":
                if self._rf_since is None or t > self._rf_since + 1:
                    self.rf_overlaps.append([t / AUDIO_RATE, (t + 1) / AUDIO_RATE])
                self.rf_overlaps[-1][1] = (t + 1) / AUDIO_RATE
                self._rf_since = t
        for who, st in self.stations.items():
            self.doing = f"sending {st.name} its block"
            st.deliver(self.t + TICK, heard[who], stalled[who])
        self.t += TICK

    def _note_radiated(self, st: Station, signal: np.ndarray) -> None:
        """Extend the station's radiated intervals by this tick, a radiated block at a time."""
        blocks = signal[::RADIATED_BLOCK]
        for b, on in enumerate(blocks):
            t = self.t + b * RADIATED_BLOCK
            if on and st.radiating_since is None:
                st.radiating_since = t
                st.radiated.append([t / AUDIO_RATE, t / AUDIO_RATE])
            if on:
                st.radiated[-1][1] = (t + RADIATED_BLOCK) / AUDIO_RATE
            elif st.radiating_since is not None:
                st.radiating_since = None

    def _note_deaf_hits(
        self,
        st: Station,
        rx: str,
        tx: str,
        signal: dict[str, np.ndarray],
        keyed: dict[str, np.ndarray],
        vox: dict[str, np.ndarray],
        deaf: np.ndarray,
    ) -> None:
        """How much of the peer's signal reached the station, after the path's travel time,
        while it could not hear it — and why: its own transmission, its own keyed silence, its
        VOX holding it keyed, or its receiver still recovering."""
        line = np.concatenate((self._in_flight[(tx, rx)], signal[tx]))
        arriving, self._in_flight[(tx, rx)] = line[:TICK], line[TICK:]
        lost = arriving & deaf
        if not lost.any():
            return
        own = signal[rx]
        st.deaf_hits["transmitting"] += int((lost & own).sum())
        st.deaf_hits["vox"] += int((lost & vox[rx]).sum())
        st.deaf_hits["own_silence"] += int((lost & keyed[rx] & ~own & ~vox[rx]).sum())
        st.deaf_hits["recovering"] += int((lost & ~keyed[rx]).sum())

    def report(self) -> dict:
        seconds = lambda n: round(n / AUDIO_RATE, 3)  # noqa: E731
        return {
            "scenario": self.scenario.get("name"),
            "seconds": self.t / AUDIO_RATE,
            "latency_s": DEVICE_LATENCY_S,
            "stations": {
                st.name: {
                    "keyed": [[round(x, 3) for x in iv] for iv in st.intervals],
                    "radiated": [[round(x, 3) for x in iv] for iv in st.radiated],
                    "stalls": [[round(x, 3) for x in iv] for iv in st.stalls],
                    "stall_dropped_s": round(st.stall_dropped / AUDIO_RATE, 3),
                    "deaf_hits_s": {k: seconds(v) for k, v in st.deaf_hits.items()},
                    "transmissions": st.transmissions,
                }
                for st in self.stations.values()
            },
            "both_keyed_s": round(self.both_keyed / AUDIO_RATE, 3),
            "overlap_s": {k: seconds(v) for k, v in self.overlap.items()},
            "collisions": [[round(x, 3) for x in iv] for iv in self.collisions],
            "collision_split_s": [
                {k: seconds(v) for k, v in split.items()} for split in self.collision_split
            ],
            "rf_overlaps": [[round(x, 3) for x in iv] for iv in self.rf_overlaps],
            "outages": self.outages,
        }


MACHINE_SEED = 7919
"""Offset of the stalls' seeds from the scenario's: the channel's draws stay as they were."""


def machine(st: Station, spec: dict, seed: int, who: str) -> None:
    """Set the station's computer from its scenario's ``machine`` table."""
    st.backlog_limit = round(float(spec.get("backlog_s", 2.0)) * AUDIO_RATE)
    st.stall_every = float(spec.get("stall_every_s", 0.0))
    length = spec.get("stall_s", [0.5, 3.0])
    lo, hi = (length, length) if isinstance(length, int | float) else length
    st.stall_range = (float(lo), float(hi))
    if st.stall_every > 0:
        st.rng = np.random.default_rng([seed, MACHINE_SEED, sum(map(ord, who))])


def watchdog(server: Server) -> None:
    """Say once, when the air has stood still for :data:`STALL_REPORT_S`, where the server is —
    the air time, what it is doing, what it is reading from each station and the READYs it is
    owed — with every thread's stack. The main loop also arms ``faulthandler``'s own timer each
    second of air, which dumps the stacks without needing the interpreter: if a loop holds it,
    this thread cannot run."""
    last, said = -1, False
    while True:
        time.sleep(STALL_REPORT_S)
        if server.t != last:
            last, said = server.t, False
            continue
        if said:
            continue
        said = True
        stations = "; ".join(
            f"{st.name} owes {st.owed} READY, reading {st.reading}"
            for st in server.stations.values()
        )
        print(
            f"stalled at {server.t / AUDIO_RATE:.2f} s of air for {STALL_REPORT_S:.0f} s: "
            f"{server.doing}; {stations}",
            flush=True,
        )
        faulthandler.dump_traceback(file=sys.stdout, all_threads=True)
        sys.stdout.flush()


def accept_station(listener: socket.socket, radios: dict) -> Station:
    sock, _ = listener.accept()
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    st = Station("?", sock, 0, 0, 0)
    magic = st.read_exact(4)
    if magic not in (b"AEC1", b"AEC2"):
        raise SystemExit(f"not a daemon: {magic!r}")
    st.reads_gaps = magic == b"AEC2"
    rate, length = struct.unpack("<IH", st.read_exact(6))
    if rate != AUDIO_RATE:
        raise SystemExit(f"a daemon at {rate} Hz; the channel runs at {AUDIO_RATE}")
    st.name = st.read_exact(length).decode()
    return st


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--scenario", required=True, type=Path)
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--port-file", type=Path, help="write the port listened on here")
    ap.add_argument("--report", type=Path, help="where to write the air's account")
    ap.add_argument("--max-seconds", type=float, help="stop after this much air time")
    ap.add_argument(
        "--progress-file", type=Path, help="kept at the air time reached, every second of it"
    )
    ap.add_argument(
        "--command-file",
        type=Path,
        help=(
            "read every second of air for new lines: `outage S` cuts the path both ways for "
            "S s, `stall WHO S` stops a station's loop for S s"
        ),
    )
    args = ap.parse_args()
    scenario = tomllib.loads(args.scenario.read_text(encoding="utf-8"))
    stations = scenario.get("stations", {})
    names = stations_of(scenario)
    callsign = {who: stations.get(who, {}).get("callsign", who) for who in names}

    listener = socket.create_server(("127.0.0.1", args.port))
    port = listener.getsockname()[1]
    if args.port_file:
        args.port_file.write_text(str(port), encoding="utf-8")
    print(f"channel server on 127.0.0.1:{port}", flush=True)
    joined = [accept_station(listener, stations) for _ in names]
    by_name = {st.name: st for st in joined}
    try:
        on_air = {who: by_name[callsign[who]] for who in names}
    except KeyError:
        on_air = dict(zip(names, joined, strict=True))
    for who, st in on_air.items():
        radio = stations.get(who, {}).get("radio", {})
        st.tx_delay = round(float(radio.get("tx_delay_ms", 0)) * AUDIO_RATE / 1000)
        st.vox_hold = round(float(radio.get("vox_hold_ms", 0)) * AUDIO_RATE / 1000)
        st.recovery = round(float(radio.get("rx_recovery_ms", 0)) * AUDIO_RATE / 1000)
        machine(st, stations.get(who, {}).get("machine", {}), int(scenario.get("seed", 1)), who)
    server = Server(scenario, on_air)
    limit = args.max_seconds or float(scenario.get("seconds", 3600))
    commands_done = 0
    threading.Thread(target=watchdog, args=(server,), daemon=True).start()
    try:
        while server.t / AUDIO_RATE < limit:
            server.tick()
            if server.t % AUDIO_RATE < TICK:
                faulthandler.dump_traceback_later(2 * STALL_REPORT_S, file=sys.stdout)
                if args.progress_file:
                    # written in place: a reader can catch it empty or half written, and keeps
                    # its last reading then (session_matrix's Run.air_s). Replacing it whole
                    # fails on Windows while the reader has it open.
                    args.progress_file.write_text(f"{server.t / AUDIO_RATE:.2f}", encoding="utf-8")
                if args.command_file and args.command_file.exists():
                    lines = args.command_file.read_text(encoding="utf-8").splitlines()
                    for line in lines[commands_done:]:
                        word, _, arg = line.partition(" ")
                        if word == "stall":
                            who, _, length = arg.partition(" ")
                            if who in on_air:
                                on_air[who].stall(server.t, round(float(length) * AUDIO_RATE))
                        if word == "outage":
                            server.outage_until = server.t + round(float(arg) * AUDIO_RATE)
                            server.outages.append(
                                [server.t / AUDIO_RATE, server.outage_until / AUDIO_RATE]
                            )
                    commands_done = len(lines)
    except (ConnectionError, OSError):
        pass
    finally:
        faulthandler.cancel_dump_traceback_later()
        if args.report:
            args.report.write_text(json.dumps(server.report(), indent=1), encoding="utf-8")
        for st in on_air.values():
            st.sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
