# ADR-0066: A session starts at the rung the call measured, with what came with the call

**Status:** accepted, 2026-10-12. Model first (`RateController.first_mode`, `_required`;
`LinkEngine._first_move`, the `first` timer), then the port (`first_mode`, `required`;
`first_move`, `Timer::First`); the Test's message goes with its call in
`aetherd/src/station/fieldtest.rs`. No wire change, no configuration key: beta.105 and this
build interoperate.

## 1. Context

WC4Y's Test of 2026-10-11 15:52Z (80 m, 3.590 MHz, 500 Hz, Moderate; his side uploaded):
KK4ODA-1 heard his probe at −2 dB, he heard its answer at −9.5. The Test completed — probe,
call, a 512-byte message, an eight-rung ladder, a 768-byte file — and its message took 71 s,
57.7 bit/s, in two ways slower than the path:

1. **The first burst went two rungs low.** KK4ODA-1 heard his probe at −2 dB, and his call,
   a tone-floor frame too, about as well. With the controller's margin (3 dB) and hysteresis (1.5 dB) the fastest rung that
   fits is the floor's top, tone4x100-75 (rung 3, −13 dB on AWGN, needing −8.5): the rung above
   it is the first OFDM one, QPSK ⅓ at −6 dB, needing −1.5. `first_mode` keeps
   `first_mode_back` (2) rungs in hand below the fit, so the first burst went at rung 1,
   tone-36 — half the bytes a frame of rung 3 carries — with ten decibels to spare. Every burst
   after it went at rung 3 and decoded whole at −4 dB. ADR-0008 measured the two steps on the
   OFDM rungs, a decibel or two apart; the floor's top rung fits a measurement up to seven
   decibels above it, and counted in rungs the caution is that much larger.
2. **A poll went first.** The Test queued its message once the session was up. The engine
   reacts to the acceptance as it decodes it, before the daemon hears of the session, so a
   caller with nothing queued had already polled (ADR-0053) and the message waited for the poll
   and its answer: two floor frames, 7.4 s.

`bench/scenarios/80m-floor-top-start-500` reproduces both: the message at 57.2 bit/s on three
seeds of three, the poll, the first burst at rung 1.

## 2. Decision

1. **On the floor the steps in hand are decibels.** When the fit is a floor rung, the first
   burst goes at the fastest rung between the fit and the rung `first_mode_back` below it
   that the measurement clears by as much as those steps would have kept at the fit's own
   edge (`_required(fit) − _required(back)`). At the edge it is the two rungs, as before; a
   measurement well inside the floor's top rung starts there. On the 500 Hz ladder: a call
   read at −4.2 dB or more starts at rung 3 (was 1), from −5.5 dB at rung 2; on the 2 300 Hz
   ladder from −3.7 dB at rung 5, the fast kinds' fastest (was 3), from −5.6 dB at rung 4.
   Among the OFDM rungs nothing changes. The receiving station's seed, which is what its first
   acknowledgement recommends, uses the same function.
2. **A caller's first move waits for the next tick.** With nothing queued at the acceptance,
   the engine arms a timer of no length instead of polling at once; the daemon hands over what
   it holds when it hears of the session, in the same pass, and the timer then finds a burst
   to send. With nothing, it polls with the offer as before. A station that sends nothing as
   the session comes up — a Winlink client, whose gateway speaks first — polls in the same pass
   as before.
3. **The Test's message goes with its call** when the probe was answered and has sized it; the
   call's measurement still sizes it when the probe went unanswered (KE4QCM, 2026-10-09). A
   call that is not answered takes the message back out of the outgoing buffer, so it never
   opens the next session (ADR-0052).

## 3. Measured

`test_on_the_floor_the_steps_in_hand_are_decibels` and
`test_what_comes_with_the_session_goes_in_the_first_burst` in the model,
`on_the_floor_the_steps_in_hand_are_decibels` and
`what_comes_with_the_session_goes_in_the_first_burst` in the port, and the Test's own unit
test (the first three frames sent are the probe, the call and the message). The A/B over every
harness scenario, three seeds each, against the baseline taken on the same daemon code and
seeds (`bench/baselines/scenarios.csv`), is added here when its runs finish.

## 4. Not done

* **The climb.** The file went at rung 3 at −4 dB while the ladder had passed rungs 4–5 4/4 and
  rung 6 3/4 at −3/−4 dB. On 2026-10-11 00:57Z the same rungs passed 2/4, 1/4, 1/4 at the same
  readings: the margin that keeps the link at rung 3 is the path's, not a fault.
* **A countdown in tone frames.** A receiving station waits about a second after a tone burst's
  last frame to be sure no frame follows (KK4ODA-1 answered tone bursts 1.35 s after WC4Y's
  release, OFDM bursts 0.6 s). A last-frame flag in the tone frame's sync patterns would save
  that on every tone burst; it is a protocol change, for when one is due.
* **The ladder's failing rungs.** Three failing rungs in a row stop the ladder, and each failing
  rung's frames are retransmitted at that rung until they decode: 71 s of WC4Y's 345 s Test for
  rungs 7–9. Re-encoding them at the rung the receiver recommends would carry the same bytes in
  about the same air time, so nothing is gained by it; stopping sooner would measure less.
