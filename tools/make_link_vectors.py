"""Generate link-layer cross-validation vectors for the Rust core (roadmap P3-2).

    python tools/make_link_vectors.py [--out core/aether-link/tests/data/link_vectors.json]

Frame formats are wire formats: a byte in the wrong place is a station that cannot be talked
to. Every field of every container is exercised here and compared for equality on the Rust
side — there is nothing approximate about a frame layout.

The rate controller is included too, as a *trace*: the same sequence of observations is fed to
both implementations and the recommendations must match burst for burst. It is a state
machine over floating-point comparisons, so an off-by-one in the hysteresis would show up
here and nowhere else.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

import random

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.datagram import body as datagram_body
from aether_model.link.datagram import fragments, max_payload
from aether_model.link.frames import (
    DATA_HEADER,
    MISSED_UNKNOWN,
    PROTOCOL_VERSION,
    REPORT,
    ConnectBody,
    DataFlags,
    DataHeader,
    DataKind,
    GrantFrame,
    GrantKind,
    ProbeBody,
    data_capacity,
    encode_data,
    level_of,
    pack_callsign,
    pieces,
    with_level,
)
from aether_model.link.granted import GrantConfig, GrantedEngine
from aether_model.link.harness import phy_timing
from aether_model.link.rate import (
    AWGN_THRESHOLD_DB,
    CONTROL_THRESHOLD_DB,
    ECHO_LIMIT,
    FRAME_S,
    NARROW_AWGN_THRESHOLD_DB,
    NARROW_CONTROL_THRESHOLD_DB,
    NARROW_ECHO_LIMIT,
    NARROW_FRAME_S,
    NARROW_PAYLOAD_BYTES,
    PAYLOAD_BYTES,
    FrameOutcome,
    usable_modes,
)


def callsign_cases() -> list[dict]:  # type: ignore[type-arg]
    return [
        {"call": call, "packed": pack_callsign(call).hex()}
        for call in ("W4ODA", "KK4XYZ", "M0ABC", "VK2DEF/P", "A1", "9Z9ZZZ9ZZ", "K1A")
    ]


def data_frame_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for capacity in (26, 144, 732):
        for body_len in (0, 1, 17, capacity - 5, capacity - 3):
            if body_len < 0:
                continue
            body = bytes((i * 37) % 256 for i in range(body_len))
            for kind in DataKind:
                header = DataHeader(kind=kind, seq=(body_len * 7) % 256, session=capacity % 256)
                out.append(
                    {
                        "kind": kind.name,
                        "seq": header.seq,
                        "session": header.session,
                        "capacity": capacity,
                        "body": body.hex(),
                        "encoded": encode_data(header, body, capacity).hex(),
                    }
                )
    return out


def piece_cases() -> list[dict]:  # type: ignore[type-arg]
    """A stuck frame's body in pieces (ADR-0086): where it is cut, the one-byte-short last piece
    that goes as two, and each piece's frame."""
    out = []
    for capacity in (24, 51, 137):
        room = capacity - DATA_HEADER - 2
        for body_len in (1, room - 2, room - 1, room, room + 1, 2 * room - 1, 72, 288):
            body = bytes((i * 53 + 11) % 256 for i in range(body_len))
            parts = pieces(body, capacity)
            frames = []
            for offset, piece, last in parts:
                header = DataHeader(
                    DataKind.DATA, 77, 200, DataFlags.FRAGMENT, offset=offset, last=last
                )
                frames.append(
                    {
                        "offset": offset,
                        "last": last,
                        "piece": piece.hex(),
                        "encoded": encode_data(header, piece, capacity).hex(),
                    }
                )
            out.append(
                {
                    "capacity": capacity,
                    "body": body.hex(),
                    "seq": 77,
                    "session": 200,
                    "pieces": frames,
                }
            )
    return out


def datagram_cases() -> list[dict]:  # type: ignore[type-arg]
    """A KISS client's frame as DATAGRAM fragments (ADR-0019): the split, the headers and
    the one-byte-short remainder that goes as two fragments."""
    out = []
    for capacity in (24, 26, 144):
        full = capacity - DATA_HEADER
        for size in (1, full - 2, full - 1, full, full + 1, 100, max_payload(capacity)):
            payload = bytes((i * 131 + 7) % 256 for i in range(size))
            number = (size * 13) % 255 + 1
            out.append(
                {
                    "capacity": capacity,
                    "number": number,
                    "payload": payload.hex(),
                    "fragments": [f.hex() for f in fragments(payload, number, capacity)],
                }
            )
    return out


def datagram_body_cases() -> list[dict]:  # type: ignore[type-arg]
    """What a datagram carries: the sender's callsign, the KISS frame type and the frame."""
    out = []
    for source, frame_type, size in (("KK4ODA-1", 0, 20), ("W1AW", 1, 33), ("N0CALL/P", 2, 1)):
        frame = bytes((i * 29 + 3) % 256 for i in range(size))
        out.append(
            {
                "source": source,
                "frame_type": frame_type,
                "frame": frame.hex(),
                "body": datagram_body(source, frame_type, frame).hex(),
            }
        )
    return out


def connect_body_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for src, dst, caps, version, snr, turnaround, identifier in (
        ("W4ODA", "KK4XYZ", 0, 1, None, 0.0, 0.0),
        ("M0ABC", "VK2DEF/P", 0b101, 1, 12.5, 0.0, 0.0),
        ("A1", "9Z9ZZZ9ZZ", 255, 7, -7.4, 0.0, 0.0),
        ("KK4ODA", "N0CALL-T", 0b010, 1, 200.0, 0.0, 0.0),
        # protocol 8 (ADR-0069): the turnaround in 0.1 s steps up to 1.5 s, the identifier in
        # 0.5 s steps, both rounded up; the level bits in a request
        ("KK4ODA", "W1AW", with_level(0b010, 2), PROTOCOL_VERSION, None, 0.25, 4.3),
        ("W4ODA", "KK4XYZ", with_level(0b001, 3), PROTOCOL_VERSION, -3.0, 0.2, 0.0),
        ("A1", "9Z9ZZZ9ZZ", 0, PROTOCOL_VERSION, 12.5, 2.0, 200.0),
        ("M0ABC", "VK2DEF/P", 0b010, PROTOCOL_VERSION, 0.0, 0.0, 0.5),
    ):
        body = ConnectBody(
            src,
            dst,
            caps=caps,
            version=version,
            snr_db=snr,
            turnaround_s=turnaround,
            identifier_s=identifier,
        )
        decoded = ConnectBody.decode(body.encode())
        out.append(
            {
                "src": src,
                "dst": dst,
                "caps": caps,
                "version": version,
                "snr_db": snr,
                "turnaround_s": turnaround,
                "identifier_s": identifier,
                "decoded_snr_db": decoded.snr_db,
                "decoded_turnaround_s": decoded.turnaround_s,
                "decoded_identifier_s": decoded.identifier_s,
                "level": level_of(caps),
                "encoded": body.encode().hex(),
            }
        )
    return out


def grant_frame_cases() -> list[dict]:  # type: ignore[type-arg]
    """Protocol 8's control frame (ADR-0069): every field at its edges, the session's high bit
    dropped, the SNR's ties to even and its six-bit clamp; protocol 9's measurement bit, which
    is the identifier's in a DISC and a DISC_ACK, and a report (ADR-0077); protocol 12's slot
    family and count of frames never detected, clamped to three bits (ADR-0099)."""
    out = []
    cases = [
        GrantFrame(GrantKind.HEADER, 42),
        GrantFrame(
            GrantKind.HEADER,
            135,
            count=6,
            data_floor=True,
            level=3,
            fam=True,
            flag=True,
            measure=True,
            base=250,
            bitmap=0xFFFE,
            snr_db=31.6,
            slot_floor=True,
            missed=6,
            n=7,
        ),
        GrantFrame(GrantKind.ANSWER, 1, count=2, level=1, base=3, bitmap=0b1011, snr_db=12.5),
        GrantFrame(GrantKind.ANSWER, 127, flag=True, snr_db=-0.5, missed=MISSED_UNKNOWN, n=5),
        GrantFrame(GrantKind.DISC, 64, fam=True, level=2, snr_db=-40.0),
        GrantFrame(GrantKind.DISC_ACK, 255, ident=True, snr_db=-31.4, base=17),
        GrantFrame(GrantKind.HEADER, 9, count=9, n=12, snr_db=13.5, missed=9),
        GrantFrame(GrantKind.HEADER, 3, ident=True, measure=False, base=7, bitmap=0b0110),
        GrantFrame(GrantKind.DISC, 5, measure=True),
        GrantFrame(GrantKind.ANSWER, 11, measure=True, base=250, bitmap=REPORT | 0b10110),
        GrantFrame(GrantKind.HEADER, 12, base=4, bitmap=0xFFFF),
    ]
    for frame in cases:
        encoded = frame.encode()
        decoded = GrantFrame.decode(encoded)
        out.append(
            {
                "kind": frame.kind.name,
                "session": frame.session,
                "count": frame.count,
                "data_floor": frame.data_floor,
                "level": frame.level,
                "fam": frame.fam,
                "flag": frame.flag,
                "ident": frame.ident,
                "measure": frame.measure,
                "base": frame.base,
                "bitmap": frame.bitmap,
                "snr_db": frame.snr_db,
                "slot_floor": frame.slot_floor,
                "missed": frame.missed,
                "n": frame.n,
                "encoded": encoded.hex(),
                "decoded": {
                    "kind": decoded.kind.name,
                    "session": decoded.session,
                    "count": decoded.count,
                    "data_floor": decoded.data_floor,
                    "level": decoded.level,
                    "fam": decoded.fam,
                    "flag": decoded.flag,
                    "ident": decoded.ident,
                    "measure": decoded.measure,
                    "base": decoded.base,
                    "bitmap": decoded.bitmap,
                    "snr_db": decoded.snr_db,
                    "slot_floor": decoded.slot_floor,
                    "missed": decoded.missed,
                    "n": decoded.n,
                },
                "received": [decoded.received(q) for q in range(0, 256, 13)]
                + [decoded.received((decoded.base + d) % 256) for d in range(17)],
                "reports": decoded.reports,
                "measured": [decoded.measured(i) for i in range(-1, 17)],
            }
        )
    return out


def probe_body_cases() -> list[dict]:  # type: ignore[type-arg]
    # the SNR byte is the connect body's: whole dB, ties to even, clamped to +/-40, 0x7F for
    # "not measured" (what a PROBE always says)
    out = []
    for src, dst, snr, caps in (
        ("W4ODA", "KK4XYZ", None, 0),
        ("KK4XYZ", "W4ODA", 12.5, 0b010),
        ("M0ABC", "VK2DEF/P", -7.4, 0b001),
        ("A1", "9Z9ZZZ9ZZ", 200.0, 0b011),
        ("N0CALL", "W1AW", -200.0, 0b100),
        ("N0CALL", "W1AW", 0.0, 255),
    ):
        body = ProbeBody(src, dst, snr, caps=caps)
        decoded = ProbeBody.decode(body.encode())
        out.append(
            {
                "src": src,
                "dst": dst,
                "snr_db": snr,
                "caps": caps,
                "encoded": body.encode().hex(),
                "decoded_snr_db": decoded.snr_db,
            }
        )
    return out


def controller_trace(air: AirInterface, seed: int, steps: int = 400) -> dict[str, object]:
    """The rate controller alone (ADR-0094, ADR-0099), input by input: a seed, then frames at the recommended rung and
    others (decoded, failed, never detected, combined), echo readings and bare SNRs drawn at
    random, and what it recommends and holds after each — every rule of the controller, where a
    session's next transmission would hide one (an echo reading's choice is taken again at the
    next transmission's)."""
    timing = phy_timing(air.params)
    ctl = GrantedEngine("TOOL", timing, GrantConfig()).rate
    modes = sorted(ctl.thresholds)
    rng = random.Random(seed)
    snr = rng.uniform(-10.0, 20.0)
    ops: list[dict[str, object]] = []

    def state() -> dict[str, object]:
        common = {"recommend": ctl.recommend(), "snr": ctl.snr_db, "echo": ctl.echo}
        return {
            **common,
            "estimates": [ctl.estimate(m) for m in ctl.modes],
            "values": [ctl.value(m) for m in ctl.modes],
        }

    ctl.seed(snr)
    ops.append({"seed": snr, **state()})
    for _ in range(steps):
        snr = max(-25.0, min(30.0, snr + rng.gauss(0.0, 2.0)))
        roll = rng.random()
        if roll < 0.6:
            frames = []
            for _ in range(rng.randint(1, 6)):
                mode = ctl.recommend() if rng.random() < 0.75 else rng.choice(modes)
                detected = rng.random() < 0.85
                decoded = detected and rng.random() < 0.6
                frames.append(
                    FrameOutcome(
                        mode,
                        timing.data_frame_s_for(mode),
                        data_capacity(timing.capacity(mode)),
                        detected,
                        decoded,
                        detected and rng.random() < 0.15,
                        snr if detected else None,
                    )
                )
            reading = snr if rng.random() < 0.9 else None
            ctl.observe_frames(frames, reading)
            ops.append(
                {
                    "frames": [
                        [f.mode, f.air_s, f.capacity, f.detected, f.decoded, f.combined]
                        for f in frames
                    ],
                    "snr_in": reading,
                    **state(),
                }
            )
        elif roll < 0.85:
            share = rng.choice([None, rng.uniform(0.0, 0.012)])
            ctl.observe_echo(share, snr)
            ops.append({"echo_in": share, "snr_in": snr, **state()})
        elif roll < 0.9:
            # the control frames' families the engine says are in use (C1's exchange price)
            families = (rng.random() < 0.5, rng.random() < 0.5)
            ctl.families = families
            ops.append({"families": list(families), **state()})
        else:
            reading = snr if rng.random() < 0.8 else None
            ctl.observe_snr(reading)
            ops.append({"observe_snr": reading, **state()})
    return {
        "air": air.params.bandwidth.value,
        "thresholds": [ctl.thresholds[m] for m in modes],
        "payload": [timing.capacity(m) for m in modes],
        "capacity": [data_capacity(timing.capacity(m)) for m in modes],
        "frame_s": [timing.data_frame_s_for(m) for m in modes],
        "floor_modes": timing.floor_modes,
        "echo_limit": [(timing.echo_limit or {}).get(m) for m in modes],
        # C1 ranks per second of a whole exchange (control frames [floor, ordinary], the gaps,
        # a full transmission's frames by rung); its families stay the data's in a trace
        "control_s": [ctl.control_s[True], ctl.control_s[False]] if ctl.control_s else [],
        "gap_s": ctl.gap_s,
        "burst_frames": [ctl.burst_frames.get(m, 0) for m in modes],
        "modes": list(ctl.modes),
        "ops": ops,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-link/tests/data/link_vectors.json")
    args = ap.parse_args()

    document = {
        "note": "Generated by tools/make_link_vectors.py from the Python reference model. "
        "Frame encodings are exact; the rate-controller traces are compared value by value.",
        "awgn_thresholds": [float(AWGN_THRESHOLD_DB[m]) for m in sorted(AWGN_THRESHOLD_DB)],
        "usable_modes": [int(m) for m in usable_modes()],
        # the wide ladder's payloads and frame lengths (ADR-0013: the tone floor's two rungs
        # first), which usable_modes compares in bytes a second
        "payload_bytes": [int(PAYLOAD_BYTES[m]) for m in sorted(PAYLOAD_BYTES)],
        "frame_s": [float(FRAME_S[m]) for m in sorted(FRAME_S)],
        "narrow_awgn_thresholds": [
            float(NARROW_AWGN_THRESHOLD_DB[m]) for m in sorted(NARROW_AWGN_THRESHOLD_DB)
        ],
        "narrow_payload_bytes": [
            int(NARROW_PAYLOAD_BYTES[m]) for m in sorted(NARROW_PAYLOAD_BYTES)
        ],
        "narrow_frame_s": [float(NARROW_FRAME_S[m]) for m in sorted(NARROW_FRAME_S)],
        # the lossy pipe's control frames, indexed by family (ordinary, floor)
        "control_thresholds": [float(CONTROL_THRESHOLD_DB[f]) for f in (False, True)],
        "narrow_control_thresholds": [float(NARROW_CONTROL_THRESHOLD_DB[f]) for f in (False, True)],
        "narrow_usable_modes": [
            int(m)
            for m in usable_modes(NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S)
        ],
        "callsigns": callsign_cases(),
        "data_frames": data_frame_cases(),
        "pieces": piece_cases(),
        "connect_bodies": connect_body_cases(),
        "probe_bodies": probe_body_cases(),
        "datagrams": datagram_cases(),
        "datagram_bodies": datagram_body_cases(),
        "grant_frames": grant_frame_cases(),
        # the rate controller, input by input, on both airs (ADR-0094, ADR-0099)
        "controller_traces": [
            controller_trace(air, seed)
            for air, seed in ((WIDE, 1), (NARROW, 2), (WIDE, 3), (NARROW, 4))
        ],
        # the echo share above which each rung is kept off the ladder (ADR-0087), by mode
        "echo_limits": [ECHO_LIMIT.get(m) for m in sorted(AWGN_THRESHOLD_DB)],
        "narrow_echo_limits": [NARROW_ECHO_LIMIT.get(m) for m in sorted(NARROW_AWGN_THRESHOLD_DB)],
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    counts = {k: len(v) for k, v in document.items() if isinstance(v, list)}
    print(f"wrote {out} ({counts})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
