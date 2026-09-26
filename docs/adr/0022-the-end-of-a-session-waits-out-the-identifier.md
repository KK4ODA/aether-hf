# ADR-0022: The end of a session waits out the other station's identifier

**Status:** accepted, 2026-09-25. Model first (`aether_model/link/engine.py`), then the port
(`aether-link` `engine.rs`) and the daemon (`core/aetherd/src/station.rs`: `held_for_busy`,
`append_cw_id`). The frames and the link protocol are unchanged.

## 1. Context

KK4ODA-1 closed a short chat session with ND1J on 7.082 MHz at 500 Hz (2026-09-25,
`20260925-225955_KK4ODA-1_ND1J`, both stations on beta.63), and its operator heard it transmit
while ND1J was sending his Morse identifier. The recording and the log show the sequence:

| s | KK4ODA-1 | ND1J |
|---|---|---|
| 140.5–141.3 | DISC | |
| 141.3–142.8 | listens: noise only | holds his DISC_ACK |
| 142.8–143.7 | DISC again | DISC_ACK (its last 150 ms audible at 143.7) |
| 144.0– | | Morse identifier, 1 500 Hz |
| 145.1–146.8 | DISC, and a fourth straight behind it in one keying | identifier |
| 147.9 | "closed (no disc ack)" | |

Four faults, two at each end:

1. **The answer waited for a clear channel.** ND1J's station waits for a clear channel
   (`wait_for_clear`). A session's frames are let through (`channel_clear`), but the engine
   answers a DISC and ends its session in one step, so the DISC_ACK reached the queue from a
   station that was already idle — and the DISC it answered had just marked the channel busy
   for two seconds (`BusyDetector::mark_frame`). The answer went 2.4 s after the DISC, past the
   caller's wait of 1.5 s. A probe's answer had been exempted for exactly this reason (OTA-2).
2. **A retry does not listen.** The caller's retry was timed only from its own DISC. It did not
   move for a frame it heard arriving, and a session's frames never wait for a busy channel, so
   nothing held a DISC over the identifier that follows the answer — which no frame announces.
3. **A false detection armed an acknowledgement.** Waiting for the DISC_ACK, the caller took
   the identifier for a data frame (mode 12 by its chips, −11 dB, undecodable), recorded it as
   a burst, and armed an ACK; a leaving station's ACK is another DISC, and it went out in the
   same keying as the retry.
4. **A station's own identifier was not in its waits.** The identifier is appended to a
   burst by the daemon (`append_cw_id`), after the engine has timed its wait for an answer from
   the end of the frames. A DISC carrying the identifier had its retry fall due inside the
   identifier and queued straight behind it, over the answer; and the station's closing
   identifier, queued as the answer to its DISC decoded, went out over the identifier the
   other station appends to that answer.

## 2. Decision

* **Engine** (model, then port): a sender in `DISCONNECTING` does not take data frames — there
  is no burst to acknowledge. A disconnecting station that hears a frame arriving moves the
  DISC's retry past that frame's end and the time to decode it (`on_preamble`, as a caller and
  a prober already do).
* **Daemon**, `held_for_busy`:
  * A DISC_ACK is a response and is never held by `wait_for_clear`.
  * The end of a session waits out the other station's identifier, whatever `wait_for_clear`
    says, for at most `OTHER_ID_WAIT_MAX_S` (15 s: ten characters at 10 wpm take thirteen):
    a DISC while the channel is busy for any reason but a decoded frame (the frame it follows
    marks the channel itself); a DISC_ACK the same, after listening `ANSWER_LISTEN_S` (0.5 s)
    — a DISC carrying an identifier is decoded before that identifier has sounded long enough
    for the detector's attack (200 ms in 400); and a standalone closing identifier while the
    channel is busy for any reason, since the answer it follows is decoded before the
    identifier behind it can be heard.
* **Daemon**, `append_cw_id`: the engine's timers move by the identifier's length plus the
  busy detector's hangover (`on_tx_delayed`), since the other station's answer comes after the
  identifier and after its own detector has let the channel go.

## 3. Consequences

* An orderly close between two identifying stations keys nothing twice and never keys both at
  once, with or without `wait_for_clear` at the answering end and whether or not the DISC
  carries the caller's identifier (`a_disconnect_is_answered_at_once_and_nobody_keys_over_the_answer_or_an_identifier`,
  two daemons over real audio). The engine's two rules are tested in both suites.
* Every DISC_ACK leaves half a second later. A DISC and a closing identifier can wait up to
  15 s on a channel something else holds; past that the session closes anyway.
* **Open:** the identifier a station owes every ten minutes rides on whatever it transmits
  then, mid-session, and the other station's next frame — an ACK after a burst, a burst after
  an ACK — is not held for it: session frames still go whatever the channel holds, because
  holding them for a busy channel would stall a session under any interference. Telling the
  other station that an identifier follows, and for how long, needs the frame to say so (the
  DISC's unused `base`/`bitmap` could carry it; an ACK has no spare field) and is a protocol
  change for another day.
