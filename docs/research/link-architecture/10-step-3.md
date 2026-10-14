# Step 3: the harness A/B, protocol 8 against protocol 7

ADR-0069 §5's third step: the scenario harness, both protocols, ten draws a scenario, judged on
criteria stated before the runs. [06-recommendation.md](06-recommendation.md) §4 states them.
§1 makes each one exact. It was committed, with the judge that applies it
(`research/link_arch/step3.py`, tested in `research/link_arch/tests/test_step3.py`), before
either arm ran. The runs and the verdict follow it.

## 1. Registered before the runs

### 1.1 The arms

* **Protocol 7**: the daemon built from `55d8ac1`, the last commit whose daemon runs protocol 7.
  That is beta.105's engine with ADR-0065…0067, reading configuration schema 11.
* **Protocol 8**: the daemon built from the commit that registers this, reading schema 12.
* **One harness for both**: `tools/session_matrix.py` and `tools/channel_server.py` at that same
  commit, including the VOX radio keyed on audio ([09-step-2.md](09-step-2.md) §3.2).
* **One configuration file per daemon**: each gets the file its own schema reads (`schema_of`).
  A scenario's `turnaround_ms` therefore reaches protocol 8 only. Protocol 7 has no such key.

### 1.2 The runs

* **Which scenarios**: every scenario in `bench/scenarios/*.toml`, 44 of them. The stress set
  (tag `stress`) is among them. `long/` is not.
* **Ten draws each**: variant *k* = 0…9 has seed *s* + 1000*k*, where *s* is the scenario's own
  seed (`--seeds 10`). Both arms use the same seeds.
* **The order**: variant by variant, protocol 7's pass of variant *k* and then protocol 8's, for
  *k* = 0…9. Each pass runs with `--jobs 3` on the same machine, with nothing else running.
  A passing run's audio is dropped (`--drop-passing-audio`). A failing run keeps its record.
* **Reruns**: a run whose runner failed (no `result.json` or `channel.json`) is run again once
  in its arm, and the report says so. Nothing else is run again.
* **The record**: `step3.py collect` folds each pass into one CSV line a run, in
  `research/link_arch/results/step3_p7.csv` and `step3_p8.csv`. These are committed after each
  pair of passes. `step3.py judge` gives the verdict.

### 1.3 The criteria

**C1: no radiated overlap, and no frame keyed into a receiver the protocol was told was
deaf.** This is judged on the protocol 8 arm.

* **(a) Radiated overlaps: zero.**
  * A radiated overlap is a stretch in which the channel server reports two transmitters
    radiating (`rf_overlaps`). It is put down to the transmission each station had on the air
    when it started.
  * Only one kind is left out: an overlap of two *initiations*. An initiation is a
    transmission a station starts on its own, outside any exchange with the other. These are a
    call's first try (`connect`) and a probe's (`probe`), a beacon, a datagram, drive bursts, a
    keying test, and an identifier sent alone. Carrier sense orders these, not a link protocol.
  * Anything else counts, whichever station keyed first: a retry, an answer, anything in a
    session.
* **(b) Deaf hits: zero seconds counted.**
  * The measure is the channel server's `deaf_hits_s`: the other station's signal that arrived
    while this station was keyed in its own silence, held by its VOX, or recovering. Arriving
    while it transmitted is (a).
  * A station's hits count when its radio keeps assumption S4 and not every transmission found
    arriving in its deafness was an initiation.
  * A radio keeps S4 when its deafness after its frames is no longer than its declared
    turnaround or `D + KEY` (0.65 s), whichever is longer. That deafness is the VOX hold plus
    the receiver's recovery, or, for a radio keyed by a line, the daemon's keyed tail (at most
    0.25 s) plus the recovery.
  * Hits into a radio that breaks S4 are reported. `80m-vox-caller-learned-gap-500` has one: an
    undeclared 0.9 s VOX hold.
* **Reported, not judged**: the VOX and guard seconds of the keyed-overlap split (06 §4), and
  the overlaps of two initiations.

**C2: completion at least protocol 7's, scenario by scenario.**

* For each scenario, protocol 8 completes at least as many of its ten runs as protocol 7.
* A run completes when every judgement of the runner holds except `[expect] max_collisions`.
  That count includes keyed silence ([09-step-2.md](09-step-2.md) §3.3), and C1 judges
  overlaps instead.
* The runner's invariants count among the judgements: no daemon exits or panics, no key is held
  over 32 s, and no host is told `CONNECTED` while not listening.

**C3: air time.**

* For each scenario, *r* is the median air time of protocol 8's ten runs over protocol 7's.
  Every run counts, completed or not.
* A class's figure is the geometric mean of its scenarios' *r*. It must be at most 1.15 in
  every class.
* The geometric mean over all 44 must be at most 1.10.
* The classes are disjoint. A scenario goes in the first that matches:

| class | rule | scenarios |
|---|---|---:|
| gateway | tagged `gateway` or `host` | 5 |
| robustness | tagged `robustness`, `machine` or `vox` | 8 |
| band | 20 m and 15 m | 7 |
| 500 Hz | the rest, at 500 Hz | 12 |
| 2300 Hz | the rest | 12 |

**C4: Test rates, protocol 8 at least 0.85 × protocol 7's, pooled.**

* For each scenario with a Test, the median message rate and the median file rate are taken
  over the runs that report one.
* A scenario counts where both arms report the rate in at least three runs.
* The geometric mean of the ratios, protocol 8 over protocol 7, must be at least 0.85, for the
  message and for the file separately.

### 1.4 If a criterion fails

The A/B stops (06 §4).

* **The explanation comes first.** The failure is read from its runs' records: both stations'
  recordings, the transmission tags and the channel's account of the air. It is named as a
  broken assumption (S1–S7) or rule (R1–R12) of [04-formal-spec.md](04-formal-spec.md), with a
  trace, before anything changes.
* **A new rule must justify itself.** It names the invariant it restores, and it moves no
  deadline on undecoded evidence.
* **Then the arm runs again in full.** Protocol 8's whole arm is run again on the changed build,
  not the failing scenario alone.

## 2. The runs

Both arms ran all 44 scenarios at ten seeds each, from 2026-10-13 20:12Z to 2026-10-14 05:26Z.
Nothing had to be run again: every run left its record. The CSVs are
`research/link_arch/results/step3_p7.csv` and `step3_p8.csv`.
`uv run python research/link_arch/step3.py judge research/link_arch/results/step3_p7.csv
research/link_arch/results/step3_p8.csv` prints the per-scenario table.

### 2.1 The verdict: Step 3 fails

| criterion | holds | value |
|---|---|---|
| C1a radiated overlaps (protocol 8) | **no** | 17, in 8 of 440 runs |
| C1b deaf hits counted (protocol 8) | **no** | 3.2 s |
| C2 completion, scenario by scenario | **no** | four scenarios short (below) |
| C3 air time, every class ≤ ×1.15 | yes | 2300 Hz ×1.018, 500 Hz ×1.042, band ×1.006, gateway ×1.056, robustness ×0.856 |
| C3 air time, pooled ≤ ×1.10 | yes | ×0.995 |
| C4 Test message rate ≥ ×0.85 | yes | ×1.053 over 12 scenarios |
| C4 Test file rate ≥ ×0.85 | yes | ×1.124 over 12 scenarios |

The scenarios short on C2, completed of ten:

| scenario | protocol 7 | protocol 8 | why (§3) |
|---|---:|---:|---|
| `80m-poor-500-pactor-signalink` | 10 | 5 | ordinary answers lost (§3.4) |
| `40m-rtty-contest-2300` | 9 | 6 | the abort's DISC held (§3.3), a call lost (§3.7) |
| `80m-leave-together-500` | 9 | 7 | a slow weak path (§3.5) |
| `80m-floor-top-start-500` | 10 | 9 | a cut header held the DISC (§3.6) |

### 2.2 The two arms side by side

| | protocol 7 | protocol 8 |
|---|---:|---:|
| runs completed | 418 / 440 | 427 / 440 |
| radiated overlaps counted | 245 (272 s), in 82 runs | 17 (18 s), in 8 runs |
| deaf hits counted | 238 s | 3.2 s |
| deaf hits reported (a radio breaking S4) | 50 s | 36 s |
| keyed into a VOX hold (reported) | 100 s | 36 s |
| keyed inside a guard (reported) | 68 s | 8 s |

* **Every one of protocol 8's 17 overlaps is one of two kinds.**
  * Eleven are ND1J's call into the gateway's session (§3.2, four seeds).
  * Six are the slow station keying 0.27–0.29 s after a stall ended (§3.1, five seeds).
  * In none did the two stations of a session overlap on their own timing.
* **Every counted deaf hit is one of the same two.** The reported ones are the undeclared VOX
  hold of `80m-vox-caller-learned-gap-500`.
* **Protocol 7 fails both VOX-caller scenarios in every seed** (a message does not arrive).
  Protocol 8 completes all twenty. The declared turnaround takes `80m-vox-caller-declared-500`
  from 650 s to 249 s.
* **Where protocol 8 is slower,** the scenario is a weak 500 Hz path or a busy gateway:
  * `80m-floor-top-start-500` ×1.26;
  * `40m-wide-calls-narrow-unprobed` ×1.26;
  * `40m-gateway-busy-when-called-2300` ×1.23;
  * `80m-flutter-500` ×1.20;
  * `80m-asymmetric-500` ×1.15.

  No class bound is crossed.

### Changes to the judge after the registration

* **The channel's station names (variant 0, before variant 1's collect).** The channel server's
  account names each station by the callsign its daemon gave, not by the scenario's key
  (`a`, `b`, …). `deaf_hits` looked the radio up by the name it was handed. It therefore read every
  station as a line-keyed radio with no recovery, and found no peer to put a hit down to.
  Under that reading, `80m-vox-caller-learned-gap-500`'s undeclared VOX hold was counted, though
  §1.3 says it is reported. The judge now maps callsigns to keys as the harness assigns them
  (`station_keys`), and its test is written on a callsign-named account. The rule is
  unchanged. Every collect re-reads all the runs, so the committed CSVs carry the fix from
  variant 1 on.
* **What each station was radiating (variant 1, before variant 2's collect).** An overlap was
  put down to the transmission each station *began last* before it, as `air_account.py` does.
  With three stations, that can be one already over. In `40m-gateway-busy-when-called-2300`
  (seed 1062), a gateway's answer that had ended 0.7 s before was named in place of the call
  that was radiating. The judge now takes only what a station was radiating at the overlap's
  start (`radiating_at`). The rule is unchanged, and this case still counts: one of its two
  transmissions is not an initiation.

## 3. Failures found while the runs went on

Each is read from its runs' records as §1.4 requires. C1 failed with variant 0's first
overlap. The runs go on all the same, and nothing is changed until they end (the author's
decision, 2026-10-13). The protocol 7 baseline is needed whatever happens, and one pass finds
every kind of failure at once. Then protocol 8's arm runs again in full.

### 3.1 A slow station keyed late on a clock behind the air (C1, `80m-slow-machine-500`)

* **What happened.** Seed 81: KE4QCM, the slave, whose computer stalls, keyed an answer 0.45 s
  over the master's next header.
* **The record.** The channel's account against KE4QCM's sidecar: at each of its keyings the
  station's clock stood 0.25 s behind the air, the card's latency, through fifteen stalls. At
  this one it stood 2.09 s behind. The stall (466.82–469.32 s of air) cost 0.52 s of audio, which
  the station skipped (`skip_captured`). The 1.84 s the channel kept were still to be processed
  when the station keyed, straight after the pass that ended the stall.
* **The assumption broken: S2.** The keying gate judges `not_after` on the station's clock: the
  audio it has *processed*. Behind it lies everything captured and not yet processed. The gate
  passed an answer that radiated 1.84 s past its slot.
* **What restores it.** The gate's clock has to be the air's: the processed clock plus the
  capture backlog. The card's playback clock keeps real time through a stall, so its lead over
  the processed clock, beyond its steady value, measures the backlog. That holds on a real card
  as on the channel link. Seed 1081 shows the same kind once more (a 0.11 s deaf hit).

### 3.2 A third station's call keyed into a session's gap (C1, `40m-gateway-busy-when-called-2300`)

* **What happened.** Seed 1062: ND1J had waited 108 s for a clear channel to call the gateway,
  busy in a session with KE4QCM. ND1J keyed its 5.4 s floor call 0.13 s after the master's header
  ended. The gateway's granted answer, the master's next header and the gateway's next answer all
  went over it: three radiated overlaps. The two session stations never overlapped each other.
* **The rule.** The registration counts these, since only two initiations are left out.
* **The assumption.** The spec's I1 is about a session's two transmitters, and nothing in
  S1–S7 covers a third station. Its call relies on carrier sense. Between a header and its slot
  the air is quiet for `D + KEY + GUARD` (0.70 s) or more, and the busy detector's hangover does
  not bridge that.
* **What restores it.** The header states its end and the slot it grants. A station outside the
  session that decodes it knows the channel is spoken for until the slot's latest end and the
  master's next key: a reservation, as 802.11's NAV. A rule of that kind names I1 extended to
  every station that hears the session, and it moves no deadline on undecoded evidence.
* **A harder case: a station that does not hear one side** (seed 5062). ND1J keyed its call
  3 s into the master's six-frame transmission, after 37.8 s of waiting: not in a gap but over
  a transmission it should have heard. Its carrier sense did not see the other client. A
  reservation helps only as far as ND1J decodes the gateway's own frames: the answers it grants
  and is granted.

### 3.3 An abort's DISC waited for a clear channel (C2, `40m-rtty-contest-2300`)

* **What happened.** Seeds 19 and 1019: the Test's budget ran out mid-file and the master aborted.
  Its leaving DISC was held for a clear channel in the contest's QRM: 29.4 s in one run, and
  still unsent when the run ended in the other. The called station never heard it and was still
  connected at the end, so the runs did not complete. Protocol 7 failed seed 19 too, but not
  seed 1019.
* **The defect: the daemon's.** `held_for_busy` lets a transmission of the engine's skip the
  hold while the engine is Connected or Disconnecting, or when the transmission has a slot. After
  an abort the engine is Idle, and the master's DISC has no slot, so it was held as if the
  station had started it on its own. It is the end of the session, not a new initiation (09 §2.1:
  "Holds apply only to what a station starts on its own").
* **What restores it.** Only calls and probes, their retries included, wait for the channel.
  Every other transmission of the engine goes without the hold.

### 3.4 Ordinary answers lost on a lopsided path (C2, `80m-poor-500-pactor-signalink`)

* **What happened.** Seed 2008: the 2 kB message reached the slave, but after the scenario's
  300 s budget. Protocol 7 delivered the same seed's in 237 s. There was no overlap.
* **The record.** The slave sent 21 answers, 6 on the tone floor and 15 ordinary. The master read
  all 6 floor answers and 4 of the 15 ordinary ones; it detected 3 more it could not read, and
  missed 8 entirely. That left 11 silent slots, each costing an exchange. The other two seeds had
  4 and 8.
* **The rule.** A header asks for floor answers when the master's last three readings of the
  slave are under the ordinary threshold plus 3 dB, or after **two** silent slots in a row
  (`_send_header`). The DISC already asks after one. The readings come only from frames
  that decoded, so they do not see PACTOR bursts killing 0.43 s answers. A heard floor answer
  then resets the silence, and the next header asks for ordinary answers again.
* **Not a safety failure.** Nothing radiated over anything. The rule chose the family on
  evidence that was always one slot behind.
* **Candidates, to be measured after the runs.**
  * Ask for floor answers after one silent slot, as the DISC does.
  * Keep asking for them while any of the last few slots was silent. ADR-0059 did this for
    protocol 7's acknowledgements.

  Either is judged on the link bench and the harness before it is taken. Neither moves a
  deadline: only the family of a frame changes.
* **Seeds 3008 and 5008** are the same: 10 silent slots in 3008. Protocol 7 completed all of the
  scenario's seeds so far.

### 3.5 A slow weak path (C2, `80m-leave-together-500`)

* **What happened.** Seed 5042, in the second session: a 1 kB message on an NVIS path at about
  −4 dB did not arrive within 300 s. Protocol 8 took 338 s; protocol 7 delivered the same seed's
  in 222 s. Both arms fail seed 1042 the same way.
* **The record.** The slave decoded 21 of the 64 data frames it detected. The other 43 include
  noise detections at rungs the master never sent, which do not count (07 §12). Of the real
  ones, rung 3 decoded 2 of 17 across RV 0, 2 and 3. Rung 2 decoded 11 of 11. Protocol 7 tried
  rung 3 eleven times, never decoded it, and settled on rungs 1–2.
* **Open.** The path is one where the rung chosen decides the time. Whether protocol 8 asks too
  high on such paths, or this seed's fades fell on its rung-3 frames, is read from all ten seeds'
  message times when the runs end, before any rule is named. Seed 6042 failed the same way;
  protocol 7 completed it.
* **Across the ten seeds.** Where both arms delivered, protocol 8 was not slower (median 218 s
  against 232 s). The difference is the tail.
  * At the slave, the three failing seeds have one thing in common: rung 3 (`tone4x100-75`)
    decoded 2 of 13, 1 of 23 and 2 of 17. Rung 2 decoded nearly every frame, and OFDM rung 4
    about half.
  * The model puts rung 3 several decibels *below* rung 4 on NVIS and Moderate paths (ADR-0049's
    note), so a rung 3 that decodes worse than rung 4 is not the path's doing.
  * The failing runs kept their audio. Replaying the slave's recording frame by frame says
    whether those frames decode offline. If they do, the live receiver lost them; if they do
    not, it was the channel and the rate controller.

### 3.6 A header cut before its end held the DISC back (C2, `80m-floor-top-start-500`)

* **What happened.** Seed 6061: the Test's budget ran out while the master was transmitting a
  floor header and three floor data frames. The abort cut the transmission 1.04 s in, inside the
  3.2 s header. The master's DISC went 29 s later, as the run ended, and the slave was still
  connected. Protocol 7 completed the seed.
* **The rule, as built.** A cut transmission is reported to the engine as ending where its
  frames would have (09 §2.2: the other station anchors its slot on what the header stated).
  The engine then waits out the slot that header granted before its DISC.
* **What the record shows.** The slave decoded no header (it never answered), and could not
  have: a frame cut before its end does not decode (S1). Nothing was granted, so there was no
  slot to wait out.
* **What restores it.** A header cut before its own end grants nothing. The transmission is
  reported as ending at the cut, and the DISC goes after the previous slot. A header cut after its
  end still anchors the slot it stated, as now. Neither moves a slot the other station can
  hold.

### 3.7 A call lost in a contest (C2, `40m-rtty-contest-2300`)

* **What happened.** Seed 8019: the Test's call timed out after 120 s, and the called station
  was left connected. Protocol 7 connected on the same seed.
* **The record.**
  * The probe was held 29.7 s for a clear channel. The Test's 30 s probe deadline counts from
    its request, so it reported "no answer" just as the answer arrived. Protocol 7's Test did
    the same.
  * Protocol 8's first try waited out that answer (ADR-0016's rule). Protocol 7's went at once,
    radiated over the answer, and was not heard.
  * The called station accepted both of protocol 8's tries. Both acceptances were lost to the
    contest at the caller.
  * The second try was held 25.6 s for a clear channel, into the contest's longest busy stretch.
  * Protocol 7's tries came every 10 s, and its third acceptance was read.
* **What it is.** Nothing radiated over anything. A hostile channel, carrier sense on a busy
  frequency, and a Test whose deadlines count from a request rather than from the transmission.
  Protocol 8 spaces a call's tries by the acceptance slot it grants, which put its second try
  behind the busy stretch. To be judged after the runs:
  * the Test's probe and call deadlines counted from the transmission, which is shared with
    protocol 7;
  * whether protocol 8's call spacing costs connections. This is read from every seed's call
    times, not from this one.

## 4. What comes next

Each failure is named in §3. The changes, in the order they will be made, each with a test
that fails without it:

1. **The keying gate on the air's clock (§3.1, S2).** The daemon: the gate's `now` is the
   processed clock plus the capture backlog, measured as the card's playback clock's lead over
   it beyond its steady value.
2. **Only calls and probes wait for the channel (§3.3).** The daemon's `held_for_busy` judges a
   transmission by what it is, not by the engine's state.
3. **A header cut before its end grants nothing (§3.6).** The daemon reports such a
   transmission as ending at the cut.
4. **Floor answers after a silent slot (§3.4).** Model first, then the port and the replay.
   Judged on the link bench and on the PACTOR scenario's ten seeds before it is taken.
5. **A reservation from another session's frames (§3.2, I1 for a station outside the
   session).** Model first: what a decoded header or answer of another session says about the
   channel. Then the daemon's carrier sense uses it.
6. **The weak path (§3.5):** the replay decides whether there is anything to change.

Not in this round: the Test's deadlines (§3.7). They are shared with protocol 7, and changing
them for one arm would bias the comparison. They are fixed after Step 3, for both.

Then protocol 8's arm runs again in full, against the same protocol 7 baseline (§1.4).
