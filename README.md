# Aether HF

[![CI](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml/badge.svg)](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An open-source HF data modem for amateur radio, being built to do the job VARA HF does —
reliable ARQ links for Winlink, VarAC, Pat, BBS and similar software over real, noisy,
fading HF channels — with a **publicly documented air interface**, a **VARA-compatible host
interface** so existing applications work unchanged, and a **headless core** that runs on a
Raspberry Pi gateway as happily as on a Windows desktop.

> **Status: pre-alpha, not usable on the air.**
> The repository currently contains a Python *reference model* and a calibrated *channel
> simulator*. There is no working modem yet. The first on-air-capable build is the goal of
> roadmap Phases 1–3. Nothing here has measured performance, and this README will not claim
> any until a committed benchmark curve backs it.

## Where things stand

A full technical audit was done on 2026-09-13 — read [`docs/AUDIT.md`](docs/AUDIT.md). In
short: the original prototype's DSP, FEC, sync, audio and protocol layers each had a
disqualifying defect, and its test suite had been relaxed until it passed. Every finding is
now encoded as a strict `xfail` test in `model/tests/test_legacy_*.py`; each one flips to a
real assertion in the pull request that fixes it.

The plan from here is [`docs/ROADMAP.md`](docs/ROADMAP.md) (architecture, DSP decisions,
protocol/API design, testing strategy, release engineering, phased tasks) and the field
requirements distilled from how the community received Mercury, the other VARA alternative,
in [`docs/COMMUNITY-CONCERNS.md`](docs/COMMUNITY-CONCERNS.md).

| Phase | Scope | State |
|---|---|---|
| 0 — Audit & stabilization | tooling, strict tests, CI, calibrated simulator, ADRs | **in progress** (this branch) |
| 1 — Core HF modem | real LDPC (3GPP TS 38.212 BG2), OFDM TX/RX, sync, end-to-end loopback, benchmarks | next |
| 2 — Link robustness | ARQ, rate control, HARQ-IR, low-SNR modes, PAPR study | |
| 3 — Application integration | Rust core, `aetherd`, PTT/CAT, VARA-compatible TCP, Pat/VarAC/Winlink verification, Pi gateway build | |
| 4 — Desktop application | Tauri GUI, setup wizard, diagnostics | |
| 5 — Release infrastructure | signed installers, auto-update, nightly/beta/stable | |
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
docs/          AUDIT.md · ROADMAP.md · COMMUNITY-CONCERNS.md · adr/ (decisions) · spec/ (later)
model/         Python reference model: aether_model/ (channel.py, waveform.py, legacy modules) + tests/
tools/         audit probe scripts (frozen evidence) · benchmark runner (later)
core/ app/     Rust workspace and Tauri app — created in Phase 3 / 4
vectors/       golden test vectors — created in Phase 1
```

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
  the guarantee behind every future benchmark number.
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
