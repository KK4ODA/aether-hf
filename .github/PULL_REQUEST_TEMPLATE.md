## What

<!-- One paragraph. Link the roadmap task ID (e.g. P1-2) or issue. -->

## Why

## Benchmark delta

<!-- Required for any change under model/aether_model, core/ or the air interface.
     Paste the relevant rows from tools/bench (once it exists) or say
     "no change expected — refactor only" and how you verified that. -->

| Mode | Channel | Metric | Before | After |
|---|---|---|---|---|

## Checklist

- [ ] `uv run pytest` green; no `xfail` marker removed without its defect being fixed, none added without an audit/ADR reference
- [ ] `ruff check`, `ruff format --check`, `mypy` clean
- [ ] Docs/spec/ADR updated if behaviour or a decision changed
- [ ] No performance claim without a committed curve
