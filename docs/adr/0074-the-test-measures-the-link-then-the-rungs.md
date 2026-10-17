# ADR-0074: The Test measures the link, then the rungs

**Status:** accepted, 2026-10-15, the author's decision on report 12 §21
(`docs/research/link-architecture/12-two-paths-80m.md`). Engine rule model first
(`aether_model/link/granted.py`: `_TxRecord.pinned`, `_data_frames`), then the port
(`aether-link/src/granted.rs`) and the replay vectors (the `ladder` session). The Test itself is
the daemon's (`aetherd/src/station/fieldtest.rs`); the panel's step list follows it. No wire
change: a station on this build and one on v0.2.0-beta.106 to beta.109 work together, and Test
with each other. **The pinned frame's re-encode after its first copy is superseded by
ADR-0077** (link protocol 9): the ladder measures with frames of their own, which are never sent
again; the order (transfers, then the ladder) stands.

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

The 14 scenarios that run a Test, three draws each (`session_matrix.py --seeds 3`), this build
against v0.2.0-beta.109's daemon.

| | beta.109 | this build |
|---|---|---|
| runs passed | 42 of 42 | 42 of 42 |
| radiated overlaps | 0 | 0 |
| silent slots (the masters' counters) | 295 | 211 |
| message rate | | the same (×1.00 median; it goes before the ladder either way) |
| file rate, per-scenario median | | ×1.30 (×0.67 to ×1.88; lower in 4 of 14) |
| the Test's length, per-scenario median | | ×1.01 (×0.83 to ×1.28) |

Per scenario, the medians of three draws (message and file in bit/s):

| scenario | file | ladder rungs | Test s |
|---|---|---|---|
| `80m-weak-both-ways-ladder-500` (ND1J's path) | 32 → 61 | 8 → 8 | 476 → 575 |
| `80m-floor-top-start-500` | 48 → 84 | 7 → 8 | 454 → 423 |
| `80m-asymmetric-500` | 60 → 82 | 7 → 6 | 342 → 355 |
| `80m-flutter-500` | 50 → 74 | 7 → 10 | 412 → 529 |
| `80m-lopsided-acks-500` | 59 → 68 | 8 → 9 | 478 → 535 |
| `80m-slow-machine-500` | 62 → 76 | 6 → 7 | 448 → 491 |
| `80m-local-rfi-ladder-500` | 78 → 72 | 10 → 7 | 463 → 420 |
| `80m-nvis-evening-500` | 30 → 26 | 5 → 5 | 530 → 452 |
| `20m-dx-moderate-2300` | 397 → 632 | 6 → 8 | 294 → 304 |
| `40m-good-2300-rtty-crashes` | 196 → 338 | 7 → 6 | 346 → 340 |
| `40m-moderate-2300-vara-qrm` | 489 → 555 | 8 → 7 | 282 → 279 |
| `80m-thunderstorm-agc-2300` | 289 → 403 | 7 → 8 | 274 → 298 |
| `40m-drift-and-clocks-2300` | 519 → 446 | 9 → 7 | 368 → 336 |
| `40m-rtty-contest-2300` | 173 → 115 | 8 → 3 | 493 → 411 |

* **The file is the link's own now.** It rose most where the ladder's leftovers had run into
  it: ND1J's path ×1.9, the 40 m crashes ×1.7, the 20 m DX path ×1.6.
* **Where it fell, the ladder had been warming the link up.** On the good 2 300 Hz paths the
  old ladder's clean bursts at high rungs lowered the receiver's margin before the file: a climb
  the link would not have made on its own. The new number is what an operator's transfer gets.
* **A longer Test is the ladder getting its time.** Flutter's ladder tried 10 rungs where it
  had 7: the old order stopped it to leave the file's time.
* **One draw lost its ladder.** In the RTTY contest's first draw the file crawled under the RTTY
  (87 bit/s) and ran to the budget, and the run ended without a ladder. On beta.109 the same draw
  ran its ladder and then lost the file the same way. The transfers come first now; the report
  says *ladder skipped: the file took the budget*.
* **Within the nightly's baseline.** Every Test scenario's air time and rates lie inside its
  stored ten-run range, so the nightly flags nothing slower.

**Found by the A/B, and fixed (outside the protocol).** In three draws across both builds a file
that crawled ended the session on the Test's deadline, and the sidecar had no outcome and no file:
the session's end closed the recording (the harness's stations record every session) before the
run wrote its report. While the Test steps, its run is out of the station, so the session-end
handler took no Test to be running. It is now told (`test_stepping`), and the session history
marks such a session a Test, which it had not.
`a_file_that_crawls_ends_the_run_with_its_report_in_the_sidecar` fails without the fix.
