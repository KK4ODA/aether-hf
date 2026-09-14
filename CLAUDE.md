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
- `core/` — the shipped Rust workspace (ADR-0001). `aether-fec` is done and is **bit-exact
  with the model**: `cargo test` in `core/` runs both its own tests and the cross-validation
  against `tests/data/fec_vectors.json`. Regenerate those vectors with
  `python tools/make_fec_vectors.py` **only** when the model deliberately changed — a
  mismatch otherwise is a bug in the core, not stale vectors. The LDPC base-graph tables are
  generated from the model's JSON by `build.rs`, so there is one source of truth.
- `app/` (Tauri) arrives in Phase 4 per ADR-0001.

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
Phases 0–2 are complete. **Phase 3 is complete through P3-4**, on branch `phase-2`:

* **P3-1/P3-2** — the whole modem is ported and cross-validated. `aether-fec` (CRC, LDPC,
  rate matching), `aether-phy` (waveform, constellations, modes, frame codec, OFDM, preamble,
  transmitter, receiver, acquisition, the 48 kHz audio front end, the impulse blanker, the
  streaming receiver and ADR-0004 peak reduction) and `aether-link` (frame formats, rate
  control, the ARQ engine, a lossy-pipe two-station simulator).
* **P3-3** — `core/aetherd/`: cpal audio behind an `AudioIo` trait, serial RTS/DTR and
  `rigctld` keying, the key-time watchdog, a busy detector, and the run loop. Two stations
  complete a session over real 48 kHz audio in the test suite.
* **P3-4** — the control API of `docs/spec/control-api.md`, JSON over WebSocket and
  `POST /v1/<method>`.

Run the Rust tests with `cargo test --release --workspace` — acquisition is ~20x slower in a
debug build — and `cargo clippy --all-targets --all-features -- -D warnings`.

**Next: P3-5**, the VARA-compatible TCP adapter, verified with Pat.

Every ported layer has a `tests/model_vectors.rs` fed by a `tools/make_*_vectors.py`
generator, and CI regenerates them and fails on drift. **A vector mismatch means the core is
wrong** — regenerate only for a deliberate model change. Integer quantities are compared for
equality; floating-point ones to a tolerance carried in the vector file.

`docs/spec/air-interface.md` is public and its numeric tables are generated —
run `python tools/make_spec.py` after any waveform, mode or constant change, or
`model/tests/test_spec.py` fails.

Note for Phase 3: `LinkEngine.on_preamble` needs the real streaming receiver to report a
detected preamble before the frame is decoded; `PhyTiming.preamble_detect_s` is what turns
that signal on, and leaving it `None` is the safe fallback (costs ~13 % throughput).

Link layer (`aether_model/link/`): `frames.py` (DATA/CONTROL/connect formats, callsign
packing), `engine.py` (`LinkEngine`, event-driven: `connect/send/disconnect/tick/on_frame`
→ `Action`s), `rate.py` (rate recommendation), `sim.py` (`TwoStationSim` discrete-event
driver), `harness.py` (real-PHY bridge). The DATA header is static across a frame's
retransmissions so the receiver can soft-combine identical codewords; the RV rides in the
chips, not the header.
Benchmarks: `tools/bench_phy.py` (≈ 40 min full grid) and `tools/bench_ldpc.py`; golden
vectors: `tools/make_vectors.py` (regenerate only with an ADR).
