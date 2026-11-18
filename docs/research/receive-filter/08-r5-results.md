# R5 judged: steady looks make it worse

*Written 2026-11-18, against `07-r5-registered.md`, which was committed before any run. The
criteria are `05-busy-band.md` §7's, unchanged. **R5 misses more criteria than R4, and so do all
three ablations.** It is not merged; its patch is `scratch/r5.patch`. The cut grid (H(b)) was
stopped once the corpus had decided it, and the field set is still owed (§4).*

## 1. The corpus

Both draws, the same clips as `06-r4-results.md` (`scratch/criteria_table.py`):

| Draw | Build | K1 (of 48) | K2 (of 160) | K3 | K5 in time / cleared / warned | Q1 (of 84) | Q2 (of 24) |
|---|---|---|---|---|---|---|---|
| 0 | R3 | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 18 | 22 |
| 0 | R4 | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 22 | 23 |
| 0 | R5 | **47** | **2** | **3** | 23/23 · 12/12 · 12/12 | 20 | 20 |
| 0 | R5a | 47 | 5 | 6 | 23/23 · 11/12 · 12/12 | 19 | 20 |
| 0 | R5b | 46 | 6 | 7 | 22/22 · 11/12 · 12/12 | 19 | 20 |
| 0 | R5c | 47 | 1 | 2 | 23/23 · 12/12 · 12/12 | 18 | 20 |
| 1 | R3 | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 20 | 22 |
| 1 | R4 | 48 | 2 | 2 | 24/24 · 12/12 · 12/12 | 20 | 23 |
| 1 | R5 | 48 | **6** | **6** | 24/24 · 12/12 · **11/12** | 19 | 21 |
| 1 | R5a | 48 | 6 | 6 | 24/24 · 12/12 · 11/12 | 19 | 21 |
| 1 | R5b | 48 | 6 | 6 | 24/24 · 12/12 · 11/12 | 19 | 20 |
| 1 | R5c | 48 | 6 | 6 | 24/24 · 12/12 · 11/12 | 18 | 21 |

K6 (C1) is unchanged in every build: 52 of 52 hot clips, none other.

Against the registered predictions (07 §3):
* **K1, K2, K3 and K5 were predicted met. They are missed.** R5 misses a cut R3 and R4 name, warns
  on up to six clean clips, and on draw 1 one cut made is no longer named in time.
* **Q1 was predicted missed, at about 12–18. It is 19–20.** As predicted, the warm-up, the
  always-on and the 500 Hz span cases stay.
* **Q2 was predicted 23–24. It is 20–21.**

## 2. Why: the FT8 band

Every new miss and every new false warning is on the FT8 band:
* shift-up-600, deep floor, at 2300 Hz;
* shift-down-300 and shift-up-300 at 500 Hz, shift-down-600 at 2300 Hz;
* ssb2100, both floors, at 500 Hz;
* tilt, deep floor, at 500 and 2300 Hz;
* lowcut600, shallow floor, at 2300 Hz.

On `shift-up-600-ft8-deep` at 2300 Hz, R3 names the bottom cut in 279 of its readings. R5 and R5c
never get past *waiting*, though all 120 looks are kept: the reading has no drop to give.

The leave-out takes out a bin that stands 14 dB over its sides. Then a bin a signal held in more
than half the looks is blank in the profile, and an edge with too few bins left unblanked around
it has no reading. That balance was found with single 85 ms looks. In those, an FT8 station's bins
scatter as much as the noise, so they are taken out in some looks and not in others. In a
half-second mean they are steady, so they are taken out nearly every time. With fifteen stations
on the band, the bins near an edge go blank, or the blanks shift what is left.

R5c (steady looks alone, the rest R3's) shows the same. So it is the steady looks, not R4's
changes, that cause it. Making steady looks work would mean changing the leave-out too: a third
change to the same reading, which the rule for a part that needs a third fix says to review, not
patch.

## 3. The harness: H(a) only

H(a) on the R5 daemon, three draws of the eight scenarios: 24 of 24 runs pass with no radiated
overlap. **One run warned:** station b of the RTTY contest's base draw, at its 12th look (top
12.4 dB), and clear at the next. That is against R4's 2 of 24 and R3's three RTTY draws of three.
Steady looks quiet the warm-up a little, but this does not offset §1. The cut grid was stopped
once the corpus had decided R5.

## 4. Still owed

The upload script still refused this session's key (`fetch_uploads.py list`, 2026-11-18), so the
field set (K2 field, K4) and the mini PC's cost (K9) are owed for every build, R3's included.

## 5. What it leaves

Every reading of this family has now been tried against §7, and none passes:
* R4: Q1, Q2 by one, K2/K3 on draw 1, H(a);
* R5 and its ablations: more.

What none of them can pass is Q1 itself. Q1 counts the station's first half minute, a signal on
nearly all the time, and a wide signal filling the 500 Hz window. A reading that tells a signal
from the filter by time cannot clear any of them (`06-r4-results.md` §3.1–3.3). What is left is
the author's:
1. ship R4, accepting those three cases;
2. keep today's reading (R3);
3. use the other station's decoded frames to cancel a warning (W2, `05-busy-band.md` §6.2). This
   reverses the author's Gate 2 answer, and acts only after a session.

## 6. The author's decision

On 2026-11-18 the author chose none of the three: the warning goes. False warnings would be noise
for every new operator, and no other modem has one. The check is the waterfall, whose red lines mark
the signal's edges. ADR-0132 records it. The clipping warning stays. This folder's scripts and
`tools/rxfilter_corpus.py` run at `b31eb8d`, the last tree with the reading.
