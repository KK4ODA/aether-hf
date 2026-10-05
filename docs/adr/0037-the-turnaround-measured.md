# ADR-0037: The turnaround, measured — no receive recovery window; the busy detector forgets the channel before its own transmission; the turnaround is recorded

**Status:** accepted, 2026-10-05. The daemon (`busy.rs` `skip`; `station.rs` `tx_end`, `rx_trace`
and `preamble` recording events, `forced_release`; `FrameRecord.start_s`/`end_s`), the tool
`tools/turnaround_plot.py`. No wire change, no configuration change.

## 1. Context

The FTDX10's spectrum scope shows a burst of noise just after every transmission, VARA's as well as
Aether's. ADR-0010 recorded the same flash in September and left it alone as the rig's receiver
recovering from its own RF. With KE4QCM's acceptances lost (ADR-0036), the author asked whether
Aether mishandles the moment after the key comes up: whether the receiver's recovery reaches the
audio and is read as noise, a busy channel, a lower SNR, a frame, or corrupted frames. The
reference was a 147 s phone video (4K, 24 fps, the rig's speaker audible) of a VARA HF 2750 Hz
Winlink session from KK4ODA to W4NWG on 3.587 MHz. It was recorded 2026-10-05 10:53–10:55Z with an
average SNR of −9.5 dB, on an FTDX10 with AGC AUTO and a 3 kHz roofing filter.

## 2. What the video shows

**Measured.** The speaker mutes while the rig transmits, which times every switch to 25 ms. The
scope crops were taken at the full 24 fps.

* **Rhythm.**
  * Handshake (0–21 s): this station keyed for 1.48 s, then heard 1.28 s, every cycle within ±50 ms.
  * Data phase: W4NWG's bursts heard for 5.2–5.8 s, each answered by a 1.48 s transmission. This
    station's own bursts were 5.05–5.5 s, each followed by a 1.6–1.8 s receive window.
* **W4NWG's answer** shows on the scope 0.2–0.35 s after this station's transmission leaves it
  (27.2 s, 57.5 s, 96.1 s and 135.3 s into the video). VARA does not keep a long guard; it answers
  about as fast as Aether (0.25 s plus the key lead).
* **After each transmission:**
  1. The scope is blank for 80–125 ms.
  2. A hump of noise appears for one or two scope frames (40–80 ms), 85–170 ms after the
     transmission. It is shaped like the 3 kHz receive passband, from the dial to about +3 kHz.
  3. For 100–200 ms after that the noise across the span is slightly raised.

  This happened after short and long transmissions alike, early and late: 5.2, 7.9, 44.2, 57.6, 96.3
  and 135.4 s. It was weak at 27.3 s and not seen at 82.3 s.
* **The speaker shows no surge.** Over the first 300 ms of each of the 28 receive windows the level
  stays within ±2–4 dB of the window's median, the scatter of the noise itself, with no rise.
  The hump is in the scope's path. It is not in the receive audio, the only thing a modem hears.
* **Early and late.**
  * The noise in the speaker was about 2 dB lower in the second half (median −7.4 to −8.5 against
    −5.5 to −6.4 dB, 1–2.5 kHz).
  * Sunrise on 80 m came during the recording, and D-layer absorption lowers the band's noise then.
  * On the scope, a floor that sits near its reference line drops out of sight for a decibel or two.
  * The camera's angle and exposure also changed after 24 s.

  The hump itself was still there at 96 and 135 s. The impression that the effect fades is more
  likely the lower noise across the span, which made the scope quieter between transmissions.

**Inference.**
* The rig mutes its receive audio through its own recovery. The flash is the IF chain coming back
  with its gain up (the AGC released while the receiver was blanked during transmit) and settling
  within a scope frame or two.
* Aether's recordings agree, and they are the receive audio itself. Over the 53 releases of the two
  WC4Y sessions of 2026-10-05 (`tools/turnaround_plot.py`):
  * the audio comes back 50 ms after Aether's release;
  * from then on it is within ±1 dB of the level 1.5–3 s later.

**Speculation, not pursued.** The scope's own processing may show the hump more strongly than the
receiver produces it. Whether VARA ignores audio after its release cannot be seen. Its peer's answer
lands 0.2–0.35 s after the release, and its own listening must have started by then.

## 3. What Aether does with the same moment (the code)

* The key comes up when the card's clock reaches the end of the burst's keyed silence (`key_tail_s`
  0.05 + `playback_lead_s` 0.25). `finish_transmission` then releases at the capture-clock time
  and sets `deaf_until` 0.75 s later (`CAPTURE_LAG_ALLOWANCE_S`, the FTDX10's measured 250–325 ms of
  digital silence plus room).
* **Deafness covers the busy detector only.** It also covers the passband monitor and the bandwidth
  module's quiet test. The receiver gets every sample after the release at once. Nothing in it is
  level-adaptive across a transmission:
  * the OFDM bank normalises each row by its own window's energy;
  * the tone detector judges each symbol against the other tones of that symbol;
  * OFDM noise is estimated per frame;
  * the impulse blanker's reference is a median over about 113 ms.

  A step from the rig's silence to noise does not bias any of them.
* **One defect.** `BusyDetector::skip` left the attack's 16 votes and the shape path's window as
  they were before the station keyed:
  * A peer's signal that ended within about 175 ms of the key left eight votes over the threshold.
  * The first block after the deafness then made the channel busy with nothing on the air: "Level",
    at a level equal to the floor (reproduced in a test: −36.9 dBFS against a −37.0 floor).
  * A peaked shape window could likewise be spliced with audio from after the transmission.

## 4. Decision

1. **No receive recovery window.** The transient the video shows does not reach the receive audio,
   in VARA's session or in Aether's recordings. A window that ignored the first few hundred
   milliseconds would cost a peer that answers as fast as W4NWG does, and buy nothing.
2. **`skip` clears the attack's votes and the shape window** (`busy.rs`). The channel before this
   station's own transmission says nothing about the channel after it.
   `a_signal_heard_before_our_own_transmission_does_not_make_the_channel_busy_after_it` fails
   without the fix.
3. **The turnaround is recorded**, so the next question about it is answered from a recording:
   * **`tx_end`** beside every release: the card's clock at the release, where it drains, the keyed
     tail, how long the station stays deaf, and whether the transmission was cut.
   * **`rx_trace`** every 50 ms for 3 s after a release: the block's own power, the busy detector's
     level and floor, busy, deaf.
   * **`preamble`** for every announcement, heeded or not: how long ago it started, its length, tone
     or OFDM, its confidence.
   * **`start_s` and `end_s`** on every frame record (`t_s` stays the time it was reported).
   * The watchdog and an abandoned transmission now write the release and its deafness too
     (`forced_release`). Before, they wrote nothing, and the replay stayed muted until the next
     release.
4. **`tools/turnaround_plot.py`** lines every release of one or more recordings up at t = 0. It
   draws the passband level against the level after it settles, the frames' starts and, from beta.76
   sidecars, the busy detector's trace. With `--detect` it finds the releases in any recording's
   own silence, so a VARA session recorded from the rig's USB audio plots beside an Aether one.

## 5. Consequences

* ADR-0036's premise stands, with a nuance:
  * VARA answers as fast as Aether, and VARA works with KE4QCM's SignaLink.
  * So either his DLY is short, or VARA survives losing the first 0.1–0.3 s of a frame where
    Aether does not.
  * Aether's answer frames begin with what acquisition needs: an OFDM preamble, or the tone floor's
    first 320 ms sync block. The tone detector also confirms on its middle and end blocks.
  * Whether a frame missing its first quarter second still decodes is the test to run next. It is a
    better fix than a longer gap if it fails.
* **Measured the same day** (`tools/bench_clipped.py`, 30 frames a point; the receiver's audio
  is zeroed for the frame's first part, as a radio still keyed or still switching delivers it):

  | Frame | Channel, SNR | First 0 / 100 / 200 / 300 / 500 ms lost: decoded |
  |---|---|---|
  | 500 Hz `tone-control` (calls, answers on the floor) | AWGN −10, Good −6, AWGN 0 | 30/30 at every clip; 28–30 on Good |
  | 500 Hz `tone-36` (floor data, connect bodies) | the same | 27–30 at every clip |
  | 500 Hz OFDM control, 0.43 s (ACKs, polls at an OFDM rung) | AWGN 0, Good +4 | 30 and 26 with nothing lost; **0 from 100 ms** |
  | 2 300 Hz OFDM control, 0.43 s | AWGN 0, Good +4 | 30 and 29; **0 from 100 ms** |

  The tone floor's three sync blocks carry a frame whose start was never heard. The ordinary
  family's control frame is all preamble at its start and only 0.43 s long, so a tenth of a second
  lost loses it whole. A peer still keyed by its VOX hold, or a receiver back late, loses every
  OFDM acknowledgement and poll and none of the floor's frames. That is KE4QCM's pattern: his
  floor calls and this station's floor acceptances get through, and the session dies at the first
  OFDM exchange. VARA's answer to the same hold is not visible. ADR-0036's gap is the fix this
  station can make alone.
* **This station's own keyed tail.** After a burst's last sample the key stays down for `key_tail_s`
  (0.05 s) plus `playback_lead_s` (0.25 s). Since ADR-0010 §7 the key is released against the
  card's own clock, so the playback lead in the tail now covers only the codec's buffering after the
  card's callback. An Aether peer answers 0.25 s plus its key lead after the frame's end. A tail
  that outlasts it by more than the radio's switching time costs this station the first part of the
  peer's OFDM answer, and with it the whole frame. The `tx_end` and `rx_trace` events measure the
  margin on the air. A shorter tail is the candidate fix, once a recording shows the margin is the
  problem.
* The sidecar grows by about 60 lines a release, under 3 s of trace each. The format is unchanged:
  new keys only, which older readers skip.

## 6. The controlled test (on the air, the author's)

1. Beta.76 on both ends, `[record] auto = true`. The same frequency and time of day for VARA and
   for Aether, back to back, so the band is the same.
2. For VARA, record the rig's USB receive audio with any recorder for the whole session.
3. `python tools/turnaround_plot.py aether.wav vara.wav --detect-for vara.wav --png both.png`.
   Compare:
   * how long the audio takes to come back after each release;
   * any rise in the first second;
   * from the Aether trace, whether busy goes on after a release with nothing heard.
4. A clipped-start test on the bench: replay a recorded answer frame with its first 100, 200 and
   300 ms zeroed, through `aetherd --replay`, to see what each frame family survives.
