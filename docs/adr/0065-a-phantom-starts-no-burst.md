# ADR-0065: A phantom starts no burst, and a left session's DISC waits out what it hears arriving

**Status:** accepted, 2026-10-12. Model first (`LinkEngine._phantom`), then the port (`phantom`
in `aether-link/src/engine.rs`); the DISC's hold is the daemon's (`arriving_until`,
`held_for_busy` in `aetherd/src/station.rs`). No wire change, no configuration key.

## 1. Context

WC4Y's Test of 2026-10-11 15:52Z (80 m, 3.590 MHz, 500 Hz; his side uploaded, KK4ODA-1's not):
after the ladder's rung-3 burst KK4ODA-1's acknowledgement counter went from #5 to #7, and #6
never reached WC4Y. His audio holds nothing in the 6.4 s he listened after his rung-4 burst
(the passband within a decibel of the noise), he sent the burst again, and #7 acknowledged all
four frames. So #6 was keyed while WC4Y was transmitting — over his burst or over his resend.

**Corrected the same day, from KK4ODA-1's side of the session:** #6 was not keyed over him. It
went 0.35 s after his release, an *ordinary* acknowledgement to the ladder's first OFDM burst,
to a station that had read this one's floor acknowledgements at −7.8 and −9.2 dB — under the
500 Hz ordinary control frame's −6 dB threshold — and his receiver never detected it. That is the
answer's family, not a phantom (ADR-0068 §2). The harness's phantoms below are real, and this
decision stands on them.

`bench/scenarios/80m-floor-top-start-500` puts the two stations on that path (Moderate, −3 dB
one way, −9 the other). Three seeds on beta.105 had three collisions, all of one kind:

1. The receiving station had acknowledged a burst. Within the next few seconds the detector
   reported an ordinary DATA frame — acquisition 1.02–1.05, under the trusted 1.3, read at −3 to
   −13 dB, chips naming rungs 4, 9 and 11 — that failed to decode: noise, or the other station's
   next frames misread, their start lost in the receiver's recovery after its own key came up.
2. `_on_data` took any frame of the session that reached it for a frame of a burst. A failed
   frame alone started one, and the acknowledgement was armed for its end plus the reply delay:
   0.6–0.7 s after it, believing nothing of its countdown.
3. The other station's next transmission had begun under it and was not yet announced, or was
   still on the air. The acknowledgement — 3.2 s on the floor since ADR-0059 — went over it:
   twice over the end of a three-frame retransmission, once over the first frame of a 22 s
   burst at rung 0.

The same runs showed the phantom's other face. An ordinary detection over the end of a tone
burst's last frame joined that burst, and the burst's end moved to the phantom's — the answer
came a second later than after any other burst, 2.0 s instead of 1.1 s in the harness. WC4Y's
last one-frame burst of the day was answered 2.70 s after his release where every other tone
burst was answered in 1.35 s.

Tone-floor frames are confirmed over their sync blocks before the detector reports them
(ADR-0049, ADR-0060); the phantoms were all ordinary.

## 2. Decision

1. **An ordinary DATA frame that did not decode and whose acquisition the PHY does not trust
   is part of a burst only after an ordinary frame of that burst.** Alone, or after floor
   frames, it starts nothing and holds nothing: no acknowledgement is armed for it and the
   burst's end does not move to it. A burst is of one family (ADR-0009), so after tone frames
   it is nobody's. What it carried for combining is kept, as for a frame of a burst already
   answered (ADR-0047).
2. Unchanged: inside an ordinary burst such a frame is the burst's — a faded last frame as
   often as not (ADR-0040); a trusted frame that fails still starts a burst and is answered (a
   burst at a rung the path cannot carry); tone frames are as before.
3. **The DISC of a session left — an abort's, its repeats, the answer to a frame of the session
   left unheard (ADR-0052) — waits out the frames this station hears arriving**, whatever
   `wait_for_clear` says, up to the key-time limit and 5 s. Found by the same A/B: a Test whose
   budget ran out aborted, and its DISC went 1.4 s into the other station's acknowledgement,
   which it had heard arriving; both were lost and the session ended in DISC repeats keyed over
   the answer.

## 3. Measured

The harness A/B is in ADR-0066 §3, which changed the same sessions' start.

`test_a_phantom_heard_alone_is_not_acknowledged`,
`test_a_phantom_over_the_end_of_a_floor_burst_does_not_hold_its_answer` and
`test_a_phantom_after_a_frame_of_the_burst_is_still_the_bursts` in the model,
`a_phantom_heard_alone_is_not_acknowledged` and
`a_phantom_over_the_end_of_a_floor_burst_does_not_hold_its_answer` in the port, and
`an_abort_waits_out_a_frame_heard_arriving` in the daemon (which fails without the hold).

## 4. What KK4ODA-1's side said

Whether #6 on 2026-10-11 was this: no (§1). The phantom's other face was there, though: the
session's last answer went 2.46 s after the last tone frame's end, held by an ordinary detection
0.32 s after that end (acquisition 1.07, failed at −12.6 dB) that the daemon heeded as a
follow-on (ADR-0040). This decision keeps the failed frame out of the burst; the announcement
alone still holds an answer to the phantom's end, which ADR-0068 retires.
