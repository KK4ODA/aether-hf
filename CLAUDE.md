# Aether HF — working notes for Claude Code

## What this is
Open-source HF ARQ data modem (VARA-HF-class) for amateur radio. Read in this order:
`docs/AUDIT.md` (what was wrong at the start), `docs/ROADMAP.md` (the plan and the phase task
IDs), `docs/COMMUNITY-CONCERNS.md` (what users will judge us on), `docs/adr/` (decisions).

## Layout
- `model/aether_model/` — Python reference model: `channel.py` (calibrated HF simulator),
  `waveform.py` (ADR-0002 numerology), `fec/` (TS 38.212 LDPC, CRC), `phy/` (constellations,
  OFDM, preamble, sync, receiver, passband, streaming, pipeline), `frame/` (modes, codec),
  `hal/` (audio backends). `protocol/` and `host/` plus `constants.py` are **legacy stubs**
  replaced in Phases 2–3; do not extend them.
- `model/tests/` — pytest, all strict. A known defect gets an `xfail(strict=True)` naming
  the finding (marker `audit`) and loses the marker in the PR that fixes it. **Never** loosen
  an assertion to make a test pass — add an ADR if a target genuinely changes.
  `test_vectors.py` pins the transmitter bit-exactly to `vectors/`.
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
Phase 1 is complete in the model; Phase 2 is in progress on branch `phase-2`. Done:
P2-3 low-SNR acquisition (PMF-FFT bank in `phy/sync.py`, frame type by preamble sequence,
mode + RV by pilot-symbol chips; 100 % at −5 dB); P2-1 ARQ engine + session FSM
(`aether_model/link/`, PHY-agnostic, event-driven) with two harnesses — a lossy-pipe sim
(`link/sim.py`) and a two-modem harness over the real PHY (`link/harness.py`); P2-2
rate-control hysteresis and the `tools/bench_link.py` goodput benchmark. Next: P2-2a
(start-of-frame signal to the link layer, ≈ 25 % throughput), P2-4 PAPR study.

Link layer (`aether_model/link/`): `frames.py` (DATA/CONTROL/connect formats, callsign
packing), `engine.py` (`LinkEngine`, event-driven: `connect/send/disconnect/tick/on_frame`
→ `Action`s), `rate.py` (rate recommendation), `sim.py` (`TwoStationSim` discrete-event
driver), `harness.py` (real-PHY bridge). The DATA header is static across a frame's
retransmissions so the receiver can soft-combine identical codewords; the RV rides in the
chips, not the header.
Benchmarks: `tools/bench_phy.py` (≈ 40 min full grid) and `tools/bench_ldpc.py`; golden
vectors: `tools/make_vectors.py` (regenerate only with an ADR).
