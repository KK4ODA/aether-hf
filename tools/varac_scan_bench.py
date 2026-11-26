# SPDX-License-Identifier: MIT OR Apache-2.0
"""A bench for VarAC's Frequency Scanner: what it tells the modem when it moves the radio, and
what keeps it on a frequency (ADR-0138 §7; the procedure is
``docs/research/varac-scanner/00-bench.md``).

    python tools/varac_scan_bench.py run --out DIR [--plan FILE] [--seconds S]
    python tools/varac_scan_bench.py report DIR

``run`` stands in for the modem on VARA's host ports (8300 and 8301) and sits between VarAC and
Hamlib's ``rigctld`` running the dummy rig (``rigctld -m 1 -t 4533``): VarAC's rig control points
at this tool (4532), which passes every byte on. So the frequency changes VarAC makes and the lines
it says to the modem land in one file, ``DIR/timeline.log``, on one clock. The stand-in answers as
Aether's adapter does to an idle station (``BUFFER 0`` on attach, ``OK``, ``VERSION Aether HF …``,
``TUNE -12``, a call nobody answers ended by ``DISCONNECTED``). A line typed on its console, or
appended to ``DIR/inject.txt``, goes to VarAC as the modem's; a plan sends lines at set times after
a frequency change (``move2 +50 PENDING``), so a hold can be asked for just before the scanner's
next step. ``report`` reads the timeline: the moves and the stay between them, what VarAC said to
the modem around each move, and for each window the modem held something up (``BUSY ON``, a
``PENDING``, a session, ``PTT ON``) whether the scanner moved inside it.

Standard library only: it runs on the bench PC with any Python 3.12.
"""

from __future__ import annotations

import argparse
import contextlib
import itertools
import re
import socket
import statistics
import sys
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path

VERSION = "0.2.0-beta.180"
"""What ``VERSION`` answers after ``Aether HF``: a version VarAC 15.0.18 accepts (it refuses a
plain number it can read as under 4.8.8, CLAUDE.md, *The VERSION gate*)."""

CALL_S = 30.0
"""How long a ``CONNECT`` VarAC makes waits before the stand-in ends it with ``DISCONNECTED``:
nobody answers on this bench."""

BEACON_KEY_S = 4.0
"""How long a beacon keys when ``--key-beacons`` is given: about a tone-floor frame."""

KEEPALIVE_S = 10.0
"""How often Aether's adapter tells a quiet program ``IAMALIVE`` (``host/server.rs``); the
stand-in does too, and leaves those lines out of the timeline."""

AROUND_MOVE_S = 3.0
"""What the report shows of VarAC's lines on either side of a move."""


class Timeline:
    """Every event of a run, one line each: wall clock, seconds since the start, source, text.
    Written as it happens and flushed, so a run stopped by Ctrl+C keeps all of it."""

    def __init__(self, path: Path) -> None:
        self.start = time.time()
        self._lock = threading.Lock()
        self._file = path.open("a", encoding="utf-8")

    def record(self, source: str, text: str) -> float:
        now = time.time()
        stamp = datetime.fromtimestamp(now, UTC).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
        line = f"{stamp} {now - self.start:10.3f} {source:7s} {text}"
        with self._lock:
            # a thread still winding down after the run closed writes nothing
            if self._file.closed:
                return now - self.start
            self._file.write(line + "\n")
            self._file.flush()
            print(line, flush=True)
        return now - self.start

    def close(self) -> None:
        with self._lock:
            self._file.close()


# ── the rig: what VarAC asks rigctld ──────────────────────────────────────────────────────

_PREFIX = re.compile(r"^[+;|,]")


def rig_command(line: str) -> tuple[str, str] | None:
    """A frequency or PTT command in Hamlib's rigctld protocol, as ``("freq", hz)`` or
    ``("ptt", value)``; ``None`` for anything else. The short form (``F 7105000``, ``T 1``) and
    the long one (``\\set_freq 7105000``, with an extended-response prefix ``+``, ``;``, ``|``
    or ``,``) both count; a VFO named before the value is passed over."""
    words = _PREFIX.sub("", line.strip()).split()
    if not words:
        return None
    name, args = words[0], words[1:]
    if name in ("F", "\\set_freq") and args:
        value = args[-1]
        with contextlib.suppress(ValueError):
            return ("freq", str(round(float(value))))
    if name in ("T", "\\set_ptt") and args:
        return ("ptt", args[-1])
    return None


# ── the plan: lines sent at set times after a move ────────────────────────────────────────


@dataclass
class Step:
    """One planned line: sent ``offset_s`` after its anchor, the start (``move`` 0) or the
    ``move``-th frequency change."""

    move: int
    offset_s: float
    line: str
    sent: bool = False


_STEP = re.compile(r"^(start|move(\d+))\s+\+?(\d+(?:\.\d+)?)\s+(.+)$")


def parse_plan(text: str) -> list[Step]:
    """``move2 +50 PENDING``: 50 s after the second frequency change; ``start +5 BUSY ON``: 5 s
    after the run began. ``#`` starts a comment; ``{mycall}`` is the callsign VarAC said first in
    ``MYCALL``."""
    steps = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        found = _STEP.match(line)
        if not found:
            raise ValueError(f"plan line {number}: {raw!r} is not '<start|moveN> +SECONDS LINE'")
        move = 0 if found.group(1) == "start" else int(found.group(2))
        steps.append(Step(move, float(found.group(3)), found.group(4).strip()))
    return steps


# ── the run ───────────────────────────────────────────────────────────────────────────────


@dataclass
class Bench:
    """The stand-in modem, the pass-through to rigctld, the plan and the injections, sharing one
    timeline."""

    out: Path
    host_port: int = 8300
    rig_port: int = 4532
    rigctld: tuple[str, int] = ("127.0.0.1", 4533)
    version: str = VERSION
    key_beacons: bool = False
    plan: list[Step] = field(default_factory=list)
    call_s: float = CALL_S

    def __post_init__(self) -> None:
        self.out.mkdir(parents=True, exist_ok=True)
        self.timeline = Timeline(self.out / "timeline.log")
        self.stop = threading.Event()
        self.mycall = ""
        self.moves: list[float] = []  # seconds since the start, one a frequency change
        self.last_hz: str | None = None
        self.calling = False
        self.in_session = False  # a CONNECTED said and no DISCONNECTED yet
        self._command: socket.socket | None = None
        self._send_lock = threading.Lock()
        self._last_said = time.monotonic()
        self._listeners: list[socket.socket] = []

    # the VARA side

    def say(self, line: str, why: str = "") -> None:
        """A line to VarAC on the command port, CR-terminated as VARA's are."""
        with self._send_lock:
            sock = self._command
            if sock is None:
                self.timeline.record("note", f"not attached: {line!r} not said{why}")
                return
            # stamped before it goes: what VarAC does on hearing it comes after it in the timeline
            if line != "IAMALIVE":
                self.timeline.record("modem>", line + why)
            self._last_said = time.monotonic()
            with contextlib.suppress(OSError):
                sock.sendall((line + "\r").encode())
        if line.startswith("CONNECTED"):
            self.in_session = True
        elif line.startswith("DISCONNECTED"):
            self.in_session = False

    def _answer(self, line: str) -> None:
        words = line.split()
        head = words[0].upper() if words else ""
        if head == "MYCALL" and len(words) > 1:
            self.mycall = words[1]
        if head == "VERSION":
            self.say(f"VERSION Aether HF {self.version}")
        elif line.upper() == "TUNE ?":
            self.say("TUNE -12")
        elif head == "BUFFER":
            self.say("BUFFER 0")
        elif head == "CONNECT":
            self.say("OK")
            self.calling = True
            threading.Thread(target=self._call_ends, daemon=True).start()
        elif head in ("DISCONNECT", "ABORT"):
            self.say("OK")
            if self.calling or self.in_session:
                self.calling = False
                self.say("DISCONNECTED")
        elif head == "CQFRAME":
            self.say("OK")
            if self.key_beacons:
                threading.Thread(target=self._key_beacon, daemon=True).start()
        else:
            self.say("OK")

    def _keep_alive(self) -> None:
        while not self.stop.is_set():
            time.sleep(0.5)
            if self._command is not None and time.monotonic() - self._last_said >= KEEPALIVE_S:
                self.say("IAMALIVE")

    def _call_ends(self) -> None:
        deadline = time.monotonic() + self.call_s
        while self.calling and not self.stop.is_set() and time.monotonic() < deadline:
            time.sleep(0.1)
        if self.calling:
            self.calling = False
            self.say("DISCONNECTED", "  (nobody answered)")

    def _key_beacon(self) -> None:
        time.sleep(0.3)
        self.say("PTT ON", "  (the beacon)")
        time.sleep(BEACON_KEY_S)
        self.say("PTT OFF", "  (the beacon)")

    def _serve_command(self, sock: socket.socket) -> None:
        with self._send_lock:
            self._command = sock
        self.timeline.record("note", "VarAC attached to the command port")
        self.say("BUFFER 0")
        pending = b""
        while not self.stop.is_set():
            try:
                chunk = sock.recv(4096)
            except OSError:
                break
            if not chunk:
                break
            pending += chunk
            while True:
                cut = re.search(rb"[\r\n]", pending)
                if cut is None:
                    break
                raw, pending = pending[: cut.start()], pending[cut.end() :]
                line = raw.decode(errors="replace").strip()
                if line:
                    self.timeline.record("varac>", line)
                    self._answer(line)
        with self._send_lock:
            if self._command is sock:
                self._command = None
        self.timeline.record("note", "VarAC left the command port")

    def _serve_data(self, sock: socket.socket) -> None:
        self.timeline.record("note", "VarAC attached to the data port")
        while not self.stop.is_set():
            try:
                chunk = sock.recv(65536)
            except OSError:
                break
            if not chunk:
                break
            self.timeline.record("data>", f"{len(chunk)} bytes")
        self.timeline.record("note", "VarAC left the data port")

    # the rig side

    def _serve_rig(self, client: socket.socket) -> None:
        try:
            upstream = socket.create_connection(self.rigctld, timeout=5)
        except OSError as error:
            self.timeline.record("note", f"rigctld at {self.rigctld[0]}:{self.rigctld[1]}: {error}")
            client.close()
            return
        upstream.settimeout(None)
        threading.Thread(target=self._pump, args=(upstream, client), daemon=True).start()
        pending = b""
        while not self.stop.is_set():
            try:
                chunk = client.recv(4096)
            except OSError:
                break
            if not chunk:
                break
            with contextlib.suppress(OSError):
                upstream.sendall(chunk)
            pending += chunk
            while b"\n" in pending:
                raw, pending = pending.split(b"\n", 1)
                self._rig_line(raw.decode(errors="replace").strip())
        for sock in (client, upstream):
            with contextlib.suppress(OSError):
                sock.close()

    def _rig_line(self, line: str) -> None:
        if not line:
            return
        command = rig_command(line)
        if command is None:
            # the frequency read every couple of seconds is kept out: it is most of the traffic
            if line not in ("f", "\\get_freq", "+\\get_freq"):
                self.timeline.record("rig<", line)
            return
        kind, value = command
        if kind == "ptt":
            self.timeline.record("ptt", f"{value}  ({line})")
            return
        if value == self.last_hz:
            self.timeline.record("rig<", f"{line}  (the same frequency)")
            return
        self.last_hz = value
        at = self.timeline.record("move", f"{len(self.moves) + 1} {value}  ({line})")
        self.moves.append(at)

    @staticmethod
    def _pump(src: socket.socket, dst: socket.socket) -> None:
        while True:
            try:
                chunk = src.recv(4096)
            except OSError:
                break
            if not chunk:
                break
            try:
                dst.sendall(chunk)
            except OSError:
                break
        with contextlib.suppress(OSError):
            dst.shutdown(socket.SHUT_WR)

    # the plan and the injections

    def _run_plan(self) -> None:
        while not self.stop.is_set():
            now = time.time() - self.timeline.start
            for step in self.plan:
                if step.sent:
                    continue
                anchor = 0.0 if step.move == 0 else None
                if step.move and len(self.moves) >= step.move:
                    anchor = self.moves[step.move - 1]
                if anchor is not None and now >= anchor + step.offset_s:
                    step.sent = True
                    where = "start" if step.move == 0 else f"move{step.move}"
                    line = step.line.replace("{mycall}", self.mycall or "N0CALL")
                    self.say(line, f"  (plan: {where} +{step.offset_s:g})")
            time.sleep(0.05)

    def _watch_inject(self) -> None:
        path = self.out / "inject.txt"
        path.touch()
        seen = path.stat().st_size
        while not self.stop.is_set():
            time.sleep(0.2)
            size = path.stat().st_size
            if size <= seen:
                seen = min(seen, size)
                continue
            with path.open("rb") as handle:
                handle.seek(seen)
                added = handle.read(size - seen)
            # a line is taken whole: one written without its end yet waits
            if b"\n" not in added:
                continue
            complete = added[: added.rindex(b"\n") + 1]
            seen += len(complete)
            for raw in complete.decode("utf-8-sig", errors="replace").splitlines():
                if raw.strip():
                    self.say(raw.strip(), "  (inject.txt)")

    def _watch_console(self) -> None:
        for raw in sys.stdin:
            if self.stop.is_set():
                return
            if raw.strip():
                self.say(raw.strip(), "  (typed)")

    # start and stop

    def _listen(self, port: int, serve: Callable[[socket.socket], None], many: bool) -> int:
        server = socket.create_server(("127.0.0.1", port))
        self._listeners.append(server)

        def accept() -> None:
            while not self.stop.is_set():
                try:
                    sock, _ = server.accept()
                except OSError:
                    return
                thread = threading.Thread(target=serve, args=(sock,), daemon=True)
                thread.start()
                if not many:
                    thread.join()

        threading.Thread(target=accept, daemon=True).start()
        return int(server.getsockname()[1])

    def start(self, console: bool = True) -> tuple[int, int, int]:
        """Listen, and return the command, data and rig ports (a port of 0 picks a free one)."""
        command = self._listen(self.host_port, self._serve_command, many=False)
        data = self._listen(self.host_port + 1 if self.host_port else 0, self._serve_data, True)
        rig = self._listen(self.rig_port, self._serve_rig, many=True)
        self.timeline.record(
            "note",
            f"modem on {command} (data {data}), rig pass-through on {rig} to rigctld at "
            f"{self.rigctld[0]}:{self.rigctld[1]}, {len(self.plan)} planned lines",
        )
        threading.Thread(target=self._run_plan, daemon=True).start()
        threading.Thread(target=self._keep_alive, daemon=True).start()
        threading.Thread(target=self._watch_inject, daemon=True).start()
        if console:
            threading.Thread(target=self._watch_console, daemon=True).start()
        return command, data, rig

    def close(self) -> None:
        self.stop.set()
        for server in self._listeners:
            with contextlib.suppress(OSError):
                server.close()
        with self._send_lock:
            if self._command is not None:
                with contextlib.suppress(OSError):
                    self._command.close()
        self.timeline.record("note", "stopped")
        self.timeline.close()


# ── the report ────────────────────────────────────────────────────────────────────────────


@dataclass
class Event:
    t: float
    source: str
    text: str


def read_timeline(path: Path) -> list[Event]:
    events = []
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split(maxsplit=3)
        if len(parts) < 3:
            continue
        with contextlib.suppress(ValueError):
            events.append(Event(float(parts[1]), parts[2], parts[3] if len(parts) > 3 else ""))
    return events


@dataclass
class Window:
    """A stretch in which the modem held something up for VarAC."""

    what: str
    begin: float
    end: float | None


_OPENS = {"BUSY ON": "BUSY ON", "PENDING": "PENDING", "PTT ON": "PTT ON"}
_CLOSES: dict[str, tuple[str, ...]] = {
    "BUSY OFF": ("BUSY ON",),
    "CANCELPENDING": ("PENDING",),
    "CONNECTED": ("PENDING",),
    "PTT OFF": ("PTT ON",),
    "DISCONNECTED": ("session", "PENDING"),
}


def windows(events: list[Event]) -> list[Window]:
    """The modem's holds, from what it said: ``BUSY ON`` to ``BUSY OFF``, ``PENDING`` to
    ``CONNECTED``, ``CANCELPENDING`` or ``DISCONNECTED``, ``CONNECTED`` to ``DISCONNECTED``
    (a session), ``PTT ON`` to ``PTT OFF``. One still open at the end has no end."""
    open_: dict[str, Window] = {}
    found: list[Window] = []
    for event in events:
        if event.source != "modem>":
            continue
        line = event.text.split("  (")[0].strip()
        if not line:
            continue
        head = " ".join(line.split()[:2]) if line.startswith(("BUSY", "PTT")) else line.split()[0]
        for what in _CLOSES.get(head, ()):
            if what in open_:
                open_.pop(what).end = event.t
        if head in _OPENS and head not in open_:
            window = Window(_OPENS[head], event.t, None)
            open_[head] = window
            found.append(window)
        if head == "CONNECTED" and "session" not in open_:
            window = Window("session", event.t, None)
            open_["session"] = window
            found.append(window)
    return found


def report(directory: Path) -> str:
    events = read_timeline(directory / "timeline.log")
    moves = [e for e in events if e.source == "move"]
    said = [e for e in events if e.source == "varac>"]
    lines = [f"{directory}: {len(events)} events, {len(moves)} frequency changes"]
    if len(moves) > 1:
        stays = [b.t - a.t for a, b in itertools.pairwise(moves)]
        lines.append(
            f"stay between changes: median {statistics.median(stays):.1f} s, "
            f"from {min(stays):.1f} to {max(stays):.1f} s"
        )
    lines.append("")
    lines.append(f"what VarAC said to the modem within {AROUND_MOVE_S:g} s of each change:")
    quiet = 0
    for move in moves:
        near = [e for e in said if abs(e.t - move.t) <= AROUND_MOVE_S]
        if not near:
            quiet += 1
            continue
        words = ", ".join(f"{e.text} ({e.t - move.t:+.1f} s)" for e in near)
        lines.append(f"  change {move.text.split()[0]} at {move.t:.1f} s: {words}")
    lines.append(f"  nothing around {quiet} of {len(moves)} changes")
    lines.append("")
    lines.append("the modem's holds, and the changes inside them:")
    held = windows(events)
    if not held:
        lines.append("  none said")
    for window in held:
        end = window.end if window.end is not None else events[-1].t
        inside = [m for m in moves if window.begin <= m.t < end]
        after = [m for m in moves if m.t >= end]
        tail = (
            f"; next change {after[0].t - end:.1f} s after it ended"
            if after and window.end is not None
            else ""
        )
        closed = f"{window.end:.1f}" if window.end is not None else "the end"
        lines.append(
            f"  {window.what} {window.begin:.1f}-{closed} s ({end - window.begin:.1f} s): "
            f"{len(inside)} changes inside{tail}"
        )
    return "\n".join(lines)


# ── the command line ──────────────────────────────────────────────────────────────────────


def _address(text: str) -> tuple[str, int]:
    host, _, port = text.rpartition(":")
    return (host or "127.0.0.1", int(port))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else "")
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run", help="stand in for the modem and pass the rig through")
    run.add_argument("--out", type=Path, required=True, help="the run's folder")
    run.add_argument("--plan", type=Path, help="lines to send at times after a change")
    run.add_argument("--seconds", type=float, help="stop after this long (else Ctrl+C)")
    run.add_argument("--host-port", type=int, default=8300)
    run.add_argument("--rig-port", type=int, default=4532, help="where VarAC's rig control points")
    run.add_argument("--rigctld", type=_address, default=("127.0.0.1", 4533))
    run.add_argument("--version", default=VERSION, help="what VERSION answers after 'Aether HF'")
    run.add_argument(
        "--key-beacons", action="store_true", help="answer a CQFRAME with PTT ON and PTT OFF"
    )
    show = commands.add_parser("report", help="read a run's timeline")
    show.add_argument("dir", type=Path)
    args = parser.parse_args(argv)
    if args.command == "report":
        print(report(args.dir))
        return 0
    plan = parse_plan(args.plan.read_text(encoding="utf-8")) if args.plan else []
    bench = Bench(
        args.out,
        host_port=args.host_port,
        rig_port=args.rig_port,
        rigctld=args.rigctld,
        version=args.version,
        key_beacons=args.key_beacons,
        plan=plan,
    )
    bench.start()
    try:
        deadline = time.monotonic() + args.seconds if args.seconds else None
        while deadline is None or time.monotonic() < deadline:
            time.sleep(0.2)
    except KeyboardInterrupt:
        pass
    finally:
        bench.close()
    print()
    print(report(args.out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
