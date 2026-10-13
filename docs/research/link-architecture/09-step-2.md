# Step 2: protocol 8 in the port and in the daemon

ADR-0069 §5 builds protocol 8 in gated steps. Steps 0 and 1 are in
[08-steps-0-and-1.md](08-steps-0-and-1.md). This document reports Step 2:

* **the port**: `aether_link::granted::GrantedEngine`, checked against the model input by input;
* **the daemon**: `aetherd` runs it, with the keying gate, the engine's own clock of what it
  has on the air, and the Morse identifier where the engine puts it.

Step 3, the harness A/B with pre-registered criteria, comes next.

## 1. The port

### 1.1 What was built

* **Frames** (`frames.rs`): `GrantFrame` and `GrantKind`, the 19-byte connect body with the
  turnaround and the identifier's length, the capability level bits (`with_level`,
  `level_of`). Bit-exact with the model through `link_vectors.json`.
* **The engine** (`granted.rs`): a rule-by-rule port of `aether_model.link.granted`. Its draws
  are the model's: SplitMix64 in both (`Draws`), so a session number and a call's jitter come
  out the same from the same seed.
* **`Action::Transmit`** gained `not_after` (the keying gate's limit), `not_before` (the slot's
  start, for an answer decided ahead of it) and `identifier_s` (the Morse identifier the engine
  placed ahead of the frames). Protocol 7's engine sets none of them.

### 1.2 How it is checked: trace replay

`tools/make_granted_vectors.py` runs the model's engines through 20 sessions and records every
input and every action drained: clean, weak, fading with the station model, outages, a phantom
storm, aborts, probes across bandwidths, identifiers, the Test's ladder, stranded frames, the
HARQ limit, VOX turnarounds. `core/aether-link/tests/granted_vectors.rs` hands the port the same
inputs, answers its decodes from the record — checking the HARQ buffer it combines with — and
compares every action and, after every input, the state, role, session, level, rung, queues,
next deadline and counters.

Planted bugs in the port were caught. Where one survived, a session was added that catches it.
The vectors hold plain floats, parsed exactly (`serde_json`'s `float_roundtrip`), so the
platform-tolerant drift check (`tools/check_vectors.py`) applies to them as to the others.

### 1.3 What the port found in the model

* **The link timeout** is checked when it falls due, against whole exchanges of the family the
  link runs in then (`08-steps-0-and-1.md` §2.6).
* **A transmission split evenly between two rungs** is judged at the slower. Python's set order
  had decided it, which no port can follow.
* **Render-ahead** (07-protocol-8.md §10): an answer is decided as soon as what it says is final
  and handed over with `not_before`. Deciding it at the last moment left the daemon no time to
  render four tone frames.

## 2. The daemon

### 2.1 What it does now

* **The engine's transmissions are their own kind** (`Outgoing::Link`). Only those are reported
  to the engine: `on_tx_done(now, frames_end)` when one ends, `on_tx_dropped` when one is not
  sent. A beacon, a datagram or a tune tone is not in its account.
* **`frames_end`** is the card's clock at the release, less the keyed tail, plus the card's
  output latency: where the last frame ended on the air, which is where the other station
  measures the slot from.
* **The keying gate** (`within_window`). An answer rendered ahead of its slot waits for
  `not_before`. One that would start radiating past `not_after` is not sent, whatever held it:
  it is dropped, the engine is told, the log and the recording say so, and `dropped_late`
  counts it. Radiation starts `key_lead_s` plus the card's output latency after the key
  (`radiation_latency_s`); the engine's `tx_latency_s` is the same figure, and follows the
  card's own report once it gives one.
* **Holds apply only to what a station starts on its own** (`held_for_busy`): a call, a probe, a
  beacon, a datagram, the Morse identifier on its own, and only while `wait_for_clear` says so.
  Nothing of a session waits for the channel: a slot is used or forfeited. Gone with this: the
  answer gap's hold (ADR-0036), ADR-0022's waits for the other station's identifier on a DISC or
  DISC_ACK, `answers_burst` (ADR-0042) and the abort's wait for a frame heard arriving
  (ADR-0065). The engine times an abort's DISC itself: a master's once the slot it granted is
  over, a slave's in the old session's next slot.
* **Announcements change nothing in a session** (ADR-0069 §2.5). A trusted preamble still
  lights the receive indicator and tells a caller or a prober that a frame is arriving. The
  follow-on heed (ADR-0040) is gone.
* **The Morse identifier goes where the engine puts it** (07-protocol-8.md §5.4). The daemon
  tells the engine when one falls due in a session (`set_identifier_due`) and how long it is
  (`set_identifier_s`, from the speed the rules allow). The engine places it ahead of the
  master's next header, or ahead of the slave's answer in a slot granted with its time, and
  ahead of the first DISC and of the DISC_ACK at the end. The daemon renders exactly the time
  the engine counted: the identifier, then silence. One the rules refuse leaves silence of its
  length, so the frames still go where the engine put them. One whose transmission the gate
  drops is owed again. Outside a session (beacons, datagrams) the daemon's own rules place it,
  as before.
* **The key limit**: the engine keeps the identifier's room itself, so its `max_burst_s` is the
  key's less the lead and tail, whether or not an identifier falls due.
* **Roles**: `status.role` and the `state` events say `master` (the caller) and `slave`. The
  host adapter tells a call it answered from one it placed by `(slave)`. The panel no longer
  shows a sending or receiving role, since there is none to show; it says what closing does
  for each.
* **Counters**: `silent_slots`, `forfeits`, `headers_elided`, `left_answered` from the engine,
  `dropped_late` from the daemon. The turn's counters, the answer gap's and the follow-on's are
  gone.
* **The link protocol is 8**: `frames::PROTOCOL_VERSION` is what a station speaks, what a
  release states and what the shell's `LINK_PROTOCOL` is held to. Protocol 7's engine, kept
  until Step 5 deletes it, reads `TURNS_VERSION`.

### 2.2 What building it found

* **The daemon still compared calls against protocol 7.** `bandwidth.rs` judges a call's
  version before the engine sees it: a call "in another protocol" fills the mismatch banner,
  and a 2 300 Hz station answers a narrow call at 500 Hz only when the versions match. Against
  `PROTOCOL_VERSION` = 7, every protocol 8 call was a mismatch, and a wide station never moved
  to answer a narrow one. Renaming the constants fixed it (two station tests had caught it).
* **The host adapter read the role from the `connected` event's detail**, `(irs)`. With
  `(slave)` unread, every session would have looked like one the program placed, and a call
  answered while a scanning RMS Trimode had `LISTEN` off would not have been held for it
  (ADR-0051).
* **A transmission cut short granted the slot its header stated.** An abort, the key watchdog
  or a failed transmitter cuts a transmission; the daemon had reported it to the engine as
  ending at the cut, and the engine measured the slot it granted from there. The other station
  measures it from the end the header stated. In the harness the caller's leaving DISC keyed at
  the start of the called station's answer (`40m-abort-and-recall-2300`, and the Test whose
  budget ran out in `40m-rtty-contest-2300`). The daemon now records, when it keys a
  transmission, where its frames end if it plays whole, and reports a cut one as ending there
  (`KeyWindow::whole_end`). `an_abort_that_cuts_a_header_waits_out_the_slot_the_header_stated`
  fails without it.
* **The engine's account of its transmissions runs in order.** `on_tx_done` and `on_tx_dropped`
  each take the oldest transmission handed over. An abort had reported the queued ones dropped
  before the one on the air done; the order is now the order they were handed over in, on an
  abort, a watchdog trip, a failed transmitter and a rules refusal.
* **A station that falls behind keeps the air's clock.** The station's clock is the audio it
  has heard, and the capture queue drops its oldest audio when the run loop falls behind (a
  slow machine; KE4QCM's fell 0.3–27 s behind). The clock then stood behind the air by what was
  dropped, and an answer whose slot was set from a header read before the stall went out that
  late — past the keying gate, which reads the same clock (`80m-slow-machine-500`: 0.85 s, over
  the caller's next header). The audio backends now say how much they lost ahead of a block
  (`AudioIo::capture_gap`: the sound card from its queue, the channel link from a message the
  channel server sends a daemon that asks for it, `AEC2`), and the station moves its clock past
  it, starting the receiver again after the gap and writing silence into a recording
  (`Station::skip_captured`). The late answer is now the gate's to drop.
  `a_station_that_fell_behind_moves_its_clock_past_the_audio_it_lost`.
* **The station tests that measured protocol 7's rules** were rewritten to the rules that
  replace them, each naming what it now holds:
  * the answer gap → `an_answer_waits_out_the_callers_turnaround`: a caller declaring 1.2 s
    gets its acceptance 0.55 s later than one declaring none (the slot waits for
    `max(D + KEY, T)`);
  * both stations answering late → `a_session_crosses_with_both_radios_slow_to_turn_around`:
    no answer dropped late, no slot forfeited;
  * the identifier after the frames (ADR-0068 §2.5) →
    `the_identifier_the_engine_places_goes_ahead_of_the_frames` and
    `in_a_session_both_stations_identify_where_the_engine_puts_it`;
  * the closing identifier: one keying for the DISC and the caller's identifier, one for the
    DISC_ACK and the called station's, whether or not the interval had run out (protocol 7 sent
    a second keying when it had not);
  * ADR-0065's abort → `an_abort_waits_out_the_slot_it_granted`;
  * the follow-on heed → `a_faint_arrival_lights_nothing`.

### 2.3 The harness, once over

A sanity pass, not Step 3's A/B: every scenario in `bench/scenarios/` once on the protocol 8
daemon (one seed, `session_matrix.py --jobs 3`).

* **The quick set** (`quick-awgn-2300`, `40m-trimode-gateway-2300`,
  `40m-gateway-busy-when-called-2300`): passes, no keyed overlap of any kind, no slot unheard,
  forfeited or dropped.
* **All 43**: 41 pass. Radiated overlaps in three, each traced to a defect of the daemon's and
  fixed (§2.2): `40m-abort-and-recall-2300` (0.46 s) and `40m-rtty-contest-2300` (0.24 s), the
  cut transmission; `80m-slow-machine-500` (1.37 s over three stalls), the clock. Run again
  after the fixes: all three pass with no overlap; the slow machine's station moved its clock
  past 0.88, 2.84 and 1.62 s of lost audio and forfeited one slot rather than answer late.
* **The two VOX scenarios** showed VOX-held overlaps — a transmitter held on by its VOX while
  the other station's answer went out — 2.2 s in `80m-poor-500-pactor-signalink` (which failed
  on its collision count) and 12.7 s in `80m-vox-caller-learned-gap-500`. Read again (§3.3), the
  first was the channel server's: it keyed a VOX radio for the daemon's whole keying, silent
  lead and tail included, which no VOX hears.
* **`80m-leave-together-500` failed**: both operators pressed Disconnect in the middle of a
  6 kB transfer on a weak 500 Hz path, and neither session had ended within 120 s of air. The
  caller went on sending its queue — four tone frames a transmission — because protocol 8's
  rule (07 §5.3 as first written) closed a session the called station asked to close only once
  the caller's own queue was delivered, bounded by a patience that on this path was two
  minutes. Protocol 7 closed between bursts when the receiving station pressed Disconnect
  (ADR-0023), which is what the scenario holds. The rule now: when the slave asks, the master sends its DISC once what the
  slave still had is delivered, its own queue left undelivered (model first:
  `_can_leave`/`can_leave`; `test_the_slave_asking_to_leave_stops_the_masters_own_data`; the
  replay gained a `leaving` session, which fails the port with the old rule).

## 3. The radio's turnaround and the answer gap (schema 12)

A configuration schema step ships with a version (the shell's `SCHEMA_HISTORY` test ties the
schema to the package version), so this went in with the bump to `0.2.0-beta.106` — on the
working branch only: no tag, no release, `master` untouched (the author's choice, 2026-10-13).

### 3.1 What was built

* **`[radio] turnaround_ms`** (0–1500, default 0, live): how long after its own transmission the
  operator's radio takes to hear again — a SignaLink's DLY, a VOX hold, a slow relay. The engine
  declares it in the call and the acceptance (tenths of a second, up to 1.5 s), and the other
  station starts this one's slots that much later (`max(D + KEY, T)`). A live change reaches the
  engine at once (`set_turnaround_s`) and is declared from the next call. Configuration schema
  12 (`turnaround`, a step that changes nothing; fixture `0.2.0-beta.105-vox.toml`, a VOX station
  with an answer gap, whose every setting survives), the shell's `SCHEMA_HISTORY` line
  `("0.2.0-beta.106", 12)`, Setup step 4's *Turnaround* box, the manual.
* **`[radio] answer_gap_ms` keeps a meaning** (model first, then the port and the replay): the
  least time between the other station's frames and this one radiating, for a peer slower to
  hear again than it declares — a VOX station whose operator left *Turnaround* at 0, or any
  beta.105 or older. The daemon hands the engine, every pass, the larger of the setting and the
  gap it learned for the station it is calling or in a session with (ADR-0054;
  `Station::answer_gap_s`, `set_answer_gap`). As the master the engine keys no sooner than the
  gap after the slave's frames (`_peer_wait_s`/`peer_wait_s`); as the slave it asks for the gap
  level that starts its slots the gap after the master's (`_asked_level`/`asked_level`, in its
  answers and its acceptance's capability byte). It never moves a slot on its own: the master
  grants the level, and the slave keys only where the grant says. An acceptance cannot honour
  the called station's gap — its slot is the one the call granted — which is what the caller's
  declared turnaround is for; an unanswered call steps its level every second try.
  `test_the_answer_gap_holds_a_station_off_the_others_frames` (model) holds both directions
  with radios that hold 1.1 s and declare nothing; the replay gained an `answer-gap` session.
  `the_radios_turnaround_and_the_answer_gap_reach_the_engine` (daemon).
* **The harness writes each daemon the file its own build reads** (`session_matrix.py`
  `schema_of`, from the binary's `--example-config`), so protocol 7's daemon still runs beside
  this one in Step 3's A/B, and a scenario may set a station's `turnaround_ms`. New scenario:
  `80m-vox-caller-declared-500`, `80m-vox-caller-learned-gap-500`'s path with the VOX caller
  declaring a 1 s turnaround.

### 3.2 The harness's VOX radio

The channel server keyed a VOX radio for the whole of the daemon's keying: the key lead and the
keyed tail, which are silence, held it as long as the frames did, and its hold counted from the
end of the tail. A real VOX keys on audio, and its hold — the operator's delay time — counts from
where the audio stops. The server now keys a VOX radio only while its audio carries energy
(`_radiate`); `test_a_vox_radio_keys_on_the_audio_not_on_the_silence_around_it` fails on the old
model. A radio keyed by a line (`ptt`, CAT) is unchanged.

### 3.3 Measured

One draw each, the same seeds, protocol 8 with schema 12 (the overlaps are the channel server's
VOX-held ones; none of the runs had a radiated overlap):

| scenario | before (§2.3) | schema 12 | schema 12, VOX on audio |
|---|---|---|---|
| `80m-poor-500-pactor-signalink` | fail, 2.2 s held, 288 s | fail, 2.2 s, 288 s | **pass, 0 s**, 328 s |
| `80m-vox-caller-learned-gap-500` | pass, 12.7 s, 687 s | pass, 8.0 s, 550 s | pass, 3.3 s, 442 s |
| `80m-vox-caller-declared-500` | — | pass, 3.2 s, 468 s | **pass, 0 s**, 243 s |

* **PACTOR and the SignaLink.** Its 2.2 s were the daemon's silent lead under the other
  station's VOX hold: no frame was keyed into a deaf receiver. With the VOX keyed on audio there
  is nothing to count, and the scenario passes.
* **The declared turnaround** takes the VOX caller's path to no overlap at all, in half the air
  time of the learned gap: the slots are where the radio can hear them from the first call.
* **The learned gap** still loses 3.3 s: it is learned by losing answers (ADR-0054, 250 ms a
  step), so a station's first sessions with a VOX peer pay for it. That is what the declared
  turnaround replaces.
* **What Step 3 judges.** `[expect] max_collisions` counts any overlap of keyings, and the two
  stations' silent lead and tail are not radiation. Step 3's pre-registered criteria judge
  radiated overlaps and frames keyed into a deaf receiver (a VOX hold, a radio's recovery), not
  that count.

## 4. Not done in Step 2

* **The capture path's latency.** A frame's time is its place in the captured audio, which is
  the air's clock; the station's "now" lags the air by the card's input latency, so an answer
  keyed at `not_before − tx_latency` radiates that much late. The harness delivers capture with
  no latency, so it cannot show this; a real card's input latency (tens of milliseconds) comes
  out of `SLACK`. Measured in Step 4 before deciding. (Audio *lost* in capture is handled:
  §2.2.)
