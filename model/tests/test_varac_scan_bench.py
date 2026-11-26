# SPDX-License-Identifier: MIT OR Apache-2.0
"""The VarAC scanner bench (``tools/varac_scan_bench.py``, ADR-0138 §7): it answers a VARA
client as Aether's adapter does, sees VarAC's frequency changes in what passes to rigctld, sends a
planned line at its time after a change, and reports the changes inside the modem's holds."""

from __future__ import annotations

import socket
import sys
import threading
import time
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))

import varac_scan_bench as bench


def test_the_rig_commands_that_move_or_key_the_radio_are_read_in_both_forms() -> None:
    assert bench.rig_command("F 7105000") == ("freq", "7105000")
    assert bench.rig_command("\\set_freq 14105000.000000") == ("freq", "14105000")
    assert bench.rig_command("+\\set_freq VFOA 7105000") == ("freq", "7105000")
    assert bench.rig_command("T 1") == ("ptt", "1")
    assert bench.rig_command(";\\set_ptt 0") == ("ptt", "0")
    assert bench.rig_command("f") is None
    assert bench.rig_command("M USB 3000") is None


def test_a_plan_names_its_anchor_offset_and_line() -> None:
    steps = bench.parse_plan(
        "# BUSY just before the third change\nmove2 +50 BUSY ON\nmove2 +80 BUSY OFF\n"
        "start +5 PENDING   # a comment\n"
    )
    assert [(s.move, s.offset_s, s.line) for s in steps] == [
        (2, 50.0, "BUSY ON"),
        (2, 80.0, "BUSY OFF"),
        (0, 5.0, "PENDING"),
    ]
    with pytest.raises(ValueError, match="plan line 1"):
        bench.parse_plan("after the move, PENDING\n")


def _lines(sock: socket.socket, want: int, timeout: float = 5.0) -> list[str]:
    sock.settimeout(timeout)
    got = b""
    while got.count(b"\r") < want:
        got += sock.recv(4096)
    return [line for line in got.decode().split("\r") if line]


def _fake_rigctld() -> tuple[socket.socket, int]:
    """Answers every command as Hamlib's dummy rig does to a set: ``RPRT 0``."""
    server = socket.create_server(("127.0.0.1", 0))

    def serve() -> None:
        with server:
            try:
                sock, _ = server.accept()
            except OSError:
                return
            with sock:
                while True:
                    try:
                        chunk = sock.recv(4096)
                    except OSError:
                        return
                    if not chunk:
                        return
                    sock.sendall(b"RPRT 0\n" * chunk.count(b"\n"))

    threading.Thread(target=serve, daemon=True).start()
    return server, int(server.getsockname()[1])


def test_the_bench_answers_varac_sees_its_moves_and_reports_the_holds(tmp_path: Path) -> None:
    rig_server, rig_port = _fake_rigctld()
    run = bench.Bench(
        tmp_path / "run",
        host_port=0,
        rig_port=0,
        rigctld=("127.0.0.1", rig_port),
        plan=bench.parse_plan("move1 +0.2 BUSY ON\nmove2 +0.2 BUSY OFF\n"),
    )
    command_port, data_port, proxy_port = run.start(console=False)
    try:
        varac = socket.create_connection(("127.0.0.1", command_port))
        data = socket.create_connection(("127.0.0.1", data_port))
        # VarAC waits for the empty queue before it says anything
        assert _lines(varac, 1) == ["BUFFER 0"]
        varac.sendall(b"MYCALL WD5EMA WD5EMA-T\rVERSION\r")
        assert _lines(varac, 2) == ["OK", f"VERSION Aether HF {bench.VERSION}"]
        rig = socket.create_connection(("127.0.0.1", proxy_port))
        rig.sendall(b"F 7105000\n")
        assert rig.recv(64) == b"RPRT 0\n"
        # the planned line goes 0.2 s after the first change
        assert _lines(varac, 1) == ["BUSY ON"]
        rig.sendall(b"F 7105000\nF 14105000\n")
        assert _lines(varac, 1) == ["BUSY OFF"]
        data.sendall(b"hello")
        time.sleep(0.3)
        for sock in (rig, data, varac):
            sock.close()
    finally:
        run.close()
        rig_server.close()
    text = bench.report(tmp_path / "run")
    # the same frequency set again is no change
    assert "2 frequency changes" in text, text
    assert "BUSY ON" in text and "1 changes inside" in text, text
    timeline = (tmp_path / "run" / "timeline.log").read_text(encoding="utf-8")
    assert "varac>  MYCALL WD5EMA WD5EMA-T" in timeline
    assert "data>   5 bytes" in timeline


def test_a_session_and_a_pending_close_as_the_modem_says() -> None:
    def said(t: float, line: str) -> bench.Event:
        return bench.Event(t, "modem>", line)

    held = bench.windows(
        [
            said(1.0, "PENDING  (inject.txt)"),
            said(2.0, "CONNECTED KK4XYZ WD5EMA 500"),
            said(9.0, "PTT ON"),
            said(9.5, "PTT OFF"),
            said(20.0, "DISCONNECTED"),
            said(30.0, "BUSY ON"),
        ]
    )
    assert [(w.what, w.begin, w.end) for w in held] == [
        ("PENDING", 1.0, 2.0),
        ("session", 2.0, 20.0),
        ("PTT ON", 9.0, 9.5),
        ("BUSY ON", 30.0, None),
    ]
