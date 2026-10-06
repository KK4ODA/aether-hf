# ADR-0041: Each frame of a burst says how many follow it

**Status:** accepted, 2026-10-06. Model first (`phy/preamble.py` `follows_turn`/`follows_of`,
`FrameHeader.follows`, `rx.py` `ReceivedFrame.follows`; `link/phy.py` `TxFrame.follows`,
`SoftFrame.follows`; `link/engine.py` `_send_burst`, `_ack_after`), then the port
(`aether-phy` `preamble.rs`, `rx.rs`, `tx.rs`, `ofdm.rs` `pilot_symbol_turned`, `modem.rs`
`data_burst_following`/`rung_burst_following`/`Received::follows`; `aether-link` `phy.rs`,
`engine.rs` `ack_after`, `sim.rs` `without_countdown`/`collisions`; `aetherd` frame reports,
the `frame` event and sidecar frames carry `follows`). **Link protocol version 5**: beta.80 and
beta.79 do not connect. No configuration change.

## 1. Context

ADR-0040 found the receiving station acknowledging in the middle of the sending station's
burst — 21 times in ND1J's eight-minute Test of 2026-10-06 — because a burst carries no length
and the receiver takes silence after a frame for its end. When the burst's next frame arrives
faded, there is no silence to be had: the frame is there, just not heard. ADR-0040 heeds a
faint arrival where the next frame would begin; in four of the 21 there was no arrival at all.
Only the burst itself saying it is not over covers those.

The length cannot go in the DATA header: a retransmission is the same codeword under another
redundancy version, and the receiver adds the copies together, so nothing in the codeword may
change with a frame's position in a burst — and a retransmission usually lands at another
position in a later burst. The redundancy version had the same problem and solved it outside
the codeword, in the chips on the data carriers of the pilot symbols (§3.2 of the spec): one PN
sequence per (mode, RV), picked by the receiver by the magnitude of its correlation against
the chips, phase-referenced to the comb pilots' channel estimate.

## 2. Decision

1. **The countdown is a turn of the chips.** A DATA frame's chips are multiplied by `j^f`, `f`
   the number of frames of its burst that follow it, at most 3 (`MAX_FOLLOWS`; 3 = "three or
   more"). The receiver's (mode, RV) decision is by magnitude and does not see the turn; `f` is
   the nearest quarter turn of the chosen correlation's phase, and the pilot symbols are then
   known with the turned chips. `f = 0` is the frame of protocol 4, bit for bit.
2. **The sender counts its burst** (`_send_burst`): frame *i* of *n* says `min(n − 1 − i, 3)`.
3. **The receiver waits for what is announced** (`_ack_after`): after a frame of a burst the
   acknowledgement waits `f` frame lengths (the frame's own) before the reply delay — a frame
   lost in a fade holds the answer as surely as one heard. The countdown is believed from a
   frame that decoded or whose acquisition was trusted; a phantom's chips are noise. A later
   frame re-arms the wait from its own count.
4. **Tone frames carry none.** They have no chips; their arrivals are announced by a sync block
   and trusted (ADR-0013), and none of the collisions was a tone frame's.
5. Protocol version 5: a version 4 receiver would read a turned frame's chips as no sequence
   at all.

Considered: a 1-bit "more follows" (sign only) — the same cost, less information, and two
consecutive lost frames would still be answered over; a count in the acquisition preamble — the
Schmidl–Cox symbols name only the frame type, and more sequences there cost acquisition
threshold; a burst length agreed in the handshake — a sender's bursts are shorter whenever its
queue runs out, and the receiver would wait for frames that never come.

## 3. Measurements

**Reading the countdown** (`tools/bench_follows.py`, 100 frames a point, the first OFDM rung of
each air, a random count and RV a frame, carrier offset ±50 Hz): over AWGN, Good, Moderate and
Poor at −6, −3, 0, +3 and +6 dB, on both airs, **every frame whose mode and RV were read right
had its countdown read right — 3 615 of 3 615 — including those that did not decode**; none
read short. The same grid sent unturned (`--no-countdown`, the same draws) decodes the same
frames: 3 186 decoded with the countdown, 3 185 without, every point within the draws' spread.

**The link** (`a_receiver_does_not_answer_over_a_frame_it_lost_in_a_fade`, both suites: 500 Hz,
14 dB, three data frames in ten never detected, 3 kB): the receiver's transmissions over the
sender's fell from 19 to 0. On the link bench's fading pipe (`bench_link --fading`, 5 trials, 140
sessions an air) nothing changed — the same seconds to the second and the same acknowledgement
timeouts on both airs — because that pipe announces every frame it carries: the countdown only
acts when a frame goes unheard.

Through the real modem (`bench_link --backend phy --continuous --bandwidth 500`, Moderate and
Poor at 0 and +6 dB, 4 kB, two trials a point, the same seeds before and after): 2 615 → 2 510 s
in all (−4 %), frames resent 112 → 99, acknowledgement timeouts 13 → 10, no session slower. The
harness detects in a buffer that holds the frame it sent, so it seldom loses a frame outright —
the case the countdown is for; the air does (ADR-0040's 21 collisions in eight minutes).

## 4. Consequences

* A receiver answers a burst once it has ended, whichever of its frames it heard.
* A frame read with a wrong count is benign one way (a count too high waits up to three frames
  longer) and harmful only the other (too low); none was read low on the bench.
* The sender's own wait for the acknowledgement is unchanged: the receiver now answers at the
  burst's end where before it sometimes answered early.
* ADR-0040's rule stays: it covers a frame whose own count was never read.
* Tests: `test_a_data_frame_says_how_many_of_its_burst_follow_it` (model PHY, both airs),
  `a_data_frame_says_how_many_of_its_burst_follow_it` (port), the PHY vectors' three turned
  cases (bit-exact), `each_frame_of_a_burst_says_over_the_air_how_many_follow_it` (daemon, over
  audio), the fading-session test in both suites, and the protocol-version tests.
