# ADR-0053: The caller's first poll offers the turn

**Status:** accepted, 2026-10-08. Model first (`LinkEngine._send_poll(offer)`, `_poll_offer`,
the POLL branch of `_on_control`, `_turn_taken_unread`), then the port (`send_poll_offering`,
`poll_offer`); with it, a busy-detector fix in the daemon (`BusyDetector::mark_frame`). No
version change: beta.93 and this build interoperate.

## 1. Context

The three-clients scenario (`40m-gateway-three-clients-2300`, eight seeds) showed the gateway's
first 120-byte reply to its weak client taking exactly 23 s on five seeds and 10–11 s on three.
Not a timer: the weak path (0/+2 dB, Poor) sits on the line between the fast tone rungs and the
tone floor, and on the floor every frame has a fixed length. The reply waited behind:

| what | length |
|---|---|
| the client, with nothing to send, polls to confirm the session | 3.2 s |
| the gateway answers `WANT_TX` | 3.2 s |
| the client hands over the turn (`TURN`) | 3.2 s |
| the gateway's two data frames | 2 × 5.36 s |
| turnarounds | ~2.5 s |

Twelve of the 23 seconds are the change of direction. Every Winlink session through a gateway
opens this way: the gateway speaks first (its SID), the client has nothing queued.
ADR-0047's offer removes that round at the end of a burst, but a caller with nothing to send
sends no burst — it polls, and the poll offered nothing.

## 2. Decision

1. **The caller's first poll carries `OFFER`** (`offer_turn`, on). The called station answers
   it as it answers an offering `TURN`: an `ACK` with `TAKEN` and its first burst in the same
   transmission when it has data and nothing of the caller's is missing, the `ACK` alone
   otherwise.
2. **A lost `TAKEN` is read from the burst after it**, as after a burst's offer (ADR-0047): a
   trusted ordinary data frame while the offering poll is unanswered means the turn was taken.
3. **Only the first poll offers.** Later polls are keepalives or recovery, where the turn's
   holder is in question.
4. **No version change.** Control-frame flags are read without checking (`payload[0] & 0x0F`),
   so a station that knows no offer on a poll answers it as a plain poll, and a caller that does
   not offer gets the old answer.

## 3. A bug found on the way

The first A/B raised the three-clients scenario's collisions from 1 to 4 in eight seeds — all
at a client's leaving, none at a session's start. The gateway's `DISC_ACK` sometimes went
2–3 s after the client's `DISC`, and when the client's retry fell in that wait the two
collided. beta.93 had the same waits (2 in 8 seeds); the offer only moved timings so that more
of them met the retry.

The cause was the busy detector. When the `DISC` decoded, `mark_frame` named the busy channel a
frame's — but the attack window still held the frame's own loud blocks, and the next block found
a majority of them over the threshold and named it energy again. A `DISC_ACK` waits for energy no
frame accounts for (ADR-0022's wait for the other station's identifier), so it waited out the
hangover of the very `DISC` it answered. `mark_frame` now clears the attack votes, as `skip()`
does for the station's own transmission: the frame explains them
(`a_decoded_frame_explains_the_energy_that_was_its_own`).

## 4. Measured

Link simulator (`test_a_callers_first_poll_offers_the_turn`, both suites; eight seeds): a
120-byte greeting arrives 31.5 → 28.5 s after the call on the floor (−4…−6 dB), ~0.5 s sooner
on faster rungs.

The scenario harness, the same 24 scenario seeds on each build (`runs/ab53`, not committed):

| scenarios (seeds) | beta.93 | offer only | offer + busy fix |
|---|---|---|---|
| three clients (8): mean air / collisions | 216.4 s / 1 | 221.2 s / 4 | **203.4 s / 1** |
| — first reply, strong / middling / weak client | 4.6 / 7.9 / 19.9 s | 4.0 / 6.0 / 17.0 s | **4.0 / 5.9 / 14.0 s** |
| busy gateway when called (4) | 172.3 s / 11 | 145.7 s / 5 | **139.3 s / 2** |
| Trimode gateway (4) | 63.0 s / 0 | 63.4 s / 1 | 61.1 s / 0 |
| Winlink exchange, 40 m (4) | 83.9 s / 0 | 83.8 s / 0 | 83.9 s / 0 |
| Winlink exchange, 80 m 500 Hz (4) | 279.9 s / 1 | 279.9 s / 1 | 279.6 s / 1 |

Every run passed on every build. The Winlink exchanges are unchanged to the second: there the
client calls with data queued, sends a burst first, and never polls. Harness runs of one build
are not identical (daemon threads), so single-run differences of a few seconds are noise; the
first-reply times and the collisions are the signal.

## 5. Not done

* Offering on later polls (§2.3).
* The weak path's remaining late answers (2 in 34 in the three-clients runs) are the fading
  path's own: a client's acknowledgement keyed before the gateway's burst ended, and the floor's
  ordinary turnaround.
