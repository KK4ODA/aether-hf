# ADR-0042: Whole sessions between real daemons, through simulated band conditions

**Status:** accepted, 2026-10-06. `aetherd --channel` (`core/aetherd/src/channel_link.rs`), the
channel server (`tools/channel_server.py`), the band's other signals and static
(`model/aether_model/qrm.py`), the scenarios (`bench/scenarios/`), the runner
(`tools/session_matrix.py`); CI runs the quick scenarios on every push, the nightly workflow the
rest. One daemon fix it found (§4). No wire change, no configuration change.

## 1. Context

Every field problem of the last two weeks lived between the pieces the tests cover: the
acknowledgements keyed over the other station's faded frames (ADR-0040), a VOX interface's hold
after transmitting (ADR-0036), a sound card's silence after the key comes up (ADR-0037), the busy
detector's view of the channel before this station's own transmission (ADR-0037). The model's
link benches run the engines on calibrated channels but not the daemon; the daemon tests run the
daemon on a noiseless wire; `two_daemons.rs` runs two daemons on a plain-noise socket in real
time. Each field report cost the author and another station an evening on the air before a fix
could be checked. The author asked for the whole pipeline, under the conditions he works in:
80 and 40 m, wide VARA-class neighbours, PACTOR and RTTY now and then, significant fading, and
atmospheric crashes.

## 2. Decision

1. **The station's clock is the audio's** (`Station::now`), so a daemon given its audio a block
   at a time, as fast as it takes it, runs a session as fast as its CPU allows. `--channel
   HOST:PORT` connects the daemon to a channel server instead of a sound card
   (`ChannelLink`): a capture is one 20 ms block the server sends once both stations have asked
   for theirs; what the station plays goes to the server as it hands it over; the playback
   clock is the server's, silence counted as a card counts it. A daemon on `--channel` keys
   nothing and is not judged by the rules unless its file names a profile, as on `[sim]`.
   It is a command-line option, not a configuration key: no schema change.
2. **The channel is the reference model's** (`tools/channel_server.py`): each direction is the
   A/B bench's `CableChannel` — the calibrated ITU-R F.1487 fading, including the NVIS profile,
   and noise at the 3 kHz SNR — so a scenario's channel is the one the benchmark curves are
   quoted on, not a second implementation to calibrate.
3. **The band's other occupants and its static** (`aether_model.qrm`), model first, streaming and
   seeded like every other impairment: a VARA-class 2.4 kHz multicarrier ARQ station (data
   bursts and its partner's acknowledgements), a PACTOR-class 500 Hz π/4-DQPSK station on the
   1.25 s cycle, 45.45 Bd 170 Hz RTTY in overs, each on a fading path of its own, and
   atmospheric crashes — Poisson arrivals of clustered impulses under a decaying envelope, their
   peaks log-normal above the floor, a parametric stand-in for ITU-R P.372's impulsive
   atmospheric noise. Built from the signals' public descriptions only.
4. **The radios at both ends**, from the recordings: a card's latency to the air (0.25 s), a
   transmitter's start that never radiates (`tx_delay_ms`), a VOX hold (`vox_hold_ms`), a
   receiver's silence after unkeying (`rx_recovery_ms`), and silence while transmitting. The
   server reports every stretch both stations were keyed on the air at once — a collision, which
   no recording at either end shows.
5. **Scenarios are data** (`bench/scenarios/*.toml`, README there): the stations, the path each
   way, the other signals and static each one hears, the radios, a script (`probe`, `connect`,
   `message N`, `reply N`, `test`, `wait S`, `disconnect`, `beacon`) and what a pass is
   (connected, delivered, the Test's outcome, a clean end, at most so many collisions).
   `--daemon-b` runs the called station on another build.

Considered: porting the channel to Rust inside the daemon's `[sim]` (a second channel to
calibrate, and the model's random streams cannot be reproduced bit for bit); the in-process
station harness (`Air` in `station.rs`) with fading added (fast, but it skips the process —
control API, recording, the run loop — that the field runs); real time over `[sim]` (a
twenty-minute session takes twenty minutes).

## 3. Measured

Speed, two daemons and the channel server on four cores (this container): the smoke scenario,
74 s of air in 15.5 s; the 80 m and 40 m set two at a time, 383–735 s of air in 141–287 s —
2.5–5 times faster than the air. The first run of that set (beta.80 plus §4's fix):

| scenario | outcome |
|---|---|
| 40 m Good 2300 Hz, RTTY and heavy crashes | pass: the Test complete, rung 10, no collisions |
| 40 m Moderate 2300 Hz, a VARA-class neighbour | the messages crossed; the Test's 16 kB file timed out |
| 80 m NVIS 500 Hz, the ND1J path | the Test's file timed out; 5 collisions, 12.6 s keyed together |
| 80 m Poor 500 Hz, PACTOR, a SignaLink's 400 ms hold | delivered; 8 collisions, 8.4 s keyed together |

Those three are the next work, each reproduced here before anything is changed. With §4's two
fixes the same set passes, all four: the 80 m Poor path 8 collisions → 1; the 80 m NVIS path
5 collisions (12.6 s) → 2 (3.1 s) and its Test complete; the 40 m Moderate path's Test complete
(rung 14), where the file had timed out.

## 4. What it found on its first run

The smoke scenario — a clean wire, a message each way, the receiving station disconnecting the
moment the reply arrives — failed on one collision. The receiving station's DISC, which goes in
place of the burst's acknowledgement (ADR-0023), was held by the rule that makes a DISC wait out
the other station's Morse identifier (ADR-0022): the busy detector was still holding the burst
the DISC answered, by its energy as well as its decode, for two seconds; the sender heard no
answer, sent the burst again, and the two were keyed at once — five seconds of it in the
daemon test that reproduces it. That is ND1J's "the disconnect button does not work" seen from
the receiving side. A receiving station's DISC within the detector's frame hold of the burst it
answers now goes when the acknowledgement would have (`held_for_busy`, `answers_burst`); a DISC
retry still waits out the identifier.
`a_receiving_station_that_disconnects_answers_the_burst_with_its_disc_at_once` fails without it.

**And on the 80 m Poor path** (the PACTOR and SignaLink scenario), eight collisions: the
receiving station acknowledged about 1.3 s before the sender's six-frame bursts ended. Its
recording showed why — the third frame said three more followed, the fourth arrived too faint to
believe its count, and the acknowledgement, re-armed from that frame alone, was set for the
fourth frame's end, over the two the third had announced. The burst now ends where the latest
count heard says it does (`_burst_end`/`burst_end`, `announced_end` per record; model first, both
suites): a later frame never brings it earlier. In the faded-session test with three frames in
ten unheard and three more read too faintly, the collisions went from 12 to 2 (19 with no
countdown); the two left are the count's ceiling — a frame says "three or more", and a burst of
six whose later frames are all lost (`a_frame_read_too_faintly_to_believe_does_not_cut_the_burst_short`).

## 5. Consequences

* A field report becomes a scenario first: reproduced, fixed, and kept as a check.
* The quick set runs in CI on every push (seconds); the 80 m and 40 m set nightly (minutes) and
  before a release; a release's daemon against the last one's (`--daemon-b`) says whether the
  stations on the air still connect.
* The harness models what the recordings showed; a radio behaviour it does not model (an AGC's
  recovery, a codec's clock drifting against the other's) is added when the air shows it.
* Tests: `test_qrm.py` (block-exact streams, power, occupied band, keying rhythm, crash rate),
  `test_channel_server.py` (latency, the transmitter's delay, a VOX hold, collisions),
  `channel_link`'s clock test, and the daemon test above.
