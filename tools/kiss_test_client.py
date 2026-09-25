"""A KISS-over-TCP test client, for trying Aether's KISS port (or any KISS TNC) by hand.

Connects to a KISS TCP endpoint, sends frames, and shows what comes back — decoded as AX.25
where it is AX.25, with hex dumps — so the port can be tested without an APRS program:

    python tools/kiss_test_client.py listen                          # show what arrives
    python tools/kiss_test_client.py ax25 KK4ODA-1 APRS "hello"      # an AX.25 UI frame
    python tools/kiss_test_client.py ax25 KK4ODA-1 APRS ">test" --via WIDE2-1
    python tools/kiss_test_client.py raw c0 00 41 42 c0 --fragment 1 # bytes, one per write
    python tools/kiss_test_client.py escapes                         # FEND/FESC in the data
    python tools/kiss_test_client.py burst --count 50 --size 200     # many frames, quickly
    python tools/kiss_test_client.py params --txdelay 30 --persist 63

Every command takes ``--host`` (127.0.0.1) and ``--port`` (8100, Aether's KISS default) and
``--listen SECONDS`` to wait for frames afterwards. ``--fragment N`` writes N bytes at a time
with a pause between writes, to prove the far end reassembles frames split across TCP reads
— including an escape split from what it escapes.

Only the standard library: nothing to install.
"""

from __future__ import annotations

import argparse
import contextlib
import socket
import sys
import time
from dataclasses import dataclass

FEND = 0xC0
FESC = 0xDB
TFEND = 0xDC
TFESC = 0xDD

DATA = 0x00
TX_DELAY = 0x01
PERSISTENCE = 0x02
SLOT_TIME = 0x03
TX_TAIL = 0x04
FULL_DUPLEX = 0x05
RETURN = 0xFF


# ── KISS ──────────────────────────────────────────────────────────────


def kiss_encode(data: bytes, command: int = DATA, port: int = 0) -> bytes:
    """One frame for the wire, the type byte escaped along with the data."""
    kind = RETURN if command == RETURN else ((port & 0x0F) << 4) | (command & 0x0F)
    out = bytearray([FEND])
    for byte in bytes([kind]) + data:
        if byte == FEND:
            out += bytes([FESC, TFEND])
        elif byte == FESC:
            out += bytes([FESC, TFESC])
        else:
            out.append(byte)
    out.append(FEND)
    return bytes(out)


@dataclass
class KissFrame:
    port: int
    command: int
    data: bytes


class KissDecoder:
    """Frames out of a stream that arrives in pieces (the same rules as the daemon's)."""

    def __init__(self) -> None:
        self.buffer = bytearray()
        self.in_frame = False
        self.escaped = False
        self.bad: str | None = None

    def push(self, chunk: bytes) -> list[KissFrame | str]:
        out: list[KissFrame | str] = []
        for byte in chunk:
            if byte == FEND:
                if self.in_frame:
                    if self.bad or self.escaped:
                        out.append(self.bad or "frame ended inside an escape")
                    elif self.buffer:
                        kind = self.buffer[0]
                        if kind == RETURN:
                            out.append(KissFrame(0, RETURN, bytes(self.buffer[1:])))
                        else:
                            out.append(KissFrame(kind >> 4, kind & 0x0F, bytes(self.buffer[1:])))
                self.buffer.clear()
                self.in_frame = True
                self.escaped = False
                self.bad = None
                continue
            if not self.in_frame or self.bad:
                continue
            if self.escaped:
                self.escaped = False
                if byte == TFEND:
                    self.buffer.append(FEND)
                elif byte == TFESC:
                    self.buffer.append(FESC)
                else:
                    self.bad = f"0x{byte:02X} after FESC"
            elif byte == FESC:
                self.escaped = True
            else:
                self.buffer.append(byte)
        return out


# ── AX.25 ─────────────────────────────────────────────────────────────


def ax25_address(call: str, last: bool, command_bit: bool) -> bytes:
    """A callsign and SSID as AX.25 carries them: six characters shifted left, then the SSID
    byte with its reserved bits set, the C bit, and the end-of-addresses bit."""
    base, _, ssid_text = call.upper().partition("-")
    ssid = int(ssid_text) if ssid_text else 0
    if not 1 <= len(base) <= 6 or not 0 <= ssid <= 15:
        raise ValueError(f"not an AX.25 callsign: {call}")
    shifted = bytes((ord(c) << 1) & 0xFE for c in base.ljust(6))
    ssid_byte = 0x60 | (ssid << 1) | (0x80 if command_bit else 0) | (0x01 if last else 0)
    return shifted + bytes([ssid_byte])


def ax25_ui(source: str, destination: str, info: bytes, via: list[str] | None = None) -> bytes:
    """An AX.25 UI frame as a KISS client hands it over: addresses, control 0x03, PID 0xF0
    (no layer 3), the information field — no flags and no FCS, which the TNC adds."""
    via = via or []
    out = ax25_address(destination, last=False, command_bit=True)
    out += ax25_address(source, last=not via, command_bit=False)
    for index, digi in enumerate(via):
        out += ax25_address(digi, last=index == len(via) - 1, command_bit=False)
    return out + bytes([0x03, 0xF0]) + info


def ax25_describe(frame: bytes) -> str | None:
    """ "SRC>DST,DIGI*:info" for an AX.25 frame, or None when it is not one."""
    calls = []
    offset = 0
    while True:
        if offset + 7 > len(frame):
            return None
        raw = frame[offset : offset + 7]
        text = "".join(chr(b >> 1) for b in raw[:6]).strip()
        if not text or not all(c.isalnum() for c in text):
            return None
        ssid = (raw[6] >> 1) & 0x0F
        name = f"{text}-{ssid}" if ssid else text
        if len(calls) >= 2 and raw[6] & 0x80:
            name += "*"  # a digipeater that has repeated it
        calls.append(name)
        offset += 7
        if raw[6] & 0x01:
            break
        if len(calls) > 10:
            return None
    if len(calls) < 2 or offset >= len(frame):
        return None
    control = frame[offset]
    info = frame[offset + 1 :]
    if control == 0x03 and info:  # UI: a PID byte, then the information
        info = info[1:]
    path = ",".join(calls[2:])
    head = f"{calls[1]}>{calls[0]}" + (f",{path}" if path else "")
    text = info.decode("latin-1")
    printable = "".join(c if 32 <= ord(c) < 127 else "." for c in text)
    return f"{head} [ctl 0x{control:02X}]: {printable}"


# ── the connection ────────────────────────────────────────────────────


def hexdump(data: bytes, width: int = 16) -> str:
    lines = []
    for offset in range(0, len(data), width):
        chunk = data[offset : offset + width]
        hexes = " ".join(f"{b:02x}" for b in chunk)
        text = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        lines.append(f"  {offset:04x}  {hexes:<{width * 3}} {text}")
    return "\n".join(lines)


def send(sock: socket.socket, data: bytes, fragment: int, pause: float) -> None:
    """Write the bytes whole, or ``fragment`` at a time with a pause between writes."""
    if fragment <= 0:
        sock.sendall(data)
        return
    for offset in range(0, len(data), fragment):
        sock.sendall(data[offset : offset + fragment])
        time.sleep(pause)


def show(item: KissFrame | str, dump: bool) -> None:
    stamp = time.strftime("%H:%M:%S")
    if isinstance(item, str):
        print(f"{stamp}  malformed frame: {item}")
        return
    if item.command == DATA:
        described = ax25_describe(item.data)
        print(
            f"{stamp}  port {item.port} data, {len(item.data)} bytes: {described or '(not AX.25)'}"
        )
    else:
        print(f"{stamp}  port {item.port} command 0x{item.command:02X}, {len(item.data)} bytes")
    if dump:
        print(hexdump(item.data))


def listen(sock: socket.socket, seconds: float, dump: bool) -> int:
    decoder = KissDecoder()
    sock.settimeout(0.2)
    deadline = time.monotonic() + seconds if seconds > 0 else float("inf")
    count = 0
    while time.monotonic() < deadline:
        try:
            chunk = sock.recv(4096)
        except TimeoutError:
            continue
        except OSError as error:
            print(f"connection lost: {error}")
            break
        if not chunk:
            print("the far end closed the connection")
            break
        for item in decoder.push(chunk):
            show(item, dump)
            count += 1
    return count


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8100)
    parser.add_argument("--fragment", type=int, default=0, help="write this many bytes at a time")
    parser.add_argument("--pause", type=float, default=0.02, help="seconds between fragments")
    parser.add_argument("--listen", type=float, default=0.0, help="then show frames this long")
    parser.add_argument("--hex", action="store_true", help="hex dumps of every frame")
    parser.add_argument("--kiss-port", type=int, default=0, help="the KISS port nibble")
    sub = parser.add_subparsers(dest="action", required=True)

    sub.add_parser("listen", help="show frames until interrupted")

    ax = sub.add_parser("ax25", help="send an AX.25 UI frame")
    ax.add_argument("source")
    ax.add_argument("destination")
    ax.add_argument("info")
    ax.add_argument("--via", action="append", default=[])

    raw = sub.add_parser("raw", help="send bytes exactly as given (hex), already framed or not")
    raw.add_argument("bytes", nargs="+")
    raw.add_argument("--frame", action="store_true", help="wrap them as a KISS data frame")

    sub.add_parser("escapes", help="send a frame full of FEND/FESC/TFEND/TFESC")

    burst = sub.add_parser("burst", help="send many frames as fast as possible")
    burst.add_argument("--count", type=int, default=20)
    burst.add_argument("--size", type=int, default=100)
    burst.add_argument("--source", default="N0CALL")

    params = sub.add_parser("params", help="send KISS parameter commands")
    params.add_argument("--txdelay", type=int)
    params.add_argument("--persist", type=int)
    params.add_argument("--slottime", type=int)
    params.add_argument("--txtail", type=int)
    params.add_argument("--fullduplex", type=int)

    args = parser.parse_args()
    try:
        sock = socket.create_connection((args.host, args.port), timeout=5)
    except OSError as error:
        sys.exit(f"cannot connect to {args.host}:{args.port}: {error}")
    print(f"connected to {args.host}:{args.port}")
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    if args.action == "listen":
        with contextlib.suppress(KeyboardInterrupt):
            listen(sock, 0, args.hex)
        return

    frames: list[bytes] = []
    if args.action == "ax25":
        payload = ax25_ui(args.source, args.destination, args.info.encode(), args.via)
        frames.append(kiss_encode(payload, DATA, args.kiss_port))
        print(f"AX.25 UI {args.source}>{args.destination}: {len(payload)} bytes")
        if args.hex:
            print(hexdump(payload))
    elif args.action == "raw":
        data = bytes.fromhex("".join(args.bytes))
        frames.append(kiss_encode(data, DATA, args.kiss_port) if args.frame else data)
    elif args.action == "escapes":
        payload = ax25_ui("N0CALL", "TEST", bytes([FEND, FESC, TFEND, TFESC, FESC, FEND]) * 4)
        frames.append(kiss_encode(payload, DATA, args.kiss_port))
        print(f"a frame of {len(payload)} bytes holding every special byte")
    elif args.action == "burst":
        for n in range(args.count):
            info = f"burst {n:04d} ".encode().ljust(args.size, b".")
            frames.append(kiss_encode(ax25_ui(args.source, "TEST", info), DATA, args.kiss_port))
        print(f"{args.count} frames of about {args.size + 16} bytes")
    elif args.action == "params":
        for command, value in (
            (TX_DELAY, args.txdelay),
            (PERSISTENCE, args.persist),
            (SLOT_TIME, args.slottime),
            (TX_TAIL, args.txtail),
            (FULL_DUPLEX, args.fullduplex),
        ):
            if value is not None:
                frames.append(kiss_encode(bytes([value & 0xFF]), command, args.kiss_port))

    for frame in frames:
        send(sock, frame, args.fragment, args.pause)
    print(f"sent {len(frames)} frame(s), {sum(len(f) for f in frames)} bytes on the wire")
    if args.listen > 0:
        received = listen(sock, args.listen, args.hex)
        print(f"{received} frame(s) received")
    sock.close()


if __name__ == "__main__":
    main()
