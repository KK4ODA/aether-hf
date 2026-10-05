# ADR-0038: ND1J's two sessions — a hopeless frame is re-encoded sooner, a frame of nobody's session is no part of a burst, the keyed tail follows the sound card, and a waiting beacon does not go into a session

**Status:** accepted, 2026-10-05.
* **Kept:** the link engine (early re-encoding; frames outside the session; model first, both
  suites) and the daemon (the keyed tail from the card's reported delay, `AudioIo::
  output_latency_s`; a waiting beacon dropped when a session starts).
* **Built, measured and not kept:** pulling the recommendation down from control frames, and
  answering on the tone floor by the peer's report.
* No wire change, no configuration change.

## 1. Context

Two sessions in which ND1J called KK4ODA-1, on 3.590 MHz at 500 Hz, 2026-10-05 20:09 and 20:14
UTC (beta.76, KK4ODA's side only: sidecars and log, no audio). In both, ND1J was the sending
station (ISS) and KK4ODA-1 the receiving one (IRS).

* **Session 1.** The path was +9 to +11 dB at the start: his probe, his beacons, his first polls.
  It faded to about −1 dB within 90 s. The fade was symmetric: his polls report hearing this
  station at 9, 5, 2, 0 and then −1 dB.
  * Two frames at rung 10 decoded at +7 dB. Then he was idle, polling every 12 s.
  * At 118 s he sent again, at rung 12 into −1 to −5 dB, and 30 frames failed.
  * A retransmission keeps the rung its frame was first sent at (ADR-0020), and this station's
    acknowledgements, short OFDM frames at about −1 dB at his end, mostly did not reach him.
  * His frames stayed at rung 12 through RV 0–3 and round again for 100 s, until the link timed
    out.
  * A beacon of his, heard at 260 s, drew an acknowledgement: by then his side had very likely
    given up, while this station still held the session.
* **Session 2.** His rung-4 and rung-6 frames decoded here at −3 to +3 dB. His polls said he
  heard this station at −3 to −5 dB, at the edge of the ordinary control frame (−4.5 dB at
  500 Hz).
  * He resent what had arrived, polled four times in the ordinary family and disconnected after
    100 s.
  * A current build moves unanswered polls to the floor after two silences (ADR-0032), so his
    station probably runs an older beta. His files will say.
* **The turnaround** (ADR-0037's trace, on the air for the first time):
  * this station's audio came back 60–120 ms after each release;
  * his answers began 90–180 ms after it — a margin of 0–100 ms;
  * `tools/bench_clipped.py` loses a short OFDM frame whole from 100 ms on.

## 2. Decision

1. **A hopeless frame is re-encoded sooner.** A frame is re-encoded after
   `reencode_early_after` (2) transmissions instead of `max_combines` (4) when the peer's last
   measurement of this station's frames (`peer_snr_db`) is further below the frame's rung
   threshold than combining all `max_combines` transmissions could make up: 10·log10 of their
   number, plus `reencode_hopeless_db` (1 dB). Its copies were being combined toward a sum that
   could never decode.
   * Only a measurement will do. Judging by the rung the peer recommends was tried and broke
     the HARQ tests: the recommendation sits a margin and the ladder's spacing below the path,
     and on the tone floor 15 dB below a frame that combining still rescues.
   * Link bench (fading pipe, ±8 dB ramp, 4 kB):
     * all 288 sessions at −6 to +12 dB on Good, Moderate and Poor, both bandwidths: total time
       +0.1 % at 500 Hz and +0.6 % at 2300 Hz, no session lost;
     * the 180 sessions at −6 and −3 dB at 2300 Hz: −0.3 % in all;
     * an ablation with the old count reproduces the old numbers exactly, so all of the
       difference is this rule.
2. **A frame of nobody's session is no part of a burst** — a beacon, a probe or its answer, a
   datagram, another session's frame. It used to be:
   * counted as a failed frame of the burst, which widened the margin;
   * acknowledged, by the acknowledgement its preamble had armed.

   Now it is dropped from the burst, and with nothing else in the burst the acknowledgement is
   withdrawn (`_RxRecord.outside` / `RxRecord::outside`).
3. **The keyed tail follows the sound card.**
   * The playback callback now reads cpal's own timestamp of when its first sample will leave
     the device, and keeps the longest delay reported (`AudioIo::output_latency_s`; the run
     loop passes it to `Station::device_latency`).
   * The tail after a burst's last sample is `key_tail_s` plus that delay plus 30 ms
     (`DEVICE_LATENCY_MARGIN_S`). It is never less than 50 ms (`DEVICE_LATENCY_MIN_S`) and never
     more than the assumed playback lead (0.25 s).
   * A card that reports nothing keeps the whole lead, as before.
   * `tx_end` records the tail used and the card's report.
   * The quarter second was kept for a card that typically needs a few tens of milliseconds, and
     it is what ate the margin above.
4. **A beacon still waiting when a session starts is dropped.** It was held for a busy channel,
   and the busy channel was the call. `beacon()` already refuses during a session; this is the
   one queued before the session began. The log says so: `beacon: dropped: a session started
   before it went out`.

## 3. Measured and not kept

* **The recommendation pulled down by control frames.** A poll's SNR fell to the fastest rung it
  carried, without a failure, never climbing (`RateController.track`). Replica of session 1: a
  message at +10 dB, a fade to −2 or +2 dB during 60 s of idle polls, then 1 kB; 16 sessions a
  point, all delivered either way. Median time after the fade:
  * 500 Hz: 65 → 77 s and 25 → 29 s;
  * 2300 Hz: 18 → 40 s and 10 → 7 s.

  Where the acknowledgements arrive, a burst at too fast a rung fails and its failure brings the
  rung down within a burst. Pulling it down in advance, through the margin, went further than the
  path needed (rung 7 where rung 12 still worked at 2300 Hz).
* **Answers on the floor by the peer's report.** The IRS answered on the floor while the peer's
  last report was within 3 dB of the ordinary control frame's threshold, with 2 dB of hysteresis.
  Lopsided 500 Hz path, 1 kB, 12 sessions a point, the way back 4, 6 or 8 dB weaker: identical
  delivery, and 0–3 s slower. That held against a sender without ADR-0032 too.
  * At those SNRs the session already runs on the tone rungs, and its answers with it.
  * Where the data is fast enough to be OFDM, the short answers at −2 to −4 dB mostly arrive.
* What would have saved both sessions is the other station reading this one's answers. That is
  ADR-0032 and ADR-0034 on a current build, and the "not read" flag ADR-0034 left for a protocol
  change.

## 4. Consequences

* Sidecars from beta.77 carry `latency` in `tx_end`. The first recording on the author's FTDX10
  says what WASAPI reports and how much margin the shorter tail bought.
* Open: whether ND1J's build predates ADR-0032 — his files will say. If it does, the answer is an
  update on his side, not a change here.
