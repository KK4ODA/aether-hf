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

from aether_model.link.frames import (
    ConnectBody,
    ControlFlags,
    ControlFrame,
    ControlKind,
    DataHeader,
    DataKind,
    ProbeBody,
    encode_data,
    pack_callsign,
)
from aether_model.link.rate import (
    AWGN_THRESHOLD_DB,
    NARROW_AWGN_THRESHOLD_DB,
    NARROW_FRAME_S,
    NARROW_PAYLOAD_BYTES,
    RateController,
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


def connect_body_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for src, dst, caps, version, snr in (
        ("W4ODA", "KK4XYZ", 0, 1, None),
        ("M0ABC", "VK2DEF/P", 0b101, 1, 12.5),
        ("A1", "9Z9ZZZ9ZZ", 255, 7, -7.4),
        ("KK4ODA", "N0CALL-T", 0b010, 1, 200.0),
    ):
        body = ConnectBody(src, dst, caps=caps, version=version, snr_db=snr)
        out.append(
            {
                "src": src,
                "dst": dst,
                "caps": caps,
                "version": version,
                "snr_db": snr,
                "decoded_snr_db": ConnectBody.decode(body.encode()).snr_db,
                "encoded": body.encode().hex(),
            }
        )
    return out


def probe_body_cases() -> list[dict]:  # type: ignore[type-arg]
    # the SNR byte follows the control frame's convention: whole dB, ties to even,
    # clamped to +/-40, 0x7F for "not measured" (what a PROBE always says)
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


def control_frame_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    cases = [
        (ControlKind.ACK, ControlFlags.NONE, 0, 0, None, 0, 0),
        (ControlKind.ACK, ControlFlags.WANT_TX, 100, 0b1011, -7.0, 6, 5),
        (ControlKind.ACK, ControlFlags.BREAK | ControlFlags.WANT_TX, 255, 0xFFFF, 40.0, 13, 15),
        (ControlKind.POLL, ControlFlags.NONE, 7, 0, 12.5, 3, 1),
        (ControlKind.TURN, ControlFlags.NONE, 0, 0, None, 0, 0),
        (ControlKind.DISC, ControlFlags.NONE, 0, 0, -40.0, 0, 0),
        (ControlKind.DISC_ACK, ControlFlags.NONE, 0, 0, -100.0, 0, 0),
    ]
    for kind, flags, base, bitmap, snr, mode, counter in cases:
        frame = ControlFrame(
            kind=kind,
            session=42,
            flags=flags,
            base=base,
            bitmap=bitmap,
            snr_db=snr,
            recommended_mode=mode,
            counter=counter,
        )
        encoded = frame.encode()
        decoded = ControlFrame.decode(encoded)
        out.append(
            {
                "kind": kind.name,
                "session": 42,
                "flags": int(flags),
                "base": base,
                "bitmap": bitmap,
                "snr_db": snr,
                "recommended_mode": mode,
                "counter": counter,
                "encoded": encoded.hex(),
                "decoded_snr_db": decoded.snr_db,
                "received": [decoded.received(s) for s in range(0, 256, 17)],
            }
        )
    return out


def rate_trace_cases() -> list[dict]:  # type: ignore[type-arg]
    """The same observations fed to both implementations; the recommendations must match."""
    out = []
    scenarios = {
        "clean_climb": [(snr, 6, 0, None) for snr in [14.0] * 20],
        "low_snr": [(snr, 6, 0, None) for snr in [-2.0] * 15],
        "collapse": [(18.0, 6, 0, None)] * 12 + [(4.0, 2, 4, 10)] * 12,
        "boundary": [(9.0, 6, 0, None)] * 3 + [(9.0, 3, 3, 8)] * 3 + [(9.0, 6, 0, 6)] * 12,
        "targeted_widen": [(12.0, 0, 4, 4)] * 3 + [(12.0, 6, 0, 4)] * 9,
    }
    for name, observations in scenarios.items():
        rc = RateController()
        track = []
        for snr, ok, failed, mode in observations:
            rc.observe(snr, ok, failed, mode)
            track.append(
                {
                    "recommend": int(rc.recommend()),
                    "margin_db": round(float(rc.margin_db), 9),
                    "snr_db": None if rc.snr_db is None else round(float(rc.snr_db), 9),
                }
            )
        out.append(
            {
                "name": name,
                "observations": [
                    {"snr_db": snr, "ok": ok, "failed": failed, "mode": mode}
                    for snr, ok, failed, mode in observations
                ],
                "track": track,
            }
        )
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-link/tests/data/link_vectors.json")
    args = ap.parse_args()

    document = {
        "note": "Generated by tools/make_link_vectors.py from the Python reference model. "
        "Frame encodings are exact; the rate-controller traces are compared value by value.",
        "awgn_thresholds": [float(AWGN_THRESHOLD_DB[m]) for m in sorted(AWGN_THRESHOLD_DB)],
        "usable_modes": [int(m) for m in usable_modes()],
        "narrow_awgn_thresholds": [
            float(NARROW_AWGN_THRESHOLD_DB[m]) for m in sorted(NARROW_AWGN_THRESHOLD_DB)
        ],
        "narrow_payload_bytes": [
            int(NARROW_PAYLOAD_BYTES[m]) for m in sorted(NARROW_PAYLOAD_BYTES)
        ],
        "narrow_frame_s": [float(NARROW_FRAME_S[m]) for m in sorted(NARROW_FRAME_S)],
        "narrow_usable_modes": [
            int(m)
            for m in usable_modes(NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S)
        ],
        "callsigns": callsign_cases(),
        "data_frames": data_frame_cases(),
        "connect_bodies": connect_body_cases(),
        "probe_bodies": probe_body_cases(),
        "control_frames": control_frame_cases(),
        "rate_traces": rate_trace_cases(),
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    counts = {k: len(v) for k, v in document.items() if isinstance(v, list)}
    print(f"wrote {out} ({counts})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
