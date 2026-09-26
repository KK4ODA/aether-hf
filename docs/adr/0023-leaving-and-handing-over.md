# ADR-0023: Leaving and handing over — a receiver's disconnect, the TURN's wait, and who keeps the turn

**Status:** accepted, 2026-09-25. Model first (`aether_model/link/engine.py`, and the
simulator's `unheard` hook in `sim.py`), then the port (`aether-link` `engine.rs`, `sim.rs`) and
the daemon and panel (`status.closing`; the Disconnect button). The frames and the link protocol
are unchanged, and a station of an earlier version works with this one.

## 1. Context

Seven sessions between KK4ODA-1 and KE4QCM on 2026-09-25 (7.082 and 3.590 MHz, 500 Hz, betas
.63–.65) were on a poor path: control frames decoded at −1 to −9 dB, data only now and then.
Two things in them were not the path.

**Disconnect did nothing.** Five of the seven ended "aborted": the operator pressed Disconnect,
nothing happened, and he pressed Abort. In every one KK4ODA-1 was the receiving station, and a
receiving station's disconnect only put its DISC in place of the next acknowledgement — which
comes when the other station's next burst does. On a path where nothing decodable was arriving
there was no next acknowledgement.

**Both stations held the turn.** In the session of 23:30:58 KE4QCM (the caller) handed the turn to
KK4ODA-1 (TURN at 59.8 s), which sent its message as a tone-floor frame: tone-36, then tone-24,
5.4 s each, every 11 s, never acknowledged. KE4QCM polled at 91 s and 122 s — it had taken the
turn back — and KK4ODA-1, sure it held the turn, ignored both polls and gave up: "no response".
The cause is in the TURN: its sender waited for the answer for as long as one frame of the rung it
recommends — about a second of OFDM — and the called station's first burst after a fade is on the
tone floor, five seconds long. The TURN was repeated over the answer, the sender deaf while it
keyed, three times, and then it "carried on as the sender". The called station's control frames
had been reaching KE4QCM all session (its acknowledgements asked for the turn and got it), so a
poll answered would have been heard.

## 2. Decision

* **A receiving station leaves between bursts.** `disconnect()` on the receiving side sends the
  DISC at once unless a burst is arriving (a frame recorded, or an acknowledgement due), whose
  acknowledgement it then replaces as before. A sending station still sends what is queued
  first. `disconnect_requested` (the port's `disconnect_requested()`) says a close is pending;
  the daemon reports it as `status.closing`.
* **The TURN's wait covers the longest first frame there is** — the tone floor's data frame, or
  the recommended rung's if that is longer — and a frame heard arriving while it waits moves the
  next TURN past it (`on_preamble`, as a DISC's retry does since ADR-0022).
* **The caller keeps the turn.** A sending station that hears the other station poll knows both
  hold the turn. The called station yields: it takes the receiving role, answers the poll, and
  its acknowledgement asks for the turn back. The caller ignores the called station's poll as
  before, so the two can never both yield. `_caller` is set when a connect is accepted.
* **An acknowledgement asks for the turn (WANT_TX) whenever the station has work**, frames sent
  and not yet acknowledged included. Only the queue counted, so a station that gave up the turn
  with frames in flight — a BREAK, or now a yield — asked for nothing back.

The panel: Disconnect reads **Stop calling** while a call is being made and aborts it (no session
has come up, so there is nothing to close); the line under the buttons says what a close is
waiting for, and that Abort closes at once.

## 3. Consequences

* The receiving side's Disconnect closes in seconds (the bench pair: 5 s, both stations idle).
  Tested in both suites with a sender that polls nobody for five minutes.
* A fade that lets control frames through and no data no longer ends in "no response": the
  called station yields to each poll, is handed the turn again, and its message goes when the path
  lets it (both suites: a 150 s fade of the called station's data, `TwoStationSim(unheard=…)` /
  `with_unheard`). Without the yield the test ends "no response", as the session did.
* A lost TURN is repeated after about six seconds rather than two. It is rare, and a repeat keyed
  over the answer is worth nothing.
* A session whose control frames flow and whose data never does now lasts until an operator
  closes it — which Disconnect now does promptly.
