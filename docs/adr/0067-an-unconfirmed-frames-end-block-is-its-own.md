# ADR-0067: An unconfirmed tone frame's end block is still its own

**Status:** accepted, 2026-10-12. Model first (`ToneStream._lapsed`, `_announce` in
`aether_model/phy/tone.py`), then the port (`lapsed`, `announce` in `aether-phy/src/tone.rs`).
Receiver only: no wire change, no configuration key; the vectors are unchanged.

## 1. Context

Found by the A/B of ADR-0065 and ADR-0066 on `20m-polar-path-2300` (20 m across the auroral
zone: 3 ms spread, 10 Hz Doppler, 12 dB), in the build before them as much as after. Of the 307
tone arrivals the receiving stations heeded over the first 48 runs, 19 never became a frame
report, and three of those — all on the polar path — began exactly where an earlier announced
frame's **end block** begins: a frame length less one sync block after it (5.04 s for a data
frame, 2.88 s for a control frame).

A tone frame's end block repeats its first (ADR-0013). `ToneStream._announce` takes a first
block found inside a frame already announced for that frame's own middle or end block
(`_own_block`), and an announcement lapses once its frame's end has passed. The end block is
read as a first block only once the block is in — as the frame ends — and by the time the
reading is judged the announcement it belongs to has lapsed. When the frame was taken, the
overlap with the frame taken stops the reading (ADR-0060). When it was not — the Doppler spread
kept it from being confirmed — the reading was announced as a frame of its own.

The station heard a 5.36 s frame arriving that never came. A receiving station held its
acknowledgement to that frame's end (ADR-0061): G4XYZ's answer went out 6.4 s after the real
frame began its end block, past the sender's wait, and the sender's retry and the answer were
keyed together (collision at 151 s of the polar run). A leaving station's DISC and the other
station's resent burst met twice the same way at the session's end. On the air, WC4Y's
acceptances of 2026-10-05 failed confirmation with their end block a symbol late (ADR-0049):
the same frames.

## 2. Decision

An announcement whose frame ends untaken is kept for two sync blocks after its end, for the
own-block test only (`_lapsed`): a first block read at its middle or end block, or weaker
than it inside its span, is that frame's, after the frame's end as before it. The next frame
of a burst begins at the frame's end, outside its span, and is announced as ever.

## 3. Consequences

`test_an_unconfirmed_frames_end_block_is_not_announced_as_a_frame` (model, a data and the
control kind, the frame kept from confirming) and
`an_unconfirmed_frames_end_block_is_not_announced_as_a_frame` (port) fail without the change.
The harness A/B is in ADR-0066 §3.
