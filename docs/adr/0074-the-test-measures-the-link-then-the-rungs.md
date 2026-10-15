# ADR-0074: The Test measures the link, then the rungs

**Status:** accepted, 2026-10-15, the author's decision on report 12 §21
(`docs/research/link-architecture/12-two-paths-80m.md`). Engine rule model first
(`aether_model/link/granted.py`: `_TxRecord.pinned`, `_data_frames`), then the port
(`aether-link/src/granted.rs`) and the replay vectors (the `ladder` session). The Test itself is
the daemon's (`aetherd/src/station/fieldtest.rs`); the panel's step list follows it. No wire
change: a station on this build and one on v0.2.0-beta.106 to beta.109 work together, and Test
with each other.

## 1. Context

The author, reading report 12's change 2 (R9's silence step): *isn't this pointing to how a Test
session is designed? A Test should mimic real link behaviour.* It was. The Test ran probe → call →
message → **ladder** → file → disconnect. The ladder pinned four 10-byte frames at each rung,
recorded the first answer as the rung's measurement, and then waited for all four to be
**delivered** before the next rung. A frame that failed went again at the rung that had just
failed, at RV 0, 2 and 3, until re-encoding took it to the recommendation. On ND1J's Tests of
2026-10-15 (report 12 §21):

* in the two morning Tests the ladder took 299 and 272 s, of which 130 and 134 s delivered
  filler;
* its resends at pinned rungs were 13 of the 21 silent slots;
* its re-encoded 10-byte bodies, padded to 75 bytes, were the "lopsided" frames;
* in morning 2 its last rung's frames were still undelivered when the file began. Two silent
  slots on them stepped ND1J's rung down, and the file's first bursts went at rungs 0 and 1. That
  was change 2's whole measured case.

The file the Test reported on a weak path was not the link's rate.

## 2. Decision

1. **The transfers first, the ladder last**: probe → call → message → file → ladder →
   disconnect. The message and file are measured as an operator's transfer would go, with nothing
   pinned before them. The file is sized from the message's rate in the budget left less the
   ladder's reserve (`LADDER_RESERVE_S`, 150 s). The ladder starts a rung under the fastest the
   transfers were acknowledged at, and starts no rung with under `LADDER_RUNG_LEFT_S` (60 s) left.
   The workarounds the old order needed are gone: `file_reserve_s` (the time the ladder left the
   file) and `file_rate_bps` (the message's rate scaled for the rung the ladder left the link on).
   * ADR-0017 had put the ladder first because three Tests with ND1J on a slow path (2026-09-25)
     spent the budget on the transfers and never reached a rung. The file has been sized to the
     path since, and now also leaves the ladder its reserve.
2. **A ladder frame is measured once** (engine, R9). A frame sent at a pinned rung is re-encoded
   at the recommendation after its first copy, when the recommendation is lower, and goes on as
   any data does. The first copy, in the rung's answer, is the measurement (`LadderRung`
   counts only fresh frames); a second copy at a rung that failed measures nothing and delays
   everything.
   * *The rule it restores*: R9's "the sender follows" the recommendation, for everything but a
     measurement's first copy. It moves no deadline: it changes the rung of a frame's next copy.
3. **The Test ends at the last rung's answer.** It goes to the disconnect without waiting for the
   ladder's leftovers. Filler nobody reads is delivered within the disconnect's drain (ADR-0072),
   or left with the DISC. Between rungs the Test still waits for delivery, so a rung's burst
   holds only fresh frames.
4. **Report 12's change 2 is not made.** The silence step stays as R9 has it. Its one measured
   case was the ladder's leftovers running into the file, which 1–3 remove.

## 3. Consequences

* **The field log.** The ladder's results (`ladders.csv`) mean the same as before: first copies,
  decoded or not. Message and file rates from this build on are the link's own; from beta.109
  and before, on a weak path, the file carried the ladder's leftovers and reads low. The log's
  notes say which build a Test ran.
* **The panel** shows the steps in the new order (*step 5 of 6, climbing the mode ladder*).
* **The ladder's filler still costs a frame per failed 10-byte body** at the recommended rung:
  5.36 s on the tone floor. Frames that measure without carrying data — a frame kind the slave
  counts in its answer and never delivers — would cost nothing, but need a new kind and an answer
  field: a protocol change, left for a later bump.

## 4. What was measured

**The model**: `test_a_ladder_frame_is_measured_once_and_then_goes_at_the_rung_asked_for` fails
without the rule (pinned frames sent two to four times at the pinned rung) and passes with it
(once each, then at the recommendation, everything delivered). ADR-0073's header test made its
RV 2 and 3 copies with the ladder; it now makes them with a fade of the session's data, which
keeps them coming for ordinary data.

**The harness**: see §5.

## 5. The harness A/B

To be filled from the run of this change: the 14 scenarios that run a Test, three draws each,
against v0.2.0-beta.109's daemon.
