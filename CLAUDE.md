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
- Conventional Commits; short-lived branches, **fast-forwarded** into `master` (no squash:
  the release notes are generated from the commits by git-cliff). Push after each commit.
- Code style: ruff (line length 100), mypy strict for new modules, docstrings explain *why*.

## Current phase
Phases 0–5 are done and on `master`, **releases are flowing** (`v0.2.0-beta.2` through
`beta.11` on 2026-09-14/15, signed: `TAURI_SIGNING_PRIVATE_KEY` is set; the author runs
the beta channel and updates in place), and **Phase 6 (field validation) is in progress** —
its tooling is built (P6-1…P6-5), Pat and Winlink Express pass the bench, the first
on-air attempt found two bugs (below), and the air is what remains (P6-6).

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
  **Cutting a release:** `python tools/release.py bump X.Y.Z[-beta.N]` (rewrites every
  version site including `uv.lock` — the gate job runs `uv sync --locked`, and beta.1
  failed on exactly that), commit, tag `vX.Y.Z`, push `master` and the tag, watch the
  Release workflow. Notes cover the commits since the previous tag. A `-beta.N` tag is a
  GitHub pre-release and also refreshes the rolling `channel-beta` release, whose
  `latest.json` is what a `[update] channel = "beta"` installation reads — never delete
  it. Stable uses GitHub's own *latest* release, which excludes pre-releases. The updater's
  private key is at `~/.tauri/aether-hf.key` on the author's machine, *not* in the
  repository.
* **Phase 6** — session recordings (`core/aetherd/src/record.rs`, `[record]`), replay
  (`replay.rs`, `aetherd --replay`, `field/sessions/` + `tests/field.rs`), the simulated
  channel (`sim.rs`, `[sim]`; `tests/two_daemons.rs` runs two real daemons through a
  session), `tools/compare_air.py`, `docs/user/field-test.md`, `field/LOG.md`. The
  two-daemon test found two engine bugs (`PhyTiming.tx_latency_s`; `on_tx_done` retries a
  burst) — fixed in the model first, then the port. `tx_level` is a sine amplitude; the
  waveform's RMS is `tx_level / √2`. **The bench** (`docs/spec/host-interfaces.md` §7):
  Pat 1.0.0 and Winlink Express 1.8.5.0 each complete a P2P B2F session with a 6 kB
  attachment over two daemons joined by `[sim]`. Winlink Express found that `MYCALL` had
  never reached the modem — the engine now answers to a list of callsigns
  (`set_callsigns`, `connect(…, as_call)`, model first), the control API has
  `callsigns.set` and `connect` takes `callsign`. VarAC talks to the adapter but needs a
  500 Hz waveform (P7-0, ahead of FM). Host programs are driven by hand: scratch copies
  only, never the author's real installs, and never the proprietary `VARA.exe`.

* **The first on-air attempt** (a video of the rig's scope, 2026-09-14) found two bugs the
  bench cannot: the key was released while the sound card still held the last quarter
  second of the burst (the tail now covers the playback lead; `the_key_outlasts…` test),
  and a burst held back by the busy detector left the engine's timers running, so retries
  went out in pairs (`on_tx_delayed`, model first; the station also stays deaf to its own
  tail for a lead after unkey). A second video confirmed both fixed: clean burst ends,
  single frames, the widening backoff. The wideband flash the FTDX10's scope shows after
  every transmission is the rig's receiver recovering from its own RF — it follows a plain
  tune tone too and not a silent keying — so it is not a modem artefact; do not chase it.
  Lesson: the simulated channel carries audio whether the radio is keyed or not, so
  anything about keying, latency or the busy detector needs a real rig or a paced loopback.
* **The panel's appearance** (`app/ui/style.css`) is a token system, dark by design and
  independent of the OS theme (light is an opt-in `data-theme="light"`); semantic status
  colours carry meaning only. The artwork is in `Logos/`; `tools/make_icons.py` writes the
  bundler's icons and the panel's mark, favicon and splash logo from it. The shell starts
  the daemon `CREATE_NO_WINDOW` and asks Tauri for a dark title bar. The panel's tabs are
  Status (readings, chart, counters as pills), Session (call, keying and drive, record,
  send/receive), Setup (one numbered flow: callsign, radio interface and modem devices,
  receive level, misc modem settings, application settings, save), Log and Help; the
  desktop shell's native Help *menu* keeps updates and the version restore. The author's
  wording rule: plain names ("Modem devices", "Counters"), never "the three devices the
  modem uses". Keying is a serial line (RTS/DTR/both), **CAT on the radio's own port**
  (`[ptt] kind = "cat"`: Yaesu `TX2;`/`TX0;`, Kenwood `TX;`/`RX;`, Icom CI-V `1C 00` at
  the rig's address; `CatProtocol` in `ptt.rs` is pure and tested), or `rigctld`; CAT and
  rigctld also put the dial frequency into recordings. Phase 8 (`docs/ROADMAP.md`) is
  Aether on a phone: a Pi-sized box the phone drives over Bluetooth/Wi-Fi first, then the
  app, then the modem in the phone — one application over the control API for all three.

Run the Rust tests with `cargo test --release --workspace` — acquisition is ~20x slower in a
debug build — and `cargo clippy --all-targets --all-features -- -D warnings`; the shell is a
separate package (`cd app/src-tauri && cargo clippy --all-targets -- -D warnings`). To try
the panel: `aetherd --config <file> --dry-run` with `[control] ui_dir` pointing at `app/ui`,
then open `http://127.0.0.1:8515/`.

**Next: the air** (P6-6: audio cable → ground wave → NVIS → long paths → RMS gateway
trial, twenty logged sessions across three channel classes in `field/LOG.md`, recalibrate
on the disagreements) and the human items still open (BPQ32 over the simulated channel;
three external hams through the wizard with the beta installer). Phase 7 starts with the
500 Hz waveform (P7-0); FM stays on the back burner by decision; Phase 8 is the phone;
Phase 9 is the modem's second rung — an audio-level A/B bench against the author's
registered VARA first (`tools/channel_cable.py`, to be written), then a faster start from
the connect frames' SNR, the deferred pilot/prefix/2750 Hz experiments, modes below
200 bit/s (500 Hz waveform as their home) and time diversity — each only with a curve on
Good, Moderate and Poor.

**Never run an installer or the packaged app from a Claude session on the author's
machine.** The session's view of `AppData` and `HKCU` is the desktop app's virtualised
one (files there can be stale copies), but the Desktop and Start-menu folders are real:
a silent NSIS install rewrote the author's shortcuts to a scratch directory once. Inspect
an installer by extracting it, not by running it; the real install is
`C:\Users\Facundo\AppData\Local\Aether HF\`, updated in place by the updater.

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
