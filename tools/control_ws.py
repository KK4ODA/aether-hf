# SPDX-License-Identifier: MIT OR Apache-2.0
"""A connection to a daemon's control API that stays open: what a program that scans holds.

The scenario harness (``session_matrix.py``) drives the daemons with one-shot ``POST
/v1/<method>`` requests, each its own connection. A scan belongs to the connection that asked
for it, and ends when that connection closes (ADR-0129), so a station that scans needs a
connection that lasts: this is the control API's WebSocket (``ws://127.0.0.1:<port>/v1``,
``docs/spec/control-api.md`` §2), standard library only. A thread reads what the daemon sends,
answers its pings, keeps the answers to requests and drops the events.
"""

from __future__ import annotations

import base64
import json
import os
import socket
import struct
import threading


class ControlSocket:
    """One WebSocket client of a daemon's control API."""

    def __init__(self, port: int, timeout: float = 15.0) -> None:
        self.timeout = timeout
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                f"GET /v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
                f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        head = b""
        while b"\r\n\r\n" not in head:
            got = self.sock.recv(1)
            if not got:
                raise ConnectionError("the daemon closed the upgrade")
            head += got
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise ConnectionError(head.split(b"\r\n", 1)[0].decode(errors="replace"))
        self.sock.settimeout(None)
        self._lock = threading.Lock()
        self._answers: dict[str, dict] = {}
        self._arrived = threading.Condition()
        self._next = 0
        self.closed = False
        threading.Thread(target=self._read, daemon=True).start()

    def request(self, method: str, params: dict | None = None) -> dict:
        """Send a request and wait for its answer: ``{"ok": …, "result"|"error": …}``."""
        with self._lock:
            self._next += 1
            ident = f"s{self._next}"
        self._send(json.dumps({"id": ident, "method": method, "params": params or {}}))
        with self._arrived:
            got = self._arrived.wait_for(
                lambda: ident in self._answers or self.closed, timeout=self.timeout
            )
            if not got or ident not in self._answers:
                return {"ok": False, "error": {"code": "no_answer", "message": method}}
            return self._answers.pop(ident)

    def close(self) -> None:
        """Close the connection: the daemon ends what it owned (its scan, its claim)."""
        self.closed = True
        try:
            self._frame(0x8, b"")
            self.sock.close()
        except OSError:
            pass

    def _send(self, text: str) -> None:
        self._frame(0x1, text.encode())

    def _frame(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        n = len(payload)
        if n < 126:
            head = struct.pack("!BB", 0x80 | opcode, 0x80 | n)
        elif n < 1 << 16:
            head = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, n)
        else:
            head = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self._lock:
            self.sock.sendall(head + mask + masked)

    def _exact(self, n: int) -> bytes:
        out = b""
        while len(out) < n:
            got = self.sock.recv(n - len(out))
            if not got:
                raise ConnectionError("closed")
            out += got
        return out

    def _read(self) -> None:
        try:
            while not self.closed:
                b0, b1 = self._exact(2)
                opcode, n = b0 & 0x0F, b1 & 0x7F
                if n == 126:
                    (n,) = struct.unpack("!H", self._exact(2))
                elif n == 127:
                    (n,) = struct.unpack("!Q", self._exact(8))
                payload = self._exact(n)
                if opcode == 0x9:
                    self._frame(0xA, payload)
                elif opcode == 0x8:
                    break
                elif opcode == 0x1:
                    message = json.loads(payload)
                    if "id" in message and message.get("id") is not None:
                        with self._arrived:
                            self._answers[str(message["id"])] = message
                            self._arrived.notify_all()
        except (OSError, ConnectionError, ValueError):
            pass
        finally:
            self.closed = True
            with self._arrived:
                self._arrived.notify_all()
