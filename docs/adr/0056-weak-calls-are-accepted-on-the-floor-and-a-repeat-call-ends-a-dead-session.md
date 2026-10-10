# ADR-0056: A weak call is accepted on the floor, and a station that calls again ends its dead session

**Status:** accepted, 2026-10-10. Model first (`LinkEngine._weak_call`, `_called_again`,
`LinkConfig.floor_answer_margin_db`), then the port (`weak_call`, `called_again`,
`floor_answer_margin_db`). No wire change: beta.95 and this build interoperate.

## 1. Context

KE4QCM's sessions of 2026-10-10 (3.590 MHz, 500 Hz, his FT-100 keyed by VOX) read from
KK4ODA-1's sidecars and audio:

1. His calls alternate families (ADR-0016). Those that decoded here came in the **ordinary**
   family at +1…+6.5 dB, and the acceptance went back in the family the call arrived in
   (ADR-0009). The ordinary acceptances did not reach him: the path was lopsided (he heard this
   station several decibels worse than it heard him, as ND1J's 0/+5 dB path had been), and his
   VOX hold took the start of whatever came back quickly.
2. Having heard no acceptance, he gave up and **called again**, which starts a new session
   number. This station was still in the first session, waiting for his first frame, and took
   the new call for another session's frame — ignored (ADR-0038) — until the first session's
   link timeout ran out. By then he had stopped calling.

## 2. Decision

1. **A call heard in the ordinary family less than `floor_answer_margin_db` (12 dB) above the
   ordinary control frame's AWGN threshold is accepted on the tone floor** — below about +7 dB
   on either air. The acceptance is the one frame a session cannot do without; the floor reaches
   14 dB lower, and costs 2.6 s more air once a session, on a path where the ordinary answer was
   a gamble. A strong call is answered in its own family as before. The caller decodes both
   families whatever it sent, and its wait moves past a frame it hears arriving (ADR-0016).
2. **A call from the station this one is in session with, under a new session number, ends the
   session and is answered as from idle** (`disconnected` with the reason *<call> called
   again*). One station is in one session: a new number from the same callsign means it has
   given the old one up, whatever this station thought. A call from anybody else during a
   session is still no business of the session's.

## 3. Measured

`tools/bench_calls.py`, 500 Hz, −6…+6 dB, AWGN/Good/Moderate/Poor, 40 trials each: identical
before and after (every call connects, median 11 s). The bench's paths are symmetric and the
first try is on the floor, so its acceptance already went on the floor; the rule only acts on
an ordinary try that arrives, which on that pipe is a strong one. The model's
`test_a_called_again_session_completes` loses the first acceptance, has the caller give up
silently and call again, and the message crosses under the second session.

## 4. Not done

* Answering on the floor whenever the caller's own report of this station (none exists before
  the session) would say the path is lopsided: the protocol carries no such report in a call.
