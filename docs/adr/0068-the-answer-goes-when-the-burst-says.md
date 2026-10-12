# ADR-0068: The answer goes when the burst says, never when the receiver guesses

**Status:** proposed, 2026-10-12 — for the author's decision. Nothing is built. If accepted it is
link protocol 8 (an "Update required" release, ADR-0041 §4), designed and benched in the model
before any field build.

## 1. Context

Since the first on-air tests (ADR-0017, 2026-09-25) there have been 51 decisions in 18 days.
About half of them change when a station answers, polls, hands over the turn or decides that a
burst is over: 0022, 0023, 0027–0034, 0040, 0041, 0044–0047, 0053, 0059–0063, 0065, 0067.
More and more of them repair what an earlier one opened. ADR-0040 had faint detections hold
the acknowledgement; ADR-0061 had announced arrivals hold it; ADR-0065 and ADR-0067 removed
phantoms that those two let in. The link engine is 2 690 lines in the model and 5 370 in the
port, and most of its growth is this logic. The author's question (2026-10-12): are we finding
protocol problems on every test because the design is not robust, or because it is too
complicated?

The physical layer is not where the trouble is: the rungs decode on the air where the AWGN
table says they should, and the tone floor carries sessions at −9 dB that no earlier build
could. The trouble is one structural choice, made early for good reasons, and everything built
on it:

* **A burst carries no length.** A frame's header has to be identical across its
  retransmissions so they combine (`frames.py`), so the receiver infers the burst's end. It does so from
  silence (a quiet that depends on the family: 0.2 s ordinary, 0.99 s on the floor), from
  preambles announced before their frames decode (ADR-0012, ADR-0030, ADR-0061), from an OFDM
  countdown that can say only "at most" (ADR-0041, ADR-0046: two bits, counted in pairs), and
  from rules for frames it could not read (ADR-0040, ADR-0045, ADR-0047, ADR-0065).
* **The detector is set low to reach the floor**, so false detections are routine: phantom
  ordinary frames several times a session on the harness's 80 m path, a tone end block read
  as a first block. Every rule
  that keys on a detection inherits them.
* **Three frame families on each air** (the tone floor, the middle kinds — fast tones wide,
  four-tone narrow — and OFDM), each with its own length and announcement latency, and
  control frames in either family, so each timing rule has cases.

Each on-air test, and each harness run, explores combinations of these three, and fixing
incident by incident converges slowly.

## 2. Proposal

**One principle: a station answers a burst at a time that a frame it could read stated.** It
never answers because of silence, an announcement, or a frame it could not read.

1. **Every DATA frame states exactly how many frames of its burst follow it**, outside the
   codeword, so a failed frame whose acquisition is trusted still says it:
   * OFDM: the mode chips' turn as now (ADR-0041), but exact. Two ways to fit the six-frame
     burst: (a) three bits, the chips' quarter turn plus one more orthogonal chip pattern per
     (mode, RV); or (b) bursts of at most four frames, which fits two bits and costs 7–10
     points of air share at 1.05 s frames (0.84 → 0.77 wide with an ordinary acknowledgement,
     0.61 → 0.51 narrow with a floor one). The bench decides.
   * Tone frames: the **end block's pattern**, chosen from four per kind, carries 0–3. The end
     block then no longer repeats the first, so it can never be read as a frame's start
     (ADR-0067's cause, gone at the root). The tone burst's key-time cap (five frames,
     ADR-0017) already fits.
2. **The answer is due at `end + follows × length + turnaround`** of the latest frame that
   stated it, read from a frame that decoded or whose chips or pattern were trusted.
   Announcements and silence no longer move it. A burst nothing of which was readable is not
   answered.
3. **The sender waits exactly that answer slot**: its burst's end, the turnaround, the answer's
   length and a fixed margin. Then it sends again. No extension for frames heard arriving.
4. **POLL, TURN and DISC are bursts of one frame with nothing following**, under the same rule,
   and their answers likewise.

### What it retires

Most of the rules that exist only to guess an end: the family-dependent quiet
(`_irs_reply_delay`); announcements moving the acknowledgement (`on_preamble` for an IRS, the
`_arriving` hold of ADR-0061); faint follow-on arrivals (ADR-0040, `FOLLOW_ON_*`); the pairs
and "never short" bound (ADR-0046); `_burst_closed` (ADR-0045, now always); `_phantom` and
`_answered_already` (a frame that cannot be read starts nothing); the sender's waits stretched
for arrivals (ADR-0030); and most of the floor-quiet arithmetic in the poll and TURN waits
(ADR-0028, ADR-0034). Rate control, HARQ, the families, the floor and the turn offer stay.

### What it costs and risks

* A protocol change: version 8 talks to nothing older.
* A burst whose readable frames all came early and whose tail faded is answered on time (the
  statement is exact), but a burst nothing of which was readable now costs the sender one
  answer slot instead of an answer saying "nothing". At the floor that is about 4 s.
* Option (b) costs throughput on the OFDM rungs. Option (a) costs PHY work and new vectors.
* The trusted test (`trusted_measurement`) becomes the gate for timing, so its false-trust
  rate matters more: it must be measured on noise and on the harness's QRM first.

## 3. How it would be judged

Model first, behind a `LinkConfig` switch, so the same seeds run both ways:

1. `tools/bench_link.py` on the fading pipe, every class, −6…+12 dB, both airs: goodput no worse
   than ±3 % except where (b) is chosen.
2. The scenario harness, every scenario, three seeds: no scenario slower beyond the baseline's
   range, collisions down, and the stress set's failures down.
3. The engine smaller: the lines and timers it removes, counted.
4. Then P9-1 against VARA on cables, and alternated on one path on the air.

If it fails 1 or 2, it is rejected with its numbers, as ADR-0062 was.

## 4. Open questions for the author

* (a) or (b) for the OFDM count — or bench both?
* Is a protocol break acceptable now, with three or four regular testers, rather than later?
* Should the next field builds hold still (no link changes) until this is decided, so the
  testers' sessions measure one engine?
