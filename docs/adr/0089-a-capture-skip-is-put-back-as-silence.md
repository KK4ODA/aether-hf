# ADR-0089: A capture skip is put back as silence

**Status:** accepted, 2026-10-22, the author's go on report 14's change 2
(`docs/research/link-architecture/14-wc4y-beta118.md` §5, §14). Amends ADR-0071 §3, which made
the capture's skips a report only "until a two-sided recording shows the skips mark real losses".
The daemon alone (`aetherd/src/audio.rs`); no model, **no wire change, no configuration key, no
schema bump**. The harness's channel link greets `AEC3` and its server models the skips.

## 1. Context

A sound card's capture callback says when its first sample was captured. ADR-0071 §3 compares each
callback's timestamp with the last one's plus the samples it held: a jump forward of 10 ms or more
is audio the device's clock moved past that never arrived. The receiver heard the audio either side
spliced together, and a tone frame across the splice lost its timing: the sync blocks after it sit
where the first one does not say they are.

Report 14 had both stations' recordings of a session in which KK4ODA-1's capture reported two
28 ms skips, one at the start and one inside the second frame of a rung-4 transmission that
KK4ODA-1 never detected. The two sides agree the audio was lost: the offset between the two
recordings stepped by 74 ms there, the skips accounting for 56 ms of it, and the next frame landed
62 ms early on KK4ODA-1's clock. Putting 28 ms of zeros back at each skip and replaying decoded the
frame (93 bytes), and nothing else in the replay changed. The second gap had to sit at 75.46–75.49
s, just before where the skip was reported; 30 ms either side failed. The capture callback knows
that place exactly: the gap lies just before the first sample of the callback whose timestamp
jumped.

## 2. Decision

1. **The capture callback puts the skip back** (`capture_into`). A forward jump of
   `SKIP_MIN_S` (10 ms) to `PAD_MAX_S` (0.5 s) goes into the capture queue as that much silence,
   before the callback's samples (`CaptureSkips::note` returns it). Every later sample then stands
   where the air put it: a tone frame's sync blocks stay where its first one says, LDPC carries
   the symbols lost, and the station's clock stays on the air's.
2. **A longer jump is reported and not put back.** The largest the mini PC has reported is 148 ms.
   A jump of over half a second is more likely the card's clock stepping than audio lost, and
   silence for it would move the station's clock ahead of the air by as much. A jump back stays a
   report, as before.
3. **Not `capture_gap`.** `Station::skip_captured`, which the capture queue's own losses use, moves
   the clock on and builds the receiver again: right for the seconds a stalled loop loses, and a
   frame across them is gone anyway. For a skip of tens of milliseconds it would lose the frame
   the silence recovers.
4. **A frame a skip fell inside is still not trusted** (ADR-0076). The silence keeps the frame's
   timing, but its measurement holds the silence, and the rate controller should not learn from it.
5. **Said everywhere it was.** The `audio` warn line says the receiver heard silence in its place,
   or how much was in jumps too long to put back. The recording's `capture_skip` event says
   `lost <s> padded <s>`, and the recording holds the silence, so a replay hears what the receiver
   heard. The diagnostic bundle's `audio` gains `capture_padded_ms`.

## 3. Measured

* `a_capture_skip_is_put_back_as_silence_where_the_timestamps_jumped` (`audio.rs`): 28 ms lost
  between two 10 ms callbacks become 1 344 zeros between their samples; audio delivered twice adds
  and removes nothing.
* `a_frame_with_audio_lost_inside_it_decodes_once_the_silence_is_put_back`: a 500 Hz rung-4 frame
  (tone4x100-93) at 0 dB in 10 ms callbacks, 28 ms lost between two of them a third and two thirds
  into the frame. Spliced (a backend whose timestamps say nothing) it does not decode; with the
  silence put back it does, and the queue holds every sample where the air put it.
* `a_jump_in_the_capture_timestamps_is_audio_lost_and_jitter_is_not`: jumps up to the cap are put
  back, a jump of a second is not.

**The harness.** The channel server's `[stations.<who>.machine]` gains `skip_every_s` and
`skip_ms`: what fell in a skip never reaches the station. A daemon that greets `AEC3` is told each
skip ahead of the block after it (`MSG_SKIP`, as its capture timestamps would say it), and the
channel link puts it back through the same `CaptureSkips::note`; an older daemon hears the audio
run together, and its clock falls behind the air by the sum. `channel.json` lists each station's
`capture_skips`. The scenario `80m-capture-skips-500` is report 14's path with KK4ODA-1's capture
skipping 20–150 ms about every 30 s, four times as often as its mini PC on its worst evening:

| `80m-capture-skips-500`, three draws | completed | air, median | message, median | file, median |
|---|---|---|---|---|
| beta.119's daemon (no padding, the busy detector before ADR-0088) | 3 / 3 | 331 s | 65.3 bit/s | 79.4 bit/s |
| this build | 3 / 3 | 312 s | 70.5 bit/s | 104.5 bit/s |

At that rate few skips fall inside a frame (5–9 a session, 0.45–0.76 s in all), and the two builds
differ in their busy detector too: this one starts its sessions a second later, so each run is
another draw of the channel (ADR-0088 §5). To isolate the padding, the same scenario with a skip about
every 8 s (a scratch copy, not committed) ran four draws on this build and on this build greeting
`AEC2`, which hears the audio spliced:

| every 8 s, four draws | completed | answers dropped late | silent slots at the caller | the slave behind the air |
|---|---|---|---|---|
| spliced | **0 / 4** (link timeout or no response, mid-transfer or mid-ladder) | 8–15 a run | 6–17 | up to 1.3 s |
| put back | **4 / 4** | 0 | 0–2 | 0 |

The splice costs more than the frame across it. The station's clock is the audio it has heard, so
every skip puts it that much further behind the air. ADR-0069's keying gate judges a slot on the
air's clock (`CaptureLag`) and refuses an answer that would start past it, so once the sum of
the skips passes the slack the slave's answers are dropped as late one after another, the
caller's slots go silent, and the link times out. On the air
KK4ODA-1's mini PC skipped 2.9 s in eight hours, 49 skips in its worst 95 minutes: a long session
there would have reached the slack.
