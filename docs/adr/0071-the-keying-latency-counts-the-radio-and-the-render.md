# ADR-0071: A station's keying latency counts its radio and its render

**Status:** accepted, 2026-10-15. Daemon only: `aetherd/src/station/latency.rs`,
`Station::render_pending`, `render_took` and the keying gate in `station.rs`, `fill_card` in
`main.rs`, `CaptureSkips` in `audio.rs`. No wire change, no protocol rule, no configuration key,
no schema step; the link engine, the model and every vector are unchanged. A station on this
build and one on v0.2.0-beta.106 work together.

## 1. Context

Protocol 8's first session on the air, WC4Y's Test with KK4ODA-1 of 2026-10-14, recorded at both
ends (`docs/research/link-architecture/11-step-4-first-air.md` §4):

* **Every answer reached the air late**, 76–197 ms after its slot's start. The engine and the
  keying gate both took this station's keying latency to be the key lead and the sound card's
  reported output latency, 0.122 s. Three waits sat between deciding a transmission and its
  radiation and were counted nowhere:
  * the radio's keying command, which the run loop waits for (`ThreadedPtt::key`): the
    FTDX10's `TX1;` 19 ms, an IC-7300's CI-V at 9600 Bd about 45–65 ms;
  * rendering a transmission decided at the last moment: 30–80 ms for a 3.2 s tone control
    frame on the mini PC at the radio;
  * the radios' own audio paths, 65–70 ms a station, which one station cannot measure.
* **The gate judged a start short by the same amount.** S2 of `04-formal-spec.md` ("radiation
  starts no later than slot + slack, or not at all") held by 153 ms of margin, not by
  construction. A slower radio, a busier computer or a CAT port that stalls would have eaten
  that margin unseen.
* **The mini PC's capture lost 35 and 40 ms of audio and nothing said so** (§4.4). The queue
  counts only what it drops itself; a loss at the device or the USB link reaches the receiver
  as a splice, and a tone frame across one was lost.

## 2. Decision

1. **Two waits are learned per station** (`latency.rs`), each as its last nine values to the
   millisecond, so a station keyed by a serial line, or a harness, learns none:
   * the radio's keying command, timed around `ptt.key`;
   * the render of each of the engine's transmissions, read on the sound card's playback clock
     either side of it (`fill_card` renders first, then reads the clock again before the gate). A
     harness whose clock stands still while the daemon works (the lockstep channel, ADR-0042)
     reads none.
2. **The engine is told lead + card + typical key + typical render** (the medians;
   `engine_latency_s`). It decides a last-moment answer that much earlier, forfeits one decided
   too late for its slot, asks for a wider gap level when this exceeds `KEY`, and keys its own
   transmissions that much ahead. These are the engine's existing uses of "this station's own
   keying latency" (`07-protocol-8.md` §3.2, §3.3, §10); none changes.
3. **No transmission of the engine's radiates before the engine reckoned.** One without a
   `not_before` (the master's transmissions, calls and probes) gets one: the moment it was handed
   over plus the engine latency it was given. The gate holds a transmission until the air's
   clock, read after the render, plus the key lead, the card and the **fastest** recent keying
   command reaches its `not_before`. A wait learned long, or a render faster than typical, makes
   a transmission wait for its time; it never puts one on the air early.
4. **None radiates past its slack (S2 by construction).** The gate refuses a transmission the
   typical keying command would start past its `not_after`. Once the key is up, it judges the
   start again with the wait just measured, and a start past `not_after` lets the key go with
   nothing played. Keying with no audio radiates nothing. It is counted (`dropped_late`) and the
   engine is told (`on_tx_dropped`), as for a drop before the key.
5. **The frames' end of a transmission cut short** is placed from where the air's clock stood
   once the key was up, not from the pass's start.
6. **Reported:**
   * a `keying` log line when the learned waits move the latency by 10 ms or more;
   * the diagnostic bundle's `keying` (lead, card, both waits' typical, least, most and count,
     the engine latency);
   * the recording's `tx_end`, with this transmission's `key` and `render`.

## 3. Capture skips: a report only

The capture callback compares each callback's capture timestamp with the last one's plus the
samples it held (`CaptureSkips`). A jump forward of 10 ms or more (a quarter of a tone symbol) is
audio the device's clock passed that never arrived. A jump back is audio delivered twice. Each is
a `warn` log line, a `capture_skip` recording event, and counts in the diagnostic bundle's
`audio` (`capture_skips`, `capture_skipped_ms`, `capture_largest_skip_ms`, `capture_repeats`,
`capture_repeated_ms`).

The station's clock is **not** moved by them. Whether a USB codec's capture timestamps on Windows
see a device-level loss at all is not known (report §14). Moving the clock on a false reading
would misplace every slot after it. Once a station's recording shows a skip reported where
`tools/two_sides.py` finds a step in the offset, and none where it does not, the skips can be
handed to `capture_gap` (`Station::skip_captured`), so the receiver hears a hole and not a splice.

## 4. What stays uncounted

The radios' audio paths, about 65–70 ms at each station on 2026-10-14. One station cannot
separate its transmit path from the other's receive path. A reference at one antenna can (report
§14). They stay inside `SLACK`'s margin. On this build, answers should reach the air about that
far after their slot's start, where they were 76–197 ms before. `tools/two_sides.py` on the next
two-sided session reads it.

## 5. Tests

* `station/latency.rs`: medians, the recent window, the half-millisecond floor.
* `the_radios_keying_wait_is_learned_and_the_engine_told`: a 30 ms keying command, learned and
  added to the engine's latency, and the log line.
* `a_key_that_comes_up_past_the_slack_sends_nothing`: a 60 ms key with 30 ms of slack is let go
  with nothing played and dropped. The next such answer is refused before the key.
* `a_transmission_the_engine_keys_at_once_goes_no_sooner_than_it_reckoned`: with an 80 ms render
  learned, a call is keyed 80 ms after the engine decided it. The test fails without decision 3
  (keyed at once).
* `a_jump_in_the_capture_timestamps_is_audio_lost_and_jitter_is_not` (`audio.rs`).
