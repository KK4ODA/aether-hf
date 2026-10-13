# The experimental comparison

This chapter compares five arms of the link protocol under the same seeded channels, payloads
and station behaviour. It reports:

* the campaign's design and simulator (§1–§2);
* how each overlap of the two transmitters is classified (§3);
* safety, completion, time, chat latency and air per kilobyte (§4–§6);
* the adversarial scenarios (§7);
* how far the simulator agrees with the harness (§8);
* the limitations (§9);
* the methodology the next round needs (§10).

Every number is reproducible from `research/link_arch/` (§11).

## 1. Design

**Arms.** Two are the model's real `LinkEngine`; three are research engines:

| Arm | Engine | What it is |
|---|---|---|
| Protocol 7 | `LinkEngine`, `stated_answers` off | the engine on the air today (beta.105) |
| ADR-0068 | `LinkEngine`, `stated_answers` on | protocol 8 as built at `7546df3` |
| ADR-0068, 6-frame | `engines/stated6.py` | candidate A: six-frame bursts, count 3 = "three or more" |
| Granted slots | `engines/gs.py` | candidate D |
| Fixed cycle | `engines/fc.py` | candidate C |

**Grid.** Each arm runs 5 040 sessions:

* 2 airs (2300 and 500 Hz);
* 4 channels (AWGN and ITU-R F.1487 Good, Moderate and Poor through the calibrated fading
  pipe, P9-6);
* 7 SNRs (−6 to +12 dB in steps of 3);
* 3 workloads;
* 5 station profiles;
* 6 trials.

**Workloads:**

* **bulk** — the caller sends 4 000 bytes;
* **exchange** — Winlink-shaped. Proposals, answers and messages alternate direction, the
  messages are 800–3 000 and 500–2 000 bytes, and each waits a host's reaction time;
* **chat** — 8–14 keyboard lines with thinking time.

The horizon is 1 500 s.

**Pairing.** Every arm runs the same trial number with the same seeds:

* **The same draws, regardless of arm:** the fading realisation, the payload and the engines'
  own seeds.
* **Draws keyed by sender and time:** detection, decoding and phantoms, through a hash, so two
  arms draw alike when they send at the same instant.
* **Draws from a per-station stream:** the station profile's holds, keying jitter, latency
  tails and stalls, seeded by the trial. These drift apart once two arms' frames differ.

Ratios are taken per pair, and reported as geometric means with 95 % bootstrap intervals over
pairs. Raw distributions are given where they matter.

## 2. The simulator

`advsim.py` drives the engines event by event:

* a frame's preamble is detected (or not) when it is in;
* the frame's fate (decoded, failed but trusted, failed untrusted, lost) is decided at its end
  from the fading pipe's effective SNR;
* the result reaches the engine after the station's decode latency.

A station's own transmissions make it deaf, and a frame cut by its own keying is lost. What it
adds over `TwoStationSim` is the station profile:

| Parameter | ideal | field | hostile-c | hostile |
|---|---|---|---|---|
| keying lead beyond what the engine is told (CAT, card), uniform | 0 | 0–0.15 s | 0–0.6 s | 0–0.6 s |
| daemon hold before keying (busy, identifier), engine told | — | 5 %, 0.2–1.5 s | — | 15 %, 0.3–2.0 s |
| answer gap (ADR-0036), the other engine not told | — | — | — | 0.5 s |
| VOX hold / receiver recovery | 0 / 0 | 0 / 0.05 s | 0.15 / 0.15 s | 0.15 / 0.15 s |
| decode latency: base + exponential tail; extra for a floor frame | 0 | 0.15 + 0.08 s; +0.3 s | 0.15 + 0.2 s; +0.5 s | as hostile-c |
| receiver stall per frame | — | 0.5 %, 0.5–3 s | 2 %, 0.5–4 s | as hostile-c |
| phantoms a minute; share trusted | — | 1; 5 % | 4; 10 % | 4; 10 % |

`field-asym` is `field` with the caller hearing 6 dB worse than it is heard.

**What every engine is told.** The figures are the same for all arms, by profile:

* the extra turnaround;
* the jitter its waits must absorb;
* the decode budget, ordinary and floor (`setups.KNOBS`).

The model engine gets them as `turnaround_s`, `ack_margin_s` and `detect_latency_s`. The
prototypes get them as gap, slack and budgets. What `hostile` adds beyond them is deliberately
*not* told to any engine.

## 3. How an overlap is classified

`classify.py` takes every pair of transmissions that radiated at the same instant. It names
the mechanism from the reason each was keyed:

* the engine's call chain and timer;
* what it answered and when that frame ended;
* the hold the daemon applied;
* both engines' beliefs at that moment.

| Class | Meaning |
|---|---|
| `early-answer` | an answer keyed before the end of the transmission it answers |
| `late answer (held)` | an answer keyed after the sender's wait expired, because of a daemon hold or keying lead |
| `late answer (decoded-late)` | the same, because the decode itself was late |
| `late answer (holder-timeout)`, `(holder-moved-on)` | an answer that met the holder's repeat, or its next transmission |
| `answer-answer` | both answering, each believing the other transmitted last |
| `dual-holder` | both believed they held the turn |
| `unsolicited (request)`, `(irs_disc)`, `(leave)` | a transmission no frame solicited: a turn request, a receiving station's DISC, a left session's DISC |
| `keying` | guard silence only: keyed, not radiating (not counted as RF overlap) |

## 4. Safety

**RF overlaps per session**: mean and 95 % bootstrap interval, the share of sessions with
any, and seconds of overlap per session.

| Arm | ideal | field | field-asym | hostile-c | hostile |
|---|---|---|---|---|---|
| Protocol 7 | 0.24 [0.20, 0.28]; 13 %; 0.23 s | 0.80 [0.73, 0.87]; 44 %; 1.32 s | 1.16 [1.07, 1.24]; 57 %; 2.27 s | 2.60 [2.43, 2.76]; 74 %; 4.94 s | 3.12 [2.94, 3.31]; 79 %; 5.76 s |
| ADR-0068 | 0.18 [0.15, 0.20]; 14 %; 0.14 s | 0.79 [0.73, 0.86]; 46 %; 1.11 s | 0.80 [0.74, 0.87]; 45 %; 1.52 s | 0.81 [0.75, 0.87]; 51 %; 1.52 s | 1.50 [1.41, 1.60]; 68 %; 2.29 s |
| ADR-0068, 6-frame | 0.17 [0.14, 0.20]; 14 %; 0.14 s | 0.76 [0.70, 0.82]; 46 %; 1.05 s | 0.80 [0.73, 0.86]; 47 %; 1.46 s | 0.86 [0.79, 0.92]; 51 %; 1.41 s | 1.44 [1.35, 1.53]; 68 %; 2.13 s |
| **Granted slots** | **0** | **0** | **0** | **0** | **0** |
| Fixed cycle | 0 | 0 | 0 | 0 | 0.001 |

The fixed cycle's single overlap is outside its contract: keying jitter beyond the slack it was
told. Granted slots drops such a transmission at the gate (0.86 drops per session in `field`,
2.74 in `hostile`), which costs a slot but no overlap.

**The distribution under the implicit-turn engines**, as the share of sessions with 0, 1, 2
and ≥ 3 overlaps:

| Profile | Protocol 7 | ADR-0068 | ADR-0068, 6-frame |
|---|---|---|---|
| field | 56 / 23 / 10 / 10 % (max 6) | 54 / 23 / 15 / 8 % (max 6) | 54 / 26 / 13 / 7 % (max 6) |
| hostile-c | 27 / 17 / 16 / 40 % (max 17) | 49 / 31 / 12 / 8 % (max 6) | 49 / 31 / 11 / 9 % (max 7) |

**Ambiguous ownership** is the time both engines believed they held the turn, as seconds per
session and the share of sessions with any:

| Profile | Protocol 7 | ADR-0068 | ADR-0068, 6-frame |
|---|---|---|---|
| field | 0.91 s, 36 % | 0.62 s, 26 % | 0.57 s, 25 % |
| hostile-c | 1.56 s, 39 % | 1.08 s, 34 % | 1.11 s, 35 % |

The prototypes have no standing slave authority, so this measure is zero for them *by
construction*. Their safety evidence is the RF-overlap count and the model check.

**Mechanisms** (per session, field profile; the full table is in `results/tables.md`):

| Mechanism | Protocol 7 | ADR-0068 | 6-frame |
|---|---|---|---|
| unsolicited (request) | 0.227 | 0.392 | 0.388 |
| late answer (held) | 0.157 | 0.280 | 0.271 |
| late answer (decoded-late) | 0.244 | 0.067 | 0.063 |
| late answer (holder-moved-on) | 0.007 | 0.035 | 0.017 |
| answer-answer | 0.031 | 0.014 | 0.013 |
| early answer | 0.134 | 0.000 | 0.000 |
| dual-holder | 0.001 | 0.000 | 0.003 |

## 5. Completion

These are the share of sessions that delivered everything intact, and the share that also
closed cleanly with a DISC exchange at both ends:

| Arm | ideal | field | field-asym | hostile-c | hostile |
|---|---|---|---|---|---|
| Protocol 7 | 100.0 / 98.9 % | 99.4 / 95.9 % | 98.4 / 93.5 % | 95.9 / 93.2 % | 93.9 / 87.7 % |
| ADR-0068 | 100.0 / 99.0 % | 98.6 / 95.9 % | 98.4 / 95.8 % | 95.2 / 92.8 % | 95.3 / 92.3 % |
| ADR-0068, 6-frame | 100.0 / 99.2 % | 97.9 / 95.7 % | 98.2 / 96.4 % | 95.4 / 92.5 % | 94.5 / 92.0 % |
| **Granted slots** | **100.0 / 100.0 %** | **100.0 / 100.0 %** | **99.3 / 99.1 %** | **99.9 / 99.9 %** | **99.8 / 99.0 %** |
| Fixed cycle | 100.0 / 100.0 % | 99.8 / 99.8 % | 97.6 / 97.0 % | 99.9 / 99.9 % | 98.6 / 96.1 % |

**How the sessions that did not complete ended:**

| Arm | Not completed | How they ended |
|---|---|---|
| Protocol 7 | 124 | mostly a link timeout at one or both ends (85), 39 still up at the horizon |
| ADR-0068 | 125 | 117 link timeouts at both ends |
| ADR-0068, 6-frame | 140 | 126 link timeouts at both ends |
| Granted slots | 10 | all still up and delivering at the 1 500 s horizon. All at −6/−3 dB on ITU Good, exchange workload, asymmetric or hostile profile: slow, not lost |
| Fixed cycle | 41 | 33 still up at the horizon, 8 "no response" |

## 6. Time

### 6.1 Completion time against protocol 7

These are paired geometric-mean ratios with 95 % bootstrap intervals, over sessions both arms
completed. Below 1 is faster.

| Profile | Workload | ADR-0068 | ADR-0068, 6-frame | Granted slots | Fixed cycle |
|---|---|---|---|---|---|
| ideal | bulk | ×1.019 [1.005, 1.034] | ×0.994 [0.984, 1.003] | ×1.064 [1.047, 1.080] | ×1.272 [1.249, 1.295] |
| ideal | exchange | ×1.047 [1.037, 1.058] | ×1.016 [1.007, 1.024] | ×1.065 [1.051, 1.080] | ×1.578 [1.551, 1.606] |
| ideal | chat | ×1.010 [1.005, 1.015] | ×1.010 [1.006, 1.015] | ×1.093 [1.084, 1.102] | ×1.487 [1.466, 1.508] |
| **field** | bulk | ×1.146 [1.126, 1.167] | ×1.105 [1.086, 1.125] | ×1.103 [1.083, 1.123] | ×1.338 [1.312, 1.365] |
| **field** | exchange | ×1.217 [1.197, 1.240] | ×1.133 [1.115, 1.150] | ×1.101 [1.085, 1.119] | ×1.626 [1.594, 1.658] |
| **field** | chat | ×1.012 [1.005, 1.019] | ×1.009 [1.002, 1.016] | ×1.035 [1.026, 1.044] | ×1.479 [1.458, 1.502] |
| field-asym | bulk | ×1.133 [1.113, 1.154] | ×1.082 [1.063, 1.101] | ×1.116 [1.094, 1.140] | ×1.353 [1.321, 1.384] |
| field-asym | exchange | ×1.223 [1.204, 1.244] | ×1.179 [1.158, 1.198] | ×1.123 [1.103, 1.142] | ×1.752 [1.701, 1.800] |
| field-asym | chat | ×1.015 [1.006, 1.025] | ×1.017 [1.007, 1.028] | ×0.997 [0.985, 1.009] | ×1.525 [1.495, 1.555] |
| hostile-c | bulk | ×1.115 [1.088, 1.141] | ×1.010 [0.990, 1.029] | ×1.057 [1.036, 1.078] | ×1.214 [1.185, 1.243] |
| hostile-c | exchange | ×1.134 [1.115, 1.153] | ×1.070 [1.053, 1.087] | ×1.063 [1.046, 1.080] | ×1.457 [1.423, 1.492] |
| hostile-c | chat | ×0.981 [0.971, 0.991] | ×0.983 [0.973, 0.993] | ×0.976 [0.966, 0.987] | ×1.371 [1.350, 1.391] |
| hostile | bulk | ×1.164 [1.138, 1.189] | ×1.086 [1.066, 1.107] | ×1.119 [1.094, 1.144] | ×1.675 [1.626, 1.726] |
| hostile | exchange | ×1.268 [1.240, 1.295] | ×1.149 [1.127, 1.171] | ×1.208 [1.183, 1.233] | ×1.894 [1.844, 1.946] |
| hostile | chat | ×0.975 [0.963, 0.985] | ×0.980 [0.968, 0.990] | ×0.969 [0.955, 0.983] | ×1.506 [1.477, 1.536] |

**Against ADR-0068 in the field profile**, granted slots is ×0.962 [0.943, 0.982] on bulk,
×0.905 [0.889, 0.922] on exchange and ×1.024 [1.016, 1.034] on chat.

**The distribution of the per-pair ratio** (field, against protocol 7) is given as
p10 / p50 / p90, with the share of pairs where the arm was faster:

| Workload | ADR-0068 | ADR-0068, 6-frame | Granted slots | Fixed cycle |
|---|---|---|---|---|
| bulk | 0.94 / 1.13 / 1.44 (20 %) | 0.93 / 1.09 / 1.35 (27 %) | 0.92 / 1.08 / 1.36 (24 %) | 1.09 / 1.33 / 1.72 (5 %) |
| exchange | 1.01 / 1.20 / 1.49 (9 %) | 0.97 / 1.12 / 1.36 (21 %) | 0.94 / 1.09 / 1.33 (21 %) | 1.27 / 1.63 / 2.02 (1 %) |
| chat | 0.95 / 1.00 / 1.10 (46 %) | 0.95 / 1.00 / 1.09 (48 %) | 0.95 / 1.03 / 1.15 (32 %) | 1.27 / 1.44 / 1.83 (0 %) |

**Bulk goodput** (field), as bit/s p10 / p50 / p90 over channels and trials. A failed session
counts as 0.

| Air | SNR | Protocol 7 | ADR-0068 | Granted slots | Fixed cycle |
|---|---|---|---|---|---|
| 2300 | −6 | 74 / 112 / 150 | 52 / 92 / 122 | 58 / 106 / 146 | 60 / 92 / 132 |
| 2300 | 0 | 110 / 149 / 220 | 91 / 117 / 196 | 102 / 134 / 193 | 82 / 115 / 166 |
| 2300 | +6 | 289 / 385 / 543 | 259 / 364 / 544 | 262 / 356 / 505 | 197 / 275 / 406 |
| 2300 | +12 | 665 / 844 / 939 | 691 / 827 / 898 | 592 / 731 / 893 | 430 / 518 / 621 |
| 500 | −6 | 42 / 60 / 78 | 34 / 46 / 63 | 43 / 53 / 73 | 32 / 50 / 65 |
| 500 | 0 | 62 / 73 / 152 | 56 / 61 / 118 | 55 / 64 / 134 | 50 / 56 / 110 |
| 500 | +6 | 99 / 127 / 308 | 92 / 120 / 282 | 101 / 137 / 264 | 73 / 108 / 221 |
| 500 | +12 | 251 / 314 / 467 | 253 / 313 / 450 | 208 / 285 / 430 | 173 / 243 / 363 |

**Where granted slots loses.** It gives up 13 % (2300 Hz) and 9 % (500 Hz) at +12 dB, where
its per-transmission header and reserved gaps are a larger share of a short, fast session.
Below +6 dB it is ahead of ADR-0068 and behind protocol 7.

### 6.2 Where the time goes

`where_time_goes.py` splits each session up to its last delivery into three parts:

* **radiated air**;
* **keying overhead**: keyed, not radiating;
* **dead air**: nobody keyed.

The median ratio is over sessions all arms completed (bulk and exchange, 3 trials, the full
channel and SNR grid). `gs0` is granted slots told the truth about an ideal station: no decode
budget and no slack. It is safe only there.

| Profile | Arm | Time / protocol 7 | Radiated / protocol 7 | Dead air per change of direction | Changes of direction |
|---|---|---|---|---|---|
| ideal | protocol 7 | 1.000 | 1.000 | 0.65 s | 34 |
| ideal | ADR-0068 | 1.028 | 1.004 | 0.55 s | 41 |
| ideal | granted slots | 1.047 | 1.038 | 0.63 s | 37 |
| ideal | granted slots, no reserve | 1.026 | 1.037 | 0.46 s | 37 |
| field | protocol 7 | 1.000 | 1.000 | 0.99 s | 35 |
| field | ADR-0068 | 1.171 | 1.182 | 0.80 s | 48 |
| field | granted slots | 1.087 | 1.057 | 1.08 s | 37 |

**Reading.**

* **ADR-0068's penalty is radiated air, not dead air.** It sends 18 % more: repeats after late
  answers and lost statements, and 37 % more changes of direction (four-frame bursts).
* **Granted slots' penalty is mainly radiated air too.** Its 5.7 % more air is the header per
  master transmission (0.434 s ordinary). Dead air per change of direction is 0.09 s more than
  protocol 7's.
* **In the ideal world, reserving for the worst decode costs about two points** (gs against
  gs0).
* **Protocol 7 is fastest** because it answers whenever it decodes, and its sender's waits
  stretch over anything it hears arriving. That is precisely what produces its overlaps
  (§4).

**A calibration found by this decomposition.** The prototype first sent its first header on
the tone floor, which is 3.2 s. The acceptance already carries how the slave heard the call.
Using that report, as every later header uses the slave's answers, made granted slots
×0.966 [0.953, 0.979] faster on field bulk and ×0.979 [0.968, 0.990] on exchange. Chat did not
change. The tables above include it.

### 6.3 Chat and air

**Chat line latency.** These are the median over sessions of each session's median line, the
p90 over sessions of each session's slowest line, and the duty (keyed seconds over session
seconds, both stations):

| Profile | Arm | Completed | Median line | p90 of slowest | Duty |
|---|---|---|---|---|---|
| field | protocol 7 | 330/336 | 7.7 s | 38.3 s | 0.400 |
| field | ADR-0068 | 325/336 | 8.5 s | 36.5 s | 0.429 |
| field | ADR-0068, 6-frame | 319/336 | 7.9 s | 35.4 s | 0.424 |
| field | granted slots | 336/336 | 8.3 s | 44.3 s | 0.368 |
| field | fixed cycle | 336/336 | 17.5 s | 95.3 s | 0.302 |
| hostile-c | protocol 7 | 304/336 | 10.2 s | 53.9 s | 0.406 |
| hostile-c | ADR-0068 | 302/336 | 10.2 s | 45.1 s | 0.405 |
| hostile-c | granted slots | 336/336 | 9.8 s | 50.1 s | 0.351 |

Granted slots' tail is longer than ADR-0068's in `field` (44 against 37 s for the slowest
line). A line typed at the slave on an idle link waits for an open window. It completes every
session and keys 13 % less.

**Keyed seconds per delivered kilobyte** (both stations, field):

| Workload | Protocol 7 | ADR-0068 | ADR-0068, 6-frame | Granted slots | Fixed cycle |
|---|---|---|---|---|---|
| bulk | 42.7 | 53.2 | 47.1 | 46.5 | 53.3 |
| exchange | 47.0 | 59.6 | 50.9 | 48.7 | 61.4 |
| chat | 118.5 | 141.6 | 136.1 | 113.4 | 153.5 |

## 7. The adversarial scenarios

Each scenario runs 16 sessions per arm (both airs, 8 trials) on ITU Good: 6 dB at 2300 Hz,
3 dB at 500 Hz, field profile. A scenario's adversary acts on specific frames, by kind and
ordinal, so it does the same thing to every arm. Cells give completed sessions, RF overlaps
per session and the median completion time.

| Scenario | What the adversary does | Protocol 7 | ADR-0068 | 6-frame | Granted slots | Fixed cycle |
|---|---|---|---|---|---|---|
| ack-lost | every 3rd acknowledgement from B unheard | 16, 1.19, 385 s | 16, 0.56, 418 s | 16, 0.38, 367 s | 16, **0**, 332 s | 16, 0, 404 s |
| data-lost | A's 2nd and 5th bursts entirely unheard | 16, 0.44, 286 s | 15, 0.31, 209 s | 16, 0.25, 222 s | 16, **0**, 263 s | 16, 0, 269 s |
| handover-lost | B's first data transmission (the change of direction) unheard | 16, 1.31, 197 s | 16, 1.25, 242 s | 16, 1.31, 209 s | 16, **0**, 202 s | 16, 0, 278 s |
| handover-ctl-lost | only its opening control frame unheard | 16, 1.00, 195 s | 16, 0.94, 218 s | 15, 0.75, 189 s | 16, **0**, 183 s | 16, 0, 315 s |
| late-decode | B decodes every 4th transmission 1.5 s late | 16, 6.94, 195 s | 16, 0.56, 324 s | 16, 0.62, 232 s | 16, **0**, 258 s | 16, 0, 330 s |
| last-undecodable | the last frame of A's first 8 bursts detected, never decoded | 16, 0.50, 397 s | 16, 0.25, 397 s | 16, 0.19, 368 s | 16, **0**, 372 s | 16, 0, 543 s |
| partial-bursts | every other frame of A's first 8 bursts undecodable | 16, 0.50, 338 s | 16, 0.38, 435 s | 16, 0.19, 434 s | 16, **0**, 342 s | 16, 0, 513 s |
| chat-handover-lossy | 30 % of the control frames opening a data transmission lost | 16, 1.62, 277 s | 16, 2.19, 289 s | 16, 1.50, 289 s | 16, **0**, 299 s | 16, 0, 410 s |
| phantom-storm | 30 false detections a minute at both stations | 10, 10.50, 164 s | 13, 0.56, 256 s | 12, 0.69, 444 s | **16**, **0**, 278 s | 16, 0, 581 s |
| asymmetric-10dB | A hears B 10 dB worse | 16, 1.81, 450 s | 15, 1.12, 540 s | 16, 1.75, 590 s | 16, **0**, 454 s | 15, 0, 891 s |
| slow-computer | B's decodes 1.35 s late, beyond every budget | 15, 12.62, 278 s | 15, 15.62, 553 s | 15, 17.62, 439 s | **16**, **0**, 314 s | **0**, 0, — |
| ptt-jitter | B's keying 0–1.0 s later than it was told | 16, 1.50, 206 s | 16, 3.88, 315 s | 16, 3.56, 283 s | 16, **0**, 263 s | 15, 0, 393 s |
| fade-ramp | +12 → −8 dB and back every 4 min, 12 kB | 15, 3.06, 825 s | 14, 0.50, 1 354 s | 12, 0.62, 503 s | **16**, **0**, 771 s | 3, 0, 558 s |
| outage | the band dead from 60 to 100 s | 3, 2.00, 152 s | 2, 0.81, 472 s | 1, 0.62, 275 s | **16**, **0**, 315 s | 16, 0, 392 s |

**The outage row is a parameter effect, not architecture.** The prototypes derive their link
timeout from four of their longest exchanges (about 244 s). Given the same timeout
(`legacy_lt`, `stated_lt`), protocol 7 completes 16/16 and ADR-0068 15/16, at 262 and 295 s.

**The adversarial list in the brief, and where each item is covered:**

| Item | Covered by |
|---|---|
| ACK lost but data received | `ack-lost` |
| Data lost but the sender assumes the slot completed | `data-lost` |
| Both sides time out near the same instant | `handover-lost`; TLA `ImplicitTurn_forfeit_perfect`. In granted slots only the master has a timer that keys |
| TURN lost or delayed | `handover-lost`, `handover-ctl-lost`, `chat-handover-lossy`, `late-decode` |
| TURN duplicated | **not covered** as a scenario; HARQ repeats exercise duplicates of data frames only |
| Preamble detected, frame not decoded | `last-undecodable`, `partial-bursts`, `phantom-storm` |
| Asymmetric SNR | `asymmetric-10dB`, the `field-asym` profile |
| Long, variable decoder time | `slow-computer`, `late-decode`, the profiles' tails and stalls |
| PTT release or key-up delays | `ptt-jitter`; the profiles' lead jitter, VOX and recovery |
| Alternating 500 and 2300 Hz | **not covered**: a bandwidth change happens between sessions (ADR-0026). It belongs on the harness |
| High loss during rate-ladder transitions | `fade-ramp` (partial). The Test's pinned ladder is not simulated |
| Consecutive bursts with partial reception | `partial-bursts` |

## 8. Does the simulator agree with the harness?

The brief's harness measured ADR-0068 against protocol 7: messages 17 % slower, files 26 %
slower, and airtime ×0.94. The simulator's field profile gives:

* completion time ×1.146 (bulk) and ×1.217 (exchange). A rate 17 % lower is time ×1.20, and
  26 % lower is ×1.35;
* keyed air per kilobyte +25 %.

Completion time is consistent in direction and magnitude. Air disagrees in sign. Two harness
artefacts can explain that disagreement:

* the harness's `air_s` is its run time, settle waits included (harness audit §5), not keyed
  time;
* its collision count includes guard silence.

The harness's collision halving (37 → 19) appears in the simulator only for the hostile
profiles: −52 to −69 % per session. In the field profile it is −1 %. The early answers ADR-0068
removes are replaced by more requests and late answers. **This is consistent but untested.**
Classifying the harness's own collisions needs the instrumentation of §10.1.

## 9. Limitations

**What the simulator does not reproduce:**

* **Not the real PHY in the loop.** Frame fates come from the calibrated fading pipe (EESM per
  frame class, P9-6), as `bench_link.py --fading` uses. The detection model is a logistic
  around each family's threshold.
* **Phantoms are synthetic.** The rates come from the ADRs' field and harness descriptions, not
  from a measured false-alarm process. A trusted phantom is drawn at a fixed share.
* **The profiles are judgement.** `field` was set from the harness's radio model, the daemon's
  constants and the field ADRs. `hostile` is deliberately past them.

**What the prototypes leave out:**

* **They are research code, not ports.** Each shares the model's HARQ, rate controller and
  frame formats, and adds its own control frame (seven bytes, `Ctl`).
* **They omit the Morse identifier, beacons, datagrams, probes and the Test session.** The
  model engines carry those paths, but the workloads do not exercise them.
* **The horizon (1 500 s) truncates a few very slow sessions.** Every granted-slots
  non-completion is one of them.

**What the statistics do not establish:**

* **The results are paired by seed, and the intervals are over pairs.** They do not include
  variation between channel models or profiles; those are shown separately.
* **No claim of parity with VARA follows from any of this.**

## 10. Methodology for the next round

### 10.1 The scenario harness

The scenario harness (`tools/channel_server.py`, `tools/session_matrix.py`) is the closest
thing to the air short of the air. It needs four changes before its collision counts can
decide anything:

1. **Attribute every transmission.** The daemon sends, with each `MSG_PLAY`, a tag with:
   * the burst id;
   * the frame kinds and control subtypes;
   * the engine timer that keyed it;
   * the role;
   * the hold applied.

   The channel server logs the tag with the on-air interval, the *radiated* mask and the
   sidecar's clock origin.
2. **Count overlaps at each receiver**, after path delay. Split them into guard (keyed silence),
   VOX hold and radiated. Classify radiated overlaps with this review's classes.
3. **Report distributions.** Air time, Test rates and overlaps per scenario as p10 / p50 / p90
   over at least 10 seeds. Include failing runs, with failed goodput as 0, not only survivors.
4. **Remove the runner's wall-clock jitter.** Step in air time through the channel server, so a
   fixed seed reproduces.

### 10.2 Randomised simulation

`campaign.py` as here: every arm on the same seeds. To decide between two engines:

* **at least 6 trials** per cell of the grid;
* the paired geometric-mean ratio with a bootstrap interval over pairs;
* per-pair ratio quantiles;
* completion and clean-end shares.

**A change counts as an improvement** only if:

* its ratio's interval excludes 1 in the profile that matters (`field`);
* it does not worsen any profile beyond its own interval;
* it does not lower completion.

The fading pipe's per-frame draws should be keyed by block, RV and direction (harness audit
§4). The pairing then survives arms that send different frames.

### 10.3 Hardware in the loop

Two stations, two radios on a dummy load through an attenuator, both computers' real sound
cards and keying, a common clock by GPS-disciplined NTP. The protocol for each radio pair:

* **The keying interface's real timing.** Measure PTT assert → RF and audio end → RF release
  with a scope on the RF sample port: CAT, RTS, VOX (SignaLink), CM108.
* **Sessions of each workload** at fixed attenuation steps.
* **Per transmission:** keyed and radiated intervals, from the RF detector, and the daemon's
  tag. Overlaps are counted at the RF level.
* **Stalls on purpose:** a CPU-load generator on one machine, to exercise gap levels and
  forfeits.

### 10.4 Comparing with VARA HF

The aim is the same conditions for both modems, with every setting reported. Nothing in it
infers VARA's internals.

* **Cable first.** Two computers and two radios into dummy loads, with an attenuator and the
  P9-1 channel simulator (`tools/channel_cable.py`) between the audio ports, or a calibrated
  RF channel simulator.
  * The same payloads (a fixed Winlink message set: 1, 6 and 30 kB with attachments) and the
    same host program (Winlink Express P2P or Pat).
  * The same bandwidth (500 or 2300 Hz; VARA 2750 compared with Aether 2300 only as such).
  * The same drive measured at the RF port.
  * The same ITU-R F.1487 channel and SNR steps: −6 to +12 dB, 3 dB apart.
  * At least 10 sessions per point per modem, **interleaved** (A, B, A, B), so slow drift
    affects both.
* **On the air.** Alternate the modems on one path at 15-minute intervals for an evening,
  with the same stations, antennas, power and dial. Log SNR from both modems' own readouts.
* **Report:**
  * completion share;
  * completion time;
  * goodput distribution;
  * time to the first byte.

  Each modem is judged by what its host program reports; nothing is measured from VARA's
  signal.

  A VARA licence level, version and setting changes the result and must be stated.
* **Criteria, stated before the runs.**
  * "Comparable" means Aether's median completion time within ×1.15 of VARA's, with
    completion share no lower, on Good and Moderate from 0 to +12 dB.
  * Below 0 dB, report without a threshold.
  * Do not claim parity from fewer than 10 sessions per point, or from simulation.

### 10.5 The metrics the brief asks for, and where each comes from

| Metric | Source here | Source on the harness or HIL |
|---|---|---|
| goodput, completion time | `main.csv.gz` (`goodput_bps`, `done_s`) | `session_matrix` result plus tags |
| airtime | `air_a_s`, `air_b_s`; `time.csv` | radiated mask |
| turnaround overhead | dead air per change of direction (`time.csv`) | tags plus RF |
| collisions by type | `main_overlaps.csv.gz` (`class`) | tags classified as §3 |
| retransmissions | engine stats (not in the campaign rows) | sidecars |
| recovery latency after a failed burst | **not measured**: add the time from a lost answer to the next acknowledged frame | tags |
| link failures | `end_a`, `end_b` | `ends` |
| ambiguous ownership | `dual_s` | the engine's role in the tag |

## 11. Reproduction

From the repository root, after `python -m uv sync`:

```
# the main campaign (25 200 sessions, about 6 min on 4 cores)
uv run python -m research.link_arch.campaign --arms legacy,stated,stated6,gs,fc \
    --bandwidth 500,2300 --channels awgn,good,moderate,poor --snr=-6,-3,0,3,6,9,12 \
    --kinds bulk,exchange,chat --profiles ideal,field,field-asym,hostile-c,hostile \
    --trials 6 --jobs 4 --out main.csv --overlaps-out main_overlaps.csv
uv run python -m research.link_arch.report_tables main.csv main_overlaps.csv
# the adversarial scenarios, and the outage with a matched link timeout
uv run python -m research.link_arch.scenarios --out scenarios.csv --trials 8
uv run python -m research.link_arch.scenarios --arms legacy_lt,stated_lt --only outage \
    --out outage_lt.csv --trials 8
# where the time goes
uv run python -m research.link_arch.where_time_goes --out time.csv
# the invariants
uv run pytest research/link_arch/tests -q -o addopts=""
# the model checks (tla2tools.jar from the TLA+ releases)
cd research/link_arch/tla && java -cp tla2tools.jar tlc2.TLC -config GrantedSlots_safe.cfg \
    -workers 4 GrantedSlots.tla
```

The committed results are `research/link_arch/results/`:

* `main.csv.gz` and `main_overlaps.csv.gz`, the campaign's sessions and overlaps;
* `tables.md`, the campaign's tables;
* `scenarios.csv`, the adversarial scenarios;
* `time.csv`, the time decomposition;
* `classify_existing*`, the first classification pass, before the knobs were equalised.
