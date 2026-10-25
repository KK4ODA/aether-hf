"""The release tooling that tells an operator a new version will not talk to the old one."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import channel_order  # noqa: E402
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


def test_the_beta_channel_never_moves_back(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Two tags pushed together build side by side; the older one's run, finishing last, must
    leave the channel's manifest on the newer beta."""
    assert channel_order.order("0.2.0-beta.124") > channel_order.order("0.2.0-beta.123")
    assert channel_order.order("0.2.0-beta.10") > channel_order.order("0.2.0-beta.9")
    assert channel_order.order("0.2.0") > channel_order.order("0.2.0-rc.1")
    assert channel_order.order("0.2.0-rc.1") > channel_order.order("0.2.0-beta.124")
    assert channel_order.order("v0.3.0-beta.1") > channel_order.order("0.2.0")

    def run(ours: str, there: str | None) -> int:
        (tmp_path / "ours.json").write_text(f'{{"version": "{ours}"}}', encoding="utf-8")
        now = tmp_path / "now.json"
        if there is None:
            now.unlink(missing_ok=True)
        else:
            now.write_text(f'{{"version": "{there}"}}', encoding="utf-8")
        argv = ["channel_order.py", str(tmp_path / "ours.json"), str(now)]
        monkeypatch.setattr(sys, "argv", argv)
        return channel_order.main()

    assert run("0.2.0-beta.123", "0.2.0-beta.124") == 1
    assert run("0.2.0-beta.124", "0.2.0-beta.123") == 0
    assert run("0.2.0-beta.124", "0.2.0-beta.124") == 0  # a re-run repairs the channel
    assert run("0.2.0-beta.124", None) == 0  # a new channel
    assert run("0.2.0-beta.124", "not a version") == 2
