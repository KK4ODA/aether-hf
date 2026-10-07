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
* ``vox_hold_ms`` — the transmitter stays keyed that long after the audio stops (a
  SignaLink's DLY): nothing radiates, and the station hears nothing;
* ``rx_recovery_ms`` — the receiver delivers silence that long after the key comes up (a
  rig's T/R recovery, a codec unmuting);
* while a station transmits, its receive audio is silence, as a transceiver's is;
* ``agc`` — ``off``, ``fast``, ``auto`` or ``slow``: the receiver's AGC (:class:`Agc`), with
  ``agc_threshold_db`` above the noise it hears (12) and the hang and decay overridable.

The report says when each station was keyed on the air, and every stretch both were — a
collision, which no recording at either end can show.
"""

from __future__ import annotations

import argparse
import json
import math
import socket
import struct
import sys
import tomllib
from dataclasses import dataclass, field, replace
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import HfChannel, SampleRateOffset, noise_power_for_snr
from aether_model.qrm import (
    AtmosphericCrashes,
    OfdmArqStation,
    PactorStation,
    RttyStation,
)
from channel_cable import AUDIO_RATE, BASEBAND_RATE, CENTRE_HZ, CableChannel

TICK = 960
"""Samples a tick: 20 ms at 48 kHz, a sound card's block."""
DEVICE_LATENCY_S = 0.25
"""From the daemon (``audio::DEVICE_LATENCY_S``): played to on the air."""

MSG_BLOCK, MSG_PLAY, MSG_READY, MSG_CLEAR = 1, 2, 3, 4


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
      hears is resampled, so a frame's timing walks across a long frame."""

    def __init__(
        self,
        extras: list[object],
        level: list[list[float]] | None = None,
        cfo_drift_hz_per_s: float = 0.0,
        sro_ppm: float = 0.0,
        **kwargs: object,
    ) -> None:
        super().__init__(**kwargs)  # type: ignore[arg-type]
        self.extras = extras
        self.level = level or []
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
            "seed": seed * 1000 + i + (500 if at == "b" else 0),
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
        else:
            raise SystemExit(f"unknown qrm kind {kind!r}: ofdm-arq, pactor or rtty")
    crashes = scenario.get("crashes")
    if crashes and crashes.get("at", "both") in (at, "both") and snr_db is not None:
        out.append(
            AtmosphericCrashes(
                BASEBAND_RATE,
                noise_power_for_snr(snr_db, 1.0, BASEBAND_RATE),
                rate_per_s=float(crashes.get("rate_per_s", 0.5)),
                peak_db=float(crashes.get("peak_db", 25.0)),
                spread_db=float(crashes.get("spread_db", 6.0)),
                seed=seed * 1000 + 900 + (1 if at == "b" else 0),
            )
        )
    return out


def path_for(scenario: dict, direction: str) -> dict:
    """The path ``a_to_b`` or ``b_to_a``: ``[path]`` with the direction's overrides."""
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
            kind = self.read_exact(1)[0]
            if kind == MSG_READY:
                return
            if kind == MSG_PLAY:
                (n,) = struct.unpack("<I", self.read_exact(4))
                samples = np.frombuffer(self.read_exact(4 * n), dtype="<f4").astype(np.float32)
                self.queue.append(samples)
                self.queued += n
            elif kind == MSG_CLEAR:
                self.queue.clear()
                self.queued = 0
            else:
                raise ConnectionError(f"{self.name} sent message {kind}")

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
    def __init__(self, scenario: dict, a: Station, b: Station) -> None:
        self.scenario = scenario
        self.a, self.b = a, b
        seed = int(scenario.get("seed", 1))
        self.channels = {}
        for tx, rx, direction in (("a", "b", "a_to_b"), ("b", "a", "b_to_a")):
            path = path_for(scenario, direction)
            snr = path.get("snr_db", 10.0)
            self.channels[tx] = ScenarioChannel(
                extras_for(scenario, rx, snr, seed),
                channel=path.get("profile", "awgn"),
                snr_db=snr,
                signal_dbfs=signal_dbfs(scenario, tx),
                cfo_hz=float(path.get("cfo_hz", 0.0)),
                cfo_drift_hz_per_s=float(path.get("cfo_drift_hz_per_s", 0.0)),
                sro_ppm=float(path.get("sro_ppm", 0.0)),
                level=path.get("level"),
                seed=seed * 10 + (1 if tx == "a" else 2),
            )
        for st, who, tx in ((a, "a", "b"), (b, "b", "a")):
            radio = scenario.get("stations", {}).get(who, {}).get("radio", {})
            if radio.get("agc", "off") == "off":
                continue
            # the threshold sits above the noise the station hears, as a rig's does
            noise_rms = 10.0 ** ((self.channels[tx].noise_dbfs_3k or -60.0) / 20.0)
            hang, decay = {"fast": (20.0, 200.0), "auto": (100.0, 50.0), "slow": (400.0, 15.0)}[
                radio["agc"]
            ]
            st.agc = Agc(
                noise_rms * 10.0 ** (float(radio.get("agc_threshold_db", 12.0)) / 20.0),
                float(radio.get("agc_hang_ms", hang)),
                float(radio.get("agc_decay_db_s", decay)),
            )
        self.latency = round(DEVICE_LATENCY_S * AUDIO_RATE)
        for st in (a, b):
            st.air = np.zeros(self.latency, dtype=np.float32)
            st.air_keyed = np.zeros(self.latency, dtype=bool)
        self.t = 0  # the air clock: samples since the start
        self.both_keyed = 0
        self.collisions: list[list[float]] = []
        self._colliding_since: int | None = None

    def _radiate(self, st: Station) -> tuple[np.ndarray, np.ndarray]:
        """This tick of what the station puts on the air, and when it is keyed."""
        played, real = st.take(TICK)
        st.air = np.concatenate((st.air, played))
        st.air_keyed = np.concatenate((st.air_keyed, real))
        audio, keyed = st.air[:TICK].copy(), st.air_keyed[:TICK].copy()
        st.air, st.air_keyed = st.air[TICK:], st.air_keyed[TICK:]
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
        return audio, keyed

    def _deaf(self, st: Station, keyed: np.ndarray) -> np.ndarray:
        """Where the station hears nothing: keyed, held, or recovering."""
        t = self.t + np.arange(TICK)
        recovering = (t - st.last_keyed) <= st.vox_hold + st.recovery
        return keyed | recovering

    def tick(self) -> None:
        for st in (self.a, self.b):
            st.until_ready()
        audio_a, keyed_a = self._radiate(self.a)
        audio_b, keyed_b = self._radiate(self.b)
        heard_b = self.channels["a"].process(audio_a)
        heard_a = self.channels["b"].process(audio_b)
        heard_a[self._deaf(self.a, keyed_a)] = 0.0
        heard_b[self._deaf(self.b, keyed_b)] = 0.0
        if self.a.agc is not None:
            heard_a = self.a.agc.process(heard_a)
        if self.b.agc is not None:
            heard_b = self.b.agc.process(heard_b)
        both = keyed_a & keyed_b
        self.both_keyed += int(both.sum())
        for i in np.flatnonzero(both):
            t = self.t + int(i)
            if self._colliding_since is None or t > self._colliding_since + 1:
                self.collisions.append([t / AUDIO_RATE, (t + 1) / AUDIO_RATE])
            self.collisions[-1][1] = (t + 1) / AUDIO_RATE
            self._colliding_since = t
        self.a.send_block(self.t + TICK, heard_a)
        self.b.send_block(self.t + TICK, heard_b)
        self.t += TICK

    def report(self) -> dict:
        return {
            "scenario": self.scenario.get("name"),
            "seconds": self.t / AUDIO_RATE,
            "stations": {
                st.name: {"keyed": [[round(x, 3) for x in iv] for iv in st.intervals]}
                for st in (self.a, self.b)
            },
            "both_keyed_s": round(self.both_keyed / AUDIO_RATE, 3),
            "collisions": [[round(x, 3) for x in iv] for iv in self.collisions],
        }


def accept_station(listener: socket.socket, radios: dict) -> Station:
    sock, _ = listener.accept()
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    st = Station("?", sock, 0, 0, 0)
    magic = st.read_exact(4)
    if magic != b"AEC1":
        raise SystemExit(f"not a daemon: {magic!r}")
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
    args = ap.parse_args()
    scenario = tomllib.loads(args.scenario.read_text(encoding="utf-8"))
    stations = scenario.get("stations", {})
    callsign = {who: stations.get(who, {}).get("callsign", who) for who in ("a", "b")}

    listener = socket.create_server(("127.0.0.1", args.port))
    port = listener.getsockname()[1]
    if args.port_file:
        args.port_file.write_text(str(port), encoding="utf-8")
    print(f"channel server on 127.0.0.1:{port}", flush=True)
    joined = [accept_station(listener, stations), accept_station(listener, stations)]
    by_name = {st.name: st for st in joined}
    try:
        a, b = by_name[callsign["a"]], by_name[callsign["b"]]
    except KeyError:
        a, b = joined
    for who, st in (("a", a), ("b", b)):
        radio = stations.get(who, {}).get("radio", {})
        st.tx_delay = round(float(radio.get("tx_delay_ms", 0)) * AUDIO_RATE / 1000)
        st.vox_hold = round(float(radio.get("vox_hold_ms", 0)) * AUDIO_RATE / 1000)
        st.recovery = round(float(radio.get("rx_recovery_ms", 0)) * AUDIO_RATE / 1000)
    server = Server(scenario, a, b)
    limit = args.max_seconds or float(scenario.get("seconds", 3600))
    try:
        while server.t / AUDIO_RATE < limit:
            server.tick()
            if args.progress_file and server.t % AUDIO_RATE < TICK:
                args.progress_file.write_text(f"{server.t / AUDIO_RATE:.2f}", encoding="utf-8")
    except (ConnectionError, OSError):
        pass
    finally:
        if args.report:
            args.report.write_text(json.dumps(server.report(), indent=1), encoding="utf-8")
        for st in (a, b):
            st.sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
