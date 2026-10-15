# ADR-0073: A header where no copy can be read alone, and rungs counted by their data

**Status:** accepted, 2026-10-15, after the author read report 12
(`docs/research/link-architecture/12-two-paths-80m.md`, §12 and §12.1). Model first
(`aether_model/link/granted.py`: `READABLE_ALONE_RVS` and the elision in `_send_header`;
`aether_model/link/rate.py`: `usable_modes`), then the port (`aether-link/src/granted.rs`,
`rate.rs`) and the vectors (`granted_vectors.json`, `link_vectors.json`, regenerated for this
change). No wire change: a header is a frame every slave already reads, and the rate rule only
changes what a station recommends. A station on this build and one on v0.2.0-beta.106 to beta.108
work together.

The same release carries two changes outside the protocol, from the same report: the daemon's
`keying` line for a transmission that went on the air late in its slot, and the panel's speed
chart drawing the rung the data moved at each way (§5).

## 1. Context

ND1J's three Tests to KK4ODA-1 of 2026-10-15, read from both stations' recordings, had 22 silent
slots: master transmissions the slave did not answer. 21 were transmissions without a header, at
rungs above the one the slave had last recommended: the Test's pinned ladder rungs and their
retransmissions. Two rules met in them.

* **R7, header elision.** The master may leave a header out when "the exact counts locate the
  end": each data frame states how many follow it, so the slave can place its slot from the data
  alone.
* **R2, since Step 3.** A count is believed only from a copy that decoded alone. A count read off
  a copy HARQ rescued had once read none-to-follow where three did, and the slave answered over
  them (10-step-3.md §6).

RV 1 and RV 2 copies never decode alone, on any rung, and RV 3 only on the rungs whose RV 3
carries every systematic bit (ADR-0043). So a headerless transmission of nothing but such copies
can grant nothing, whatever the SNR. The slave, rightly, stays silent. The master waits out the
slot (4.6 s on the floor) and polls. 8 of the 21 silent slots were exactly that: rungs 5, 6, 7, 9
and 14 sent again at RV 2 and 3.

Separately, the rate controller offered the 500 Hz air's rung 4 (OFDM QPSK ⅓). It decides which
rungs are worth offering by bytes per second of PHY payload. Counted that way rung 4 beats rung 3
(tone4x100-75), 114 bit/s to 112. But every DATA frame spends 3 of its bytes on its header: a
fifth of rung 4's 15-byte frame and a twenty-fifth of rung 3's 75. After the header rung 4
carries 91 bit/s against rung 3's 107, and needs 7 dB more signal. Rung 4 was worth offering
before ADR-0015 put the four-tone kinds under it, when the rung below it carried 49 bit/s.

## 2. Decision

1. **R7 leaves a header out only when one of the data frames is an RV 0 copy**
   (`READABLE_ALONE_RVS = {0}`). RV 0 is the only version that carries every systematic bit on
   every rung. A per-rung table of where RV 3 also decodes alone would let a few more headers go;
   an all-RV 3 transmission is a frame's fourth copy, rare, and the header costs it 3.2 s at most.
   * *The invariant it restores*: R7's own premise, that the counts locate the end, and with it
     L2 (a lost answer costs one slot): every master transmission now carries a grant the slave
     can read on its own — its header, or a copy whose count R2 believes.
   * *No deadline moves on undecoded evidence.* The rule adds a header, which is decoded evidence.
2. **A rung is offered only if no other carries as much data per second of air from a threshold
   no higher**, the data being what a DATA frame carries after its 3-byte header
   (`usable_modes`). On the 500 Hz air this drops rung 4 and nothing else. The 2 300 Hz ladder is
   the same either way.

**Not decided here: R9's silence step** (report 12 §12, change 2). It stepped the master's rung
two below the recommendation after silences on transmissions sent above it, and cost morning 2's
file about 27 s. All of it came from the Test's ladder: its pinned rungs, and its failed frames
sent again at the pinned rung before re-encoding. Whether to change the rule or the Test is the
author's question (report 12 §21).

## 3. What was measured

**On the air** (report 12 §12.1, the three sessions' records): a header on every transmission of
copies that cannot be read alone would have saved about 5, 14 and 19 s at no cost. A header on
every transmission above the recommendation, the broader version first proposed, breaks even
(−0.3, −2.4 and +16.6 s): 18 such transmissions were answered anyway.

**The model**: `test_a_transmission_no_frame_of_which_can_be_read_alone_keeps_its_header` fails
without the rule. The replay vectors hold 13 transmissions with no RV 0 copy across five sessions
(`dead-path`, `floor-memory`, `harq-limit`, `ramp`, `rescued-count`). Every one now carries its
header, and the port matches the model on each.

`test_a_count_rescued_with_its_block_places_no_end` checks two things: that no rescued count misleads
the slave, and that the test would see it if one did, by believing such counts and expecting an
overlap. With this rule, the 500 Hz slave-side case no longer overlaps even with rescued counts
believed (16 seeds): every transmission there whose only counts were rescued ones was of RV 2 and 3
copies, and now keeps its header. Two rules now guard that case, so its check removes both. The
other three cases still show R2 alone is needed.

**The rate rule on the link bench's fading pipe** (report 12 §18.3): 3 kB sessions 0–13 % faster
from −2 to +2 dB on every class, nothing slower. Single short transfers (20–300 bytes, four
classes, −6 to +2 dB, ten trials) pooled ×1.005, ×0.982, ×0.954, ×0.953 of the time with rung 4.
Chat sessions were not run: the chat bench drives protocol 7's engine only.

**The harness**: see §4.

## 4. The harness A/B

The daemon built from this change against the one before it, 13 scenarios, three draws each
(`session_matrix.py --seeds 3`). The scenarios: the new `80m-weak-both-ways-ladder-500` (ND1J's
path of the second morning), the 80 m 500 Hz Tests and exchanges (`floor-top-start`,
`lopsided-acks`, `local-rfi-ladder`, `nvis-evening`, `weak`, `asymmetric`, `flutter`,
`qsb-dropout`, `winlink-exchange`), and three at 2 300 Hz (`quick-awgn`, `40m-good-throughput`,
`40m-winlink-exchange`).

| | before | after |
|---|---|---|
| runs passed | 39 of 39 | 39 of 39 |
| radiated overlaps | 0 | 0 |
| headerless transmissions with no RV 0 copy (the masters' recordings) | 92 | 0 |
| silent slots (the masters' counters) | 230 | 166 |
| air time, pooled | | ×0.982 |
| message rate (21 Test runs) | | median ×1.05, mean ×1.18; lower in 4 |
| file rate (19 Test runs) | | median ×1.06, mean ×1.08; lower in 8 |

* **Every transmission the rule targets now carries its header**, and a quarter of the silent
  slots are gone. The rest are first copies at rungs the path cannot carry, mostly the Test's
  pinned ladder (report 12 §21).
* **Nothing overlapped and nothing failed.** The 2 300 Hz scenarios are unchanged to the second.
* **The timing moved the right way, within the harness's spread.** One build on one scenario
  and seed ran 435 and 521 s of air on two runs, so three draws judge only the pooled figures.
  Step 3's thresholds (air ×1.10 pooled, Test rates ×0.85) hold with room.

## 5. Outside the protocol

* **A late keying is said.** When a transmission of the link's goes on the air 50 ms or more
  after its slot opened, or within 100 ms of its limit, by the station's own reckoning at the key,
  the log says so with what took the time: the render, the radio's keying command, how far the
  station is behind the air, each beside what it typically takes (`note_slow_keying`,
  `keying_late` in the recording, `late` in the diagnostic bundle's `keying`). On ND1J's first
  morning one answer of KK4ODA-1's rendered in 122 ms where 60 was typical and reached the air
  281 ms into its 350 ms slack, and the log said only that the radio was keyed. What the sound
  card does after the key is not in the reckoning. Capture skips stay out of `capture_gap`: they
  reported 36 of the 91 ms lost (report 12 §15.2).
* **The speed chart draws the rung the data moved at, each way.** `metrics` gains `sending_rung`
  and `receiving_rung`: the rung of the session data last sent and last decoded, in the
  throughput's 30 s window, null where none moved. The panel draws *sending at* in the transmit
  colour and *receiving at* in the receive colour, against a goodput counted both ways. The chart
  had drawn the rung the station would send at. While it only answered, that line moved nothing:
  KK4ODA-1's showed tone-36 under data arriving at tone4x100-75, and goodput stood above it, which
  the manual said could not happen.
