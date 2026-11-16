"""The data identifier's frame (ADR-0127): its bytes, and a listener with the specification.

C3 of ADR-0127: a station that has only ``docs/spec/air-interface.md`` can read the callsign. The
listener test below takes the frame's layout from the specification's own table — which bits of
which byte hold the kind, the session field, ``more`` and the characters — and the character
width from its prose, and reads the identification frames an engine sent with nothing else of
the model but its control-frame decoder.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

from aether_model.frame.modes import NARROW, WIDE
from aether_model.link.frames import (
    AcceptFrame,
    GrantFrame,
    GrantKind,
    IdentFrame,
    ident_callsign,
    identification_frames,
    is_acceptance,
    is_identification,
)
from aether_model.link.granted import GrantConfig, GrantedEngine, SlotTransmit
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container
from aether_model.link.sim import TwoStationSim

SPEC = Path(__file__).resolve().parents[2] / "docs" / "spec" / "air-interface.md"


@pytest.mark.parametrize(
    ("call", "sent"),
    [
        ("KK4ODA-1", ["KK4ODA"]),
        ("w4oda", ["W4ODA"]),
        ("N0CALL-T", ["N0CALL"]),
        ("W4/G4ABC", ["W4/G4A", "BC"]),
        ("G4ABC/W4", ["G4ABC/", "W4"]),
        ("VE3/KK4ODA/P", ["VE3/KK", "4ODA/P"]),
        ("ABCDEFGHIJKLMNOP", ["ABCDEF", "GHIJKL"]),
    ],
)
def test_a_callsign_goes_as_the_morse_identifier_sends_it(call: str, sent: list[str]) -> None:
    frames = identification_frames(call)
    assert [f.text for f in frames] == sent
    assert [f.more for f in frames] == [True] * (len(sent) - 1) + [False]
    for frame in frames:
        payload = frame.encode()
        assert is_identification(payload) and not is_acceptance(payload)
        assert IdentFrame.decode(payload) == frame
        ctl = GrantFrame.decode(payload)
        assert ctl.kind is GrantKind.DISC and ctl.session == 0


def test_no_other_control_frame_is_an_identification() -> None:
    for kind in GrantKind:
        for session in (1, 64, 127):
            assert not is_identification(GrantFrame(kind, session).encode())
    assert not is_identification(AcceptFrame(5, 9).encode())
    with pytest.raises(ValueError):
        IdentFrame.decode(bytes([0x80, 0, 0, 0, 0, 0, 0]))  # no characters
    with pytest.raises(ValueError):
        IdentFrame("TOOLONG").encode()
    assert ident_callsign(" kk4oda-15/p ") == "KK4ODA/P"


# ── a listener with the specification alone ──────────────────────────


def _spec_layout() -> tuple[dict[str, list[tuple[int, int, int]]], int, int]:
    """The identification frame's table in the specification: each field's (byte, high bit,
    low bit) pieces in the order the table lists them, and the characters' count and width."""
    text = SPEC.read_text(encoding="utf-8")
    start = text.index("An **identification frame**")
    section = text[start : text.index("A **measurement frame**")]
    words = {"six": 6, "seven": 7}
    count_word, width = re.search(r"holds (\w+) (\d+)-bit ASCII characters", section).groups()  # type: ignore[union-attr]
    fields: dict[str, list[tuple[int, int, int]]] = {}
    for row in re.findall(r"^\| ([\d–]+) \| ([\d–]+) \| ([^|]+) \|$", section, re.M):
        bytes_, bits, name = row
        first, _, last = bytes_.partition("–")
        hi, _, lo = bits.partition("–")
        key = name.split(":")[0].split(",")[0].strip()
        for byte in range(int(first), int(last or first) + 1):
            fields.setdefault(key, []).append((byte, int(hi), int(lo or hi)))
    return fields, words[count_word], int(width)


def _read(payload: bytes, pieces: list[tuple[int, int, int]]) -> int:
    value = 0
    for byte, hi, lo in pieces:
        n = hi - lo + 1
        value = (value << n) | ((payload[byte] >> lo) & ((1 << n) - 1))
    return value


def _listen(payloads: list[bytes]) -> list[str]:
    """Callsigns read from control frames with the specification's layout alone."""
    fields, count, width = _spec_layout()
    assert {"kind", "more", "session field", "characters"} <= set(fields)
    heard: list[str] = []
    part = ""
    for payload in payloads:
        if _read(payload, fields["kind"]) != 2 or _read(payload, fields["session field"]) != 0:
            part = ""
            continue
        chars = _read(payload, fields["characters"])
        text = ""
        for i in range(count):
            code = (chars >> (width * (count - 1 - i))) & ((1 << width) - 1)
            if not code:
                break
            text += chr(code)
        if _read(payload, fields["more"]):
            part += text
        else:
            heard.append(part + text)
            part = ""
    return heard


@pytest.mark.parametrize(("air", "call"), [(WIDE, "KK4ODA"), (NARROW, "W4/G4ABC")])
def test_a_listener_with_the_specification_reads_the_callsign(air: object, call: str) -> None:
    timing = phy_timing(air.params)  # type: ignore[attr-defined]

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(air.params),  # type: ignore[attr-defined]
            max_mode=air.n_rungs - 1,  # type: ignore[attr-defined]
            identifier_s=5.0,
            identify_by="data",
        )

    a = GrantedEngine(call, timing, config(), seed=3)
    b = GrantedEngine("KK4XYZ-7", timing, config(), seed=4)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=7)
    sent: list[bytes] = []
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        assert isinstance(tx, SlotTransmit)
        # every control frame either station keyed, as a listener on the channel hears them
        sent.extend(f.payload for f in tx.frames if f.container is Container.CONTROL)
        orig(who, at, tx)

    sim._launch = spy  # type: ignore[method-assign]
    a.connect("KK4XYZ-7")
    sim.run(until=40.0)
    a.disconnect()
    sim.run(until=sim.t + 120.0)
    assert sorted(_listen(sent)) == sorted([call, "KK4XYZ"])
