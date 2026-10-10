# ADR-0059: Acknowledgements go on the floor once one was lost

**Status:** accepted, 2026-10-10. Model first (`LinkEngine._note_ack_lost`, `_acks_on_floor`,
`_RxRecord.new`), then the port (`note_ack_lost`, `acks_on_floor`, `RxRecord::new`). No wire
change: the receiver of a burst already decodes acknowledgements in either family.

## 1. Context

WC4Y's Test of 2026-10-10 (80 m, 500 Hz, Moderate) completed, but slowly: 1 kB at 48 bit/s, a
ladder cut short by the budget, a 256-byte file. Both stations' sidecars, matched frame by frame:

| KK4ODA-1's acknowledgement | reached WC4Y |
|---|---|
| ordinary (0.43 s), at −0.5…−8 dB | **7 of 15** |
| tone floor (3.2 s), at −4…−7 dB | **14 of 14** |

The path was lopsided: KK4ODA-1 heard WC4Y at about 0…+3 dB, WC4Y heard KK4ODA-1 about 4 dB worse
(at 50 W against 20–25 W: his noise is the higher). An acknowledgement goes in the family of the
burst it answers (ADR-0009), so while WC4Y sent OFDM data every acknowledgement was the short
ordinary frame, at its threshold. Each lost one cost WC4Y an acknowledgement timeout (11 in the
session), the burst sent again — once as a whole burst of RV 2 copies, which never decode alone —
and a step down the ladder: the link fell from rungs 5–7 to the tone floor again and again.

ADR-0038 measured answering on the floor by the peer's SNR report and found nothing on a lopsided
path. The report is a prediction; a lost acknowledgement is the fact.

## 2. Decision

1. **A burst whose decoded frames are all blocks the receiver already had says its last
   acknowledgement was lost.** A sender only sends an acknowledged block again when it did not hear
   the acknowledgement. (A frame that does not decode says nothing: it may be new.)
2. **From then on the receiver's acknowledgements go on the tone floor, for the rest of the
   session** (event `acks`: *on the floor: WC4Y missed an acknowledgement*). Its other control
   frames keep their families. The sender already waits for an acknowledgement in the longer of its
   burst's family and the family it last heard the receiver in (ADR-0016), and moves its wait past
   a frame it hears arriving.
3. A session starts with ordinary acknowledgements again: a path's asymmetry is a guess until it
   costs something.

## 3. Measured

Link simulator, 2 kB, 16 seeds each, the receiver's ordinary control frames 0/4/8 dB under the
channel (`frame_snr_offset`), median session time, off → on:

| air | penalty | 0 dB | +4 dB | +8 dB |
|---|---|---|---|---|
| 2300 | 0 dB | 67.8 → 67.8 s | 41.5 → 41.5 | 28.5 → 28.5 |
| 2300 | −4 dB | 93.3 → 100.0 (+7 %) | 41.5 → 41.5 | 28.5 → 28.5 |
| 2300 | −8 dB | 602.8 → **107.9 (−82 %)** | 62.6 → 64.5 (+3 %) | 28.5 → 28.5 |
| 500 | 0 dB | 107.4 → 109.3 (+2 %) | 65.7 → 65.7 | 44.6 → 44.6 |
| 500 | −4 dB | 159.3 → **148.4 (−7 %)** | 65.7 → 65.7 | 44.6 → 44.6 |
| 500 | −8 dB | 1023.3 → **160.2 (−84 %)** | 116.2 → **99.8 (−14 %)** | 44.6 → 44.6 |

Every run delivered. The cost where the rule fires without need is a floor acknowledgement's
2.8 s more air an exchange; the gain where acknowledgements are a coin toss is the bursts not sent
again and the ladder not abandoned.

## 4. Harness

`80m-lopsided-acks-500` (WC4Y's path: the caller heard at +1 dB, hearing the other at −4 dB,
Moderate, a Test), six seeds, and four other 500/2300 Hz scenarios, beta.96 against this build:

| `80m-lopsided-acks-500`, 6 seeds (means) | beta.96 | this build |
|---|---|---|
| Tests complete | 6/6 | 6/6 |
| the caller's acknowledgement timeouts | 12.3 | **3.5** |
| frames sent again | 66 | **43** |
| message, bit/s | 49.0 | 51.6 |
| file, bit/s | 47.5 | 53.6 |
| highest ladder rung passed | 7.5 | 8.0 |
| collisions | 1.3 | 1.7 |

A Test runs to its budget whatever happens, so its air time says nothing; the rates are what move,
and the single-seed spread of the harness (daemon threads) is as large as the difference, so the
timeouts and the repeats are the signal. `80m-asymmetric-500` (lopsided the other way: the
called station's acknowledgements go the good way) passes on both, Test rates 66.9/63.4 →
66.9/56.3 bit/s; `80m-winlink-exchange-500`, `40m-winlink-exchange-2300` and
`40m-good-throughput-2300` are identical to the tenth of a second.
