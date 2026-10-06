"""The release tooling that tells an operator a new version will not talk to the old one."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import make_update_manifest  # noqa: E402
import protocol_notice  # noqa: E402
from aether_model.link.frames import PROTOCOL_VERSION  # noqa: E402


def test_the_manifest_states_the_link_protocol_the_modem_speaks(tmp_path: Path) -> None:
    """The shell compares a manifest's ``link_protocol`` with its own and urges an update of
    another (ADR-0041); the port's constant is the model's, which a vector test holds."""
    assert make_update_manifest.link_protocol() == PROTOCOL_VERSION
    manifest = make_update_manifest.build("0.2.0-beta.99", "https://x", tmp_path, "")
    assert manifest["link_protocol"] == PROTOCOL_VERSION


def test_a_new_protocol_opens_the_notes_with_an_update_notice() -> None:
    source = "/// doc\npub const PROTOCOL_VERSION: u8 = 7;\n"
    assert protocol_notice.protocol_of(source) == 7
    text = protocol_notice.notice("0.3.0", 7, "v0.2.9", 6)
    assert text.startswith("> **Update required — link protocol 7.**")
    assert "v0.2.9 or earlier (link protocol 6)" in text
