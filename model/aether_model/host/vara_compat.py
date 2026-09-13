"""
aether_model/host/vara_compat.py

VARA-compatible TCP host interface.

Exposes command and data ports that implement the same protocol as VARA HF,
allowing Winlink Express, PAT, VarAC, and other VARA-compatible applications
to connect without modification.

Command port: text-based, line-delimited (MYCALL, CONNECT, LISTEN, etc.)
Data port: binary stream for payload data.
"""

import contextlib
import logging
import socket
import threading
from collections.abc import Callable

from aether_model.constants import DEFAULT_CMD_PORT, DEFAULT_DATA_PORT

log = logging.getLogger(__name__)


class VaraCompatServer:
    """VARA-compatible TCP server for host application communication.

    Implements the VARA TCP API so applications like Winlink Express
    and PAT can connect to AETHER HF without modification.
    """

    def __init__(
        self,
        cmd_port: int = DEFAULT_CMD_PORT,
        data_port: int = DEFAULT_DATA_PORT,
        bind_addr: str = "127.0.0.1",
        on_command: Callable[[str], None] | None = None,
        on_data: Callable[[bytes], None] | None = None,
    ):
        self._cmd_port = cmd_port
        self._data_port = data_port
        self._bind = bind_addr
        self._on_command = on_command
        self._on_data = on_data

        self._cmd_server: socket.socket | None = None
        self._data_server: socket.socket | None = None
        self._cmd_client: socket.socket | None = None
        self._data_client: socket.socket | None = None

        self._running = False
        self._lock = threading.Lock()

        # State
        self._my_call = ""
        self._listening = False
        self._tx_buffer = bytearray()

    def start(self):
        """Start the TCP servers."""
        self._running = True

        self._cmd_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._cmd_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._cmd_server.bind((self._bind, self._cmd_port))
        self._cmd_server.listen(1)

        self._data_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._data_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._data_server.bind((self._bind, self._data_port))
        self._data_server.listen(1)

        threading.Thread(target=self._accept_cmd, daemon=True, name="VARA-Cmd-Accept").start()
        threading.Thread(target=self._accept_data, daemon=True, name="VARA-Data-Accept").start()

        log.info(f"VARA-compat server on cmd:{self._cmd_port} data:{self._data_port}")

    def stop(self):
        """Stop the TCP servers."""
        self._running = False
        for s in (self._cmd_client, self._data_client, self._cmd_server, self._data_server):
            if s:
                with contextlib.suppress(OSError):
                    s.close()
        self._cmd_client = None
        self._data_client = None

    # ── Send events to the host application ───────────────────────────

    def send_event(self, event: str):
        """Send a VARA-style event to the host (e.g. 'CONNECTED W1ABC')."""
        self._send_cmd_line(event)

    def send_data_to_host(self, data: bytes):
        """Forward received RF data to the host application."""
        if self._data_client:
            try:
                self._data_client.sendall(data)
            except OSError as e:
                log.warning(f"Data send to host failed: {e}")

    # ── Internal ──────────────────────────────────────────────────────

    def _accept_cmd(self):
        """Accept command port connections."""
        while self._running:
            try:
                self._cmd_server.settimeout(1.0)
                client, addr = self._cmd_server.accept()
                log.info(f"Host connected to command port from {addr}")
                self._cmd_client = client
                threading.Thread(target=self._read_cmd, daemon=True, name="VARA-Cmd-Read").start()
            except TimeoutError:
                continue
            except OSError:
                break

    def _accept_data(self):
        """Accept data port connections."""
        while self._running:
            try:
                self._data_server.settimeout(1.0)
                client, addr = self._data_server.accept()
                log.info(f"Host connected to data port from {addr}")
                self._data_client = client
                threading.Thread(target=self._read_data, daemon=True, name="VARA-Data-Read").start()
            except TimeoutError:
                continue
            except OSError:
                break

    def _read_cmd(self):
        """Read and process commands from the host application."""
        buf = ""
        while self._running and self._cmd_client:
            try:
                data = self._cmd_client.recv(4096)
                if not data:
                    break
                buf += data.decode("utf-8", errors="replace")
                while "\n" in buf:
                    line, buf = buf.split("\n", 1)
                    line = line.strip()
                    if line:
                        self._handle_command(line)
            except OSError:
                break
        log.info("Host disconnected from command port")
        self._cmd_client = None

    def _read_data(self):
        """Read data from the host for transmission."""
        while self._running and self._data_client:
            try:
                data = self._data_client.recv(65536)
                if not data:
                    break
                with self._lock:
                    self._tx_buffer.extend(data)
                if self._on_data:
                    self._on_data(bytes(data))
            except OSError:
                break
        log.info("Host disconnected from data port")
        self._data_client = None

    def _handle_command(self, cmd: str):
        """Process a VARA-style command from the host."""
        upper = cmd.upper()
        log.debug(f"Host command: {cmd}")

        if upper.startswith("MYCALL "):
            self._my_call = cmd[7:].strip().upper()
            self._send_cmd_line("OK")

        elif upper.startswith("CONNECT "):
            remote = cmd[8:].strip().upper()
            if self._on_command:
                self._on_command(f"CONNECT {remote}")

        elif upper == "DISCONNECT":
            if self._on_command:
                self._on_command("DISCONNECT")

        elif upper == "LISTEN ON":
            self._listening = True
            self._send_cmd_line("OK")

        elif upper == "LISTEN OFF":
            self._listening = False
            self._send_cmd_line("OK")

        elif upper.startswith("BW"):
            # BW500 or BW2300
            bw = upper[2:]
            if self._on_command:
                self._on_command(f"BW {bw}")
            self._send_cmd_line("OK")

        elif upper.startswith("COMPRESSION"):
            self._send_cmd_line("OK")

        elif upper == "BUFFER":
            with self._lock:
                n = len(self._tx_buffer)
            self._send_cmd_line(f"BUFFER {n}")

        else:
            log.warning(f"Unknown host command: {cmd}")
            self._send_cmd_line("WRONG")

    def _send_cmd_line(self, line: str):
        """Send a line to the host command port."""
        if self._cmd_client:
            with contextlib.suppress(OSError):
                self._cmd_client.sendall(f"{line}\r\n".encode())

    def get_tx_data(self, max_bytes: int) -> bytes:
        """Consume data from the TX buffer for transmission."""
        with self._lock:
            data = bytes(self._tx_buffer[:max_bytes])
            self._tx_buffer = self._tx_buffer[max_bytes:]
        return data

    @property
    def has_tx_data(self) -> bool:
        with self._lock:
            return len(self._tx_buffer) > 0
