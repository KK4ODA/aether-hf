# Aether HF

[![CI](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml/badge.svg)](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An open-source HF data modem for amateur radio, being built to do the job VARA HF does —
reliable ARQ links for Winlink, VarAC, Pat, BBS and similar software over real, noisy,
fading HF channels — with a **publicly documented air interface**, a **VARA-compatible host
interface** so existing applications work unchanged, and a **headless core** that runs on a
Raspberry Pi gateway as happily as on a Windows desktop.

> **Status: pre-alpha, not usable on the air yet.**
> The repository contains a Python *reference model* with a calibrated channel simulator
> and a complete PHY (3GPP LDPC, OFDM, acquisition, equalizing receiver, 14 modes) that
> decodes frames end-to-end through simulated HF channels and the 48 kHz audio path.
> There is no ARQ, no host interface and no shipped application yet (Phases 2–4). Every
> performance figure in this repository comes from a committed benchmark curve in
> `bench/baselines/`.

## Where things stand

A full technical audit was done on 2026-09-13 — read [`docs/AUDIT.md`](docs/AUDIT.md). In
short: the original prototype's DSP, FEC, sync, audio and protocol layers each had a
disqualifying defect, and its test suite had been relaxed until it passed. Every finding was
encoded as a strict `xfail` test and retired together with the module it documented as the
Phase 1 rewrite replaced it; the audit itself remains the record of why.

The plan from here is [`docs/ROADMAP.md`](docs/ROADMAP.md) (architecture, DSP decisions,
protocol/API design, testing strategy, release engineering, phased tasks) and the field
requirements distilled from how the community received Mercury, the other VARA alternative,
in [`docs/COMMUNITY-CONCERNS.md`](docs/COMMUNITY-CONCERNS.md).

| Phase | Scope | State |
|---|---|---|
| 0 — Audit & stabilization | tooling, strict tests, CI, calibrated simulator, ADRs | done |
| 1 — Core HF modem | real LDPC (3GPP TS 38.212), OFDM TX/RX, sync, end-to-end loopback, benchmarks, golden vectors | **done in the Python model** (`phase-1` branch) |
| 2 — Link robustness | ARQ, rate control, HARQ-IR, low-SNR modes, PAPR study | **done in the model** |
| 3 — Application integration | Rust core, `aetherd`, PTT/CAT, VARA-compatible TCP, Pat/VarAC/Winlink verification, Pi gateway build | **built** (`phase-2` branch); field verification open |
| 4 — Desktop application | Tauri GUI, setup wizard, diagnostics | **built**; usability test open |
| 5 — Release infrastructure | signed installers, auto-update, nightly/beta/stable | built: `docs/user/install.md` |
| 6 — Field validation | recorded on-air sessions folded back into the test suite | |
| 7 — Aether FM foundation | FM PHY on the same link layer | |

## Architecture in one paragraph

Application adapters (VARA-compatible TCP now; KISS/AGW later) sit on a PHY-agnostic link
layer (session, selective-repeat ARQ with HARQ-IR, rate control) and a modem framework
(frame codec, scheduler, metrics), which drive a pluggable PHY (HF OFDM first, FM later)
over a hardware abstraction (audio, PTT, rig control, or the channel simulator). The Python
model in `model/` designs and validates the waveform and generates golden vectors; the
shipped product will be a Rust core with a Tauri desktop shell (ADR-0001). Diagrams and
rationale: [`docs/ROADMAP.md` §4](docs/ROADMAP.md#4-recommended-target-architecture).

## Repository layout

```
docs/          AUDIT.md · ROADMAP.md · COMMUNITY-CONCERNS.md · adr/ (decisions) · spec/ (public
               air interface, control API, host interfaces) · user/ (install, gateway kit,
               frequency plan)
model/         Python reference model: aether_model/{channel,waveform,fec,phy,frame,link,hal} + tests/
core/          the shipped Rust workspace: aether-fec · aether-phy · aether-link · aetherd
app/           ui/ (the station panel, no build step) · src-tauri/ (the desktop shell)
deploy/        systemd unit for a gateway
tools/         benchmarks · vector generators · release.py · stage_daemon.py · bench_gate.py
bench/         committed baseline curves, including the release gate's
vectors/       golden test vectors (TX bit-exact, RX must decode)
```

**Installing it:** [`docs/user/install.md`](docs/user/install.md). **Running a gateway:**
[`docs/user/gateway-kit.md`](docs/user/gateway-kit.md).

## Working on the model

Requires Python ≥ 3.12 and [uv](https://docs.astral.sh/uv/) (`pip install uv` works too;
then use `python -m uv`).

```bash
uv sync                      # creates .venv with numpy/scipy + dev tools
uv run pytest                # strict suite: passes + documented xfails, ~15 s
uv run ruff check . && uv run ruff format --check . && uv run mypy
uv run pre-commit install    # optional: run the same checks on every commit
```

Conventions that matter:

* **SNR is always referenced to a 3 kHz noise bandwidth**, Doppler spread is the ITU-R
  F.1487 2σ value. `model/aether_model/channel.py` is calibrated to both and its tests are
  the guarantee behind every benchmark number.
* Measured so far (AWGN, FER < 5 %, random CFO/SRO): BPSK ½ at −1 dB, QPSK ½ at +2 dB,
  16-QAM ½ at +7 dB, 64-QAM ⅚ at +17 dB — see `bench/README.md` for fading channels.
* **No test threshold is ever relaxed to make a suite pass.** A known defect gets an
  `xfail(strict=True)` that names the audit finding; nothing else.
* Numbers about the waveform come from `model/aether_model/waveform.py`, never from prose.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Issues and pull requests are welcome; the roadmap's
"next 20 tasks" list is the queue. Protocol and DSP decisions go through short ADRs in
`docs/adr/`.

## Prior art and independence

Aether HF is designed from public standards and open literature (3GPP TS 38.212 LDPC,
IEEE 802.11 LDPC, ITU-R F.1487 channel models, MIL-STD-188-110, OFDM synchronization
literature) and open implementations we can learn from (codec2/FreeDV data modes, FreeDATA,
ARDOP). It does not copy or reverse-engineer VARA; the host-interface compatibility targets
VARA's *published* TCP command set only.

## License

Dual-licensed under **MIT OR Apache-2.0** — see [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE). Contributions are accepted under the same terms.

## Authors

KK4ODA — author. Development is done with Claude Code as a co-developer; all design
decisions and on-air responsibility remain with the licensed operator.
