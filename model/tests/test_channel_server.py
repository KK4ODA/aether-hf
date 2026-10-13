"""The scenario harness's channel server: the radios it models and what it counts (ADR-0042)."""

from __future__ import annotations

import json
import socket
import struct
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import channel_server as cs  # noqa: E402

RATE = cs.AUDIO_RATE


class Fake:
    """A station's end of the wire, driven by hand."""

    def __init__(self) -> None:
        self.mine, theirs = socket.socketpair()
        self.station = cs.Station("?", theirs, 0, 0, 0)
        self.heard: list[np.ndarray] = []

    def play(self, samples: np.ndarray) -> None:
        self.mine.sendall(struct.pack("<BI", cs.MSG_PLAY, len(samples)))
        self.mine.sendall(samples.astype("<f4").tobytes())

    def tag(self, tag: dict) -> None:
        raw = json.dumps(tag).encode()
        self.mine.sendall(struct.pack("<BI", cs.MSG_TAG, len(raw)) + raw)

    def ready(self, n: int = 1) -> None:
        self.mine.sendall(bytes([cs.MSG_READY]) * n)  # one write: a socketpair counts writes

    def take_block(self) -> None:
        head = b""
        while len(head) < 21:
            head += self.mine.recv(21 - len(head))
        _, _, _, n = struct.unpack("<BQQI", head)
        body = b""
        while len(body) < 4 * n:
            body += self.mine.recv(4 * n - len(body))
        self.heard.append(np.frombuffer(body, dtype="<f4").copy())


def server(radios: dict[str, dict], snr_db: float = 60.0) -> tuple[cs.Server, Fake, Fake]:
    scenario = {
        "seed": 3,
        "path": {"profile": "awgn", "snr_db": snr_db},
        "stations": {"a": {"tx_level": 0.25}, "b": {"tx_level": 0.25}},
    }
    a, b = Fake(), Fake()
    for fake, who in ((a, "a"), (b, "b")):
        radio = radios.get(who, {})
        fake.station.tx_delay = round(radio.get("tx_delay_ms", 0) * RATE / 1000)
        fake.station.vox_hold = round(radio.get("vox_hold_ms", 0) * RATE / 1000)
        fake.station.recovery = round(radio.get("rx_recovery_ms", 0) * RATE / 1000)
    return cs.Server(scenario, {"a": a.station, "b": b.station}), a, b


def run(srv: cs.Server, a: Fake, b: Fake, seconds: float, plays: dict | None = None) -> None:
    """Tick for ``seconds``; ``plays`` maps a tick number to ``(station, samples)``."""
    for k in range(round(seconds * RATE / cs.TICK)):
        for who, samples in (plays or {}).get(k, []):
            (a if who == "a" else b).play(samples)
        a.ready()
        b.ready()
        srv.tick()
        a.take_block()
        b.take_block()


def tone(seconds: float, hz: float = 1500.0) -> np.ndarray:
    t = np.arange(round(seconds * RATE)) / RATE
    return (0.25 * np.sin(2 * np.pi * hz * t)).astype(np.float32)


def envelope(blocks: list[np.ndarray]) -> np.ndarray:
    x = np.concatenate(blocks)
    n = len(x) // 480
    return np.sqrt(np.mean(x[: n * 480].reshape(n, 480) ** 2, axis=1))  # 10 ms steps


def test_a_station_is_heard_a_cards_latency_after_it_plays_and_hears_nothing_while_keyed() -> None:
    srv, a, b = server({})
    run(srv, a, b, 2.0, {5: [("a", tone(0.5))]})
    at_b = envelope(b.heard)
    on = np.flatnonzero(at_b > 0.05)
    # handed over before tick 5 (0.10 s), on the air 0.25 s later, for half a second
    assert abs(on[0] * 0.01 - 0.35) < 0.03 and abs((on[-1] + 1) * 0.01 - 0.85) < 0.03
    assert envelope(a.heard)[38:85].max() == 0.0, "a station hears itself transmitting"
    keyed = srv.a.intervals
    assert len(keyed) == 1 and abs(keyed[0][0] - 0.35) < 0.01


def test_the_start_of_a_transmission_is_lost_to_the_transmitters_delay() -> None:
    srv, a, b = server({"a": {"tx_delay_ms": 150}})
    run(srv, a, b, 2.0, {5: [("a", tone(0.5))]})
    on = np.flatnonzero(envelope(b.heard) > 0.05)
    assert abs(on[0] * 0.01 - 0.52) < 0.03, "the first 150 ms never radiated"


def test_a_vox_hold_leaves_its_station_deaf_after_the_audio_stops() -> None:
    # b transmits; a answers 100 ms after b's audio ends on the air; b's VOX holds 300 ms
    srv, a, b = server({"b": {"vox_hold_ms": 300}})
    b_starts, b_len = 5, 0.5
    answer_tick = b_starts + round((b_len + 0.1) / 0.02)
    run(srv, a, b, 3.0, {b_starts: [("b", tone(b_len))], answer_tick: [("a", tone(0.6, 1200))]})
    at_b = envelope(b.heard)
    on = np.flatnonzero(at_b > 0.05)
    b_air_end = 0.10 + 0.25 + b_len
    a_air_start = answer_tick * 0.02 + 0.25
    assert a_air_start < b_air_end + 0.3, "the answer began inside the hold"
    assert on[0] * 0.01 >= b_air_end + 0.3 - 0.02, "b heard the start of the answer while held"


def test_two_stations_keyed_at_once_are_counted_as_a_collision() -> None:
    srv, a, b = server({})
    run(srv, a, b, 2.0, {5: [("a", tone(0.5))], 20: [("b", tone(0.5))]})
    report = srv.report()
    assert len(report["collisions"]) == 1
    # a on the air 0.35–0.85 s, b from 0.65 s: overlapping 0.20 s
    assert abs(report["both_keyed_s"] - 0.20) < 0.03


def keyed_burst(seconds: float, hz: float = 1500.0) -> np.ndarray:
    """What a daemon plays for a transmission: 0.1 s of keyed silence, the signal, 0.3 s of
    keyed silence (its lead and tail)."""
    lead, tail = np.zeros(round(0.1 * RATE)), np.zeros(round(0.3 * RATE))
    return np.concatenate((lead, tone(seconds, hz), tail)).astype(np.float32)


def test_a_keyed_overlap_is_split_into_radiated_vox_and_guard() -> None:
    # a: keyed 0.35-1.25 s, signal 0.45-0.95 s. b keyed from 0.95 s, its signal from 1.05 s:
    # 0.30 s of b's keying against a's silent tail, no signal against signal
    srv, a, b = server({})
    run(srv, a, b, 3.0, {5: [("a", keyed_burst(0.5))], 35: [("b", keyed_burst(0.5, 1200))]})
    report = srv.report()
    assert len(report["collisions"]) == 1
    assert report["overlap_s"]["radiated"] == 0.0 and report["rf_overlaps"] == []
    assert abs(report["overlap_s"]["guard"] - 0.30) < 0.03
    # now b's signal meets a's: radiated, from b's signal at 0.75 s to a's end at 0.95 s
    srv, a, b = server({})
    run(srv, a, b, 3.0, {5: [("a", keyed_burst(0.5))], 20: [("b", keyed_burst(0.5, 1200))]})
    report = srv.report()
    assert abs(report["overlap_s"]["radiated"] - 0.20) < 0.02
    assert len(report["rf_overlaps"]) == 1
    # a VOX holding a keyed past its tail, over b's signal: vox
    srv, a, b = server({"a": {"vox_hold_ms": 400}})
    run(srv, a, b, 3.0, {5: [("a", keyed_burst(0.5))], 40: [("b", keyed_burst(0.5, 1200))]})
    report = srv.report()
    assert report["overlap_s"]["vox"] > 0.1 and report["overlap_s"]["radiated"] == 0.0


def test_a_tag_goes_on_the_air_with_the_first_sample_it_describes() -> None:
    srv, a, b = server({})
    a.tag({"cause": "timer:wait", "frames": [{"c": "data"}]})
    run(srv, a, b, 2.0, {5: [("a", keyed_burst(0.5))]})
    (tx,) = srv.a.transmissions
    # handed over before tick 5 (0.10 s), on the air a card's latency later
    assert tx["cause"] == "timer:wait" and abs(tx["air_s"] - 0.35) < 0.025
    radiated = srv.a.radiated
    assert len(radiated) == 1 and abs(radiated[0][0] - 0.45) < 0.005


def test_the_peer_arriving_while_deaf_is_counted_by_why() -> None:
    # b's signal arrives while a's receiver is still recovering from a's own transmission
    srv, a, b = server({"a": {"rx_recovery_ms": 300}})
    run(srv, a, b, 3.0, {5: [("a", keyed_burst(0.5))], 50: [("b", keyed_burst(0.5, 1200))]})
    hits = {k: v / RATE for k, v in srv.a.deaf_hits.items()}
    # a keyed until 1.25 s, deaf until 1.55 s; b's signal from 1.35 s
    assert abs(hits["recovering"] - 0.20) < 0.03 and hits["transmitting"] == 0.0


def test_the_level_follows_its_schedule_and_holds_its_ends() -> None:
    t = np.array([0.0, 5.0, 10.0, 15.0, 30.0])
    got = cs.level_at([[5.0, 0.0], [15.0, -20.0]], t)
    assert list(got) == [0.0, 0.0, -10.0, -20.0, -20.0]
    assert list(cs.level_at([], t)) == [0.0] * 5


def test_a_clock_offset_keeps_every_block_whole() -> None:
    ch = cs.ScenarioChannel(
        [], sro_ppm=200.0, channel="awgn", snr_db=None, signal_dbfs=-15.0, seed=1
    )
    tone = 0.25 * np.sin(2 * np.pi * 1500.0 * np.arange(cs.TICK) / RATE)
    for _ in range(100):
        assert len(ch.process(tone)) == cs.TICK


def test_the_agc_steps_down_on_a_peak_holds_then_ramps_back() -> None:
    agc = cs.Agc(threshold=0.1, hang_ms=100.0, decay_db_s=50.0)
    crash = np.zeros(cs.TICK, dtype=np.float32)
    crash[:48] = 1.0  # 20 dB over the threshold for a millisecond
    agc.process(crash)
    assert abs(agc.gain_db + 20.0) < 1e-5
    quiet = np.full(cs.TICK, 0.01, dtype=np.float32)
    for _ in range(4):  # 80 ms: still holding
        out = agc.process(quiet)
    assert abs(agc.gain_db + 20.0) < 1e-5
    assert abs(float(out[0]) - 0.001) < 1e-6
    for _ in range(10):  # 200 ms more: 20 ms of hang left, then 180 ms at 50 dB/s
        agc.process(quiet)
    assert -12.0 < agc.gain_db < -10.0


def test_a_third_station_is_heard_at_its_own_path_over_one_noise_floor() -> None:
    # a gateway b and two clients: a on the main path at +20 dB, c on its own at +8 dB; a
    # receiver has one floor, so b hears c 12 dB under a and the noise is not counted twice
    scenario = {
        "seed": 4,
        "path": {"profile": "awgn", "snr_db": 20.0, "c_to_b": {"snr_db": 8.0}},
        "stations": {"a": {}, "b": {}, "c": {}},
    }
    assert cs.stations_of(scenario) == ["a", "b", "c"]
    fakes = {who: Fake() for who in "abc"}
    srv = cs.Server(scenario, {who: f.station for who, f in fakes.items()})
    plays = {5: [("a", tone(1.0))], 100: [("c", tone(1.0))]}
    for k in range(round(4.0 * RATE / cs.TICK)):
        for who, samples in plays.get(k, []):
            fakes[who].play(samples)
        for f in fakes.values():
            f.ready()
        srv.tick()
        for f in fakes.values():
            f.take_block()
    at_b = envelope(fakes["b"].heard)
    quiet = float(np.median(at_b[150:190]))  # 1.5–1.9 s: nobody on the air
    from_a = float(np.median(at_b[45:125]))  # a on the air 0.35–1.35 s
    from_c = float(np.median(at_b[245:325]))  # c on the air 2.25–3.25 s

    def snr(rms: float) -> float:
        return float(20 * np.log10(np.sqrt(max(rms**2 - quiet**2, 1e-20)) / quiet))

    # the 3 kHz SNR reads ~2.5 dB high over the 3.4 kHz audio band: compare the two
    assert abs((snr(from_a) - snr(from_c)) - 12.0) < 1.5
    # the floor under c's signal is the floor under a's
    assert abs(20 * np.log10(quiet) - 20 * np.log10(float(np.median(at_b[350:390])))) < 1.0
    # the clients hear each other too
    assert np.flatnonzero(envelope(fakes["c"].heard) > 2 * quiet).size > 0


def run_stalling(srv: cs.Server, a: Fake, b: Fake, seconds: float, plays: dict) -> list[int]:
    """Tick like :func:`run`, but each fake answers only the blocks it was sent — none while
    its station is stalled, the whole pile after. Returns how many blocks ``b`` got a tick."""
    import select
    import threading

    owed = {"a": 1, "b": 1}
    got = []
    for k in range(round(seconds * RATE / cs.TICK)):
        for who, samples in plays.get(k, []):
            (a if who == "a" else b).play(samples)
        for who, fake in (("a", a), ("b", b)):
            fake.ready(owed[who])
            owed[who] = 0
        # a pile is more than a socket buffer holds: the server sends while the fakes read
        ticking = threading.Thread(target=srv.tick)
        ticking.start()
        while ticking.is_alive() or any(select.select([f.mine], [], [], 0)[0] for f in (a, b)):
            for who, fake in (("a", a), ("b", b)):
                if select.select([fake.mine], [], [], 0.001)[0]:
                    fake.take_block()
                    owed[who] += 1
        ticking.join()
        got.append(owed["b"])
    return got


def test_a_stalled_station_plays_on_hears_nothing_new_then_catches_up() -> None:
    srv, a, b = server({})
    # a second of b, handed over in two halves (a socket holds less than a second)
    plays = {5: [("b", tone(0.5))], 6: [("b", tone(0.5))], 60: [("a", tone(0.5))]}
    srv.b.stall(10 * cs.TICK, round(1.0 * RATE))  # ticks 10–59: while b's tone plays out
    blocks = run_stalling(srv, a, b, 2.0, plays)
    assert blocks[9] == 1 and blocks[10:59] == [0] * 49, "a stalled station is sent nothing"
    assert blocks[60] == 51, "the pile and the block after it, all at once"
    # what b queued before the stall played through it: a heard all of b's second
    on_a = np.flatnonzero(envelope(a.heard) > 0.05)
    assert abs((on_a[-1] - on_a[0] + 1) * 0.01 - 1.0) < 0.03
    # and b heard everything: nothing dropped from a pile under the 2 s backlog
    assert len(np.concatenate(b.heard)) == len(np.concatenate(a.heard))
    report = srv.report()["stations"]["?"]
    assert report["stall_dropped_s"] == 0.0


def test_a_stall_longer_than_the_backlog_drops_the_oldest_audio() -> None:
    srv, a, b = server({})
    srv.b.stall(10 * cs.TICK, round(3.0 * RATE))
    run_stalling(srv, a, b, 4.0, {})
    heard = len(np.concatenate(b.heard))
    assert heard == len(np.concatenate(a.heard)) - round(1.02 * RATE)
    assert srv.b.stall_dropped == round(1.02 * RATE)  # 3 s stalled and the block after, kept 2


def test_a_machine_draws_its_stalls_from_the_seed() -> None:
    def draws(seed: int) -> list[list[float]]:
        st = cs.Station("?", None, 0, 0, 0)  # type: ignore[arg-type]
        cs.machine(st, {"stall_every_s": 5.0, "stall_s": [0.5, 2.0]}, seed, "b")
        for t in range(0, 120 * RATE, cs.TICK):
            st.stalled(t)
        return st.stalls

    first = draws(1)
    assert first == draws(1) and first != draws(2)
    assert 10 <= len(first) <= 40
    assert all(0.49 <= end - start <= 2.01 for start, end in first)
