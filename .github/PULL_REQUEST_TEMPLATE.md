## What

<!-- One paragraph. Link the roadmap task ID (e.g. P9-5) or the issue. -->

## Why

## Benchmark delta

<!-- Required for any change under model/aether_model, core/aether-phy, core/aether-link
     or the air interface. Paste the rows from tools/bench_phy.py, bench_link.py or
     bench_floor.py that moved, or say "no change expected — refactor only" and how you
     verified that (the vectors and both suites unchanged is the usual answer). -->

| Mode | Channel | Metric | Before | After |
|---|---|---|---|---|

## Checklist

- [ ] Model first: the change is in `model/` and in `core/`, and `tests/model_vectors.rs` still passes
- [ ] No vectors regenerated — or the deliberate model change that made the old ones wrong is named above
- [ ] No test threshold relaxed; no `xfail` removed without its defect fixed, none added without an audit/ADR reference
- [ ] `uv run pytest`, `ruff check`, `ruff format --check`, `mypy` green; `cargo test --release --workspace` and `cargo clippy … -D warnings` green
- [ ] An ADR in `docs/adr/` and `python tools/make_spec.py` if anything over the air changed
- [ ] Docs updated if behaviour or a decision changed
- [ ] Designed from public sources; nothing derived from proprietary modem internals
- [ ] Commits follow Conventional Commits (the release notes are generated from them)
