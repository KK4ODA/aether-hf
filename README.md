# Aether HF

[![CI](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml/badge.svg)](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/KK4ODA/aether-hf?include_prereleases&label=release)](https://github.com/KK4ODA/aether-hf/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An open-source HF data modem for amateur radio, built to do the job VARA HF does —
reliable ARQ links for Winlink, Pat, BBS and similar software over real, noisy, fading HF
channels — with a **publicly documented air interface**, a **VARA-compatible host
interface** so existing applications work unchanged, and a **headless core** that runs on a
Raspberry Pi gateway as happily as on a Windows desktop.

> **Status: beta — verified on the bench, not yet on the air.**
> [0.2.0-beta.2](https://github.com/KK4ODA/aether-hf/releases/tag/v0.2.0-beta.2) is the
> first tagged build: a Windows installer, Linux packages and the standalone `aetherd`
> daemon for gateways. The modem (3GPP LDPC, OFDM, 14 modes from BPSK 1/5 to 64-QAM 5/6,
> selective-repeat ARQ with HARQ soft combining, rate control, compression) completes
> sessions over real 48 kHz audio in its own test suite, and **Winlink Express and Pat each
> pass a peer-to-peer session at both ends of a simulated channel**, attachments
> byte-identical on arrival. What remains is the air: on-air sessions, logged and folded back
> into the simulator, are Phase 6 and in progress. Every performance figure in this
> repository comes from a committed benchmark curve in `bench/baselines/`.

## Getting it

* **Desktop:** download the installer from
  [Releases](https://github.com/KK4ODA/aether-hf/releases) and follow
  [`docs/user/install.md`](docs/user/install.md). The setup wizard finds the sound card and
  the rig, tests keying, and sets the transmit level. The installer is not yet
  Authenticode-signed, so Windows SmartScreen will ask; `SHA256SUMS` is beside every asset.
* **Host programs:** Winlink Express (Vara HF session, TNC at `127.0.0.1:8300`, auto-launch
  off) and Pat (`varahf`) talk to the daemon's VARA-compatible port. What each client sends,
  what is verified and what is not is in
  [`docs/spec/host-interfaces.md` §7](docs/spec/host-interfaces.md). The modem runs at
  2300 Hz or, for peer-to-peer contacts, VarAC and 30 m, at **500 Hz** (`[radio]
  bandwidth`; `docs/spec/air-interface.md` §2.3) — both stations of a session use the same
  one. The VarAC bench at 500 Hz is next.
* **Gateway:** the `aetherd-…` archive and [`docs/user/gateway-kit.md`](docs/user/gateway-kit.md)
  (headless install, systemd unit, cross-compiling for ARM64). An Aether gateway must not be
  listed as a VARA gateway.
* **Updates:** the desktop application checks for a newer signed build on start (`[update]
  channel`: `stable`, `beta` or `nightly`) and keeps the previous installer for going back.

## Where things stand

A full technical audit was done on 2026-09-13 — read [`docs/AUDIT.md`](docs/AUDIT.md). In
short: the original prototype's DSP, FEC, sync, audio and protocol layers each had a
disqualifying defect, and its test suite had been relaxed until it passed. Every finding was
encoded as a strict `xfail` test and retired together with the module it documented as the
Phase 1 rewrite replaced it; the audit itself remains the record of why.

The plan is [`docs/ROADMAP.md`](docs/ROADMAP.md) (architecture, DSP decisions, protocol/API
design, testing strategy, release engineering, phased tasks) and the field requirements
distilled from how the community received Mercury, the other VARA alternative, in
[`docs/COMMUNITY-CONCERNS.md`](docs/COMMUNITY-CONCERNS.md).

| Phase | Scope | State |
|---|---|---|
| 0 — Audit & stabilization | tooling, strict tests, CI, calibrated simulator, ADRs | done |
| 1 — Core HF modem | LDPC (3GPP TS 38.212), OFDM TX/RX, sync, end-to-end loopback, benchmarks, golden vectors | done (Python model) |
| 2 — Link robustness | ARQ, rate control, HARQ-IR, low-SNR modes, PAPR, impulse noise | done (model) |
| 3 — Application integration | Rust core bit-exact with the model, `aetherd`, PTT/CAT, control API, VARA-compatible TCP, gateway kit | done; Pat and Winlink Express pass the bench, VarAC waits on a 500 Hz waveform |
| 4 — Desktop application | station panel, Tauri shell, setup wizard, diagnostics, accessibility | done; the three-ham usability test is open |
| 5 — Release infrastructure | one version number, installers that bundle the daemon, signed updates on three channels, SBOM, benchmark gate | done — `0.2.0-beta.2` |
| 6 — Field validation | recordings, replay regression tier, simulated channel, measured-vs-predicted tool, field protocol; then the air | **in progress**: tooling done, the air open (`field/LOG.md`) |
| 7 — The 500 Hz waveform and the link probe | the bandwidth P2P contacts are made in (VarAC's calling frequencies), the bandwidth in the connect handshake, an answer-only unattended mode, and a two-way SNR probe | **in progress**: the 500 Hz waveform is in the model and the daemon (`[radio] bandwidth = 500`, `BW500`, answer-only) and the probe is in (ADR-0006: Probe beside Beacon, both directions of the path without a session); the VarAC bench is what remains |
| 9 — The modem's second rung | a faster start, modes below 200 bit/s, an audio-level A/B bench against VARA HF, the deferred pilot/prefix/2750 Hz experiments, time diversity — each with its curve | next, interleaved with 7 |
| 8 — Aether on a phone | the modem in a Pi-sized box the phone talks to over Bluetooth or Wi-Fi, then a phone app, then the modem inside the phone | back burner |
| 10 — Aether FM foundation | an FM PHY on the same link layer | back burner |

## Architecture in one paragraph

Application adapters (VARA-compatible TCP now; KISS/AGW later) sit on a PHY-agnostic link
layer (session, selective-repeat ARQ with HARQ-IR, rate control, compression) and a modem
framework (frame codec, scheduler, metrics), which drive a pluggable PHY (HF OFDM first, FM
later) over a hardware abstraction (audio, PTT, rig control, or the channel simulator). The
Python model in `model/` is the specification: it designs and validates the waveform and
generates the golden vectors the Rust core in `core/` is held to, bit-exactly, in CI. The
desktop shell in `app/` supervises the daemon and serves its panel (ADR-0001, ADR-0005).
Diagrams and rationale: [`docs/ROADMAP.md` §4](docs/ROADMAP.md#4-recommended-target-architecture).

## Repository layout

```
docs/          AUDIT.md · ROADMAP.md · COMMUNITY-CONCERNS.md · adr/ (decisions) · spec/ (public
               air interface, control API, host interfaces) · user/ (install, gateway kit,
               frequency plan, field-test protocol)
model/         Python reference model: aether_model/{channel,waveform,fec,phy,frame,link,hal} + tests/
core/          the shipped Rust workspace: aether-fec · aether-phy · aether-link · aetherd
app/           ui/ (the station panel, no build step) · src-tauri/ (the desktop shell)
field/         the field log and the recorded sessions that are now regression tests
deploy/        systemd unit for a gateway
tools/         benchmarks · vector generators · release.py · compare_air.py · bench_gate.py
bench/         committed baseline curves, including the release gate's
vectors/       golden test vectors (TX bit-exact, RX must decode)
```

## Working on it

The model needs Python ≥ 3.12 and [uv](https://docs.astral.sh/uv/) (`pip install uv`
works too; then use `python -m uv`). The core needs a stable Rust toolchain; the shell
additionally needs [Tauri's prerequisites](https://tauri.app/start/prerequisites/).

```bash
uv sync                      # creates .venv with numpy/scipy + dev tools
uv run pytest                # strict suite; `-m "not slow"` for the quick set
uv run ruff check . && uv run ruff format --check . && uv run mypy
uv run pre-commit install    # optional: run the same checks on every commit

cd core && cargo test --release --workspace        # acquisition is ~20x slower in debug
cargo clippy --all-targets --all-features -- -D warnings
```

`aetherd --config <file> --dry-run` runs the daemon with no radio; two daemons joined by
`[sim]` make a bench with no radio either (`docs/user/field-test.md` §1). CI runs all of the
above on Windows and Ubuntu and regenerates every cross-validation vector, failing on drift:
a mismatch means the core is wrong, never that the vectors are stale.

Conventions that matter:

* **SNR is always referenced to a 3 kHz noise bandwidth**; Doppler spread is the ITU-R
  F.1487 2σ value. `model/aether_model/channel.py` is calibrated to both and its tests are
  the guarantee behind every benchmark number.
* Minimum usable SNR (FER ≤ 10 %, random CFO and sample-rate offset): BPSK 1/5 at −5.2 dB
  on AWGN and −0.3 dB on ITU Poor, QPSK 1/2 at +1.0 / +6.0 dB, 16-QAM 1/2 at +6.0 / +12.5 dB,
  64-QAM 5/6 at +16.9 dB on AWGN — the whole table, per channel class, is in
  [`bench/README.md`](bench/README.md).
* **No test threshold is ever relaxed to make a suite pass.** A known defect gets an
  `xfail(strict=True)` that names the finding; a target that genuinely changes gets an ADR.
* Numbers about the waveform come from `model/aether_model/waveform.py`, never from prose;
  the public [air-interface spec](docs/spec/air-interface.md)'s tables are generated from it.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Issues and pull requests are welcome; the roadmap
is the queue, and the most useful contribution right now is an on-air session logged the way
[`docs/user/field-test.md`](docs/user/field-test.md) describes. Protocol and DSP decisions
go through short ADRs in `docs/adr/`.

## Prior art and independence

Aether HF is designed from public standards and open literature (3GPP TS 38.212 LDPC,
IEEE 802.11 LDPC, ITU-R F.1487 channel models, MIL-STD-188-110, OFDM synchronization
literature) and open implementations we can learn from (codec2/FreeDV data modes, FreeDATA,
ARDOP). It does not copy or reverse-engineer VARA; the host-interface compatibility targets
VARA's *published* TCP command set only, and Aether frames are not compatible with VARA's on
the air.

## License

Dual-licensed under **MIT OR Apache-2.0** — see [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE). Contributions are accepted under the same terms.

## Authors

KK4ODA — author. Development is done with Claude Code as a co-developer; all design
decisions and on-air responsibility remain with the licensed operator.
