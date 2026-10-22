# ADR-0088: The busy detector holds through a session, starts clean, and is recorded

**Status:** accepted, 2026-10-22, the author's go on report 14's changes 1, 3 and 4
(`docs/research/link-architecture/14-wc4y-beta118.md` §7, §14). The busy detector lives in the
daemon alone (`aetherd/src/busy.rs`); there is no model of it to change first. **No wire change,
no configuration key, no schema bump**: what a station sends is untouched, and a station on this
build works with any protocol-11 station. The session sidecar gains a `busy` object, which older
readers ignore.

## 1. Context

Report 14 read WC4Y's Test of 2026-10-21 from both stations' recordings, and the busy detector
went wrong at both ends.

**KK4ODA-1, the called station: the floor became WC4Y's signal** (§7.2). A slave in a session
hears noise only in the gaps, and none of them counted. After its own answers it is deaf for
0.75 s, the card's latency and the capture's allowance, and WC4Y keyed 0.53–0.64 s after them.
After WC4Y's transmissions, the hangover of the busy-by-level they caused, 0.75 s, and the 200 ms
a block needs to be steady covered the 0.80–0.95 s before its own answer. From just after the
call the window held no block it could call noise, and ADR-0076 kept the frames' own blocks out of
the evidence, so the floor held. Then the 60 s hold ran out. At the hold's end the detector took
"the median of every block, silence excepted" (a signal that never stops is the environment), and
every block in the window was one of WC4Y's frames. The floor sat at his signal, −22 dBFS against
the band's −31, until the ladder's short OFDM frames near the end; the panel's floor, threshold and
*Signal above noise* followed it, and its help said frames never count as noise.

A scratch build showed that leaving the explained frames out of that median is not enough
(§7.2's table). Once the hold has run out the median is taken again every block, and the first
half second of each new tone frame comes before its announcement: those blocks pass and set the
floor (−24.8 dBFS). Not taking it while the window is mostly frames held it at the noise.

**WC4Y, the caller: every start began busy** (§7.1). His IC-7300's codec gave first blocks 14–31
dB under the band's. The first floor learned them, nothing louder was evidence afterwards, and the
two starts that ran past a minute stayed busy for 61 s, the hold. His gain reference had also
learned its levels on quiet 20 s before the operator set the dial, on another band. And a 7.6 s
busy hold before the call is still unexplained: a detector that has heard only the recording knows
nothing of the minutes before it, and the trace drew his floor at −28 dBFS where the live one sat
at −50.

## 2. Decision

1. **The hold's end takes no frame as noise.** Each block of the floor's window remembers whether
   a frame the receiver found covers it (`Block::framed`): set as it is taken when it falls in a
   frame already explained, and later by `explain`, which reaches back over the window when a
   frame is announced. When no block is evidence and the floor would be taken from every block
   (no floor yet, a first floor not confirmed, or the hold run out), it is **not taken while half
   the window or more is framed blocks**: that window is a session, not the environment, and the
   floor learned before holds through it. When it is taken, framed blocks are left out of the
   median. The help text's "frames the modem finds never count as noise" is then true.
2. **The start is not heard.** Neither the floor nor the gain reference learns from the capture's
   first second (`START_LISTEN_S`): a sound card can start quieter than it goes on, and a radio
   settles after a retune. And **a first floor no steady block has confirmed** (`confirmed`, set
   by the first floor taken from evidence) is not held for a minute: while no block is evidence it
   is taken again from the window every block, as before the first evidence, and only an
   evidence floor is held. A band that really is quiet at the start and then loud for good still
   reads busy until the hold says the loud band is the environment.
3. **A move of the dial starts the detector over.** When the station learns of a dial move of
   10 kHz or more (`DIAL_RETUNE_HZ`) — it tuned the radio from the dial list (`frequency.set`), or
   read the dial over CAT, rigctld or FLRig (`Station::read_dial`, every place the dial is read) —
   `BusyDetector::retune` clears the floor's window, the floor, the gain reference and its levels
   on quiet, the drawn gain and the attack, and listens again a second later. A frame's hold and
   the hangover stay: they are about the station heard, not the floor. A move under 10 kHz stays
   on the band, whose noise the floor already has. The `dial` log line says it.
4. **A recording keeps the detector.** At a recording's start the sidecar's `busy.state`
   (`BusyState`, on the recording's clock) holds what the detector had learned: the floor in its
   own units, the gain it was drawn at, whether evidence confirmed it, the hold and the three
   "until" times, the floor's window block by block with its flags, the attack's votes, and the
   gain reference's levels on quiet (or its warm-up). Through the recording `busy.readings` keeps
   `[t, level, floor, drawn gain, gain reading, busy]` four times a second, on a beat, as the panel
   polls them. `aetherd --busy-trace WAV --expect SIDECAR` takes the state over at its time
   (`BusyDetector::restore`) and explains every frame the station announced (the sidecar's
   `preamble` events, `Announcement`) at the moment it did. The station explains every
   announcement, heeded or not (ADR-0076), and so does the trace; report 14 §14 had said "each
   heeded announcement". `tools/floor_trace.py` sets the trace beside the live readings
   (`live_report`): how closely the floors agree, how often the busy states do, and every stretch
   of 2 s or more where the floors stand 3 dB apart, which is what the trace does not explain.

## 3. Measured

**This session's recordings, traced with this build** (`aetherd --busy-trace … --expect`, both
recordings from before states were kept, so the trace starts from nothing):

| floor, median (p10 to p90), dBFS | 30–96 s (before the session) | 100–285 s (the session) | 285–340 s |
|---|---|---|---|
| KK4ODA-1, beta.118's detector | −34.3 (−37.1 to −30.7) | **−22.3** (−23.7 to −20.9) | −29.7 |
| KK4ODA-1, this build | −32.7 (−35.6 to −29.9) | **−32.5** (−35.9 to −29.9) | −30.7 |
| WC4Y, beta.118's detector | −27.2 | −28.5 | −26.5 |
| WC4Y, this build | −27.3 | −28.4 | −26.6 |

The channel's noise at KK4ODA-1 was about −31 dBFS. Its floor now stays there through the session;
WC4Y's, whose gaps held noise, does not change.

**Unit tests** (`busy.rs`, `station.rs`, `record.rs`, `replay.rs`, `busy_trace.rs`):

* `a_slaves_session_leaves_the_floor_at_the_noise`: 70 s of a slave's session (frames 10 dB over
  the noise, 0.7 s gaps, each frame explained half a second in, the station deaf 0.75 s after its
  own answers). The floor's highest is under 1 dB over the noise; without rule 1 it reaches
  10.6 dB, the frames' level.
* `a_capture_that_starts_quiet_does_not_hold_the_channel_busy`: 0.3 s or 1.1 s of audio 15 dB
  quiet at the start leave the channel busy under 1 % after; 6 s of it, then the band 15 dB
  louder for good, still reads busy (over 50 %). Each case fails without rule 2.
* `a_dial_move_relearns_the_radios_levels_on_quiet`: a band 14 dB quieter, then this one, with an
  AGC's cut from 40 to 50 s. Not told of the move, the channel reads busy over 20 % of the time
  after the AGC lets go; told, under 1 %. `a_move_of_the_dial_to_another_band_starts_the_busy_detector_over`
  holds the station to it.
* `a_detector_restored_from_its_state_goes_on_as_the_live_one_did`: a state taken under an AGC's
  cut, through the sidecar's JSON, into a new detector: it agrees with the live one in 99 % of the
  blocks after and its floor within 1 dB, where one started fresh at the same moment reads busy
  after the AGC lets go.
* `a_trace_starts_from_the_recorded_state_and_explains_what_was_announced`,
  `the_busy_detector_is_kept_four_times_a_second_with_its_state_at_the_start`,
  `an_announcement_is_read_from_its_event`, and
  `test_floor_trace_says_where_the_trace_parts_from_the_live_detector`.

**The harness**: §5.

## 4. What it does not do

WC4Y's 7.6 s busy hold before the call stays unexplained: his recording has no state. The next
recording made on this build carries one, and its trace starts where the live detector was.

A start and a dial move cost a call 3.5 s: the second unheard, then the 2.5 s any detector
needs before it says the channel is clear (`settled`). A host program that scans channels
(RMS Trimode) moves the dial every few seconds; each move Aether reads starts the detector over,
so it reads "still listening" more often there, which is truer than a floor learned on another
band. A session between two other Aether stations that this station hears is framed blocks too:
the floor holds through it as through its own. A signal of anything else that never stops is still the
environment once the hold runs out, as ADR-0078 has it.

## 5. The harness

All 56 scenarios, two draws each, on this build (with ADR-0089) against beta.119's daemon:

| | beta.119's daemon | this build |
|---|---|---|
| runs completed | 112 / 112 | 112 / 112, and `80m-capture-skips-500` 2 / 2 |
| radiated overlaps | 0 | 0 |
| air time, pooled | 30 823 s | 31 259 s (×1.014) |
| Test message rate, geometric mean of 15 scenarios' medians | | ×1.033 |
| Test file rate, likewise | | ×0.998 |
| held for a busy channel (passes of the loop, both stations) | 49 822 | 63 216 |

The air time moved because of one change, and where it was tested no session behaved
differently.

* **Every first call goes a second later.** The first keying of every scenario moved from 2.75 s
  to 3.75 s: the capture's first second is not heard (rule 2), and a call waits for the detector
  to settle after it. That is 114 s of the 436 s, and the passes held for a busy channel rose
  by about 120 a run, the same second at both stations. On the air a daemon has been listening
  long before anyone calls; after a dial move the operator waits the same 3.5 s, which the manual
  says.
* **So every session is another draw.** The channel's fading and noise are seeded on the air's
  clock, and a session that starts a second later meets a different stretch of them frame by
  frame. Run one at a time on an idle machine, the two daemons repeat themselves exactly and
  differ: `40m-winlink-exchange-2300` 82.1 s on beta.119's daemon every time and 101.1 s on this
  build, `80m-winlink-exchange-500` 256.1 and 302.1 s. This build with the listening second taken
  out (`START_LISTEN_S` = 0, a scratch build) repeats beta.119's 82.1 and 256.2 s. The other
  differences between the arms are draws of the same kind: a build that differs from beta.119's
  only in the header rule ADR-0090 tried read ×1.012 against the same runs, and of the scenarios
  that moved 5 % or more on both draws this build has four slower and none faster, ADR-0090's
  build three slower and one faster. Inside a session the busy detector holds nothing: the slots
  do not wait for a clear channel, and the passband width it gates is a diagnostic. The daemon's
  cost is unchanged (`--busy-trace` over KK4ODA-1's recording: 1.96 s and 2.0 s; both daemons run
  at 3.2–3.6 times the air's speed under the same load).

`tools/scenario_trend.py` finds nothing slower or failing against `bench/baselines/scenarios.csv`
(two scenarios faster, `80m-nvis-disturbed-500` and `80m-lopsided-acks-500`'s message), and lists
`80m-capture-skips-500` as new. The baseline stays.
