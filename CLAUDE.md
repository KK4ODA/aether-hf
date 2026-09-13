# Aether HF — working notes for Claude Code

## What this is
Open-source HF ARQ data modem (VARA-HF-class) for amateur radio. Read in this order:
`docs/AUDIT.md` (what was wrong at the start), `docs/ROADMAP.md` (the plan and the phase task
IDs), `docs/COMMUNITY-CONCERNS.md` (what users will judge us on), `docs/adr/` (decisions).

## Layout
- `model/aether_model/` — Python reference model. New, trusted code: `channel.py` (calibrated
  HF simulator), `waveform.py` (ADR-0002 numerology). Everything under `dsp/`, `fec/`,
  `protocol/`, `host/`, `audio/` plus `constants.py`/`speed_levels.py` is **legacy prototype
  code scheduled for rewrite**; do not extend it, replace it per the roadmap task.
- `model/tests/` — pytest. `test_channel.py`/`test_waveform.py` must always pass.
  `test_legacy_*.py` hold strict `xfail` tests (marker `audit`) that document each audit
  defect; remove the marker in the same PR that fixes the defect. **Never** loosen an
  assertion to make a test pass — add an ADR if a target genuinely changes.
- `tools/audit_probe_*.py` — frozen evidence scripts from the audit; excluded from lint.
- `core/` (Rust workspace) and `app/` (Tauri) arrive in Phases 3–4 per ADR-0001.

## Commands
```
python -m uv sync                 # or `uv sync` if uv is on PATH
python -m uv run pytest           # ~15 s; `-m "not slow"` for the quick set
python -m uv run ruff check . && python -m uv run ruff format --check . && python -m uv run mypy
python -m uv run pre-commit install
```
CI (`.github/workflows/ci.yml`) runs exactly those on Windows + Ubuntu, Python 3.12 and 3.13.

## Conventions
- SNR is referenced to a **3 kHz** noise bandwidth; Doppler spread is the ITU-R F.1487
  **2σ** value; fading has unit mean power and is never normalised per block.
- Every impairment in the simulator is a streaming process: block-splitting must be
  bit-exact (`complex_normal()` interleaves real/imag draws for that reason).
- Waveform numbers come from `WaveformParams`, never typed into prose or tests by hand.
- Performance claims only with a committed benchmark curve behind them.
- Design from public standards (3GPP, IEEE, ITU-R, MIL-STD) and open literature; never
  from VARA internals. Host-interface compatibility uses VARA's *published* command set.
- Conventional Commits; short-lived branches; squash-merge to `main` (currently `master`).
- Code style: ruff (line length 100), mypy strict for new modules, docstrings explain *why*.

## Current phase
Phase 0 (audit & stabilization). Next: Phase 1 tasks P1-1 … P1-9 in `docs/ROADMAP.md`
§13; the ordered queue is §14. Start with P1-2 (Gray labelling) and P1-1 (TS 38.212 BG2).
