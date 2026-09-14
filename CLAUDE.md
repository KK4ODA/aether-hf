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
Phases 0–5 are built and merged to `master`; **Phase 6 (field validation) is in progress on
branch `phase-6`** — its tooling is built (P6-1…P6-5) and the air is what remains (P6-6).

* **P3-1/P3-2** — the whole modem is ported and cross-validated. `aether-fec` (CRC, LDPC,
  rate matching), `aether-phy` (waveform, constellations, modes, frame codec, OFDM, preamble,
  transmitter, receiver, acquisition, the 48 kHz audio front end, the impulse blanker, the
  streaming receiver and ADR-0004 peak reduction) and `aether-link` (frame formats, rate
  control, the ARQ engine, a lossy-pipe two-station simulator).
* **P3-3** — `core/aetherd/`: cpal audio behind an `AudioIo` trait, serial RTS/DTR and
  `rigctld` keying, the key-time watchdog, a busy detector, and the run loop. Two stations
  complete a session over real 48 kHz audio in the test suite.
* **P3-4** — the control API of `docs/spec/control-api.md`, JSON over WebSocket and
  `POST /v1/<method>`; `config.get`/`config.set`, `ptt.test`, `tune`, `audio.level`,
  `diagnostics`, `shutdown`.
* **P3-5/P3-7** — the VARA-compatible host interface (`core/aetherd/src/host/`, spec in
  `docs/spec/host-interfaces.md`), a *client* of the control API. Unverified in the field.
* **P3-6** — deflate above the ARQ (negotiated by the connect capability byte), Morse
  identification, beacons. **P3-8** — the gateway kit (`deploy/`, `docs/user/`).
* **Phase 4** — `app/ui/` (the panel: no build step, ADR-0005, served by the daemon),
  `app/src-tauri/` (the shell: supervises the daemon, attaches if one is running, asks it
  to stop on close), the setup wizard, structured logging (`core/aetherd/src/log.rs`) and
  the diagnostic bundle, accessibility and error-message passes.
* **Phase 5** — `tools/release.py` (one version number; `check` in CI, `bump` for a
  release), `tools/stage_daemon.py` (the daemon as a Tauri sidecar), the release pipeline
  (`.github/workflows/release.yml`: tags → installers, gateway tarballs, SBOM, checksums,
  notes; nightly on a schedule; `[dry-run]` in a commit message builds without publishing),
  configuration `schema_version` with a migration chain and fixtures under
  `core/aetherd/tests/data/config/`, the updater (`app/src-tauri/src/update.rs`: channels,
  signed manifests, kept installers for going back), and `tools/bench_gate.py`.
  **Cutting a release:** `python tools/release.py bump X.Y.Z`, commit, tag `vX.Y.Z`, push
  the tag. The updater's private key is *not* in the repository; the release is signed only
  when `TAURI_SIGNING_PRIVATE_KEY` is set as a repository secret.
* **Phase 6** — session recordings (`core/aetherd/src/record.rs`, `[record]`), replay
  (`replay.rs`, `aetherd --replay`, `field/sessions/` + `tests/field.rs`), the simulated
  channel (`sim.rs`, `[sim]`; `tests/two_daemons.rs` runs two real daemons through a
  session), `tools/compare_air.py`, `docs/user/field-test.md`, `field/LOG.md`. The
  two-daemon test found two engine bugs (`PhyTiming.tx_latency_s`; `on_tx_done` retries a
  burst) — fixed in the model first, then the port. `tx_level` is a sine amplitude; the
  waveform's RMS is `tx_level / √2`.

Run the Rust tests with `cargo test --release --workspace` — acquisition is ~20x slower in a
debug build — and `cargo clippy --all-targets --all-features -- -D warnings`; the shell is a
separate package (`cd app/src-tauri && cargo clippy --all-targets -- -D warnings`). To try
the panel: `aetherd --config <file> --dry-run` with `[control] ui_dir` pointing at `app/ui`,
then open `http://127.0.0.1:8515/`.

**Next: the air** (P6-6: audio cable → ground wave → NVIS → long paths → RMS gateway
trial, twenty logged sessions across three channel classes, recalibrate on the
disagreements) and the human items Phases 3–5 left open (Pat, Winlink Express, VarAC,
BPQ32 over the simulated channel; three external hams through the wizard; the first tagged
release once the signing secret is set). Phase 7 (FM) is on the back burner by decision.

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
