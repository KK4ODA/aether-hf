# ADR-0020: What a failure says — the receiver learns only from frames that could tell it something

**Status:** accepted, 2026-09-25. Model first (`aether_model/link/engine.py`, `rate.py`,
`phy.py`), then the port (`aether-link`: `engine.rs`, `rate.rs`, `phy.rs`) and the daemon
(`core/aetherd/src/station.rs`: `trusted_measurement`, `PhyFrame::trusted`). The frames and
the link protocol are unchanged: an acknowledgement's SNR byte already had a value for
"unknown".

## 1. Context

ND1J ran a Test session to KK4ODA-1 on 7.082 MHz at 500 Hz (2026-09-25, 18:36 UTC, both
stations on beta.60). It completed — probe, call, a 1 kB message, the fifteen-rung ladder, a
1 kB file — but the link ran far below the path: the message at 73 bit/s and the file at
86, where the ladder had decoded rungs 8, 11 and 12 (64-QAM ⅔) four frames out of four at
about 8 dB, and `bench_link --replay` on the session's SNR trace carries the file at about
350 bit/s. Both recordings replay offline exactly as they decoded live, so the receivers were
not the problem.

ND1J's recording also decodes the acknowledgements KK4ODA-1 sent, which carry the rung the
receiving station recommended. They show its rate controller learning the wrong lesson three
ways:

1. **A noise trigger's SNR.** An acknowledgement reported the mean SNR of every frame in the
   burst, and one burst's failed frames read −11 dB between frames decoding at +5 to +8: the
   recommendation went from rung 4 to rung 1 in one burst. Those frames had mode chips at
   1.06 and acquisition at 1.13 — the daemon's own rule (`reported_cfo`) already calls that a
   probable noise trigger and keeps its carrier offset off the panel.
2. **Retransmissions that could not decode alone.** Replayed one frame at a time, the
   session's OFDM frames decoded 75 % at RV 0, 60 % at RV 3, and 6 and 10 % at RV 1 and 2.
   That is incremental redundancy doing what it should (TS 38.212 §5.4.2.1: RV 0 starts at the
   systematic bits and RV 3 wraps round to them; RV 1 and 2 are mostly parity) — a
   retransmission is meant to be combined with what came before. But it is also how the
   retransmission of a frame the receiver *already has* arrives: after an acknowledgement the
   sender missed (ND1J heard 44 of KK4ODA-1's 60 frames). Every frame of such a burst failed,
   every failure counted, and each lost acknowledgement taught the margin up to 3 dB.
3. **The Test's ladder.** The sender pins rungs past what the path carries, on purpose; the
   receiver does not know, and its controller took every failure at rungs 10, 13 and 14 as
   news of the path. It fell to rung 0 during the ladder and held its margin at the 12 dB
   ceiling for the whole file.

## 2. Decision

The receiving station's rate controller learns only from frames that could tell it
something about the path (`LinkEngine._send_ack`, `send_ack` in the port):

* **A burst's SNR is that of the frames that were really there**: those that decoded, and
  those the PHY acquired with confidence. `SoftFrame` gains `trusted` — true by default, and
  for the daemon's frames `trusted_measurement(chips, acquisition)`, the rule `reported_cfo`
  already used: chips at `MODE_RETRY_CONFIDENCE` or better, or acquisition at
  `DETECT_CONFIDENCE_TRUSTED` (1.3) or better. A burst with none reports no SNR (the
  acknowledgement's 0x7F), and its failure is what the controller learns from.
* **A failure counts when the frame could have decoded**: a trusted frame at a redundancy
  version that decodes on its own (`SELF_DECODABLE_RVS`: 0 and 3), or one combined with an
  earlier transmission of its block (`_RxRecord.combined`). A retransmission at RV 1 or 2 with
  nothing to combine with is no news.
* **A burst faster than any rung the station has asked for is the sender's choice.** The
  engine keeps the fastest rung it has recommended in the session (`_asked`); when every frame
  of a failed burst is faster than that, the controller takes its SNR and nothing else
  (`RateController.observe_snr`). Anything slower is judged as before: a retransmission keeps
  the rung it was first sent at, so after a step down the sender still sends rungs the
  receiver once asked for, and their failures are the path's. (Judging against the *last*
  recommendation instead, or the burst's most common rung, broke the fade-following test —
  those retransmissions were taken for the sender's choice.)

## 3. What it does

On the day's bursts, replayed open loop through the model's controller — the bursts split at
the acknowledgements KK4ODA-1 actually sent, the modes those ND1J actually used — the old rules
reproduce what happened (the margin at 12 dB within 33 s, the recommendation at rungs 0–3
through the rest of the message, 2–3 through the file), and the new ones hold the margin at
2–6 dB and recommend rungs 7–10 through the message and 7–9 through the file: about three and
a half times the file's rate.

On the link bench (the fading pipe with the floor's reading cap, 2 300 and 500 Hz, four
classes, −8 to +16 dB, six trials a point) the mean goodput is unchanged: 586.6 → 586.9 bit/s
and 235.1 → 235.7, with a few points a few bit/s either way. That is expected — the pipe's
frames are always real, and it decodes any redundancy version alone — and is the check that
nothing the bench can see got worse.

Tests: in both suites, a burst's SNR leaves out an untrusted frame and a burst of them reports
none; a burst faster than ever asked moves nothing but the SNR, one at an asked rung does; a
lone RV 1 or 2 failure is no news, a combined one and an RV 0 or RV 3 one are; and end to end,
a pinned ladder far past the path leaves the receiving station's margin no wider. The
rate-trace vectors carry a sender's-choice trace. `trusted_measurement` is tested on the
frames of the day.

## 4. Not done, and why

* **A trial climb off the tone rungs.** The tone floor's SNR reading is a lower bound on a
  dispersive path (ADR-0016), and a controller that falls onto it mid-session could stay
  there. On the bench, 500 Hz sessions on the Poor class still climb to rungs 12–14, and on
  the day the tone readings sat only 1–3 dB under the OFDM ones. Held until the air or a
  curve shows the need.
* **The pipe decodes any RV alone.** The real PHY does not, so the bench cannot show what a
  lost acknowledgement costs. Making it faithful is a P9-6 item, with its own baseline.
* **How fast a failure widens the margin** (ADR-0007). Failures of frames that could have
  decoded, at rungs the station asked for, still jump it by up to 3 dB each; field sessions
  will say whether 40 m's fades want a gentler step.

## 5. Consequences

* An acknowledgement says "SNR unknown" after a burst with no frame the receiver trusted, and
  a Test report's ladder rung can carry no SNR for the same reason; `field_ingest` and
  `bench_link --replay` already take a missing one.
* Rate control on a lossy acknowledgement path no longer ratchets: the lost acknowledgement
  still costs its retransmission, not a step of margin too.
* The Test's ladder measures the rungs without teaching the file that follows it.
