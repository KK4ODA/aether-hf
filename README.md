# Aether HF

[![CI](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml/badge.svg)](https://github.com/KK4ODA/aether-hf/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/KK4ODA/aether-hf?include_prereleases&label=release)](https://github.com/KK4ODA/aether-hf/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An open-source HF data modem for amateur radio, built to do the job VARA HF does —
reliable ARQ links for Winlink, Pat, BBS and similar software over real, noisy, fading HF
channels — with a **publicly documented air interface**, a **VARA-compatible host
interface** so existing applications work unchanged, and a **headless core** that runs on a
Raspberry Pi gateway as happily as on a Windows desktop.

> **Status: beta — field testing on the air.**
> Signed builds ship from [Releases](https://github.com/KK4ODA/aether-hf/releases)
> (`0.2.0-beta.68` at the time of writing): a Windows installer, Linux packages, an unsigned
> macOS build for Apple Silicon, and the standalone `aetherd` daemon for gateways, with
> in-place updates on a beta channel. The modem — 3GPP LDPC, OFDM, 16-FSK tone frames,
> selective-repeat ARQ with HARQ soft combining, rate control, compression — climbs a
> twenty-rung ladder at 2300 Hz, from a 36 bit/s **tone floor** through four fast-tone rungs to
> 64-QAM 5/6, and a fifteen-rung one at **500 Hz** for peer-to-peer contacts. Calls, probes and
> beacons go on the tone floor, which decodes at −19 dB SNR (3 kHz) on AWGN and −11.7 to
> −14.5 dB on the ITU fading channels, and a 2300 Hz session completes on the simulated fading channels
> down to −14 dB ([ADR-0013](docs/adr/0013-the-tone-floor.md) to
> [ADR-0016](docs/adr/0016-calls-on-the-floor.md)).
> **Winlink Express, Pat and VarAC each complete a session at both ends of a simulated
> channel**, attachments byte-identical on arrival, and APRS and packet programs have a
> VARA-style **KISS port**. **Every transmission is judged against the licence's rules before
> the radio is keyed** — FCC Part 97 today, 160 m to 6 m
> ([`docs/user/fcc-regulatory-controls.md`](docs/user/fcc-regulatory-controls.md)). Sessions
> with other stations on 80 and 40 m are finding what a bench cannot, and each fix is an ADR
> (0017, 0020–0023). Every performance figure in this repository comes from a committed
> benchmark curve in `bench/baselines/`. A **Test session** (Session → *Test session*) runs a
> fixed sequence against any listening station — a probe, a message, a burst at every mode, a
> file — records it, and *Contribute the last test session*, on the same tab, turns the
> recording into a report the project can replay: how every volunteer contact becomes a
> measurement.

## Screenshots

The station panel during a Test session on the bench — two daemons on one machine over
the simulated channel at 12 dB. The caller's Session tab: the dial and the rules' verdict, the
call with the Test's progress through the rung ladder, keying and drive, and the conversation:

![The Session tab of the calling station during a Test session](docs/images/panel-session.png)

Its Status tab a couple of minutes later: the link and the channel at a glance, the speed over
the last ten minutes against the rate of the rung in use, the channel's rhythm of bursts and
acknowledgements, and the counters:

![The Status tab of the calling station during a Test session](docs/images/panel-status.png)

The other station's Diagnostics tab: the last frame's constellation (a 64-QAM rung), the
spectrum and the waterfall, and the receiver's readings:

![The Diagnostics tab of the receiving station](docs/images/panel-diagnostics.png)

## Getting it

* **Desktop:** download the installer for Windows, Linux or macOS from
  [Releases](https://github.com/KK4ODA/aether-hf/releases) and follow
  [`docs/user/install.md`](docs/user/install.md). The Setup tab walks through the callsign
  and the rules the station runs under, the radio interface (sound card, keying and CAT), the
  receive level and the modem's settings; the Session tab's *Set drive* sets the transmit level
  against the rig's ALC. The settings are kept as *profiles*, one per radio or place,
  exportable as a file for another computer. The Windows installer is not yet
  Authenticode-signed, so SmartScreen will ask, and the macOS build (Apple Silicon) is
  unsigned and untested on a real Mac — the guide says how to open it; `SHA256SUMS` is
  beside every asset.
* **Host programs:** Winlink Express (Vara HF session, TNC at `127.0.0.1:8300`, auto-launch
  off) and Pat (`varahf`) talk to the daemon's VARA-compatible port. What each client sends,
  what is verified and what is not is in
  [`docs/spec/host-interfaces.md` §7](docs/spec/host-interfaces.md); how to set each one up,
  with the host program owning the radio as it does VARA's or with Aether owning it, is
  [`docs/user/host-programs.md`](docs/user/host-programs.md). The modem runs at
  2300 Hz or, for peer-to-peer contacts and VarAC, at **500 Hz** (`[radio]
  bandwidth`; `docs/spec/air-interface.md` §2.3) — both stations of a session use the same
  one. VarAC pings and connects to it at 500 Hz on the bench.
* **APRS and packet programs:** the KISS port, VARA HF's `127.0.0.1:8100`
  ([`docs/user/kiss.md`](docs/user/kiss.md)); each frame goes on the air as a datagram that
  other Aether stations hear, whatever their bandwidth.
* **The rules:** nothing is transmitted until Setup step 1 says under which rules the station
  runs, how it is controlled and the licence class; then every transmission is checked against
  them at the dial the radio is on, and one they do not allow is not sent
  ([ADR-0018](docs/adr/0018-the-regulatory-gate.md)). The operator stays responsible.
* **Where to operate:** [`docs/user/frequency-plan.md`](docs/user/frequency-plan.md) proposes a
  calling frequency on every band from 80 m to 6 m; the panel offers them as dial memories.
* **Gateway:** the `aetherd-…` archive and [`docs/user/gateway-kit.md`](docs/user/gateway-kit.md)
  (headless install, systemd unit, cross-compiling for ARM64, automatic control inside the
  §97.221(b) sub-bands). An Aether gateway must not be listed as a VARA gateway.
* **Updates:** the desktop application checks for a newer signed build on start (`[update]
  channel`: `stable`, `beta` or `nightly`; a beta installation follows the betas) and keeps
  the previous installer for going back, settings included.

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
| 3 — Application integration | Rust core bit-exact with the model, `aetherd`, PTT/CAT, control API, VARA-compatible TCP, gateway kit | done; Pat, Winlink Express and VarAC pass the bench |
| 4 — Desktop application | station panel, Tauri shell, setup, diagnostics, accessibility | done, and redesigned in betas .60–.68; the three-ham usability test is open |
| 5 — Release infrastructure | one version number, installers that bundle the daemon, signed updates on three channels, SBOM, benchmark gate | done — betas flow, `0.2.0-beta.2` through `.68` |
| 6 — Field validation | recordings, replay regression tier, simulated channel, measured-vs-predicted tool, field protocol; then the air; then on-air crowdsourcing — a Test session every volunteer can run, whose sidecar the bench replays (P6-7) | **in progress**: tooling and the Test session built; on the air with other stations since 2026-09-23 — bursts that fit the key watchdog (ADR-0017), rate control that learns only from evidence (ADR-0020), the peer's SNR both ways (ADR-0021), the end of a session (ADR-0022) and leaving and handing over (ADR-0023) came from those sessions |
| 7 — The 500 Hz waveform and the link probe | the bandwidth P2P contacts are made in (VarAC's calling frequencies), the bandwidth in the connect handshake, an answer-only unattended mode, and a two-way SNR probe | **done on the bench**: `[radio] bandwidth = 500`, `BW500`, answer-only, the probe (ADR-0006), VarAC pinging and connecting over the simulated channel; the air with a VarAC station remains |
| 9 — The modem's second rung | a faster start, modes below 200 bit/s, an audio-level A/B bench against VARA HF, the deferred pilot/prefix/2750 Hz experiments, time diversity — each with its curve | **in progress**: a faster climb and start (ADR-0007, 0008), a calibrated fading link bench, holding the link (ADR-0012), the tone floor, fast tones and the 500 Hz middle kinds (ADR-0013–0015), calls on the floor (ADR-0016); the A/B bench's tool is written and waits on its cables (P9-1); time diversity is paused |
| 8 — Aether on a phone | the modem in a Pi-sized box the phone talks to over Bluetooth or Wi-Fi, then a phone app, then the modem inside the phone | back burner |
| 10 — Aether FM foundation | an FM PHY on the same link layer | back burner |

## Architecture in one paragraph

Application adapters (the VARA-compatible host interface and a KISS port now; AGWPE with
FM) sit on a PHY-agnostic link layer (session, selective-repeat ARQ with HARQ-IR, rate
control, compression, datagrams) and a modem framework (frame codec, scheduler, metrics),
which drive a pluggable PHY (HF OFDM and tone frames; FM later) over a hardware abstraction
(audio, PTT, rig control, or the channel simulator). Nothing reaches the transmitter without
the regulatory gate's leave. The
Python model in `model/` is the specification: it designs and validates the waveform and
generates the golden vectors the Rust core in `core/` is held to, bit-exactly, in CI. The
desktop shell in `app/` supervises the daemon and serves its panel (ADR-0001, ADR-0005).
Diagrams and rationale: [`docs/ROADMAP.md` §4](docs/ROADMAP.md#4-recommended-target-architecture).

## Repository layout

```
docs/          AUDIT.md · ROADMAP.md · COMMUNITY-CONCERNS.md · MAINTAINING.md ·
               VARA-UX-BENCHMARK.md · adr/ (decisions) · spec/ (public air interface, control
               API, host interfaces) · user/ (install, gateway kit, frequency plan, FCC
               regulatory controls, host programs, KISS programs, field-test protocol)
model/         Python reference model: aether_model/{channel,waveform,fec,phy,frame,link,hal} + tests/
core/          the shipped Rust workspace: aether-fec · aether-phy · aether-link · aetherd
               (aetherd/data/: the regulatory profiles and the measured occupancy)
app/           ui/ (the station panel, no build step) · src-tauri/ (the desktop shell)
field/         the field log, findings from the air, and the recorded sessions that are now
               regression tests
deploy/        systemd unit for a gateway
tools/         benchmarks (bench_phy, bench_link, bench_tone, bench_calls …) · vector
               generators · release.py · field_ingest.py · compare_air.py · channel_cable.py ·
               kiss_test_client.py · make_occupancy.py · make_spec.py
bench/         committed baseline curves, including the release gate's; ab/ (the VARA A/B bench)
vectors/       golden test vectors (TX bit-exact, RX must decode)
Logos/         the artwork the icons are made from
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
* Minimum usable SNR (FER ≤ 10 %, random CFO and sample-rate offset), AWGN / ITU Poor: the
  tone floor's tone-24 (36 bit/s, both bandwidths) at −19.0 / −14.5 dB; the fastest tone rung,
  tone100-153 (228 bit/s, 2300 Hz), at −11.2 / −4.1 dB; then OFDM at 2300 Hz, BPSK 1/5 at
  −5.2 / −0.3 dB, QPSK 1/2 at +1.0 / +6.0 dB, 16-QAM 1/2 at +6.0 / +12.5 dB and 64-QAM 5/6 at
  +16.9 dB on AWGN. At 500 Hz the four-tone rungs reach −14.3 and −13.0 dB on AWGN. The whole
  table, per channel class, is in [`bench/README.md`](bench/README.md).
* **No test threshold is ever relaxed to make a suite pass.** A known defect gets an
  `xfail(strict=True)` that names the finding; a target that genuinely changes gets an ADR.
* Numbers about the waveform come from `model/aether_model/waveform.py`, never from prose;
  the public [air-interface spec](docs/spec/air-interface.md)'s tables are generated from it.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the rules and
[`docs/MAINTAINING.md`](docs/MAINTAINING.md) for how pull requests are reviewed. Issues and
pull requests are welcome; the roadmap is the queue, and the most useful contribution right
now is a Test session with another station, contributed from the Session tab the way
[`docs/user/field-test.md`](docs/user/field-test.md) describes. Protocol and DSP decisions go through short ADRs in `docs/adr/`. The
[code of conduct](CODE_OF_CONDUCT.md) is the hobby's own: be excellent to each other.

## Prior art and independence

Aether HF is designed from public standards and open literature (3GPP TS 38.212 LDPC,
IEEE 802.11 LDPC, ITU-R F.1487 channel models, MIL-STD-188-110, OFDM synchronization
literature) and open implementations we can learn from (codec2/FreeDV data modes, FreeDATA,
ARDOP). It does not copy or reverse-engineer VARA; the host-interface compatibility targets
VARA's *published* TCP command set only, and Aether frames are not compatible with VARA's on
the air.

## Support

Aether HF is free software and stays that way. The bench it is measured on — radios,
interfaces, the hours on the air — is paid for by its author. If the modem is useful to
you, a contribution through
[PayPal](https://paypal.me/facundofern)
(also the *Sponsor* button above) helps keep it on the air. An on-air report is worth as
much.

## License

Dual-licensed under **MIT OR Apache-2.0** — see [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE). Contributions are accepted under the same terms.

## Authors

KK4ODA — author. Development is done with Claude Code as a co-developer; all design
decisions and on-air responsibility remain with the licensed operator.
