# Gate 0: the evidence

*Written 2026-11-13, at the author's go ("go ahead with Gate 0"), on `master` at beta.154
(`d8e58b2` and after; nothing in the controller changed since beta.150, the build the 2026-11-12
sessions ran). The brief is `00-brief.md`; its §5 lists the four items, taken here in order.*

## 1. The field replay

`tools/c1_replay.py` (test: `model/tests/test_c1_replay.py`) replays each station's C1 on a
two-sided session's own frames, each direction on its own, through `rung_reading.replay_sender`
(the controller built as the engine builds it, fed the readings, the acknowledgements, both
families, every decoded frame's echo and a silent slot's step, as the sender's engine fed them).
It reports, per direction: how many data transmissions the replay chose as the session sent; each
try of a rung above, its frames, its verdict and the margin; the exchange time C1 rated the rung
above better and neither chose nor tried it, and how much of that was under a wait a lost try had
doubled; and the best measured neighbour's delivery over the rungs used (`rung_reading.regrets`).

Every two-sided upload from 2026-11-02 to 11-12, 22 sessions (`data/field_replay.json`, without
the per-transmission states): 25 directions with three data transmissions or more.

* **The seven that agree** (the replay chose as the session sent in four transmissions of five or
  more): 108 data transmissions. Their tries: 4 taken, 5 lost, and of the 5, two lost with every
  frame of the try decoded, by 1.6 % and 1.8 % — the two near misses of `00-brief.md` §1, one at
  each air, both 2026-11-12's. C1 rated the rung above better and held off it for 142 s of 1 173 s
  of exchanges, 77 s of it under a doubled wait, all in the 500 Hz session.
* **The eighteen that part** (42 of 96 data transmissions agree) are the gateway sessions of 11-02,
  11-06, 11-07, 11-08 and the night of 11-12: short (3–14 data transmissions), weak (−3 to −9 dB),
  on the floor's tone rungs, with resends, re-encodes and pieces that the replay's feed models
  less well. They do not test the guess near the top, which is what this review is about, and are
  left as a reading.

So the air's record of a steady path is the one session pair of 2026-11-12. The harness twin of
§3 carries the rest.

## 2. The reading's calibration

The prior reads a rung's AWGN curve 3 dB under the other station's reading. So the question is
how far under the reading the truth lies, by path. `tools/bench_phy.py` on today's model, twenty
frames a point, a decibel apart, four rungs an air around the 2026-11-12 sessions' (2300 Hz: 9,
11, 12, 14; 500 Hz: 9–12), on AWGN and ITU-R F.1487 Good, Moderate and Poor:
`data/calibration_2300.csv`, `data/calibration_500.csv`. For each rung and path, the true SNR
where 10 % of frames fail, the receiver's mean reading there, the rung's AWGN 10 % point in
`rate.py`, and the prior at that reading:

| air | rung | AWGN: reading − point | prior there | Good | Moderate | Poor |
|---|---|---|---|---|---|---|
| 2300 | 9 QPSK ⅓ | +0.8 dB | 0.10 | +5.1 | +4.1 | +2.6 |
| 2300 | 11 QPSK ⅔ | +0.5 dB | 0.06 | +5.5 | +5.4 | +4.3 |
| 2300 | 12 8PSK ½ | +0.3 dB | 0.04 | +4.6 | +4.5 | +4.8 |
| 2300 | 14 16-QAM ½ | +0.8 dB | 0.11 | +5.5 | +4.5 | +6.3 |
| 500 | 9 16-QAM ½ | +0.4 dB | 0.05 | +7.4 | +5.8 | +9.7 |
| 500 | 10 16-QAM ⅔ | +0.3 dB | 0.04 | +7.6 | +7.5 | never ≤ 15 dB |
| 500 | 11 16-QAM ¾ | +0.2 dB | 0.03 | never ≤ 14 dB | never ≤ 15 dB | never ≤ 16 dB |
| 500 | 12 64-QAM ⅔ | +0.3 dB | 0.04 | +8.1 | never ≤ 19 dB | never ≤ 20 dB |

(The fading columns are the reading at the 10 % point less the AWGN point, in dB; the prior there
is 0.99–1.00 in every one.)

* **On a clean path the prior is three decibels too cautious, at every rung of both airs.** The
  reading runs 0.3–1.0 dB over the true SNR, so a rung's 10 % point arrives at a reading 0.2–0.8
  dB over its AWGN point, where the prior, 3 dB lower, gives it 3–11 %. A rung that decodes nine
  frames in ten starts its account as one that decodes one in twenty.
* **On a fading path it is not cautious enough.** The reading runs 0.4–2.7 dB *under* the true
  SNR there (the most on Good at 500 Hz), and a rung's 10 % point lies 2.6–9.7 dB over its AWGN
  point by the reading; the prior is near certain at a reading where the rung still loses one
  frame in ten, and at a few decibels less, where it loses most, still rates it well.
* **So no one margin is right**, and the brief's G1 (a smaller cut, alone) would buy the steady
  path's speed with the fading paths' failed tries. What is meant to tell the two apart is the
  family's offset, which the frames move: on 2026-11-12's steady path it had moved 2.5–2.8 dB
  by the end of the 2300 Hz session and 1.3–2.2 dB in the 500 Hz one, short of the 3 dB the
  calibration says the path wanted, and slowing as it came (a success moves it by
  `family_gain_db` × (1 − p), and p of the rung in use is near 1 within minutes). The rung above,
  1.5–2 dB higher, stays at 0.7–0.9 while it does.

## 3. The strong paths and the oracle

**The oracle on the two new scenarios**, here in the container (no Actions minutes): each run at
C1's own choice and pinned at each rung near the edge, three draws (seeds *s*, *s* + 1000,
*s* + 2000). On a clean path the draws repeat to the bit per second: every rung lies well to one
side of its cliff (§2), so each draw sends and loses the same frames, and the regret is a number,
not a spread (the 2300 Hz set ran six draws before this was seen, all alike).

| scenario | C1 | rung 9 | 10 | 11 | 12 | 13 | 14 | 15 | regret |
|---|---|---|---|---|---|---|---|---|---|
| `80m-steady-mail-2300` (bit/s) | 963 | 508 | 779 | 1053 | **1176** | — | 779 | 703 | **+22.1 %** |
| `80m-steady-mail-500` (bit/s) | 419 | 277 | 376 | **431** | 292 | lost | — | — | **+2.9 %** |

At 2300 Hz C1 settles on rung 11 where rung 12 delivers 22 % more; at 500 Hz it settles where the
best rung is, near enough. The field says the same by its own measure (§1: +15 % and +4 %).

**Two new scenarios** shaped like 2026-11-12: `bench/scenarios/80m-steady-mail-2300.toml` and
`-500.toml`, a 22.5 kB message on a clean path at +5 dB with the reverse path at +10 dB, as the
gateway and the mobile read each other. A clean path stands in for the field's because at the
field's readings its rungs decoded as AWGN's do (§2): at 2300 Hz QPSK ⅔ 115 of 115 and 8PSK ½
4 of 4, at 500 Hz 16-QAM ¾ 171 of 174 and 64-QAM ⅔ 1 of 9. One run each on the build of this
review reproduces the field closely: 22.5 kB in 187 s at 963 bit/s at 2300 Hz (the field's data
took about 170 s), 430 s at 419 bit/s at 500 Hz (the field's about 453 s at 397 bit/s), no
collision.

**The regret already measured** on the regret set's strong scenarios, from the second review's
oracle (`bench/baselines/rate_oracle_beta127.csv`, beta.127, ten draws; the receivers have
changed since by ADR-0104, so a reference, not the baseline):

| scenario | median regret | best fixed rung (draws) |
|---|---|---|
| `80m-strong-moderate-500` (a Test's file) | +21.8 % | 12 (6), 13 (2), 10, 14 |
| `40m-good-throughput-2300` | +24.0 % | 17 (4), 18 (4), 19 (2) |
| `quick-awgn-2300` (3 kB) | +0.0 % | 16 (8), 17, 18 |

**Today's C1 and today's fixed rungs on the same draws** (the oracle workflow on GitHub's
runners, 2026-11-13, beta.156's daemon, beta.127's ten seeds: run 22 ran C1 alone, run 23 ran C1
again and the rungs near each air's edge, 9–14 at 500 Hz and 14–19 at 2300 Hz;
`data/oracle_strong_beta156.csv`, `data/oracle_strong_rungs_beta156.csv`). Two measures of what C1
leaves: against the best fixed rung of each draw (hindsight, which no controller has), and
against the best single rung held on every draw (one a controller could settle on).

| scenario | C1, median bit/s (run 22 / 23) | best rung of each draw, median | regret | best single rung, median | over C1, paired |
|---|---|---|---|---|---|
| `80m-strong-moderate-500` | 318 / 310 | 371 (10, 12, 13, 14) | **+15 % / +18 %** | 12: 357 | **+4 % / +8 %** |
| `40m-good-throughput-2300` | 2 247 / 2 247 | 2 784 (17, 18, 19) | **+28 %** | 17: 2 695 | **+23 %** |
| `quick-awgn-2300` | 2 400 / 2 400 | 2 400 (16; 17 once) | 0 | 16: 2 400 | 0 |

* **The harness's own noise.** At 2300 Hz C1 repeated itself to the bit per second on all twenty
  draws of the two runs. At 500 Hz the two runs agree on 2 draws of 10 and part by up to 14 % on
  one (median −1 %): the Test's own start lands blocks apart from run to run (ROADMAP P9-18). A
  few points of the 500 Hz regret are that noise.
* **beta.127's caveat did not hold.** Today's fixed rungs on 40 m Good deliver what beta.127's
  did, draw for draw within a few per cent, and the regret is the same +28 %. At 500 Hz they
  deliver ±10 % of beta.127's by draw, and the regret is +15–18 % against today's rungs where it
  was +12 % against beta.127's.
* **40 m Good at +20 dB: a quarter left, most of it reachable.** Rung 17 held on every draw
  delivers 23 % more than C1 (median, paired), and 17 or 18 is the best rung on eight draws of
  ten. The replay (`tools/c1_replay.py` on each run's two sidecars; it chose as the run sent in 67
  of 73 of the master's data transmissions) shows why C1 does not get there. It starts at rung 16
  (15 in two draws), the message takes 51–71 s of exchanges, and in that time it tries the rung
  above once or twice. Rung 17 was taken in 5 of its 8 decided tries. Rung 18 was never taken: two
  tries lost, three still running when the message ended, 4 of 6 frames decoded. The best rungs
  are one or two above where C1 starts, and a short transfer ends before one try a rung gets there.
* **80 m Moderate at 500 Hz: little left that a controller could take.** The best rung moves from
  draw to draw (10 twice, 12 three times, 13 three times, 14 twice), and rungs 11–14 lie within
  about 10 % of each other on the median. Rung 12 held on every draw delivers 4–8 % more than C1.
  C1 sits on rungs 10–11. Its tries of rung 12 were taken 5 times and lost 6 (22 of 31 frames
  decoded), and rung 13 lost every decided try (4 of 15 frames decoded). On this path the rung
  above is a real gamble, unlike the steady path's.
* **AWGN, 3 kB: nothing left.** C1 delivers what rung 16, the best rung, does on nine draws of
  ten; on the tenth it took a second longer, and rung 17 happened to finish in 7 s.

## 4. The count

The guess rests on C1's seven settings (`rate.py`): `half_life` 12, `prior_frames` 2,
`probe_every` 6, `probe_wait_max` 48, `slope_db` 0.5, `prior_margin_db` 3, `family_gain_db` 0.5;
and on five numbers in the code: the family offset's limits (−5 and +20 dB), the reading's
smoothing (0.7 / 0.3), a try's length (`prior_frames` + 1 frames, or twice that many
transmissions) and the doubling of a lost try's wait. The controller is 710 lines in the model
(`model/aether_model/link/rate.py`) and 889 in the port (`core/aether-link/src/rate.rs`).

## 5. What Gate 0 says to Gate 2

1. **Where the speed is.** On a steady strong path at 2300 Hz C1 leaves about a fifth of the
   throughput unused (harness +22 %, field +15 %), at 500 Hz very little (+3 %, +4 %). On the
   regret set's strong fading paths, against today's fixed rungs, C1 leaves +28 % at 2300 Hz
   (Good, +20 dB; +23 % against the best single rung) and +15–18 % at 500 Hz (Moderate; +4–8 %
   against the best single rung, the rest being which rung a draw happens to favour) (§3).
2. **Why, at 2300 Hz.** The try of the rung above decoded every frame and lost its verdict by
   under 2 %, and the doubled wait then outlasted the session. The prior is three decibels too
   cautious on a clean path (§2) and counts as two frames against a try of three; the family's
   offset, which should learn the difference, moves by a twentieth of a decibel a frame once the
   rung in use decodes reliably.
3. **What a candidate must not do.** A smaller cut alone (G1) is the fading paths' loss: there the
   prior is already near certain where a rung still fails one frame in ten (§2), and the second
   review's bolder climb lost 10–25 % on weak, echo and clean paths.
4. **So the candidates to weigh first** act on what a try measured rather than on what the guess
   assumes: G3 (the verdict on the try's own frames), G4 (no doubled wait after a near miss, and a
   rung rated better tried at once), G5 (a try's frames not aged by the current rung's air before
   the verdict), and G2 or the family offset's gain (the guess corrected faster by frames that keep
   decoding). G1 only as a margin learned per family, if at all.
5. **The start and the pace of the climb matter as much as the verdict.** On the 2300 Hz Good
   path the best rung is one to three above C1's first choice, and a 16 kB message ends after one
   or two tries (§3). A candidate that only fixes near misses (G3, G4) will not get there in that
   time. The prior's margin (G1) or the family offset's gain (G2) sets where C1 starts, so those
   two count here too, checked against the 500 Hz Moderate path, where the rung above loses about
   half its tries. Gate 2's criteria should hold a short transfer on a strong path as well as a
   long one.
6. **The bar for Gate 2.** Against hindsight a controller cannot reach zero: at 500 Hz the best
   rung moves from draw to draw by more than the rungs differ. The best single rung held on every
   draw is the fairer bar, and on it the 2300 Hz Good path has 23 % to win and the 500 Hz
   Moderate path 4–8 %, about as much as C1's two runs of that path differ.
