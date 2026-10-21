# ADR-0086: A stuck frame goes in pieces

**Status:** accepted, 2026-10-21. Model first (`pieces`, `DataFlags.FRAGMENT` and the piece
header in `model/aether_model/link/frames.py`; `_split_rung`, `_piece_frames`, `_Pieces` and
the receiver's rules in `granted.py`), then the port (`frames.rs`, `granted.rs`), held together
by `link_vectors.json`'s new `pieces` cases and the regenerated `granted_vectors.json` with a
`split` session. **Link protocol 11:** a station of version 10 cannot connect to this one. No
configuration key.

## 1. Context

ADR-0085 left one case open. A stranded frame — sent four times unacknowledged at a rung the
recommendation has since left — is re-encoded at a slower rung, but only if its body fits one.
A body fixed at a fast rung often fits nothing slower: 72 data bytes at 500 Hz rung 3, or 50 at
an OFDM rung, against 48 at rung 2. Such a frame went on at its own rung, a rung the path had
refused four times, until the session timed out or the path came back.

That is what failed CI on the release commit of beta.118 (`80m-agc-carrier-outside-500`, its
own draw, on CI's Ubuntu runner, twice): ITU NVIS at 10 dB on the 500 Hz air carries the floor's
rungs 0–2 and nothing faster. A 7 ms echo smears rung 3's and rung 4's 10 ms symbols and is
longer than OFDM's 5 ms prefix. Frames committed at rung 3 or at an OFDM rung were never
delivered, and the message missed its 300 s budget. A model session on a path that refuses every
data frame from rung 3 up delivered nothing of a 1 kB message in 900 s.

A body's bytes cannot move to another sequence number: the receiver acknowledges by sequence
number, and HARQ combines copies under it. The DATA header has no field that says a frame
carries part of a body. Hence a wire change.

## 2. Decision

**The frame.** A DATA frame's flags gain `FRAGMENT` (0x02). A frame with it carries a *piece* of
the body of the frame with its sequence number, and two more header bytes: the piece's offset in
that body (12 bits) and `last` (1 bit, the piece ends the body), three bits zero. The data length
of a partial frame follows them. `pieces(body, capacity)` cuts a body into full frames' worth;
a last piece one byte short of full is the one length the container cannot carry, so it goes
as two.

**When.** A stranded frame is first re-encoded whole, as before, but no faster than the
recommendation: a rung above it strands the frame again (an OFDM body of 50 bytes went whole to
rung 3 under a recommendation of rung 2, and stuck there). A body no rung up to the
recommendation carries goes in pieces at **the usable rung of the recommendation's family up to
it that cuts the body into the fewest pieces, and of those the most robust**. Every frame of a
family is as long as the others, so the pieces are the air, and among rungs that need as many
the slowest costs nothing — ADR-0085's argument, for pieces: a 50-byte OFDM body is two pieces
at 500 Hz rung 2 and two at rung 1, and goes at rung 1; 72 bytes are two at rung 2 and three at
rung 1, and go at rung 2. The first build took the fastest rung up to the recommendation, which
gave the 50-byte body the less robust of two equal choices.

**After one copy more than whole re-encoding waits for.** The receiver tries a frame's fifth copy
with the four it holds before it starts over, and a body in pieces leaves all of that behind:
on the scenario's draw, a rung-4 frame split after four copies never got the fifth that
ADR-0085's build sent, which decoded, and the message missed its budget (6 of 24 runs on the two
builds that split after four, §3). So a frame goes in pieces after `max_combines` + 1 copies, or after
`reencode_early_after` when the other station's report says combining cannot reach its rung
(ADR-0038's *hopeless*). A block already in pieces is never combined, and goes again in pieces
after `max_combines` transmissions.

Only below **the rung the other station itself last recommended**: never on a step down for a
silent slot alone (R9). With the band gone, the master stepped its recommendation onto the
floor on silence, and OFDM frames split onto the floor made each try 25 s long against 9.5 s.
Its eight silent slots then outlasted what a timed-out slave reckons the master takes to give
up, and the two closing identifiers went on the air together
(`test_a_band_that_closes_ends_with_the_identifiers_apart`). A frame split once and stranded
again under a lower recommendation is split again, at the slower rung, at other boundaries.

**Sending.** Pieces go in transmissions of pieces alone, every one at RV 0:

* the receiver gives a frame that does not decode a sequence number by its slot in the
  transmission, from what its last acknowledgement said was missing, and a piece has no slot of
  its own in the sequence: a whole frame beside pieces would be placed wrong;
* a piece is never combined, so its every copy must decode alone.

Nothing says which pieces arrived — the acknowledgement's bitmap is by sequence number — so a
block sends each piece once before any again, as a block sent whole sends all its
unacknowledged frames again, and the blocks go in order. Filling the room left with repeats was
the first build; it cost a frame's air a repeat whether or not its piece was lost, at the rung
the other station recommends, where few are.

**Receiving.** A receiver places pieces by offset, so the pieces of two splits make the body as
well, and acknowledges the sequence number only once every byte has arrived. Once a piece
decodes in a transmission, its frames that did not decode get no sequence number and no HARQ
buffer, and what they had been given is dropped; and while a block it holds pieces of is
incomplete, it infers no sequence number for any frame.

**Counted:** `frames_split` (model `GrantStats`, the port's, the daemon's `counters`).

## 3. Measured

**The model**, a 500 Hz session on a path that refuses every data frame from rung 3 up (10 dB,
a 1 kB message, eight seeds): ADR-0085's build delivers nothing in 900 s, this one delivers in
430 s. The model tests add a path that also loses every block's first piece above rung 0, so
blocks are split twice at different boundaries (`test_a_stuck_frame_goes_in_pieces_and_its_block_arrives`),
and one where no report calls a rung hopeless, so every block goes whole five times at its last
rung before it is split (`test_a_frame_goes_in_pieces_only_after_the_copy_its_receiver_combines_last`).

**The scenario that failed CI**, `80m-agc-carrier-outside-500` at its own seed, 31, twelve runs
a build on this machine (beta.117's and ADR-0085's rows are ADR-0085 §3's):

| `80m-agc-carrier-outside-500`, seed 31 | delivered | message, median (range) |
|---|---|---|
| beta.117 | 9 of 10 | 286 s (203–290) |
| ADR-0085 (beta.118's code) | 11 of 12 | 259 s (203–294) |
| this ADR, split after four copies, the fastest rung | 10 of 12 | 213 s (213–266) |
| this ADR, split after four copies, the fewest pieces | 8 of 12 | 213 s (213–266) |
| this ADR | 12 of 12 | 266 s (213–294) |

The lost runs of both builds that split after four copies took one course, frame for frame:
rung 2 decoding 9 of 23 frames and rung 4 none, and a rung-4 block that ADR-0085's build sent a
fifth time at 308 s — decoded, the message in at 294 s — split instead into two rung-2 pieces
that did not both arrive before the budget ran out. Hence the fifth copy (§2). None of these
runs took CI's own course (rung 3 refused for the whole session with 72-byte bodies there):
the sessions part on the daemons' thread timing, and CI's runner is the test of that course.

**The link bench** (the fading pipe, ADR-0085's rules against this ADR's first build, the same
seeds): identical at every point — both airs, AWGN, Good, Moderate and Poor at −8, −4, 0 and
+4 dB, 20 (500 Hz) and 16 (2 300 Hz) sessions of 1 kB, and on the fading classes 12 and 8
sessions of 8 kB. No session split; the two rules added since change only when and at which
rung a split goes, so they cannot move it either.

**The whole harness**, 56 scenarios × 2 draws against the same draws of beta.118's code: 112 of
112 runs pass on both, no radiated overlap, pooled air time 15 716 → 15 761 s (+0.3 %), the
Tests' message rate ×1.00 and file rate ×0.98 (medians over 15). One run split a block
(`80m-trimode-gateway-client-vanishes-500`, its second draw). Whole re-encodes fell from 39 to 7:
beta.118 re-encoded frames above the recommendation under silence steps, six a run in
`15m-opening-closes-2300` and `40m-closing-identifiers-2300`, whose air is unchanged and −5 %.
The largest moves either way (`80m-slow-machine-500` +13 %, `80m-agc-carrier-outside-500`
+10 %, `80m-varac-chat-and-ping-500` −11 %) are one draw each taking another course, and the
nightly's comparison with `scenarios.csv` (`tools/scenario_trend.py`) finds nothing slower or
failing more than the baseline's own range in any of the 56.

## 4. Alternatives

* **Re-encode whole above the recommendation** (ADR-0085's rule, which allowed it): §2. On this
  path the rung above is the rung that does not carry.
* **A piece's index in place of its offset.** Shorter, but a block split again at another rung
  could not use what had arrived of the first split, and the receiver would have to drop it.
* **Pieces beside whole frames in one transmission.** The receiver could not place a whole frame
  that did not decode by its slot; it would need the slot inference to know the pieces' count,
  which the header does not carry, and combining a whole frame with a piece's buffer fails
  silently.
* **Per-piece acknowledgements.** The control frame has no spare bit, and the pieces of one
  block go in one transmission when it holds them.
* **Fill the room left with repeats**: §2.
* **The untried floor rung's new frames carry only what the next slower rung holds** (the fill
  rule tried before this one): it cost 8–20 % on the link bench, or 0–2.4 % narrowed, and it does
  nothing for a body committed at an OFDM rung.

## 5. Consequences

* A frame the path refuses whole is delivered in pieces at the rung the other station asks for.
  A transfer no longer stops for good behind one block.
* Link protocol 11: the release notes open with "Update required", and the shell's
  `LINK_PROTOCOL` is 11.
* New data still goes whole at the recommendation; the rate a station climbs to is unchanged.
* Nothing an operator sees changes but the time a stalled transfer takes to recover.
