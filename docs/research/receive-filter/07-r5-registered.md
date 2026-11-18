# R5, registered before any run: R4 with steady looks

*Written 2026-11-18, after `06-r4-results.md`, on the author's choice of its option B ("try B").
Nothing here has been built or run. The criteria are `05-busy-band.md` §7's, unchanged. The
predictions below are written before the runs, so the results can be held to them.*

## 1. Why

`06-r4-results.md` §3.5 found that R4's 20th percentile over the looks buys less than §6.1
expected, because a look is one 85 ms periodogram. Each bin's power is drawn afresh in every look,
about ±5.6 dB, as much as a moderate signal is high. So a quantile over the looks cannot tell the
looks a signal was on from the looks it was off: a bin a signal holds in two looks of three still
stands about 6 dB over its free neighbours. The same scatter, over the first dozen looks, made an
edge 8 dB down read 12–13 dB on draw 1 (§3.4).

## 2. The change

**R5 is R4 with each look the mean spectrum of the half second it covers.** Today a look is one
85 ms transform. In R5 it is the mean power of the transforms of that half second, each 85 ms long
and half a window apart (ten of them), turned to decibels after the mean.

* The looks cover the time between them, so no audio is skipped and none is read twice.
* A look is taken once the station has listened for the whole half second, and dropped if a frame
  lay under any part of it. Today both tests use the 85 ms window.
* A bin's scatter falls from about ±5.6 dB to about ±1.5 dB.
* Everything else is R4's: the leave-out, the floor test, the span-median division, the 20th
  percentile over the looks, the ±50 Hz smoothing, the reading's ninth decile, the 12 dB line, the
  hold, and the 12 and 120 looks.

The analyser keeps half a second of audio where it kept 85 ms. The display's spectrum still reads
the newest 85 ms.

* **Removes:** the single-periodogram look.
* **Adds:** the mean over the half second, in the analyser. No new constant: the span is the look
  interval, and the step is the window's usual half.
* **Constants:** twelve, as R4.

**Not added: a longer wait before the first verdict.** K5 asks a cut on a steady band to be named
within 15 s, and today it is named at 11.5 s. Twelve looks fit, and about seventeen at most. The
*half* clips put the other station on for the first 15 s. A reading that tells a signal from the
filter by time needs about 25 s to see such a signal stop, so no wait that keeps K5 clears those
clips.

## 3. Predictions, per criterion

| Criterion | R4 | R5, predicted | Why |
|---|---|---|---|
| K1, K5, K6 | met | met | Cuts do not come and go; C1 is untouched |
| K2, K3 (draw 1) | 2 clips | **met** | The start-up scatter is the single periodogram's |
| Q1 (of 84) | 22 / 20 | **still missed**, about 12–18 | The clips warned only at warm-up stay (§2), as do the always-on and 500 Hz span cases below. Steady looks clear the come-and-go cases |
| Q1, moderate signals on 84–98 % of the looks over the sloped radio | warn | **still warn** | No rule of time clears a signal on in more than four looks of five (05 §6.1) |
| Q1, a 500 Hz signal over 60 % of the 500 Hz span | warns | **still warns** | The span median is the signal's level (06 §3.2) |
| Q2 | 23 of 24 | 23–24 | The strong always-on case may stay |
| K2 field, K4 | owed | judged if the uploads can be read | — |
| H(a) | 2 of 24 | 0–2 | The RTTY contest's warm-up warnings came with 12–36 looks: steadier looks may clear them |
| H(b) | met | met | As K1 |
| K7 | met | met | Twelve constants |
| K9 | 0.072 % | under 0.2 % | Ten transforms a look where there was one |

So **R5 is not expected to pass §7**: Q1 counts the warm-up, the always-on clips and the 500 Hz span,
which no rule that reads time can clear. Its run says how much of the rest steady looks fix. That
tells the author what is left to decide about Q1 itself.

## 4. Ablations, registered with it

Each one is a scratch build judged on the same corpus draws:

* **R5a:** steady looks with the median over the looks (`TIME_QUANTILE = 0.5`). Does the 20th
  percentile still earn its place once looks are steady?
* **R5b:** steady looks with R3's division by the ninth decile.
* **R5c:** steady looks alone, everything else R3's. This is the smallest possible change.

A part whose removal fails no criterion that R5 meets goes. If R5c meets all R5 meets, R5c is the
proposal.

## 5. What is run

As `05-busy-band.md` §10 step 6, with `06-r4-results.md`'s instruments:
* the corpus, both draws, with R3, R4, R5 and the three ablations (`scratch/criteria_table.py`,
  `q1_timing.py`);
* the field set, if the uploads can be read;
* H(a) live on R5, three draws, with H1 reading the logs too;
* H(b), the cut grid, on R5;
* K9.
