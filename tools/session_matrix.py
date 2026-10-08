"""Run whole sessions between two real daemons through simulated band conditions (ADR-0042).

    python tools/session_matrix.py [scenarios…] [--tag quick] [--jobs 2]
                                   [--daemon core/target/release/aetherd] [--daemon-b PATH]
                                   [--out runs/] [--csv results.csv]

Every scenario (``bench/scenarios/*.toml`` unless named; the format is the README there) is
one on-air situation: the path, the other signals and the static each station hears, the
radios at both ends, and what the operator does. For each, this starts the channel server
(``channel_server.py``) and two daemons on it (``aetherd --channel``), drives them through the
script over the control API, as the panel would, and judges the outcome: whether they
connected, whether every message arrived whole, how the Test ended, whether the session ended
by its DISC, and how often both radios were keyed at once — which no recording at either end
can show, and which the channel server sees directly.

``--daemon-b`` runs the called station on another build: an old release against this tree is
how a change is checked for working with the stations already on the air.

The stations' clock is the audio's, and the channel hands it out as fast as the daemons can
take it, so a twenty-minute session takes a few minutes. Each run keeps both daemons' logs and
recordings and the channel's report under ``--out``.
"""

from __future__ import annotations

import argparse
import base64
import contextlib
import csv
import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_DAEMON = (
    ROOT / "core" / "target" / "release" / ("aetherd.exe" if os.name == "nt" else "aetherd")
)
SCENARIOS = ROOT / "bench" / "scenarios"
STUCK_KEY_S = 32.0
"""Longer keyed at once than this is a stuck key: the watchdog's 30 s, and its release."""
POLL_S = 0.02
"""How often the runner looks: a host program answers within milliseconds of a delivery, and
at several times real time a quarter second of wall clock was a second of air."""


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def reserve_ports(count: int) -> list[int]:
    """``count`` control ports and as many host-port pairs (the data port is the command
    port's next), all free at once: chosen one at a time, a station's data port could be the
    next station's control port, and a host's data then went to the wrong daemon."""
    held: list[socket.socket] = []
    controls: list[int] = []
    hosts: list[int] = []
    try:
        while len(controls) < count:
            s = socket.socket()
            s.bind(("127.0.0.1", 0))
            held.append(s)
            controls.append(int(s.getsockname()[1]))
        while len(hosts) < count:
            s = socket.socket()
            s.bind(("127.0.0.1", 0))
            port = int(s.getsockname()[1])
            nxt = socket.socket()
            try:
                nxt.bind(("127.0.0.1", port + 1))
            except OSError:
                s.close()
                nxt.close()
                continue
            held += [s, nxt]
            hosts.append(port)
    finally:
        for s in held:
            s.close()
    return [p for pair in zip(controls, hosts, strict=True) for p in pair]


def call(port: int, method: str, params: dict | None = None, timeout: float = 15.0) -> dict:
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/{method}",
        data=json.dumps(params or {}).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return dict(json.loads(response.read()))
    except urllib.error.HTTPError as refused:
        # a refusal is an answer, with its reason in the body
        return dict(json.loads(refused.read() or b"{}"))


def incompressible(n: int, salt: int) -> bytes:
    """Bytes the station's deflate cannot shrink, so a message takes the bursts its size says."""
    out = bytearray()
    x = 0x9E3779B97F4A7C15 ^ salt
    while len(out) < n:
        x ^= (x << 13) & 0xFFFFFFFFFFFFFFFF
        x ^= x >> 7
        x ^= (x << 17) & 0xFFFFFFFFFFFFFFFF
        out += x.to_bytes(8, "little")
    return bytes(out[:n])


STATION_TOML = """\
schema_version = 11
callsign = "{callsign}"

[audio]
sample_rate = 48000
tx_level = {tx_level}

[ptt]
kind = "none"

[radio]
bandwidth = {bandwidth}
wait_for_clear = {wait_for_clear}
answer_gap_ms = {answer_gap_ms}
max_mode = {max_mode}
cw_id = {cw_id}
cw_id_interval_s = {cw_id_interval_s}

[host]
enabled = {host}
bind = "127.0.0.1:{host_port}"
trace = true

[control]
enabled = true
bind = "127.0.0.1:{control}"

[update]
check = false

[record]
auto = true
dir = "{record}"
# never the project's upload folder from a bench (debug mode, ADR-0050)
send_to_project = false
"""


@dataclass
class Station:
    who: str
    callsign: str
    control: int
    process: subprocess.Popen[bytes]
    log: Path
    host_port: int = 0

    def status(self) -> dict:
        return dict(call(self.control, "status")["result"])

    def counter(self, name: str) -> int:
        return int(self.status().get("counters", {}).get(name, 0))


@dataclass
class Run:
    scenario: dict
    dir: Path
    progress: Path
    channel: subprocess.Popen[bytes] | None = None
    stations: dict[str, Station] = field(default_factory=dict)
    notes: list[str] = field(default_factory=list)
    delivered_ok: bool = True
    connected: bool = False
    test: dict | None = None
    probe: str | None = None
    transfers: list[str] = field(default_factory=list)
    final_states: list[str] = field(default_factory=list)
    exited: list[str] = field(default_factory=list)
    hosts: dict[str, Host] = field(default_factory=dict)
    outage_until: float = 0.0

    def command(self, line: str) -> None:
        """Tell the channel server something, at the air time it next reads (each second)."""
        with (self.dir / "channel.cmd").open("a", encoding="utf-8") as f:
            f.write(line + "\n")

    def air_s(self) -> float:
        try:
            return float(self.progress.read_text(encoding="utf-8") or 0.0)
        except (OSError, ValueError):
            return 0.0

    def wait(self, what: str, done: object, air_s: float, *, note: bool = True) -> bool:
        """Poll ``done()`` until it holds or ``air_s`` more seconds of air have passed; a
        wait that runs out is noted unless ``note`` is false (a plain pause)."""
        until = self.air_s() + air_s
        stalled_since = time.monotonic()
        last = self.air_s()
        while True:
            try:
                if done():  # type: ignore[operator]
                    return True
            except (OSError, ValueError, KeyError):
                pass
            now = self.air_s()
            if now >= until:
                if note:
                    self.notes.append(f"{what}: not within {air_s:.0f} s of air")
                return False
            if now > last:
                last, stalled_since = now, time.monotonic()
            elif time.monotonic() - stalled_since > 60:
                self.notes.append(f"{what}: the channel stopped at {now:.0f} s")
                return False
            if self.channel and self.channel.poll() is not None:
                self.notes.append(f"{what}: the channel ended at {now:.0f} s")
                return False
            time.sleep(POLL_S)


class Host:
    """A host program on a station's VARA-compatible ports, as the programs on the bench were
    seen to speak them (``host-interfaces.md`` §7): the command port's lines and the data
    port's bytes, read on threads of their own, and — for ``trimode`` — RMS Trimode's
    scanning, ``LISTEN`` off for half a second in every three and a half of air, which is
    what ADR-0051 was found by. A ``CONNECTED`` it is told while not listening is a fault, as
    Trimode ignores one."""

    def __init__(self, run: Run, station: Station, kind: str) -> None:
        self.run, self.station, self.kind = run, station, kind
        self.cmd = socket.create_connection(("127.0.0.1", station.host_port), timeout=10)
        self.data = socket.create_connection(("127.0.0.1", station.host_port + 1), timeout=10)
        # connected: from now on a read waits as long as the session does (a reader that
        # timed out after 10 s quit, and a slow path's reply was never counted)
        self.cmd.settimeout(None)
        self.data.settimeout(None)
        self.lines: list[str] = []
        self.received = 0
        self.calls_from = 0  # CONNECTEDs heard before a `host-call`
        self.listening = False
        self.connected = False
        self.problems: list[str] = []
        self.lock = threading.Lock()
        self.closed = False
        threading.Thread(target=self._read_cmd, daemon=True).start()
        threading.Thread(target=self._read_data, daemon=True).start()
        bw = "BW500" if run.scenario.get("bandwidth") == 500 else "BW2300"
        if kind == "trimode":
            opening = [f"MYCALL {station.callsign}", bw, "PUBLIC ON", "CWID ON", "LISTEN ON"]
        else:  # a Winlink Express client
            opening = [
                "PUBLIC ON",
                "CWID ON",
                "COMPRESSION TEXT",
                bw,
                f"MYCALL {station.callsign}",
                "LISTEN ON",
            ]
        for line in opening:
            self.send(line)
        if kind == "trimode":
            threading.Thread(target=self._scan, daemon=True).start()

    def send(self, line: str) -> None:
        if line.startswith("LISTEN"):
            self.listening = line == "LISTEN ON"
        self.cmd.sendall((line + "\r").encode())

    def _read_cmd(self) -> None:
        buffer = b""
        while not self.closed:
            try:
                chunk = self.cmd.recv(4096)
            except OSError:
                return
            if not chunk:
                return
            buffer += chunk
            while b"\r" in buffer:
                raw, buffer = buffer.split(b"\r", 1)
                line = raw.decode(errors="replace").strip()
                with self.lock:
                    self.lines.append(line)
                if line.startswith("CONNECTED"):
                    if not self.listening and self.kind == "trimode":
                        self.problems.append(
                            f"{self.station.who}: CONNECTED while LISTEN OFF at "
                            f"{self.run.air_s():.1f} s"
                        )
                    self.connected = True
                elif line.startswith("DISCONNECTED"):
                    self.connected = False

    def _read_data(self) -> None:
        while not self.closed:
            try:
                chunk = self.data.recv(65536)
            except OSError:
                return
            if not chunk:
                return
            self.received += len(chunk)

    def _scan(self) -> None:
        """Trimode's scan: dwell 3 s, deaf 0.5 s, in air time; it stops while connected."""
        while not self.closed:
            began = self.run.air_s()
            while not self.closed and self.run.air_s() < began + 3.0:
                time.sleep(POLL_S)
            if self.closed or self.connected:
                continue
            self.send("LISTEN OFF")
            began = self.run.air_s()
            while not self.closed and self.run.air_s() < began + 0.5:
                time.sleep(POLL_S)
            if not self.closed:
                self.send("LISTEN ON")

    def heard(self, prefix: str) -> int:
        with self.lock:
            return sum(1 for line in self.lines if line.startswith(prefix))

    def close(self) -> None:
        self.closed = True
        for sock in (self.cmd, self.data):
            with contextlib.suppress(OSError):
                sock.close()


def start(run: Run, daemon_a: Path, daemon_b: Path) -> None:
    sc = run.scenario
    port_file = run.dir / "channel.port"
    run.channel = subprocess.Popen(
        [
            sys.executable,
            str(ROOT / "tools" / "channel_server.py"),
            "--scenario",
            str(run.dir / "scenario.toml"),
            "--port-file",
            str(port_file),
            "--report",
            str(run.dir / "channel.json"),
            "--progress-file",
            str(run.progress),
            "--command-file",
            str(run.dir / "channel.cmd"),
        ],
        stdout=(run.dir / "channel.log").open("wb"),
        stderr=subprocess.STDOUT,
    )
    for _ in range(200):
        if port_file.exists() and port_file.read_text().strip():
            break
        time.sleep(0.05)
    channel = f"127.0.0.1:{port_file.read_text().strip()}"
    stations = sc.get("stations", {})
    names = stations_of(sc)
    ports = reserve_ports(len(names))
    for i, who in enumerate(names):
        # the called station runs --daemon-b; every client, --daemon
        binary = daemon_b if who == "b" else daemon_a
        cfg = stations.get(who, {})
        callsign = cfg.get("callsign", DEFAULT_CALLSIGNS.get(who, f"N4{who.upper()}AA"))
        control, host_port = ports[2 * i], ports[2 * i + 1]
        record = (run.dir / who / "recordings").resolve()
        record.mkdir(parents=True, exist_ok=True)
        config = run.dir / who / "station.toml"
        config.write_text(
            STATION_TOML.format(
                callsign=callsign,
                tx_level=cfg.get("tx_level", 0.25),
                bandwidth=cfg.get("bandwidth", sc.get("bandwidth", 2300)),
                wait_for_clear=str(cfg.get("wait_for_clear", True)).lower(),
                answer_gap_ms=int(cfg.get("answer_gap_ms", 0)),
                max_mode=int(
                    cfg.get(
                        "max_mode", 14 if cfg.get("bandwidth", sc.get("bandwidth")) == 500 else 19
                    )
                ),
                control=control,
                record=record.as_posix(),
                cw_id=str(cfg.get("cw_id", False)).lower(),
                cw_id_interval_s=float(cfg.get("cw_id_interval_s", 600.0)),
                host=str(bool(cfg.get("host", False))).lower(),
                host_port=host_port,
            ),
            encoding="utf-8",
        )
        log = run.dir / who / "aetherd.out"
        process = subprocess.Popen(
            [str(binary), "--config", str(config), "--channel", channel],
            stdout=log.open("wb"),
            stderr=subprocess.STDOUT,
        )
        run.stations[who] = Station(who, callsign, control, process, log, host_port)
    for st in run.stations.values():
        for _ in range(200):
            try:
                st.status()
                break
            except OSError:
                time.sleep(0.1)


def stations_of(scenario: dict) -> list[str]:
    """``a``, ``b``, then any more stations the scenario names — the channel server's order."""
    named = scenario.get("stations", {})
    return ["a", "b", *sorted(k for k in named if k not in ("a", "b") and len(k) == 1)]


DEFAULT_CALLSIGNS = {"a": "W4ODA", "b": "KK4XYZ", "c": "N4CCC", "d": "N4DDD", "e": "N4EEE"}


def all_idle(run: Run) -> bool:
    return all(st.status()["state"] == "idle" for st in run.stations.values())


def step(run: Run, text: str) -> bool:
    a, b = run.stations["a"], run.stations["b"]
    word, _, arg = text.partition(" ")
    if word == "wait":
        run.wait("wait", lambda: False, float(arg), note=False)
        return True
    if word == "beacon":
        call(a.control, "beacon")
        return run.wait("beacon heard", lambda: b.counter("beacons_heard") > 0, 60)
    if word == "probe":
        before = a.counter("probe_replies")
        call(a.control, "probe", {"remote": b.callsign})
        answered = run.wait("probe answered", lambda: a.counter("probe_replies") > before, 30)
        run.probe = "answered" if answered else "unanswered"
        return True  # a probe is one frame and no retry: reported, not required
    if word == "connect":
        call(a.control, "connect", {"remote": b.callsign})
        run.connected = run.wait(
            "connected",
            lambda: a.status()["state"] == "connected" and b.status()["state"] == "connected",
            float(run.scenario.get("connect_within_s", 180)),
        )
        return run.connected
    if word in ("message", "reply"):
        sender, receiver = (a, b) if word == "message" else (b, a)
        n = int(arg)
        before = receiver.counter("bytes_delivered")
        data = incompressible(n, salt=len(run.notes) + n)
        began = run.air_s()
        call(sender.control, "send", {"data": base64.b64encode(data).decode()})
        ok = run.wait(
            f"{word} {n}",
            lambda: receiver.counter("bytes_delivered") - before >= n,
            float(run.scenario.get("transfer_s", 300)),
        )
        took = max(run.air_s() - began, 0.1)
        run.transfers.append(
            f"{word} {n} B {took:.0f} s {8 * n / took:.0f} bps" if ok else f"{word} {n} B lost"
        )
        run.delivered_ok &= ok
        return ok
    if word == "test":
        params = {"remote": b.callsign}
        params.update(run.scenario.get("test", {}))
        started = call(a.control, "test.start", params)
        if not started.get("ok"):
            run.notes.append(f"test refused: {started}")
            run.test = {"outcome": "refused"}
            return False
        budget = float(params.get("budget_s", 600)) + 120
        run.wait(
            "test",
            lambda: call(a.control, "test.status")["result"]["running"] is False,
            budget,
        )
        run.test = call(a.control, "test.status")["result"].get("results") or {}
        return True
    if word == "host":
        # `host a client` / `host b trimode`: a program attaches to the station's host port
        who, _, kind = arg.partition(" ")
        run.hosts[who] = Host(run, run.stations[who], kind or "client")
        run.wait("host attached", lambda: False, 2, note=False)
        return True
    if word == "host-connect":
        # `host-connect [c]`: a client (a unless named) calls the gateway, b
        who = arg or "a"
        ha, hb = run.hosts[who], run.hosts["b"]
        was_a, was_b = ha.heard("CONNECTED"), hb.heard("CONNECTED")
        ha.send(f"CONNECT {run.stations[who].callsign} {b.callsign}")
        run.connected = run.wait(
            "host connected",
            lambda: ha.heard("CONNECTED") > was_a and hb.heard("CONNECTED") > was_b,
            float(run.scenario.get("connect_within_s", 180)),
        )
        return run.connected
    if word == "host-call":
        # `host-call c`: the client calls the gateway and the script goes on — a call made
        # into a gateway busy with another client's session; `host-wait c` sees it through
        ha = run.hosts[arg]
        ha.calls_from = ha.heard("CONNECTED")
        ha.send(f"CONNECT {run.stations[arg].callsign} {b.callsign}")
        return True
    if word == "host-wait":
        ha = run.hosts[arg]
        run.connected = run.wait(
            f"{arg} connected",
            lambda: ha.heard("CONNECTED") > ha.calls_from,
            float(run.scenario.get("connect_within_s", 180)),
        )
        return run.connected
    if word in ("host-send", "host-reply"):
        # `host-send N [c]`: the client (a unless named) to the gateway; host-reply back
        size, _, who = arg.partition(" ")
        n = int(size)
        tx, rx = (run.hosts[who or "a"], run.hosts["b"])
        if word == "host-reply":
            tx, rx = rx, tx
        before = rx.received
        began = run.air_s()
        tx.data.sendall(incompressible(n, salt=n + len(run.transfers)))
        ok = run.wait(
            f"{word} {n}",
            lambda: rx.received - before >= n,
            float(run.scenario.get("transfer_s", 300)),
        )
        took = max(run.air_s() - began, 0.1)
        run.transfers.append(
            f"{word} {n} B {took:.0f} s {8 * n / took:.0f} bps" if ok else f"{word} {n} B lost"
        )
        run.delivered_ok &= ok
        return ok
    if word == "host-disconnect":
        ha, hb = run.hosts[arg or "a"], run.hosts["b"]
        was_a, was_b = ha.heard("DISCONNECTED"), hb.heard("DISCONNECTED")
        ha.send("DISCONNECT")
        return run.wait(
            "host disconnected",
            lambda: ha.heard("DISCONNECTED") > was_a and hb.heard("DISCONNECTED") > was_b,
            120,
        )
    if word == "send":
        # queued and left to go: what happens to it is for the steps after to see
        call(a.control, "send", {"data": base64.b64encode(incompressible(int(arg), 7)).decode()})
        return True
    if word == "message?":
        # a message that may fail — through an outage, say: reported, not judged
        n = int(arg)
        before = b.counter("bytes_delivered")
        call(a.control, "send", {"data": base64.b64encode(incompressible(n, 11)).decode()})
        ok = run.wait(
            text,
            lambda: b.counter("bytes_delivered") - before >= n,
            float(run.scenario.get("transfer_s", 300)),
            note=False,
        )
        run.transfers.append(f"{text} {'arrived' if ok else 'lost (allowed)'}")
        return True
    if word == "outage":
        # the channel reads its commands once a second of air: the outage ends a second late
        run.outage_until = run.air_s() + float(arg) + 1.0
        run.command(f"outage {arg}")
        return True
    if word == "wait_clear":
        # until the outage is over: a call made in it is heard by nobody
        until = run.outage_until
        run.wait("outage over", lambda: run.air_s() >= until, max(until - run.air_s(), 0) + 5)
        return True
    if word == "wait_idle":
        # both stations back to idle by themselves — the link timeout, after an outage
        return run.wait("back to idle", lambda: all_idle(run), float(arg))
    if word == "abort":
        call(a.control, "abort")
        return run.wait("aborted", lambda: all_idle(run), 120)
    if word == "disconnect":
        leaving = {"": (a,), "a": (a,), "b": (b,), "both": (a, b)}[arg]
        for st in leaving:
            call(st.control, "disconnect")
        return run.wait("disconnected", lambda: all_idle(run), 120)
    raise SystemExit(f"unknown step {text!r}")


def session_ends(run: Run) -> list[str]:
    """How each station's last session ended, from its history — asked while it runs."""
    ends = []
    for st in run.stations.values():
        try:
            sessions = call(st.control, "sessions.list")["result"].get("sessions", [])
            ends.append(sessions[0]["end"] if sessions else "none")
        except (OSError, KeyError, IndexError, ValueError):
            ends.append("unknown")
    return ends


def judge(run: Run, ends: list[str]) -> dict:
    sc = run.scenario
    expect = sc.get("expect", {})
    report = {}
    with contextlib.suppress(OSError, ValueError):
        report = json.loads((run.dir / "channel.json").read_text(encoding="utf-8"))
    collisions = len(report.get("collisions", []))
    # always judged, whatever the scenario expects: what must never happen
    invariants = list(run.exited)
    for st in run.stations.values():
        with contextlib.suppress(OSError):
            if "panicked" in st.log.read_text(encoding="utf-8", errors="replace"):
                invariants.append(f"{st.who} panicked")
    for name, keyed in report.get("stations", {}).items():
        longest = max((iv[1] - iv[0] for iv in keyed.get("keyed", [])), default=0.0)
        if longest > STUCK_KEY_S:
            invariants.append(f"{name} keyed {longest:.0f} s at once")
    clean = all("timeout" not in e and e not in ("none", "unknown") for e in ends)
    idle = all(s == "idle" for s in run.final_states)
    failures = invariants
    for host in run.hosts.values():
        failures += host.problems
    if expect.get("connected") and not run.connected:
        failures.append("did not connect")
    if expect.get("delivered") and not run.delivered_ok:
        failures.append("a message did not arrive")
    want_test = expect.get("test")
    got_test = (run.test or {}).get("outcome")
    if want_test and got_test != want_test:
        failures.append(f"test {got_test}")
    if expect.get("clean_end") and not clean:
        failures.append(f"ended {'/'.join(ends)}")
    if expect.get("idle") and not idle:
        failures.append(f"left {'/'.join(run.final_states)}")
    if "max_collisions" in expect and collisions > int(expect["max_collisions"]):
        failures.append(f"{collisions} collisions")
    test = run.test or {}
    return {
        "scenario": sc.get("name"),
        "pass": not failures,
        "why": "; ".join(failures + run.notes) or "",
        "air_s": round(report.get("seconds", run.air_s()), 1),
        "collisions": collisions,
        "both_keyed_s": report.get("both_keyed_s"),
        "probe": run.probe or "",
        "test": got_test or "",
        "message_bps": (test.get("message") or {}).get("bps"),
        "file_bps": (test.get("file") or {}).get("bps"),
        "highest_rung": test.get("highest_passed"),
        "ends": " / ".join(ends),
        "transfers": "; ".join(run.transfers),
    }


def run_one(path: Path, out: Path, daemon_a: Path, daemon_b: Path) -> dict:
    scenario = tomllib.loads(path.read_text(encoding="utf-8"))
    name = scenario.get("name", path.stem)
    run_dir = out / name
    shutil.rmtree(run_dir, ignore_errors=True)
    run_dir.mkdir(parents=True)
    shutil.copy(path, run_dir / "scenario.toml")
    run = Run(scenario, run_dir, run_dir / "channel.progress")
    t0 = time.monotonic()
    ends: list[str] = []
    try:
        start(run, daemon_a, daemon_b)
        keep_going = bool(scenario.get("script", {}).get("continue_on_failure", False))
        for text in scenario.get("script", {}).get("steps", []):
            if not step(run, text) and not keep_going:
                break
        # the last frames and the sessions' history settle: an abort's DISC goes out after
        # the burst it cut, and the other station needs to hear it
        run.wait("settle", lambda: all_idle(run), 30, note=False)
        run.wait("settle", lambda: False, 2, note=False)
        ends = session_ends(run)
        for st in run.stations.values():
            try:
                run.final_states.append(str(st.status()["state"]))
            except (OSError, KeyError, ValueError):
                run.final_states.append("unknown")
    finally:
        for host in run.hosts.values():
            host.close()
        for st in run.stations.values():
            if st.process.poll() is not None:
                run.exited.append(f"{st.who} exited ({st.process.returncode})")
            st.process.terminate()
        if run.channel:
            try:
                run.channel.wait(timeout=10)
            except subprocess.TimeoutExpired:
                run.channel.terminate()
        for st in run.stations.values():
            try:
                st.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                st.process.kill()
    # the channel writes its account when the stations leave it
    result = judge(run, ends)
    result["wall_s"] = round(time.monotonic() - t0, 1)
    (run_dir / "result.json").write_text(json.dumps(result, indent=1), encoding="utf-8")
    return result


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("scenarios", nargs="*", type=Path)
    ap.add_argument("--tag", help="only scenarios carrying this tag")
    ap.add_argument("--daemon", type=Path, default=DEFAULT_DAEMON)
    ap.add_argument("--daemon-b", type=Path, help="the called station's build (default: --daemon)")
    ap.add_argument("--out", type=Path, default=ROOT / "runs")
    ap.add_argument("--csv", type=Path, help="append one line per scenario here")
    ap.add_argument("--jobs", type=int, default=1)
    args = ap.parse_args()
    paths = args.scenarios or sorted(SCENARIOS.glob("*.toml"))
    if args.tag:
        paths = [
            p
            for p in paths
            if args.tag in tomllib.loads(p.read_text(encoding="utf-8")).get("tags", [])
        ]
    if not paths:
        print("no scenarios", file=sys.stderr)
        return 2
    if not args.daemon.exists():
        print(f"no daemon at {args.daemon}: cargo build --release in core/", file=sys.stderr)
        return 2
    args.out.mkdir(parents=True, exist_ok=True)
    daemon_b = args.daemon_b or args.daemon
    results = []
    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor

        with ThreadPoolExecutor(args.jobs) as pool:
            results = list(pool.map(lambda p: run_one(p, args.out, args.daemon, daemon_b), paths))
    else:
        for p in paths:
            results.append(run_one(p, args.out, args.daemon, daemon_b))
            r = results[-1]
            print(f"{'PASS' if r['pass'] else 'FAIL'}  {r['scenario']}  {r['why']}", flush=True)
    columns = list(results[0])
    print()
    print(" | ".join(columns))
    for r in results:
        print(" | ".join(str(r.get(c, "")) for c in columns))
    if args.csv:
        new = not args.csv.exists()
        with args.csv.open("a", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=columns)
            if new:
                w.writeheader()
            w.writerows(results)
    return 0 if all(r["pass"] for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
