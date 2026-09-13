# Contributing to Aether HF

Thanks for helping build an open HF modem. This page is short on purpose; the long-form
plan lives in [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Ground rules

1. **Measured, not claimed.** A change to DSP or protocol code must move a number on a
   committed benchmark curve or add a test that would have failed before. "It sounded better
   on 40 m last night" is a field report (welcome — file one!), not evidence.
2. **Tests are never relaxed.** If a threshold is wrong, change it in a pull request that
   references an ADR explaining why. Known defects are `xfail(strict=True)` with the audit
   finding named; fixing the defect means removing the marker in the same PR.
3. **Public sources only.** Design from public standards and open literature (3GPP, IEEE,
   ITU-R, MIL-STD, codec2, academic papers). Do not contribute anything derived from
   reverse-engineering proprietary modems. Host-interface compatibility with VARA is limited
   to its published command set.
4. **Conventions**: SNR in a 3 kHz noise bandwidth; Doppler spread as ITU-R F.1487 2σ;
   waveform parameters from `model/aether_model/waveform.py`, never restated by hand.
5. **Licensing**: by contributing you agree your work is licensed MIT OR Apache-2.0.

## Workflow

- Branch from `main` (`master` until the rename), one topic per branch, short-lived.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(fec): TS 38.212 BG2 encoder`, `fix(channel): Doppler σ vs 2σ`, `docs(adr): …`.
- Run the same checks CI runs before pushing:
  ```
  uv run ruff check . && uv run ruff format --check . && uv run mypy && uv run pytest
  ```
- Open a pull request using the template; fill in the **benchmark delta** section for any
  DSP/protocol change (even "no change expected — refactor only").
- Squash-merge after review and green CI.

## Where to start

The ordered queue is `docs/ROADMAP.md` §14. Tasks are labelled with their phase ID (P1-2,
P2-1, …) in issues. Good first contributions: statistical tests for the channel simulator,
Gray-labelled constellations (P1-2), golden-vector tooling, user documentation for
Pat / VarAC / Winlink setup once Phase 3 lands.

## Reporting

- **Bugs**: use the bug-report issue template; attach a diagnostic bundle when the app
  exists, or the exact command and output for the model.
- **On-air reports**: use the on-air template and, if you can, attach a WAV of the received
  audio (48 kHz mono) — recordings become regression tests.
- **Security**: see [`SECURITY.md`](SECURITY.md).

## Code of conduct

Be excellent to each other, on and off the air. Technical disagreement is welcome; personal
attacks, harassment and gatekeeping are not. Maintainers may remove contributors who
ignore this.
