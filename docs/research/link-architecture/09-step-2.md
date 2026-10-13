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
* **The two VOX scenarios** show VOX-held overlaps — a transmitter held on by its VOX, deaf,
  while the other station's answer went out — 2.2 s in `80m-poor-500-pactor-signalink` (which
  fails on its collision count) and 12.7 s in `80m-vox-caller-learned-gap-500`. Nothing in this
  build declares a radio's turnaround (§2.4), which is what protocol 8 times those slots by; the
  master's silence levels recover each time, and the learned gap does not yet set a level.
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

### 2.4 Not done in Step 2

* **The turnaround setting.** The engine declares `StationConfig::turnaround_s` in its call and
  acceptance, but nothing sets it yet: `[radio] turnaround_ms`, replacing `answer_gap_ms`, is a
  configuration schema step, and a schema step ships with a release (the shell's
  `SCHEMA_HISTORY` test ties the schema to the package version). It goes in the commit that
  bumps the version for the first build with protocol 8, with Setup step 4's box and the
  manual's *Answer gap* entries. Until then the answer gap is in the file and does nothing.
* **The learned gap as a level.** ADR-0054's per-station gap is still learned and kept in
  `heard.json`; 07-protocol-8.md §10 makes it the gap level a peer is granted. Not wired.
* **The capture path's latency.** A frame's time is its place in the captured audio, which is
  the air's clock; the station's "now" lags the air by the card's input latency, so an answer
  keyed at `not_before − tx_latency` radiates that much late. The harness delivers capture with
  no latency, so it cannot show this; a real card's input latency (tens of milliseconds) comes
  out of `SLACK`. Measured in Step 4 before deciding. (Audio *lost* in capture is handled:
  §2.2.)
