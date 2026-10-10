# ADR-0060: A lost first frame no longer takes its burst with it

**Status:** accepted, 2026-10-10. Model first (`ToneStream._announced_inside`, `_first_stands`,
`_overlaps`), then the port (`announced_inside`, `first_stands`, `overlaps` in
`aether-phy/src/tone.rs`). Receiver only: no wire change, no configuration key.

## 1. Context

WC4Y's Test of 2026-10-10 13:30Z (80 m, 500 Hz, about −3 dB both ways), recorded at both ends
with audio. WC4Y's second message burst — five tone4x100-75 frames, 27 s of air — reached
KK4ODA-1 at −2 to −0.3 dB. KK4ODA-1 announced five arrivals and decoded none, so the burst went
again: 375 bytes and about 35 s lost. The replay of KK4ODA-1's recording through the current
daemon lost them too; started 4 s later, it decoded four of the five.

The tone stream's trace showed a cascade:

1. A weak hypothesis (statistic 4.4 against the real frames' 10) was taken. Its first block lay
   on noise just after the previous burst, its middle block on the silence of KK4ODA-1's own
   transmission, and its last block on the real first frame's first block. It became final
   before that first block had been announced, and the real frame, overlapping it by that one
   block, was refused (`overlaps`).
2. The real first frame was never announced, so its middle block — the same pattern again —
   was announced as a frame of its own.
3. The second frame's first block lay inside that false arrival and was refused as weaker
   (a tie at the statistic's clip), and the second frame itself was refused because an
   arrival — its own middle block — started inside it (`announced_inside`, ADR-0014).
4. The second frame's middle block was then announced, refusing the third; and so on to the
   end of the burst.

Step 1 needs the phantom; steps 2–4 need only a first frame whose first block went unheard,
which is any burst that begins while the receiver is still muted after its own transmission
(`deaf_for` 0.75 s) — a peer that answers quickly. Synthetic bursts of every tone kind on both
airs (tone-36, tone4x100-75, tone100-153) with their first half-second muted: before, no frame
of the burst was decoded; now every frame but the truncated one.

## 2. Decision

1. **A candidate whose own first block stands is not refused by an arrival at its own middle
   or end block** (`first_stands`: the first block alone passes the announcement's hit test,
   a silent symbol counting for nothing). Such an arrival is the candidate's own block read as
   a first block. Without a first block of its own the candidate is still refused: that is
   ADR-0014's reading a block-spacing early, its first block in the silence, its middle block
   on the real frame's first.
2. **A frame taken at a lower statistic that overlaps a candidate by no more than a sync block
   does not keep it out** (`overlaps(span, statistic)`, also for announcements and for
   dropping arrivals). Frames of a half-duplex burst never overlap; a stronger frame whose
   first block is a weaker frame's last is the real one.

## 3. Measured

* KK4ODA-1's recording of the session, replayed as on the day (`aetherd --replay … --expect`):
  57 → 61 frames decoded; the only differences are the four frames of the lost burst (the fifth
  read −6.9 dB and fails, now reported instead of unseen).
* Model and port: `test_a_burst_whose_first_frame_began_under_the_mute_is_not_lost_with_it`
  (fails before: no frame found) and `…_taken_by_one_block_of_a_stronger_one_gives_way`; the
  ADR-0014 tests stand unchanged in meaning.

## 4. Also read from the session (no change)

* **KK4ODA-1's one stall**: about 20 s at 293 s into its recording, while keyed — the card
  played 4.7 s past the burst before the release, and nothing was heard for 20 s. WC4Y's
  acknowledgement timeouts at 13:35:44–13:36:15Z fall in it, and his link dropped to the floor
  for a 21.7 s burst. Broadband RFI was on the band; beta.98 keys CAT on a thread of its own
  (`ThreadedPtt`), so a stuck command no longer stops the receiver.
* **Acknowledgements**: before 233 s KK4ODA-1 acknowledged on the floor (WC4Y's data was on
  tone rungs); its one ordinary acknowledgement failed at WC4Y, ADR-0059 moved the rest to the
  floor, and WC4Y decoded 23 of 23 after that.
* **The rate**: at −3 dB rungs 4–6 decoded 4/4 and rungs 7–9 0/4, as the 500 Hz table says
  (rung 6 needs −3.6 dB, rung 7 −2.1). The link ran at rungs 2–4 with its 3 dB margin — the
  designed price of a Moderate path.
* **The Test ladder** sends a failed rung's frames again at that rung (HARQ), and a few of them
  were still being resent during the file step. Test-only; left as it is.
