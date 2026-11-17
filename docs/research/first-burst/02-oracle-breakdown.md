# Where the controller's time goes (the oracle, 2026-11-17)

The author's question after variants I and J (`01-results.md`): if starting the first burst
safer or bolder cannot win overall, where does C1 actually lose time? This is a measurement, not
a proposal. Nothing in the rate controller changed; under the rule in CLAUDE.md ("The rate
controller: no more patches") it is evidence for a review.

## Method

- The daemon of `master` at `3a43f1f` (C1 as shipped; ADR-0128's switch off), built locally.
- `tools/rung_oracle.py` on the 26 scenarios of the Rate oracle workflow, three draws each (the
  scenario's seed, +1000, +2000), run in this session's container: no GitHub workflow.
- Each draw ran the controller's own choice and every pinned rung near the path's edge (the
  list below; 178 rungs, about 612 runs). The *best* rung is the pinned rung that delivered the
  most on that draw; the *regret* is how much more it delivered than the controller did.
- The measure is `rung_oracle.py`'s: the largest message's rate, or a Test's file rate, or the
  host program's send for the Trimode scenario.
- `oracle_breakdown.py` (beside this file) reads both stations' sidecars of each controller run
  and splits the data air (frames × frame length: 5.36 s a tone frame, 1.054 s an OFDM one;
  measurement frames left out) by whether each frame went below, at or above that draw's best
  rung, and how much of it went on copies of blocks already sent.

Rungs pinned:

| scenario | rungs |
|---|---|
| 15m-equatorial-flutter-2300 | 7,8,9,10,11,12,14 |
| 20m-dx-500 | 7,9,10,11,12 |
| 20m-dx-moderate-2300 | 8,9,10,11,12,14,15,16 |
| 20m-polar-path-2300 | 4,5,7,8,9,10,11 |
| 40m-good-2300-rtty-crashes | 9,10,11,12,14,15,16 |
| 40m-good-throughput-2300 | 16,17,18,19 |
| 40m-moderate-2300-vara-qrm | 9,10,11,12,14,15,16 |
| 80m-agc-carrier-outside-500 | 0,1,2,3,4,5,6,7,9,10,11 |
| 80m-flutter-500 | 4,5,6,7,9,10 |
| 80m-lopsided-acks-500 | 3,4,5,6,7,9,10,11 |
| 80m-nvis-disturbed-500 | 0,1,2,3,4,5,6,7,9,10 |
| 80m-nvis-evening-500 | 0,1,2,3,4,5,6,7,9 |
| 80m-poor-500-pactor-signalink | 3,4,5,6,7,9,10,11,12 |
| 80m-qsb-dropout-500 | 6,7,9,10,11,12 |
| 80m-slow-machine-500 | 4,5,6,7,9,10,11,12,13 |
| 80m-start-high-reading-500 | 9,10,11,12,13,14 |
| 80m-start-low-reading-500 | 4,5,6,7,9,10,11,12,13 |
| 80m-steady-mail-2300 | 10,11,12,14,15 |
| 80m-steady-mail-500 | 7,9,10,11,12,13 |
| 80m-strong-moderate-500 | 9,10,11,12,13,14 |
| 80m-thunderstorm-agc-2300 | 9,10,11,12,14 |
| 80m-trimode-client-vanishes-500 | 1,2,3,4,5,6,7 |
| 80m-weak-500 | 2,3,4,5,6 |
| 80m-weak-both-ways-ladder-500 | 1,2,3,4,5,6,7 |
| 80m-weak-wide-test-2300 | 4,5,7,8 |
| quick-awgn-2300 | 15,16,17,18,19 |

## By path class

| class | runs | median regret | data air below best | at | above | sent again |
|---|---|---|---|---|---|---|
| NVIS echo | 12 | +13% | 18% | 41% | 41% | 42% |
| QRM | 9 | +23% | 85% | 12% | 3% | 30% |
| fading | 18 | +18% | 70% | 19% | 11% | 22% |
| fast fading | 9 | +31% | 81% | 14% | 5% | 6% |
| strong | 21 | +7% | 44% | 52% | 3% | 13% |
| weak | 9 | +0% | 17% | 65% | 19% | 18% |
| all | 78 | +14% | 57% | 33% | 9% | 20% |

**57 % of the controller's data air goes at rungs slower than the best fixed one, 9 % at faster
ones.** Its loss, overall, is running too low, not trying too high.

- **Fast fading, QRM and fading run low and stay there.** Two of three polar-path draws sat at
  rung 5 where 9 was best (+160 %, +115 %); an equatorial-flutter draw at rungs 8–10 where 11
  was best (+132 %); the RTTY and thunderstorm paths at 10–11 where 14 was best; the QSB path
  put all its data at rungs 7–11 against a best of 12. The frames mostly decode (6 % of the air
  on copies sent again on fast fading): caution, not loss.
- **NVIS echo goes the other way.** 41 % of its air above the best rung, on OFDM rungs 5–7 the
  7 ms echo kills, and 42 % on copies of blocks sent again.
- **Short strong transfers do not reach the top.** `quick-awgn-2300` stayed at rungs 16–17
  where 18 was best in all three draws (+62 … +97 %): 3 kB ends before the climb.
- **Weak paths are right** (median +0 %).

## What it says about ADR-0128

Variants I and J changed only the first burst. Most of the time lost is after it: a climb too
slow on fast fading, QRM and fading, and on NVIS the opposite fault, tries upward that the echo
kills. No start rule fixes both, which is why both variants traded a gain in one class for a
loss in another. What the numbers point at, for the review: how quickly C1 trusts a rung above
once its frames decode, and a separate guard against OFDM under an echo.

## Caveats

- One draw moves a run by about ±10 points (the harness takes its steps on the wall clock):
  read classes, not single runs.
- The pinned rungs were narrowed to each path's edge; a best rung at the end of a list
  (`80m-trimode-client-vanishes-500` draw 1046, rung 7) may not be the true best.
- That draw is not a data point: every pinned rung from 1 to 6 delivered nothing, so its best
  (rung 7, 21) and its −48 % mean nothing. It is counted in the table as it ran.

## Every run

`ctrl` and `best m` are the controller's and the best rung's measure; `first rung` is each
station's first data rung (a: the caller, b: the called station).

| class | scenario | seed | best | ctrl | best m | regret | data air below/at/above best (s) | sent again (s) | first rung | rungs used |
|---|---|---|---|---|---|---|---|---|---|---|
| NVIS echo | 80m-agc-carrier-outside-500 | 31 | 2 | 28 | 40 | +43% | 15/17/35 | 34 | a:7 | 0 1 2 5 7 |
| NVIS echo | 80m-agc-carrier-outside-500 | 1031 | 2 | 47 | 45 | -4% | 0/27/0 | 5 | a:2 | 2 |
| NVIS echo | 80m-agc-carrier-outside-500 | 2031 | 2 | 39 | 51 | +31% | 8/22/6 | 11 | a:5 | 1 2 5 |
| NVIS echo | 80m-nvis-disturbed-500 | 24 | 1 | 38 | 40 | +5% | 4/0/28 | 13 | a:4 | 0 2 4 |
| NVIS echo | 80m-nvis-disturbed-500 | 1024 | 2 | 43 | 43 | +0% | 0/30/0 | 7 | a:2 | 2 |
| NVIS echo | 80m-nvis-disturbed-500 | 2024 | 2 | 32 | 41 | +28% | 6/31/13 | 22 | a:7 | 1 2 5 7 |
| NVIS echo | 80m-nvis-evening-500 | 6 | 2 | 27 | 49 | +81% | 3/19/30 | 30 | a:6 | 0 1 2 5 6 7 |
| NVIS echo | 80m-nvis-evening-500 | 1006 | 1 | 34 | 39 | +15% | 0/1/32 | 8 | a:2 | 1 2 |
| NVIS echo | 80m-nvis-evening-500 | 2006 | 2 | 39 | 44 | +12% | 8/23/6 | 12 | a:6 | 1 2 6 |
| NVIS echo | 80m-trimode-client-vanishes-500 | 46 | 2 | 42 | 42 | +0% | 0/28/28 | 24 | a:5 b:2 | 2 5 |
| NVIS echo | 80m-trimode-client-vanishes-500 | 1046 | 7 | 40 | 21 | -48% | 40/4/6 | 24 | a:2 b:2 | 1 2 5 7 9 |
| NVIS echo | 80m-trimode-client-vanishes-500 | 2046 | 2 | 30 | 47 | +57% | 11/23/41 | 41 | a:6 b:5 | 0 1 2 5 6 |
| QRM | 40m-good-2300-rtty-crashes | 9 | 16 | 550 | 676 | +23% | 103/0/0 | 31 | a:11 | 9 10 11 |
| QRM | 40m-good-2300-rtty-crashes | 1009 | 14 | 551 | 875 | +59% | 96/0/0 | 21 | a:10 | 10 11 |
| QRM | 40m-good-2300-rtty-crashes | 2009 | 14 | 526 | 689 | +31% | 60/28/0 | 40 | a:14 | 9 10 11 12 14 |
| QRM | 40m-moderate-2300-vara-qrm | 7 | 15 | 930 | 1410 | +52% | 122/5/0 | 51 | a:11 b:11 | 11 12 14 15 |
| QRM | 40m-moderate-2300-vara-qrm | 1007 | 14 | 1135 | 1135 | +0% | 97/18/15 | 42 | a:16 b:14 | 10 11 12 14 15 16 |
| QRM | 40m-moderate-2300-vara-qrm | 2007 | 14 | 985 | 1053 | +7% | 34/45/2 | 39 | a:14 b:10 | 10 11 12 14 15 |
| QRM | 80m-thunderstorm-agc-2300 | 17 | 14 | 723 | 812 | +12% | 110/0/0 | 21 | a:11 | 10 11 12 |
| QRM | 80m-thunderstorm-agc-2300 | 1017 | 14 | 700 | 883 | +26% | 81/0/0 | 8 | a:10 | 10 11 |
| QRM | 80m-thunderstorm-agc-2300 | 2017 | 12 | 982 | 1005 | +2% | 41/7/7 | 5 | a:14 | 11 12 14 |
| fading | 20m-dx-500 | 58 | 10 | 255 | 286 | +12% | 25/105/4 | 26 | a:11 b:10 | 9 10 11 |
| fading | 20m-dx-500 | 1058 | 10 | 217 | 257 | +18% | 141/21/3 | 26 | a:6 b:9 | 6 7 9 10 11 |
| fading | 20m-dx-500 | 2058 | 10 | 218 | 240 | +10% | 147/11/0 | 18 | a:9 b:10 | 6 9 10 |
| fading | 20m-dx-moderate-2300 | 51 | 12 | 916 | 945 | +3% | 17/16/96 | 45 | a:14 b:12 | 11 12 14 15 |
| fading | 20m-dx-moderate-2300 | 1051 | 11 | 876 | 870 | -1% | 36/69/32 | 42 | a:14 b:9 | 9 10 11 12 14 |
| fading | 20m-dx-moderate-2300 | 2051 | 11 | 741 | 870 | +17% | 0/16/67 | 34 | a:14 b:14 | 11 12 14 15 |
| fading | 80m-lopsided-acks-500 | 59 | 4 | 103 | 103 | +0% | 0/30/0 | 1 | a:4 | 4 |
| fading | 80m-lopsided-acks-500 | 1059 | 9 | 83 | 113 | +36% | 38/12/0 | 14 | a:9 | 4 6 7 9 |
| fading | 80m-lopsided-acks-500 | 2059 | 4 | 106 | 106 | +0% | 0/32/0 | 0 | a:4 | 4 |
| fading | 80m-poor-500-pactor-signalink | 8 | 9 | 163 | 184 | +13% | 14/46/4 | 13 | a:9 b:9 | 6 7 9 10 |
| fading | 80m-poor-500-pactor-signalink | 1008 | 9 | 115 | 160 | +39% | 58/16/11 | 18 | a:9 b:10 | 5 6 7 9 10 |
| fading | 80m-poor-500-pactor-signalink | 2008 | 9 | 102 | 149 | +46% | 54/0/0 | 3 | a:7 b:7 | 2 4 5 6 7 |
| fading | 80m-qsb-dropout-500 | 13 | 12 | 167 | 235 | +41% | 247/0/0 | 36 | a:7 | 6 7 9 10 |
| fading | 80m-qsb-dropout-500 | 1013 | 12 | 195 | 231 | +18% | 199/0/0 | 46 | a:10 | 7 9 10 11 |
| fading | 80m-qsb-dropout-500 | 2013 | 12 | 194 | 232 | +20% | 201/0/0 | 52 | a:10 | 7 9 10 11 |
| fading | 80m-slow-machine-500 | 81 | 10 | 180 | 208 | +15% | 66/12/11 | 20 | a:11 | 6 7 9 10 11 |
| fading | 80m-slow-machine-500 | 1081 | 11 | 94 | 155 | +65% | 133/0/0 | 38 | a:10 | 4 5 6 7 9 10 |
| fading | 80m-slow-machine-500 | 2081 | 10 | 129 | 230 | +79% | 85/0/0 | 15 | a:6 | 4 5 6 7 9 |
| fast fading | 15m-equatorial-flutter-2300 | 54 | 10 | 656 | 795 | +21% | 5/12/1 | 1 | a:10 b:8 | 8 10 11 |
| fast fading | 15m-equatorial-flutter-2300 | 1054 | 11 | 380 | 882 | +132% | 27/0/0 | 0 | a:8 b:9 | 8 9 10 |
| fast fading | 15m-equatorial-flutter-2300 | 2054 | 10 | 612 | 795 | +30% | 0/3/13 | 4 | a:11 b:12 | 10 11 12 |
| fast fading | 20m-polar-path-2300 | 52 | 9 | 444 | 491 | +11% | 9/22/4 | 5 | a:9 b:8 | 8 9 10 |
| fast fading | 20m-polar-path-2300 | 1052 | 9 | 187 | 486 | +160% | 15/6/0 | 0 | a:5 b:9 | 3 5 9 |
| fast fading | 20m-polar-path-2300 | 2052 | 9 | 188 | 405 | +115% | 30/0/0 | 1 | a:5 b:5 | 5 7 8 |
| fast fading | 80m-flutter-500 | 21 | 7 | 112 | 162 | +45% | 46/0/0 | 0 | a:4 | 2 4 5 6 |
| fast fading | 80m-flutter-500 | 1021 | 9 | 146 | 155 | +6% | 83/4/0 | 5 | a:7 | 6 7 9 |
| fast fading | 80m-flutter-500 | 2021 | 9 | 138 | 182 | +31% | 95/5/0 | 5 | a:6 | 6 7 9 |
| strong | 40m-good-throughput-2300 | 20 | 19 | 3114 | 3107 | -0% | 39/1/0 | 5 | a:18 b:17 | 17 18 19 |
| strong | 40m-good-throughput-2300 | 1020 | 18 | 1963 | 2807 | +43% | 11/23/8 | 8 | a:19 b:17 | 17 18 19 |
| strong | 40m-good-throughput-2300 | 2020 | 17 | 2771 | 2771 | +0% | 0/34/11 | 8 | a:17 b:19 | 17 19 |
| strong | 80m-start-high-reading-500 | 223 | 13 | 260 | 319 | +23% | 128/0/0 | 30 | a:12 | 7 10 11 12 |
| strong | 80m-start-high-reading-500 | 1223 | 12 | 330 | 326 | -1% | 95/19/11 | 23 | a:13 | 10 11 12 13 |
| strong | 80m-start-high-reading-500 | 2223 | 13 | 325 | 318 | -2% | 126/0/0 | 21 | a:12 | 7 10 11 12 |
| strong | 80m-start-low-reading-500 | 223 | 12 | 171 | 348 | +103% | 77/0/0 | 1 | a:4 | 4 5 6 7 9 10 |
| strong | 80m-start-low-reading-500 | 1223 | 10 | 289 | 315 | +9% | 4/110/15 | 21 | a:10 | 9 10 11 12 |
| strong | 80m-start-low-reading-500 | 2223 | 12 | 294 | 347 | +18% | 90/3/0 | 13 | a:9 | 9 10 11 12 |
| strong | 80m-steady-mail-2300 | 1112 | 12 | 1169 | 1180 | +1% | 0/109/5 | 2 | a:12 b:15 | 12 14 15 |
| strong | 80m-steady-mail-2300 | 2112 | 12 | 1107 | 1180 | +7% | 54/61/5 | 2 | a:11 b:14 | 11 12 14 |
| strong | 80m-steady-mail-2300 | 3112 | 12 | 1169 | 1181 | +1% | 0/109/5 | 2 | a:12 b:15 | 12 14 15 |
| strong | 80m-steady-mail-500 | 1113 | 11 | 410 | 430 | +5% | 47/267/8 | 51 | a:9 b:11 | 9 10 11 12 |
| strong | 80m-steady-mail-500 | 2113 | 11 | 410 | 430 | +5% | 47/267/8 | 51 | a:9 b:11 | 9 10 11 12 |
| strong | 80m-steady-mail-500 | 3113 | 11 | 418 | 430 | +3% | 54/254/8 | 44 | a:10 b:11 | 10 11 12 |
| strong | 80m-strong-moderate-500 | 141 | 12 | 334 | 382 | +14% | 106/2/0 | 13 | a:11 | 10 11 12 |
| strong | 80m-strong-moderate-500 | 1141 | 14 | 349 | 384 | +10% | 91/9/0 | 25 | a:14 | 10 11 12 13 14 |
| strong | 80m-strong-moderate-500 | 2141 | 11 | 330 | 345 | +4% | 79/13/0 | 6 | a:11 | 7 10 11 |
| strong | quick-awgn-2300 | 1 | 18 | 1244 | 2449 | +97% | 11/0/0 | 2 | a:17 b:16 | 16 17 |
| strong | quick-awgn-2300 | 1001 | 18 | 2308 | 3750 | +62% | 9/0/0 | 0 | a:16 b:16 | 16 |
| strong | quick-awgn-2300 | 2001 | 18 | 2286 | 3810 | +67% | 9/0/0 | 0 | a:16 b:16 | 16 |
| weak | 80m-weak-500 | 18 | 3 | 67 | 86 | +28% | 0/9/9 | 5 | a:4 | 3 4 |
| weak | 80m-weak-500 | 1018 | 3 | 77 | 77 | +0% | 0/9/7 | 3 | a:4 | 3 4 |
| weak | 80m-weak-500 | 2018 | 3 | 77 | 86 | +12% | 0/13/4 | 2 | a:3 | 3 4 |
| weak | 80m-weak-both-ways-ladder-500 | 1015 | 4 | 43 | 83 | +94% | 13/12/16 | 16 | a:4 | 2 3 4 5 |
| weak | 80m-weak-both-ways-ladder-500 | 2015 | 4 | 72 | 72 | +0% | 2/23/0 | 6 | a:4 | 3 4 |
| weak | 80m-weak-both-ways-ladder-500 | 3015 | 4 | 91 | 99 | +9% | 18/3/0 | 2 | a:3 | 3 4 |
| weak | 80m-weak-wide-test-2300 | 1023 | 5 | 174 | 174 | +0% | 0/19/0 | 0 | a:5 | 5 |
| weak | 80m-weak-wide-test-2300 | 2023 | 5 | 175 | 175 | +0% | 0/19/0 | 0 | a:5 | 5 |
| weak | 80m-weak-wide-test-2300 | 3023 | 5 | 175 | 175 | +0% | 0/19/0 | 0 | a:5 | 5 |

