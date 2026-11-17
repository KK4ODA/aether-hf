# ADR-0129: The station scans its channels for the program that asks, and never leaves one a caller may be on

**Status:** accepted for building, 2026-11-17. The author agreed the design and §7's four
answers the same day (§8). Proposed earlier the same day in answer to the author's question of how
RadioPost Gateway should scan, as Winlink gateways do with RMS Trimode. The timing is estimated on
the model's own call in `docs/research/scanning/00-timing.md`, and the harness judges the design
(§6) before it ships. RadioPost's side, with drafts for RP-4 and RP-5, is its
`docs/design/03-scanning.md` (wish W50).

## 1. Context

**Trimode's way.** A Winlink gateway scans with RMS Trimode. The program tunes the radio itself
and turns the modem's `LISTEN` off and on every 3.5 s or so, deaf for about 0.5 s each time. It
does this on one channel too. Through Aether's VARA port that design has cost five fixes, and it
still loses calls:

* ADR-0051: a `CONNECTED` said while the program's `LISTEN` is off is ignored. The soak then found
  one crossing a `LISTEN OFF` on the wire, and the adapter learned the program's beat.
* ADR-0096, ADR-0100, ADR-0103: `PENDING` first; a held `CONNECTED` that waits no longer than the
  program's scan; the acceptance's `PTT ON` before `CONNECTED`. Each stopped a "No PTT … hung"
  abort or a session the program never took up.
* 2026-11-07: a probe went unanswered because it landed in one of the program's `LISTEN OFF` gaps.
* The calling bench: 85 of 90 first decoded calls fell in the Trimode stand-in's deaf second
  (`docs/research/calling/01-results.md` §3.1). The callsign fixed where the tries fell against
  the scan, so every draw repeated the miss (§3.4).

The cause is one design choice. The scanner is a separate program, and it can only say "don't
answer now". It cannot see a frame arriving, and it cannot see audio still in the receiver from
the last channel. On some modems `LISTEN OFF` ends a call under way without a word to the other
station (Mercury's does, RadioPost's RP-4 §9.6).

**RadioPost Gateway today** listens on one dial. It tunes away for a partner's window and back
(RP-4 §4.1, through `frequency.set`). Its record can list several channels (RP-5 §4.4), but it
listens on only one of them. A client that calls another listed channel is never answered.

**Aether today:**

* `frequency.set` (`Station::tune_to`) refuses during a transmission, a session, a call, a probe,
  a Test, or while something that belongs to this dial is queued. It does not refuse while a frame
  is arriving.
* A dial move of 10 kHz or more makes the busy detector learn the band again. Nothing else
  restarts.
* The receiver keeps its lookback, and a floor frame is final only 18 symbols (0.72 s) after it
  ends (ADR-0009 §8). So a call heard on one dial can be decoded after a move, and answered on the
  next dial.

**The call** (ADR-0126) is two tries by default. From the engine, the same on both airs (`00-timing.md`
§1):

* A both-ways try is 6.41 s: the OFDM data frame (1.05 s), then tone-24 (5.36 s).
* Tries start 11–13 s apart, and 26–34 s before the last try (the pause).
* Two tries find a gateway scanning two to eight channels 26–63 % of the time (`00-timing.md` §4).

## 2. Decision (proposed)

### 2.1 The station steps the radio, the program says where

A program owns the plan: which channels, and when to stop scanning for calls of its own.
RadioPost Gateway is one such program; any control-API client can be. The station owns the
mechanism, and runs it on its run loop: the thread that hears frames, answers calls and keys the
radio. That thread already knows everything that should keep the radio where it is. Nothing uses
`listen` to scan: answering stays on throughout.

### 2.2 The control API

| Method or event | What it does |
|---|---|
| `scan.start {dials}` | Starts a scan, or replaces this connection's. `dials` is 2 to 11 distinct dials in hertz, the first the **home** dial. Answers `{dials, dwell_s, linger_s, scan_s}` (§2.3, §2.4). Refusals: `unsupported` when the keying interface cannot tune; `regulatory` for a dial where the rules would refuse this station's answer, with the dial named; `bad_params`; `claimed` when another connection's scan runs. |
| `scan.stop` | Stops this connection's scan. The radio goes to the home dial once nothing holds it where it is. |
| `status.scan` | `null`, or `owner` and `app` (as `claims`), `dials`, `at_hz`, `dwell_s`, `linger_s`, `scan_s`, `held` (why it may not move now, in a sentence, or `null`), `lingering_until_ms`, `hops`, and `dropped` (dials left out, and why). |
| `scan` event | Started, changed (a dial dropped) or stopped, with `why`: `asked`, `connection closed`, `tuned by another program`, `knob`, or `radio`. Each move is also the `dial` event it is today. |
| `connect {…, call_s}` | Optional. The call's tries keep starting for at least `call_s` seconds (§2.5). The answer carries `tries`. |

### 2.3 The rules of a move

1. **Move only when the dial is free.** All of `tune_to`'s conditions apply, and three more:
   * no frame announced and not yet reported (decoded or refused);
   * no identifier owed for a setup on this dial (ADR-0055's 30 s);
   * no linger running (rule 5).
2. **A move is a capture loss.** The audio from the tune command until the radio's change has
   come through the sound card is dropped the way a capture gap is (`Station::skip_captured`,
   `Air::lose_captured`). That is `set_frequency_hz`'s own time, the card's input latency, and
   the radio's settling: a
   constant measured on the bench (§6 step 2; the timing model assumed 0.5 s in all). The
   receiver starts again after it, so no frame begun on one dial is decoded on the next.
3. **The dwell is computed, not set.** It is the longest gap between two tries' starts before the
   pause, in the station's own call (both plans, any number of tries), plus rule 2's deafness:
   13.5 s today. A visit that long always holds the start of a try while a call is going on. A
   shorter dwell locks in phase with the tries: 3.5 s across three channels catches 0.66 of calls,
   and jitter only lifts that to 0.72 (`00-timing.md` §3). A longer dwell lengthens the cycle for
   nothing.
4. **A frame heard holds the dial until it is reported**, and an answer, a session and its closing
   identifiers hold it as they do today.
5. **Linger.** An announced frame that comes to nothing, or a probe this station answered, keeps
   the station on the dial until no frame has been announced for the call's longest gap plus
   rule 2's deafness (34.5 s today). It is ended early by a frame decoded as another station's:
   a call to another callsign, a frame of a session not this station's, a beacon. A visit lasts at
   most the longest call the engine makes (fifteen tries, about 207 s). On a path where each part
   of a try decodes half the time, lingering takes the calls caught from 79–89 % to 98–99 %
   (`00-timing.md` §5).
6. **After a session or an answered call or probe**, the scan goes on from that dial with a
   linger. A caller often calls again.
7. **The list is judged once, at `scan.start`.** Each dial is judged as this station's answer would
   be under its control mode (`regulatory_check`, direction `respond`). A dial outside the rules
   refuses the whole list, by name. The gate still judges every transmission at the dial the
   radio reports.
8. **Every move is verified.** `set_frequency_hz` reads the dial back. A dial the radio will not
   take is dropped from the scan and named in a `scan` event, once. Below two dials the scan
   stops at the home dial (`why`: `radio`).
9. **The operator wins.** These stop the scan and tell its owner, who does not start it again by
   itself:
   * a `frequency.set` from any other connection, the panel's dial list included;
   * a dial reading, taken after a move was confirmed, that is not the scan's dial (the knob
     turned).
10. **A scan belongs to its connection** (ADR-0106). When the connection closes, the scan stops
    and the radio goes home once the dial is free.
11. **One bandwidth: the station's own.** A 2 300 Hz station answers a 500 Hz call by moving
    (ADR-0026). A 500 Hz station is reached by a 2 300 Hz caller that moves after its tries
    (ADR-0058). The scan never changes the bandwidth.

### 2.4 What the program publishes

The program publishes `scan_s` = (dials − 1) × dwell: how long a caller's tries must keep starting
for one of them to fall in a visit to its channel. It is RP-5's `scan_s` (RadioPost's draft).

### 2.5 `call_s` on `connect`

Model first. `GrantedEngine.connect(…, call_s=None)` takes the fewest tries, from 2 to 15, whose
tries before the pause start over at least `call_s`, and never fewer than `call_tries`. For a
gateway scanning *n* channels that is *n* + 2 both-ways tries, the fewest that catch 99 % of calls
in the timing model (`00-timing.md` §4). A `call_s` beyond fifteen tries' reach gets fifteen; the
answer's `tries` says so. Fifteen tries reach eleven channels, which is why `scan.start` takes at
most eleven.

### 2.6 What the station shows and records

* **The panel's dial row:** *Scanning for RadioPost Gateway: 7.101 MHz (2 of 4)*, with **Stop**,
  and the dial list held while it runs.
* **A recording** carries each move as a `hop` event with the samples dropped, so that
  `aetherd --replay` starts its receiver again in the same place. Each dial is recorded as today.

## 3. What it removes

* **The listen dance.** Trimode's design never reaches RadioPost: nothing tunes the radio from
  outside the station, and RP-4 needs no "do not answer while I tune". The VARA port's holds for a
  scanning program (ADR-0051, 0096, 0100, 0103) stay: Trimode still uses them.
* **Channels nobody listens on.** RadioPost's side lists in a gateway's record only the dials it
  listens on: the scan's, or its one dial (RadioPost gap G21).
* **Jitter.** It was suggested before the timing was measured, and it does not help
  (`00-timing.md` §3).
* **Settings.** None are added. The dwell and the linger come from the call's timing, and the list
  is the program's.

## 4. Alternatives considered

* **Trimode's way through RPMI** (the gateway tunes and cycles `listen`). Rejected: §1.
* **The gateway tunes and leaves `listen` alone, relying on `tune`'s refusals.** Better, but still
  blind to a frame arriving: RPMI has no event for one, and adding one would put the modem's
  receiver timing in a host's hands. It still answers a call heard before the move on the new dial,
  and every other modem would need the same logic again. It is the fallback should a modem without
  a scan of its own ever gain a dial.
* **A short dwell that stops when it hears something.** At three channels it catches 0.66 of
  calls (phase lock), and it depends on finding a frame from its middle.
* **Several receivers in one passband** (the receive mixer's offset, ADR-0079). This hears 500 Hz
  channels inside one sideband passband at once, with no moves at all. It does not cover channels
  on different bands, which is what gateways scan. Not now.
* **A second radio** (RadioPost's wish W26, built). No scanning at all; the best answer where a
  station has the radio.
* **A call that keeps trying until it hears the gateway.** This breaks ADR-0126's cap. `call_s`
  is bounded, and the program asks for it only when the gateway says it scans.

## 5. Consequences

* **A call to a scanning gateway is longer.** With plan H it takes *n* + 2 tries. At four
  channels that is six tries, and a call nobody answers lasts about 91 s instead of 42 s.
  ADR-0126's cap is relaxed only for calls whose program asks.
* **A caller connects later.** The median, to the end of the try that was caught, is 6–46 s at two
  to eight channels, against 1–6 s on a gateway that only listens. A caller who keeps the default
  two tries misses it 37–74 % of the time.
* **The busy detector learns each channel again** after a move of 10 kHz or more, about 3.5 s.
  Only the station's own calls and probes wait for it, and the program stops the scan before it
  calls.
* **CAT traffic** is one dial command per dwell. Trimode's habit of resending the whole radio
  setup is not needed: the dial is read back.
* **The antenna** must take every channel without a tuning cycle (a resonant antenna, or a tuner
  with memory). The manual will say so beside the scan.
* **The rules.** Every dial is judged at the start. Nothing is keyed by a move.
* **The interface.** New in the control API: `scan.start`, `scan.stop`, `status.scan`, the `scan`
  event and `connect`'s `call_s`. No wire change, no configuration key, no schema bump.

## 6. How it is judged (registered before any run)

**Step 0, the harness learns dials.** The channel server keeps a dial for each station and joins
only the stations on the same dial. A move is modelled as deafness. New scenario steps: `scan WHO
d1,d2,…`, and a caller placed on one of the dials. The Trimode stand-in stays as it is.

**Step 1, model first.** `call_s` in `GrantedEngine`, with a granted vectors session that uses it.

**Step 2, the port.** The scan in the station, the control API, the panel's line, the recording's
`hop` event and the replay. The radio's settling time is measured on the bench, at the radio.

**Step 3, the harness.** A narrow miss goes to the author with its trace.

| | Criterion |
|---|---|
| **S1, wrong dial** | No frame is keyed on a dial other than the one the frame it answers was heard on. No acceptance follows a move for a call heard before it. Zero in every run. |
| **S2, held** | The scan never moves while a frame is announced and not reported, while an answer is queued, while a session or call is up, or while an identifier is owed for the dial (the station's own events say). Zero. |
| **S3, a good path** | Callers at +10 dB, on AWGN and ITU Moderate, at 2 300 and 500 Hz, against 2, 3 and 4 channels, each calling with the rule's `call_s`: at least 9 of 10 draws connect in each cell. |
| **S4, a weak path** | At the SNR where one try decodes about half the time (the calling bench), 3 channels: at least 8 of 10 draws connect. Linger on does at least as well as linger off, paired by draw. |
| **S5, no harm** | No scan running: every scenario × 3 draws stays within `scenarios.csv`'s ranges. |
| **S6, the end** | The owner's connection closing stops the scan, and the radio is on the home dial once the dial is free. Another connection's `frequency.set` stops it and the owner is told. Zero failures. |
| **S7, each part earns its place** | Linger off fails S4. The hold on an announced frame off fails S1 or S3. The move as a capture loss off fails S1 (a planted call that ends just before a move). A part that fails nothing when taken out goes. |

**Step 4, the air.** The home gateway scans two channels, and the mobile calls it on each.

## 7. For the author

1. **The operator wins** (rule 9): a `frequency.set` from another program, or the knob, stops the
   scan. The other way round, the scan would refuse them and the operator would have to stop it
   first.
2. **At most 11 channels**, which is fifteen tries' reach. A lower limit would cap the time a call
   nobody answers holds the channel: four channels give 91 s.
3. **The panel** could start a scan of the station's own, for an operator waiting for calls on
   several channels. Proposed: not now.
4. **Step 0** is harness work that must come before any criterion can run. The air could come
   first, with the harness after.

## 8. The author's answers (2026-11-17)

1. **The operator wins** (rule 9): another program's `frequency.set`, or the knob, stops the scan,
   and its owner is told.
2. **At most 11 channels.**
3. **The panel does not start scans of its own**, for now.
4. **The harness first**: step 0 and the criteria before the air.

## 9. What was built (2026-11-17, steps 1, 2 and 0)

**Step 1, the model and the port.** `GrantedEngine.call_offsets(plan)` gives each try's start of a
call nobody answers, from the helpers that schedule it, and a test holds it to what the engine does
through the simulator to a microsecond (both airs, every plan, with a turnaround and an answer gap).
`call_gaps()` gives the dwell's and the linger's gaps: 13.01 s and 34.43 s today. `tries_lasting`
and `connect(…, call_s)` are §2.5. The granted vectors gained the `call-lasting` session and a
`call_timing` table, and a planted fault in the level step fails both suites.

**Step 2, the station** (`core/aetherd/src/station/scan.rs`). §2.3's rules as written, with these
choices made in building it:

* `tune_to`'s checks are one function, `Station::dial_movable`, which the scan shares, so the two
  cannot disagree.
* A move restarts the receiver with `Air::restart_front_end` and `Air::lose_captured(0)`, at once
  and again `move_deaf_s()` later (the card's 0.25 s and `MOVE_SETTLE_S`, 0.25 s, until the bench
  measures it). The busy detector relearns on a move of 10 kHz or more, as any dial reading does.
* **The panel's Stop** is `scan.stop` from a connection with no claim: as the panel acts on any
  program's call or session (ADR-0106), it stops any program's scan, as the operator. The radio
  stays where it is, and `why` is a sixth word, `operator`.
* `connect`'s answer carries `tries`: the call's plan, which depends on how the station called was
  heard (ADR-0126 §2.3).
* **The linger starts from a frame the station heeds** (an announcement past the acquisition gate
  the station's own receive window uses), from its end, and a frame decoded as another station's
  ends it. Reading back the first build, it started from any frame reported undecoded; on a quiet
  band the receiver reports a noise candidate every few seconds, and the scan would have lingered
  on each dial to its cap.
* A recording carries each move as a `hop` event (`dial <hz> deaf <s>`), and `aetherd --replay`
  restarts its receiver at both points.
* The panel's dial box has a **Scanning** row (the dial, its place, the program; *held* or
  *lingering*; **Stop**), and the dial list is held while a scan runs. The manual (1.3.0) says so.

**Step 0, the harness.** The daemon on a channel server greets `AEC4` and has a radio
(`channel_link::ChannelRig`) that keys nothing and has a dial: it tells the server where it tuned
(message 8), and the server tells it where the radio is (the scenario's `dial_hz`, and `knob WHO HZ`
commands). The server joins only stations on one dial, runs every path on silence otherwise so its
draws stay as they were, counts collisions per dial, records each station's dials and each
transmission's, and judges S1 from the air (`dial_faults`): an acceptance with no call on its dial
in the minute before it, or after its station moved since that call. `session_matrix.py` judges
those as invariants on every run, and has the steps `scan WHO D1,D2,…` (a program's connection held
for the scan's life, `tools/control_ws.py`), `scan-stop WHO`, `knob WHO HZ`, `tune WHO HZ` and
`connect call_s=N|scan`. Every scenario before dials runs with every station on one dial, as before.

The first run (`bench/scenarios/40m-scanning-gateway-2300.toml`, three dials, AWGN +10 dB): the
gateway left 7.101 MHz for 10.145 MHz at 14.0 s, the caller's first try at 7.7 s went unheard and
its second at 19.1 s was answered at 26.2 s; no dial fault, no collision. The harness's quick set
passed unchanged.

