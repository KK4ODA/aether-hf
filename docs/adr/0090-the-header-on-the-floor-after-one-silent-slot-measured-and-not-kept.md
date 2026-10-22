# ADR-0090: The header on the floor after one silent slot — measured and not kept

**Status:** rejected, 2026-10-22. Report 14's change 6
(`docs/research/link-architecture/14-wc4y-beta118.md` §6, §14), which asked for it to be measured
on the harness before the rule changed. Built behind a configuration field (model and port,
`GrantConfig.floor_header_after`, the vectors regenerated with it), measured, and reverted. No
wire change: what each build sends is a protocol-11 header either way.

## 1. Context

A master's header goes on the tone floor after two silent slots in a row (`_send_header`:
`hdr_floor = self._hdr_floor or self._silent >= 2 or self._floor_only()`), and otherwise in the
family the slave's readings call for. In WC4Y's Test of 2026-10-22 one silent slot was followed by
an ordinary header that was lost in the same fade, which cost another slot. Moving the threshold
to one silent slot trades 2.8 s on every header sent after a single silent slot against a slot
saved whenever the fade outlasts it.

## 2. What was measured

All 56 scenarios, two draws each (the scenario's seed and seed + 1000), on two builds of the same
tree that differ only in the threshold: 2 (as shipped) and 1. Counted from both stations'
sidecars, each station's lifetime counters.

| | threshold 2 (shipped) | threshold 1 |
|---|---|---|
| runs completed | 112 / 112 | 112 / 112 |
| radiated overlaps | 0 | 0 |
| silent slots, all scenarios | 243 | 236 |
| silent slots, the 76 fading runs (Moderate, High-moderate, Poor, flutter, NVIS) | 156 | 156 |
| air time, pooled over all runs | 30 823 s | 31 185 s (×1.012) |
| air time, fading runs | 22 258 s | 22 564 s (×1.014) |
| air time, Poor (4 runs) | 872 s | 1 033 s (×1.185) |
| Test message rate, geometric mean of 15 scenarios' medians | | ×1.024 |
| Test file rate, likewise | | ×1.003 |

The first draw alone read ×1.023 on air time, the second about ×1.00: the run-to-run spread of
one build is a percent or two, and nothing in the table stands outside it except Poor, which is
slower.

## 3. Decision

The threshold stays at two silent slots. On the fading paths, where the slot was to be saved,
threshold 1 saved none (156 against 156) and cost 1.4 % of the air; the seven slots it saved in
all were on the Good paths. The field and its vector key are removed, so the engine is as
it was.

## 4. What stays open

Report 14's one lost ordinary header is one event on one path. If more two-sided recordings show
an ordinary header lost after a single silent slot, the question to measure is a narrower rule —
for instance, the floor after one silent slot only while the slave's last reading stands within a
few decibels of the ordinary control threshold — not this one.
