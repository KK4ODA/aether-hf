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
POLL_S = 0.25


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


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
schema_version = 10
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

[control]
enabled = true
bind = "127.0.0.1:{control}"

[update]
check = false

[record]
auto = true
dir = "{record}"
"""


@dataclass
class Station:
    who: str
    callsign: str
    control: int
    process: subprocess.Popen[bytes]
    log: Path

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
    for who, binary in (("a", daemon_a), ("b", daemon_b)):
        cfg = stations.get(who, {})
        callsign = cfg.get("callsign", "W4ODA" if who == "a" else "KK4XYZ")
        control = free_port()
        record = run.dir / who / "recordings"
        record.mkdir(parents=True, exist_ok=True)
        config = run.dir / who / "station.toml"
        config.write_text(
            STATION_TOML.format(
                callsign=callsign,
                tx_level=cfg.get("tx_level", 0.25),
                bandwidth=sc.get("bandwidth", 2300),
                wait_for_clear=str(cfg.get("wait_for_clear", True)).lower(),
                answer_gap_ms=int(cfg.get("answer_gap_ms", 0)),
                max_mode=int(cfg.get("max_mode", 19 if sc.get("bandwidth", 2300) == 2300 else 14)),
                control=control,
                record=record.as_posix(),
            ),
            encoding="utf-8",
        )
        log = run.dir / who / "aetherd.out"
        process = subprocess.Popen(
            [str(binary), "--config", str(config), "--channel", channel],
            stdout=log.open("wb"),
            stderr=subprocess.STDOUT,
        )
        run.stations[who] = Station(who, callsign, control, process, log)
    for st in run.stations.values():
        for _ in range(200):
            try:
                st.status()
                break
            except OSError:
                time.sleep(0.1)


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
        call(sender.control, "send", {"data": base64.b64encode(data).decode()})
        ok = run.wait(
            f"{word} {n}",
            lambda: receiver.counter("bytes_delivered") - before >= n,
            float(run.scenario.get("transfer_s", 300)),
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
    if word == "disconnect":
        call(a.control, "disconnect")
        return run.wait(
            "disconnected",
            lambda: a.status()["state"] == "idle" and b.status()["state"] == "idle",
            120,
        )
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
    clean = all("timeout" not in e and e not in ("none", "unknown") for e in ends)
    failures = []
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
        for text in scenario.get("script", {}).get("steps", []):
            if not step(run, text):
                break
        # a moment for the last frames and the sessions' history to settle
        run.wait("settle", lambda: False, 3, note=False)
        ends = session_ends(run)
    finally:
        for st in run.stations.values():
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
