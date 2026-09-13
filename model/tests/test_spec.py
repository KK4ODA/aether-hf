"""The public specification must not drift from the implementation (roadmap P2-7).

``docs/spec/air-interface.md`` is the document a third party implements against, and under
FCC §97.309(a)(4) it is what makes the mode legal to transmit. Every number in it that the
code also knows is generated; this fails if the file has not been regenerated after a change.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SPEC = ROOT / "docs" / "spec" / "air-interface.md"
TOOL = ROOT / "tools" / "make_spec.py"


def test_spec_exists() -> None:
    assert SPEC.exists(), "the air-interface specification is missing"


@pytest.mark.skipif(not TOOL.exists(), reason="tools/make_spec.py not present")
def test_generated_tables_match_the_model() -> None:
    result = subprocess.run(
        [sys.executable, str(TOOL), "--check"],
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    assert result.returncode == 0, (
        f"{SPEC.name} is out of date with the model.\n"
        f"Run: python tools/make_spec.py\n{result.stdout}{result.stderr}"
    )


def test_spec_states_the_snr_convention() -> None:
    """Every SNR in the project is referenced to 3 kHz; a public spec that omits that makes
    its own numbers unreproducible."""
    text = SPEC.read_text(encoding="utf-8")
    assert "3 kHz" in text
    assert "F.1487" in text  # the Doppler convention


def test_spec_has_no_unfilled_markers() -> None:
    text = SPEC.read_text(encoding="utf-8")
    for name in ("waveform", "layouts", "modes", "constants"):
        start = text.index(f"<!-- BEGIN:{name} -->")
        end = text.index(f"<!-- END:{name} -->")
        assert end - start > 100, f"generated block {name} is empty"
