# ADR-0076: The Test pays only for what it measures, and a frame is no noise

**Status:** accepted, 2026-10-17, the author's decision on report 13 §14 ("yes to all";
`docs/research/link-architecture/13-wc4y-beta111.md`). Engine rules model first
(`aether_model/link/granted.py`), then the port (`aether-link/src/granted.rs`) and the replay
vectors (the new `ladder-unheard` session); the rest is the daemon's (`aetherd`), the panel's and
the manual's. **No wire change**: a station on this build and one on v0.2.0-beta.106 to
beta.111 work together, and Test with each other. §5's first item — ladder frames that are not
data — is built as ADR-0077 (link protocol 9), which removes the leftovers this ADR only stopped
draining.

## 1. Context

WC4Y's Test to KK4ODA-1 of 2026-10-17 02:05Z, 80 m at 500 Hz, beta.111 at both ends, was read
from both stations' recordings (report 13). Protocol 8 held: no overlap, every answer in its
slot. What it found, besides the path:

* **The Test paid for its filler.** A rung of the ladder pins four small frames, and the first
  answer measures it; ADR-0074 then re-encodes the ones that failed at the rung asked for. That
  was rung 3, a tone frame: 5.36 s for each ten-odd bytes, sent alone (one family a burst), each
  batch with its own answer. Delivering 13 such frames took about 93 s of the Test's 435, more
  than the ladder's measurements (25 s); 26 s of it after the ladder had ended, in the
  disconnect's drain (ADR-0072).
* **The ladder pinned rungs the link never uses.** The 500 Hz rungs 4 and 8 are beaten on both
  data rate and threshold (ADR-0073's `usable_modes`); rung 8's transmission went silent and its
  four frames came back as filler.
* **Its silent slots cost a wait and a floor poll each**, about 8 s: the ladder's rungs 8 and 9
  went without their header (R7, an RV 0 copy in each), none of their frames decoded, and the
  slave had nothing to answer. KK4ODA-1 read WC4Y at about 0 dB, so the header would have been
  ordinary, 0.434 s.
* **The field log took the poll's SNR for the rung's.** After a silent slot the answer that
  reports the rung's frames answers the poll: rung 8 went into `ladders.csv` "at −4 dB", the
  poll's tone-floor reading, where its frames had arrived at 0 dB. `bench_link --replay` fits
  its penalty from these.
* **The busy detector learned the other station as noise**, at both ends: KK4ODA-1's floor read
  −18 to −22 dBFS against a real −28, WC4Y's −12 against −19. A weak station's frames sit under
  the 6 dB margin (at 0 dB in 3 kHz a 500 Hz signal lifts the passband's power by 3 dB), a tone
  frame's envelope is steady, and a decoded frame marked the channel busy only from its decode —
  so most of each frame went in as noise, and the frame's 2 s hold kept the quiet after it out.
  The panel's floor, threshold and "Signal above noise" read signal over signal, and the level
  path was blind until the band had been quiet a while.
* **The sound card lost audio inside two received frames.** KK4ODA-1's capture skipped 48 and
  18 ms inside two of WC4Y's rung-3 frames at −1 to −4 dB (one not detected, one failed). The
  failure counted against the path.
* **A call and a probe were reported late** for a slot they do not have: WC4Y's call, held 1.8 s
  by `wait_for_clear`, was "on the air about 1820 ms after its slot opened".
* **No recording held the closing identifiers.** A recording stopped at the session's end, and
  ADR-0075 puts both stations' identifiers after it.
* **The identifier sent the SSID.** "KK4ODA-1" is 105 dits, 6.3 s at 20 wpm; the hyphen and the 1
  are 2.3 s of it, and §97.119 asks for the assigned call sign.

## 2. Decision

1. **The ladder climbs only the rungs the link uses** (`fieldtest::ladder_rungs`): those
   `usable_modes_by_rate` keeps, the rate controller's own front — at 500 Hz it skips rungs 4
   and 8, at 2 300 Hz rungs 6 and 13. It starts a rung under the fastest the transfers were
   acknowledged at *on that ladder* (`skipped_rungs`). Its frames' body is the most every rung it
   climbs can carry. The Test's wording is "the transfers reached rung N" (they ran mostly at a
   lower one).
2. **A ladder rung whose transmission went unanswered records no SNR**
   (`_ladder_unheard` / `ladder_unheard`): its frames are still counted from the answer to the
   poll, but the SNR that answer carries is the poll's. The daemon logs "at ? dB", the sidecar and
   `ladders.csv` carry nothing.
3. **R7 keeps the header on a transmission whose data goes above the rung the slave recommends,
   when the header would be ordinary** (`GrantConfig.header_above_recommendation`, default on;
   `04-formal-spec.md` R7). The Test's ladder pins its rungs there; a floor header (3.2 s) broke
   even on report 12's weak paths and may still be left out. ADR-0073's guard (an RV 0 copy) is
   unchanged, and with this rule `test_a_count_rescued_with_its_block_places_no_end` needs the
   new rule off as well as ADR-0073's to show the hazard it guards.
4. **The Test closes without delivering what is still queued**
   (`GrantedEngine.disconnect(drain_s=0)` / `disconnect_within(Some(0.0))`): after the ladder
   that is its filler, which nobody reads. The master says what it left ("N bytes not delivered;
   closing …"), and the other station may report the last frames not received, which for a Test
   is so. The filler re-encoded *inside* the ladder is still delivered; only a frame kind the
   slave counts and never delivers removes it, a protocol change for the next bump (§5).
5. **A frame the receiver found is no evidence of the noise** (`BusyDetector::explain`): every
   frame announced (`heed_preambles`, heeded or not) or reported (decoded or not) takes its span
   out of the floor, the part already heard included, and the floor is worked out again at once.
   A decoded frame's hold still makes the channel busy, but it is a politeness, not a
   measurement: the steady quiet after the frame, in a session the only quiet there is, counts
   as noise (`signal_until` keeps only the level and shape paths' hangovers out). Nothing is
   marked busy by an announcement: an acquisition false-alarms, and keeping a stretch of noise
   out of the floor only holds it.
6. **A frame a reported capture skip falls inside is not trusted**: it is not counted against
   the path by the rate controller (ADR-0020's rule), and its sidecar record and `frame` event
   say `capture_skip`. The station remembers each skip's time for 30 s; a skip is noted after
   the audio it lost, by up to a slow pass, so the test reaches half a second past the frame.
   The capture timestamps see about half of what is lost (report 12 §15.2, report 13 §5), so the
   clock is still not moved by them.
7. **The `keying` "late" line and count judge only transmissions in a slot**: a call or a probe
   (`starts_an_exchange`) and an identifier that listens first are `slotless`.
8. **A recording runs on past its session's end until the closing identifiers have gone**
   (`RecordingTail`): until the station has nothing more of the session to send, then
   `RECORDING_TAIL_AFTER_S` (1.5 s), or for a slave the master's declared identifier
   (`peer_identifier_s`, a new accessor in both engines) and `RECORDING_TAIL_PEER_ID_S` (2 s), at
   most `RECORDING_TAIL_MAX_S` (30 s) after the end. A new recording, a new session's, or
   `record.stop` ends it at once. The debug-mode upload waits a minute after the session, longer
   than the tail.
9. **The Morse identifier sends the call sign without its SSID** (`cwid::sent_as`): `KK4ODA-1`
   goes as `KK4ODA` (4.0 s), `-T`/`-R` likewise; a designator written with `/` stays
   (`W4/G4ABC`, `KK4ODA/P`). What a call or acceptance declares is the length of what goes out;
   `identifier.check` gains `sent_as`. Whether a callsign must identify is unchanged
   (`required_for`, which already set the SSID aside). The author's decision as licensee.
10. **The panel**: the Mode tile and the well's rung show the rung the data moves at (receiving,
    sending, both, or the rung this station would send at when nothing moves), as the manual
    says; the well shows the callsign in a span of its own that never shortens; the speed chart
    leaves a gap outside a session; the readings' tooltips say what window each reads.

## 3. What it costs, measured

The engine rules on the model's simulator and the port's replay: `test_granted.py` (the cheap
header, the unanswered rung, the drain of nothing) and `granted_vectors.rs` (every session, the
new one included, input by input). The busy detector: `a_weak_stations_frames_do_not_become_the_
noise_floor` — thirty 5.36 s frames 4.5 dB over the noise with 0.7 s between them ratchet the
floor up more than 3 dB unexplained, and leave it within 1 dB explained; every older busy test
holds. The harness A/B against beta.111 on the Test scenarios is in §6.

## 4. Consequences

* Tests at 500 Hz climb two rungs fewer and close at once; a silent ladder slot needs a floor
  header to happen at all. On report 13's session the three Test changes (1, 3, 4) come to about
  70 s of its 435: rung 4 and its leftover (~19 s), rung 8's silence, poll and leftovers (~35 s),
  rung 9's silent slot (~8 s, its header ordinary), the drain (26 s), less rung 3 added as the
  rung under the transfers' top (~24 s) — the estimate before the harness's number.
* New Tests' ladders and old ones differ: old rows carry rungs 4 and 8 at 500 Hz, and silent
  rungs with the poll's SNR. `field_ingest.py` writes an empty SNR as it is.
* A busy detector that no longer learns the session as noise reads clear less often after one,
  and the panel's floor in a session is the noise's. Its readings during a frame are what they
  were.
* A recording is up to 30 s longer, and holds both identifiers.

## 5. Not done

* **Ladder frames that are not data**: a frame kind the slave counts in its answer and never
  delivers (report 12 §21.3, report 13 §10). It removes the filler altogether — about 93 s on
  report 13's Test — but is a protocol change; it waits for the next bump.
* **Handing capture skips to the clock** (`capture_gap`): still a partial correction of a loss
  the receiver cannot re-time mid-frame (report 12 §15.2).

## 6. The harness

The 14 scenarios that run a Test, two draws each (the scenario's seed and +1000), this build
against beta.111's daemon (`session_matrix.py --seeds 2`, the same seeds both arms):

| scenario | passed, before / after | Test air, s, before → after | message, bit/s | file, bit/s |
|---|---|---|---|---|
| 20m-dx-moderate-2300 | 2 / 2 | 372 → 337 (−9 %) | 627 → 438 | 654 → 564 |
| 40m-drift-and-clocks-2300 | 2 / 2 | 333 → 262 (−21 %) | 569 → 557 | 554 → 564 |
| 40m-good-2300-rtty-crashes | 2 / 2 | 378 → 296 (−22 %) | 367 → 409 | 323 → 437 |
| 40m-moderate-2300-vara-qrm | 2 / 2 | 362 → 343 (−5 %) | 511 → 511 | 454 → 455 |
| 40m-rtty-contest-2300 | 2 / 2 | 351 → 392 (+12 %) | 298 → 405 | 223 → 127 |
| 80m-asymmetric-500 | 2 / 2 | 358 → 296 (−17 %) | 57 → 61 | 88 → 92 |
| 80m-floor-top-start-500 | 2 / 2 | 415 → 359 (−14 %) | 68 → 68 | 83 → 83 |
| 80m-flutter-500 | 2 / 2 | 518 → 478 (−8 %) | 69 → 69 | 66 → 53 |
| 80m-local-rfi-ladder-500 | 2 / 2 | 434 → 382 (−12 %) | 78 → 78 | 73 → 73 |
| 80m-lopsided-acks-500 | 2 / 2 | 532 → 433 (−19 %) | 67 → 67 | 73 → 84 |
| 80m-nvis-evening-500 | 2 / 2 | 482 → 405 (−16 %) | 34 → 30 | 33 → 32 |
| 80m-slow-machine-500 | 2 / 2 | 438 → 480 (+10 %) | 94 → 93 | 91 → 65 |
| 80m-thunderstorm-agc-2300 | 2 / 2 | 316 → 283 (−10 %) | 385 → 417 | 406 → 463 |
| 80m-weak-both-ways-ladder-500 | 2 / 2 | 523 → 520 (−1 %) | 54 → 54 | 44 → 40 |
| **all** | **28 / 28** | **11 622 → 10 530 s (−9.4 %)** | | |

No radiated overlap in either arm. Where the message and file read the same in both arms the
draw ran identically up to the ladder; the saving is the ladder's and the close's. The two
slower scenarios are a single draw each: the RTTY contest's first draw is identical in both arms
(395 s) and its second traded a faster message (325 → 538 bit/s) for a slower file; the slow
machine's both draws lie inside the nightly baseline's air-time range (415–538 s). Three more
draws of each on both arms (variants 2–4) settle it: over five draws the RTTY contest's mean Test
air is 464 → 459 s (median 479 → 436) and the slow machine's 458 → 449 s (median 440 → 473),
every run passing, none overlapping — inside the spread one build shows from draw to draw.
