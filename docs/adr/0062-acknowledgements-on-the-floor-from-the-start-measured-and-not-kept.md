# ADR-0062: Acknowledgements on the floor from the start — measured and not kept

**Status:** rejected, 2026-10-11. Built (model and port, `LinkConfig.ack_floor_margin_db`),
measured on the scenario harness, and reverted. No wire change.

## 1. Context

ADR-0059 sends a receiving station's acknowledgements on the tone floor once one has been lost:
a burst whose decoded frames are all blocks already had says the acknowledgement did not
arrive. Each session on a lopsided path pays for that once. In WC4Y's Test of 2026-10-11 the one
lost acknowledgement (a short ordinary one during the ladder's rung 4) cost an 11 s
retransmission before the rest went on the floor and all arrived.

The question was whether a station should send its acknowledgements on the floor from the
start when it already knows the other station hears it weakly: from that station's own report
(`peer_snr_db`) or, before it has reported, from how this station hears it. That is the test
ADR-0056 already applies to accepting a call: the ordinary control frame's AWGN threshold plus
a margin.

## 2. What was measured

`ack_floor_margin_db` (unset, 12 dB, 6 dB) on the harness: `80m-lopsided-acks-500` and
`80m-asymmetric-500` four seeds each (Test sessions), and `80m-winlink-exchange-500`,
`80m-weak-500`, `80m-qsb-dropout-500` and `40m-winlink-exchange-2300` once each.

| | eight Tests, air time | collisions | Winlink 500 Hz |
|---|---|---|---|
| unset (ADR-0059 alone) | 3 227 s | 0 | 355 s |
| 12 dB | 3 491 s (+8 %) | 2 | 396 s |
| 6 dB | 3 182 s (−1 %) | 4 | 313 s |

The other three scenarios were within a few seconds of each other in every build.

## 3. Decision

Not kept. At 12 dB the floor acknowledgement's extra 2.7 s of air in every exchange outweighs
the acknowledgements it saves; at 6 dB the difference is inside the harness's run-to-run spread
(a single build differs by more between two runs) and the collisions doubled. ADR-0059's rule —
the floor once one was lost — stays the only one.

## 4. Consequences

None in the code: the option was reverted. If a field path shows several ordinary
acknowledgements lost before the first data burst repeats (ADR-0059 can only react to a lost
acknowledgement it can see), this is where to start again.
