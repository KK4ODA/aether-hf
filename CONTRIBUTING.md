# Contributing to Aether HF

Thanks for helping build an open HF modem. This page is short on purpose; the long-form
plan lives in [`docs/ROADMAP.md`](docs/ROADMAP.md), and how the maintainer reviews what
arrives is in [`docs/MAINTAINING.md`](docs/MAINTAINING.md) — read that too, so a pull
request meets the rules the first time.

## Ground rules

1. **Measured, not claimed.** A change to DSP or protocol code must move a number on a
   committed benchmark curve or add a test that would have failed before. "It sounded better
   on 40 m last night" is a field report (welcome — file one!), not evidence.
2. **Tests are never relaxed.** If a threshold is wrong, change it in a pull request that
   references an ADR explaining why. Known defects are `xfail(strict=True)` with the audit
   finding named; fixing the defect means removing the marker in the same PR.
3. **The model is the specification.** The modem and the link engine exist twice: the
   Python model in `model/` and the Rust port in `core/`, held bit-exact by generated
   vectors. A change goes into the model first, then the port; a pull request with only
   one half will be asked for the other. Vectors are regenerated only for a deliberate
   model change — a mismatch otherwise is a bug in the core, never stale vectors.
4. **Public sources only.** Design from public standards and open literature (3GPP, IEEE,
   ITU-R, MIL-STD, codec2, academic papers). Do not contribute anything derived from
   reverse-engineering proprietary modems. Host-interface compatibility with VARA is limited
   to its published command set, and Aether's air interface is its own.
5. **What goes over the air is an ADR.** A frame layout, preamble, mode table or handshake
   change breaks every installed station; it needs a decision record in `docs/adr/` and a
   spec update (`python tools/make_spec.py`) before it can be reviewed.
6. **Conventions**: SNR in a 3 kHz noise bandwidth; Doppler spread as ITU-R F.1487 2σ;
   waveform parameters from `model/aether_model/waveform.py`, never restated by hand.
7. **Licensing**: by contributing you agree your work is licensed MIT OR Apache-2.0. Code
   under another licence cannot be merged.
8. **The panel explains itself.** Every control and indicator in `app/ui/` — button,
   field, select, lamp, reading, chart, table heading — has a `title` tooltip saying
   what it does or shows, in plain words. Add one with anything you add.
9. **The panel loads clean.** A change to `app/ui/` is not done until the panel has been
   loaded against a running daemon and the browser console shows **no errors**, with the
   LINK lamp lit and the readings populated. `node --check` proves syntax, not that the
   page works: a beta once shipped with an infinitely recursive redraw that passed
   `node --check`, threw on every tick, and left the Status page blank and reading
   "Not connected to a modem".

## Workflow

- Fork the repository, branch from `master`, one topic per branch, short-lived.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat(link): …`, `fix(phy): …`, `docs(adr): …`, `bench: …`), because the release notes
  are generated from them. Pull requests are **rebased** onto `master`, never squashed, so
  each commit should stand on its own.
- Run what CI runs before pushing:
  ```
  uv run ruff check . && uv run ruff format --check . && uv run mypy && uv run pytest
  cd core && cargo test --release --workspace && cargo clippy --all-targets --all-features -- -D warnings
  cd app/src-tauri && cargo clippy --all-targets -- -D warnings
  ```
- Open a pull request using the template; fill in the **benchmark delta** section for any
  DSP/protocol change (even "no change expected — refactor only", with how you checked).
- Expect one round of review. The maintainer merges after green CI.

## Where to start

The ordered queue is `docs/ROADMAP.md` §14. Things that help right now, no modem expertise
needed:

* **On-air sessions**, logged the way [`docs/user/field-test.md`](docs/user/field-test.md)
  describes, with the recording — every one becomes a regression test the bench replays.
* **Host programs on the bench**: BPQ32 over the simulated channel (Pat, Winlink Express and
  VarAC are done — `docs/spec/host-interfaces.md` §7 says how).
* **Setup on hardware we do not have**: CM108/HID keying (SignaLink-class interfaces),
  radios whose CAT keying is untested, Linux sound-card notes for `docs/user/`.
* **Documentation**: anything you had to work out for yourself while installing.

The modem itself (Phase 9: a wide-band floor family, time diversity, the A/B bench
against VARA) is open too, with the rules above: model first, a curve for every claim.

## Reporting

- **Bugs**: use the bug-report issue template and attach the diagnostic bundle (Help tab →
  Diagnostic bundle), or the exact command and output for the model.
- **On-air reports**: use the on-air template and, if you can, attach the session recording
  (48 kHz mono WAV plus its sidecar) — recordings become regression tests.
- **Security**: see [`SECURITY.md`](SECURITY.md).

## Code of conduct

[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md): be excellent to each other, on and off the air.
