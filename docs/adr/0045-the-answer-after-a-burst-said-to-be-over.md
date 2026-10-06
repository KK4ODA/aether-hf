# ADR-0045: The answer after a burst said to be over waits only the turnaround

**Status:** accepted, 2026-10-06. `LinkEngine._burst_closed` / `burst_closed` and the request's
quiet wait (`_reaction_s` / `reaction_s`), model first; `TwoStationSim(countdown=False)` in the
model and `SentFrame.t_start`/`t_end` in the port's simulator; the scenario runner looks every
20 ms. No wire change.

## 1. Context

The author, on ADR-0044's numbers: "3.5–6 seconds sounds quite high; in practice I never see either
station waiting that long, with Winlink or VarAC." He was right about the waiting. The figure was
the chat bench's from a reply being queued to its last byte arriving — its own air time and
acknowledgement included — and the scenario harness's account of the air shows the silences
between transmissions at a median 0.11 s, the longest 1.45 s. What a change of direction does cost
is transmissions: from the end of one station's data to the start of the other's, 2.55 s (an
acknowledgement and a TURN, 0.85 s keyed each, 0.84 s of silence) when the reply was ready by the
acknowledgement, 4.77 s (a request between them, 2.2 s of silence) when it was not. And part of
the second case was the harness: its runner looked for a delivery every 0.25 s of wall clock, a
second of air at four times real time, where Pat answers within milliseconds.

Two waits in that account had nothing left to wait for:

1. **The acknowledgement's silence.** The IRS answers a burst `_irs_reply_delay()` after its end
   — a preamble's detection, `burst_gap_s` and the turnaround, 0.57 s — because a burst carries no
   length and the receiver must be sure no next frame is starting. Since ADR-0041 each ordinary
   frame says how many of its burst follow it, and a count of 0 is exact (3 means "three or
   more"). Once the frame that ends latest has said none follow, the burst is over at its end.
2. **The request's quiet.** A station asking for the turn waits until the other's answer to its
   last transmission could have announced itself — sized for the tone floor's announcement
   (0.54 s), whichever family the session is in: 1.34 s.

## 2. Decision

1. When the frame that ends latest in the burst said, believably — it decoded, or its acquisition
   was trusted — that none follow it, the acknowledgement waits only the turnaround after it
   (`_burst_closed`). Otherwise, as before.
2. The request's quiet is sized for the family the other station was last heard in; before it has
   been heard at all, for the longer of the two as before.
3. The harness's runner looks every 20 ms (`POLL_S`), so a reply is queued about as soon after a
   delivery as a host program's.

## 3. Measured

* **Link bench**, chat shape with replies 0.2–1 s after delivery (`bench_chat.py`, 16 trials, both
  bandwidths, AWGN/Good/Moderate/Poor at 0/6/12 dB): the reply's median wait fell at 23 of 24
  points, typically 0.3–0.6 s (2300 Hz Good +12 dB 3.5 → 3.0 s; 500 Hz Good +12 dB 4.4 → 3.8 s),
  p90s with it; no line lost, no session dropped. Keyed time a line rose 2–5 %: the earlier
  acknowledgement more often goes before a reply 0.2–1 s behind it is queued, and the reply then
  asks for the turn on its own (requests 0.58 → 0.77 a line). A host that answers within
  milliseconds is not behind it.
* **Scenario harness**, this build against beta.83, the runner at 20 ms for both. The runner alone
  took the 40 m Winlink exchange from 131 s to 84–97 s on beta.83; the change then 87 → 81,
  97 → 92, 84 → 81 s (three seeds). At 500 Hz, 236 → 232, 294 → 238 and 288 → 330 s; the last
  lost its time to three collisions traced to the countdown's ceiling (a burst of six of which the
  receiver decoded only the first, "three or more", twice) and to a false detection that held an
  acknowledgement past the sender's wait — both on beta.83's rules too, neither on this change's.
  Every count read on a decoded frame matched what was sent. The nightly set: the 40 m RTTY and
  crashes Test completed (it aborted on the file on beta.83); the others within a run's spread.

## 4. Not done

* The keying around each transmission — 0.1 s of lead, and a tail that covers the sound card's
  buffering (`key_tail_s` 0.05 s plus the card's reported latency, or 0.25 s when it reports
  none) — is the radio's and the card's; any sound-card modem pays it. Cutting the lead risks the
  first part of the preamble on a slow transmitter (ADR-0037), and the tail's base is 50 ms.
* Two transmissions a change of direction remain: an acknowledgement asking for the turn, and the
  TURN. Folding the turn into the acknowledgement — the receiving station answering a burst that
  emptied the sender's queue with its acknowledgement and its own data in one transmission — is
  the next step and a protocol change (ADR-0046, to be planned).
* The countdown's ceiling: a burst of six whose later frames are lost is taken to end two frames
  early (ADR-0042's open item), and a false detection after a burst still delays its answer.
