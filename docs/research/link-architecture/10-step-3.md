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

*To be written from the runs.*

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
