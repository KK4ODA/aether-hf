# The scanning proposal's timing scripts

Frozen evidence for `../00-timing.md` and ADR-0129, written on 2026-11-17 before any design was
agreed. They drive the model's own engine but are not the model, and they are not maintained.
Like the other research code they are outside lint (`pyproject.toml`'s `extend-exclude` names
`research`).

| File | What it measures |
|---|---|
| `call_schedules.py` | A call nobody answers, from `GrantedEngine` on both airs: each try's start and parts, and the call's end, for 2–15 tries, plans H and S. Writes `calls.json` (not kept: it takes a few seconds) |
| `scan_timing.py` | A gateway scanning *n* channels against those calls: the dwell and linger derived from them, the tries a call needs, and how often a call is caught. Writes `scan_timing.csv` |
| `scan_timing.csv` | The run behind `../00-timing.md`'s tables |

Run them from this directory with the model's environment:

```
python -m uv run python call_schedules.py
python -m uv run python scan_timing.py
```
