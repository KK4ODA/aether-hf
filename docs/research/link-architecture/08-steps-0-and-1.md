# Steps 0 and 1: the harness can attribute overlaps; the model engine

ADR-0069 §5 builds protocol 8 in gated steps, each judged on criteria stated before it ran. This
document reports the first two:

* **Step 0** makes the scenario harness say what each collision was.
* **Step 1** is the model engine, `aether_model.link.granted.GrantedEngine`, judged where the
  decision was measured.

The data is in `research/link_arch/results/`, with the commands to reproduce it at the end.

## 1. Step 0: the harness attributes overlaps

### 1.1 What was built

* **The daemon tags every transmission it keys.** The tag says:
  * the engine's cause (the timer or call that set it going, `Action::Transmit.cause`);
  * the role and state;
  * the frames, by kind;
  * how long the daemon held it;
  * whether it carried the identifier.

  It goes to the channel server as message type 5 (`channel_link.rs`).
* **The channel server separates what was radiated from what was only keyed.** It keeps both
  intervals (`tools/channel_server.py`). It splits every overlap into three:
  * radiated over radiated;
  * a VOX hold (keyed, radiating nothing);
  * a guard (inside a receiver's recovery).

  It also counts what each station's deaf receiver missed.
* **`tools/air_account.py` names each radiated overlap** by the transmission keyed second, over
  the other. The names are: request, leaving, answer held, answer over a repeat, answer over a
  burst, repeat over an answer, both initiating, other.

### 1.2 Protocol 7 against the ADR-0068 build

The 43 scenarios of `bench/scenarios/`, one draw each, through the daemon of each build.

| | protocol 7 | ADR-0068 build |
|---|---|---|
| scenarios passing | 42 / 43 | 43 / 43 |
| the old collision counter | 29 | 28 |
| radiated overlaps | 23 (21.5 s) | 8 (11.6 s) |
| keyed in a VOX hold | 2.6 s | 4.7 s |
| inside a guard | 7.1 s | 3.9 s |
| answer over a burst | 15 | 0 |
| answer over a repeat | 4 | 1 |
| repeat over an answer | 1 | 4 |
| leaving | 3 | 1 |
| answer held | 0 | 1 |
| both initiating | 0 | 1 |

The old counter mixed keyed silence with radiation, so it could not tell the two builds apart:
29 against 28. Split, the radiated overlaps fall from 23 to 8.

* **What ADR-0068 removed** is the early answer ("answer over a burst"), as the review predicted.
* **What remains** are the classes the review said ADR-0068 leaves:
  * a sender's repeat meeting a late answer, either way round;
  * the end of a session;
  * an answer the daemon held;
  * two stations initiating.

Protocol 7 failed one scenario, `40m-rtty-contest-2300`.

**Caveats.**

* **One draw per scenario.** Protocol 7's 13 overlaps of `40m-long-attachment-cwid-2300` are
  most of its total.
* **A loaded machine.** Both builds ran at a load average near 10 on four cores, alongside the
  model work. Late decodes are what that load produces, and they are what protocol 7 is
  sensitive to.
* **The harness is not deterministic.** Its daemons run on threads.

**Owed for Step 0's gate.** It is not closed:

* distributions over 10 seeds on an idle machine;
* air-time stepping.

### 1.3 Decode latency, measured

07-protocol-8.md §11 sets `D_ORD` and `D_FLOOR` from the harness. Every frame in the recordings
of §1.2's runs has its end on the air (`end_s`) and the moment the daemon reported it (`t_s`),
both on the receiver's audio clock (`research/link_arch/latency.py`). The latency is the
difference.

| air | family | kind | frames | median | 99th percentile | most |
|---|---|---|---:|---:|---:|---:|
| 2 300 Hz | ordinary | control | 1 254 | 0.122 s | 0.147 s | 0.151 s |
| 2 300 Hz | ordinary | data | 4 040 | 0.122 s | 0.147 s | 0.151 s |
| 2 300 Hz | floor | control | 207 | 0.130 s | 0.150 s | 0.156 s |
| 2 300 Hz | floor | data | 235 | 0.129 s | 0.156 s | 0.156 s |
| 500 Hz | ordinary | control | 1 049 | 0.123 s | 0.149 s | 0.151 s |
| 500 Hz | ordinary | data | 3 743 | 0.123 s | 0.148 s | 0.151 s |
| 500 Hz | floor | control | 672 | 0.117 s | 0.156 s | 0.157 s |
| 500 Hz | floor | data | 695 | 0.130 s | 0.156 s | 0.157 s |

Frames that decoded and frames that did not are counted together; they do not differ.
`80m-slow-machine-500`'s frames, from the station whose computer stalls, are no later: a
station's clock is the audio it has processed, so a stall does not show in this measure. Stalls
are what gap levels are for.

* **Both budgets are 0.30 s.** The tone floor reports a frame as soon as it ends. The
  provisional 0.65 s was the settling of the OFDM floor family (ADR-0009 §8), which the tone
  floor replaced. 0.30 s leaves 0.14 s over the slowest report for what the harness does not
  charge.
* **What the harness does not charge.** It runs in lockstep with the audio, so a pass of the
  receiver costs no air time. Outside it:
  * the receiver runs at 0.07× real time on a session's replay, its slowest pass 35 ms;
  * rendering takes 4–20 ms for a control frame, 0.06–0.11 s for six OFDM frames and
    0.10–0.18 s for four tone frames, about twice that under load (`aether-phy`'s transmitter,
    timed per rung);
  * a card delivers what it hears 10–100 ms late, which the harness does not model.
* **`KEY` stays 0.35 s**, the daemon's lead and the harness's card. It leaves nothing for
  rendering, so Step 2 renders a slot's data frames before the header granting it is decoded,
  and only the answer after (07 §10). A station still slower asks for a level.
* **Changed in the model.** `granted.DECODE_BUDGET_S` is `(0.30, 0.30)`. The review's campaign
  is unchanged: it gives every arm its profile's budgets (`setups.KNOBS`). The link bench moves
  by its own noise. Its AWGN cells, where timing alone moves the result, are 0.3–2 % faster.

## 2. Step 1: the model engine

### 2.1 What was built

* **`model/aether_model/link/granted.py`, `GrantedEngine`.** 2 308 lines with docstrings;
  `LinkEngine` is 3 038. It implements 07-protocol-8.md with the full interface of its §8:
  * calls;
  * probes, across bandwidths;
  * callsign lists;
  * the Test's ladder;
  * the regulatory ceiling and the key-time limit;
  * moving between airs;
  * the identifier's placement;
  * aborts and left sessions;
  * the called-again rule;
  * the version and bandwidth `ignored` events.

  Its data path is `LinkEngine`'s, ported rule by rule: selective repeat, HARQ with slot
  inference, re-encoding (ADR-0038), the RV sequence, and ADR-0020's rules for what a failure
  teaches.
* **Its frames in `frames.py`.** The 7-byte `GrantFrame`, the 19-byte protocol 8 connect body
  (turnaround and identifier length), and the level bits of the capability byte.
* **`TwoStationSim` gains the review simulator's station model**, each part behind its own
  argument and off by default:
  * a keying lead;
  * a decode latency;
  * a receiver's recovery;
  * false detections, some trusted;
  * the keying gate (`not_after`), with the engine told of a drop;
  * the identifier's air time;
  * a record of what each station radiated, with `overlaps()`.

  Protocol 7's tests run unchanged.
* **`model/tests/test_granted.py`.** 40 tests (57 cases) for 07 §9:
  * I1: no two transmitters radiating together, under the station model, on both airs, at three
    SNRs and in a 48-session randomised sweep;
  * I2–I4: exact delivery through outages; a frame of a closed slot moving nothing;
    announcements moving nothing;
  * L1–L5: a dead path ending at both ends; a slow slave asking for a level; a call reaching a
    station that decodes 1.35 s late;
  * every kept interface item.
* **A research arm** (`research/link_arch/engines/granted.py`), so the review's simulator,
  campaign and scenarios run the model engine itself. **`tools/bench_link.py --engine
  granted`** does the same for the link bench.

### 2.2 The gate (ADR-0069 §5.2)

Stated before the runs:

* no radiated overlap;
* field profile ×1.10 ± 0.05 of protocol 7;
* chat ≤ ×1.05;
* completion no lower in any profile.

**The review's campaign.** 5 040 sessions per arm, on the same seeds as the review's:

* 2 airs;
* 4 channel classes;
* −6 … +12 dB;
* bulk, exchange and chat;
* 5 station profiles;
* 6 trials.

| profile | completed: protocol 7 / ADR-0068 / prototype / **model** | radiated overlaps a session: protocol 7 / ADR-0068 / **model** |
|---|---|---|
| ideal | 100.0 / 100.0 / 100.0 / **100.0 %** | 0.24 / 0.18 / **0** |
| field | 99.4 / 98.6 / 100.0 / **100.0 %** | 0.80 / 0.79 / **0** |
| field-asym | 98.4 / 98.4 / 99.3 / **99.7 %** | 1.16 / 0.80 / **0** |
| hostile-c | 95.9 / 95.2 / 99.9 / **100.0 %** | 2.60 / 0.81 / **0** |
| hostile | 93.9 / 95.3 / 99.8 / **100.0 %** | 3.12 / 1.50 / **0** |

Completion time against protocol 7, paired by seed. The figure is the geometric mean of the
ratio over the runs both completed, with a 95 % bootstrap interval.

| profile | bulk | exchange | chat |
|---|---|---|---|
| ideal | ×1.059 [1.043, 1.074] | ×1.067 [1.055, 1.081] | ×1.060 [1.052, 1.067] |
| **field** | **×1.093 [1.074, 1.112]** | **×1.089 [1.072, 1.105]** | **×1.036 [1.027, 1.045]** |
| field-asym | ×1.112 [1.090, 1.134] | ×1.112 [1.092, 1.133] | ×0.989 [0.978, 1.000] |
| hostile-c | ×1.051 [1.029, 1.074] | ×1.047 [1.030, 1.065] | ×0.977 [0.968, 0.986] |
| hostile | ×1.096 [1.072, 1.120] | ×1.178 [1.153, 1.203] | ×0.981 [0.967, 0.994] |

No paired cell completes fewer sessions than protocol 7. The closest is field-asym exchange,
333 against 333 of 336. **The gate passes on every criterion.**

Against the prototype it was built from (`gs`), the model engine is within the prototype's
interval or faster in every field cell: bulk ×1.093 against ×1.103, exchange ×1.089 against
×1.101, chat ×1.036 against ×1.035.

**The 14 adversarial scenarios** (16 runs each, 6 dB at 2 300 Hz and 3 dB at 500 Hz, ITU Good).
The model engine completes 224 of 224 with no radiated overlap. Protocol 7 completes 203 of 224
with radiated overlaps in every scenario. Its worst cases:

* outage: 3/16;
* phantom storm: 10/16, 10.5 overlaps a run;
* slow computer: 12.6 overlaps a run.

Medians are 16-run figures (`results/scenarios_granted.csv`).

**The link bench** (`bench_link.py --fading --floor-cap`). 8 kB, every class, −6 … +12 dB, both
airs, 4 trials:

* both engines complete 112 of 112 on each air;
* the model engine takes ×1.171 (2 300 Hz) and ×1.157 (500 Hz) as long, with the measured
  decode budgets (§1.3; ×1.165 and ×1.159 with the provisional ones).

The pipe charges no keying latency, so it charges protocol 7 nothing for answering at once. It
charges protocol 8 the decode budget, keying latency and guard that every one of its gaps
reserves. That reservation is the protocol's safety margin, and the review's simulator, which
does charge keying, is the measure of its cost (07 §9).

### 2.3 What building it found

Each finding is in 07-protocol-8.md §12. Three changed results.

* **False detections stepped the rate down the floor.** On the slave, a phantom heard between
  transmissions became a failed "transmission" of its own. Frames heard during a long floor
  header joined the transmission after it. Both counted against the path (ADR-0020's rule is
  for frames that are there).
  * The rule now: a frame that did not decode counts only where its transmission's opening
    control frame says the data frames are (family, slot, count).
  * Phantom storm: 735 s against the prototype's 413 on one trial before; the scenario's
    16-run median is now 229 s against 278.
* **A single reading chose the control family.** A station asked for ordinary control frames
  on the strength of the last frame it decoded. On a fading path an ordinary frame decodes
  only at a peak, so that reading was biased high.
  * A slave 6 dB under its master asked for ordinary headers at every peak and missed the next
    two. Its data waited three exchanges each time, and sessions ran to the 1 500 s horizon.
  * Now a station asks for ordinary frames only when its last `family_readings` readings all
    clear the margin.
  * Swept 1–4 on the full campaign: 1 and 2 each leave a cell completing less than protocol 7;
    3 and 4 do not. **3 is the default.**
* **A floor-only slave could not send.** A header's rung both recommends the slave's rung and
  sizes its grant. A slave held to the tone floor by its ceiling was granted ordinary frames
  its own never fitted.
  * An answer with no data after it now says, in `data_floor`, that its data can go only on the
    floor.
  * The master then caps the rung it recommends and sizes grants by.

### 2.4 What Step 1 does not show

* **The review's simulator is the model's world.** It has the assumed decode latencies,
  keying leads and phantom rates; a real receiver, card and radio are Step 3's and Step 4's.
* **Two constants are still provisional.** `D_ORD` and `D_FLOOR` are measured (§1.3). `KEY`
  and `SLACK` are the harness's own figures until Step 2's daemon and Step 4's hardware measure
  rendering and CAT keying.
* **Pipe and harness are not the same comparison.** The link bench's ×1.16 and the review's
  ×1.09 measure different things (§2.2). The harness A/B (Step 3) is the one that counts for a
  release.

### 2.5 The family margin, swept

A station asks for ordinary control frames only when its last three readings clear the ordinary
control threshold by `answer_floor_margin_db`. The link bench's one outlier cell, 500 Hz AWGN at
−3 dB (×1.69), is that margin at work: the readings sit 1.5 dB over the threshold, short of the
3 dB margin, so every control frame goes on the floor. Protocol 7 sends ordinary ones there.

The margin was swept on the review's campaign before the port fixes it (arms `granted_a<x>`,
`results/granted_margin.csv.gz`). Against the 3 dB default, by profile, over all kinds:

* **0 dB:** 1–3 % faster on average. Completes fewer sessions than protocol 7 in field-asym
  exchange (three lost, two gained).
* **1.5 dB:** 1–2 % faster. Completes one fewer than protocol 7 in the same cell.
* **2 dB:** 0.5–2 % faster, no cell below protocol 7.
* **By cell**, a smaller margin is faster on AWGN at −3 dB (×0.74 at 0 dB on 500 Hz) and slower on
  fading paths near the boundary: Poor −3 dB ×1.40/×1.16/×1.08 at 500 Hz and ×1.22/×1.13/×1.07
  at 2 300 Hz (0/1.5/2 dB); Moderate 500 Hz −3 dB ×1.13–1.23.

**3 dB stays.** A smaller margin wins a steady channel sitting at the boundary and loses on the
fading ones, which are the air. 2 dB passes the gate's rule too, but its gain is inside the
intervals.

## 3. Reproducing

From the repository root, after `uv sync`:

```
# Step 1: the model engine on the review's campaign and scenarios
uv run python -m research.link_arch.campaign --arms granted --bandwidth 500,2300 \
    --channels awgn,good,moderate,poor --snr=-6,-3,0,3,6,9,12 --kinds bulk,exchange,chat \
    --profiles ideal,field,field-asym,hostile-c,hostile --trials 6 --jobs 4 \
    --out granted_main.csv --overlaps-out granted_main_overlaps.csv
uv run python -m research.link_arch.scenarios --arms granted --trials 8 --out scenarios_granted.csv
# the family-readings sweep: arms granted_f1 … granted_f4
# the family-margin sweep: arms granted_a0, granted_a1.5, granted_a2 (3 dB is `granted`)
# decode latency from a harness run's recordings
uv run python research/link_arch/latency.py <runs> [<runs> …]
uv run python -m research.link_arch.campaign --arms granted_f1,granted_f2,granted_f3 ...
# the link bench, either engine
uv run python tools/bench_link.py --engine granted --bandwidth 500 --fading --floor-cap \
    --snr=-6,-3,0,3,6,9,12 --trials 4
# Step 0: the harness, a build's daemon
uv run python tools/session_matrix.py bench/scenarios/*.toml --jobs 2 \
    --daemon <aetherd> --out runs --csv step0.csv
```

The campaign's table generator takes the model arm (`report_tables.py`, arm `granted`).
