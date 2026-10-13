# ADR-0068: The answer goes when the burst says, never when the receiver guesses

**Status:** superseded by ADR-0069 (2026-10-13), except §2.1: the exact counts stay, and
everything else here — the answer at the stated time, the sender's slot, the answer's family,
the turn it kept — gives way to granted slots. Before that: accepted for building, 2026-10-12. The author's answers: a protocol break is
acceptable now; the count goes whichever way is the most robust with the least complexity; and
field builds hold still until this is built and judged. It is link protocol 8, an "Update
required" release (ADR-0041 §4). Model first behind a switch, then the port, judged on the link
bench and the harness (§4) before any field build, and rejected with its numbers if it fails
there, as ADR-0062 was.

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
  retransmissions so they combine (`frames.py`), so the receiver infers the burst's end. It does
  so from silence (a quiet that depends on the family: 0.2 s ordinary, 0.99 s on the floor),
  from preambles announced before their frames decode (ADR-0012, ADR-0030, ADR-0061), from an
  OFDM countdown that can say only "at most" (ADR-0041, ADR-0046: two bits, counted in pairs),
  and from rules for frames it could not read (ADR-0040, ADR-0045, ADR-0047, ADR-0065).
* **The detector is set low to reach the floor**, so false detections are routine: phantom
  ordinary frames several times a session on the harness's 80 m path, a tone end block read as
  a first block. Every rule that keys on a detection inherits them. KK4ODA-1's side of WC4Y's
  Test of 2026-10-11 15:52Z has one: its last answer waited 2.46 s for an ordinary detection
  0.32 s after the last tone frame, which failed at −12.6 dB.
* **Three frame families on each air** (the tone floor, the middle kinds — fast tones wide,
  four-tone narrow — and OFDM), each with its own length and announcement latency, and control
  frames in either family, so each timing rule has cases.

The same session shows a second, smaller choice made after the fact: **which family an answer
goes in.** An answer goes in the family of what it answers, and moves to the floor only once
one has been lost (ADR-0059). WC4Y reported hearing KK4ODA-1 at −7 to −9 dB, under the 500 Hz
ordinary control frame's −6 dB threshold, and the first ordinary acknowledgement KK4ODA-1 sent
him (to the ladder's first OFDM burst) never reached him. His three Tests in a row (10-10,
10-11 00:57Z, 10-11 15:52Z) each lost exactly that one.

Each on-air test, and each harness run, explores combinations of these, and fixing incident
by incident converges slowly.

## 2. Decision

**One principle: a station answers a burst at a time that a frame it could read stated.** It
never answers because of silence, an announcement, or a frame it could not read.

### 2.1 Every frame states exactly how many frames follow it

The count is in **slots of the burst's data frame**, 0 to 3: a burst is at most four frames, and
a control frame that ends a burst (ADR-0047's offering TURN) takes a slot. It is carried outside
the codeword, so a failed frame whose acquisition is trusted still says it, and a frame's copies
still combine:

* **OFDM data frames: the mode chips' quarter turn, as now (ADR-0041), read exactly.** The count
  in pairs (ADR-0046) goes; the turn on the air does not change. No PHY change.
* **Tone data frames: the middle sync block.** Each data kind already has four sync patterns,
  one per redundancy version. The start and end blocks keep the frame's own, naming kind and RV
  as now, so a frame's copies still add parity. The middle block carries the kind's pattern for
  RV `(rv + count) mod 4`. A kind's four patterns share at most two of eight symbols in the same
  slot, so the count is read as eight hits against two at most. The detector takes the middle
  block's best of the kind's four patterns. No new pattern, and the patterns' bound (no two
  sharing more than two symbols under any offset and shift) still holds between every pair of
  blocks. A frame with nothing following is the frame sent today.
* **Control frames: a field in the body.** Control frames are never combined, so their body can
  change. An acknowledgement that leads the receiver's own burst (ADR-0047) says how many frames
  follow it; any other control frame says none.

Why these and not the others, measured or counted:

* **Four-frame bursts** cost 1.4 % of goodput at 500 Hz and 0.9 % at 2 300 Hz on the model's link
  bench (geometric mean over 32 points an air: 8 kB sessions, every class, −10…+18 dB, 24 trials
  a point). The worst points are 7–8 % at −2…+2 dB on the fading classes, and one 2 300 Hz Poor
  point is 17 %. The bench charges no keying, so the harness will put a number on that too.
* **Three bits in the OFDM chips** would need eighth turns, which halve the phase margin the
  count is read with, or a second chip pattern per (mode, RV), which doubles the codebook the
  receiver searches and its false modes. Either is a PHY change with new vectors.
* **The tone pattern itself as the count**, every tone frame at RV 0, would need no PHY change.
  But its copies would combine as repeats, not as added parity, and that costs where the floor
  earns its keep. Today's sequence is RV 0, 0, 2, 3, so only the third and fourth copies differ.
  On the model's frames (120 trials a point, independent fades per copy), tone-24 (rate 0.49) at
  −22 dB decodes after three copies 113 times in 120 with parity and 74 with repeats on AWGN, 54
  against 38 on Moderate, and 37 against 20 on Poor. For tone-36 (rate 0.71, like the middle
  kinds) it is worse: at −22 dB on AWGN, after four copies, 96 against 5; at −20 dB, after three
  copies, 75 against 39 on Moderate and 69 against 15 on Poor. That is two decibels or more at
  the bottom of the ladder, where nothing lower is left to step down to.
* **New patterns for a count** would need four for each one in use. The search that draws them
  found the 33rd at 4.45 million draws (`search_sync_patterns`). A block shifted in tone instead
  shares up to seven symbols with some pattern, against the family's two, and reads to the
  detector like a carrier offset.

### 2.2 The answer is due at the stated time

The answer is due at `end + follows × slot + turnaround` of the latest frame that stated it,
read from a frame that decoded, an OFDM frame whose chips were trusted
(`trusted_measurement`), or a tone frame the detector confirmed. Announcements and silence no
longer move it. A burst nothing of which
was readable is not answered.

### 2.3 The sender waits exactly the answer slot

The sender waits its burst's end, the turnaround, the answer's length (in the family §2.4 gives,
which the sender can work out too) and a fixed margin. Then it sends again. No extension for
frames heard arriving.

### 2.4 The answer goes in the family the other station can read

An answer goes on the tone floor unless the other station's latest report of this one (its
"heard there", ADR-0021) clears the ordinary control frame's threshold by the rate controller's
margin (3 dB). Before any report, it goes on the floor. Both stations know the report, so the
answer's length is known to both. This replaces ADR-0059's move to the floor after a lost
acknowledgement and ADR-0034's floor repeats. ADR-0062's rule — the floor whenever the report was
under 12 dB above the threshold — cost 8 % of air at 12 dB and was rejected. This one keeps an
ordinary answer wherever the other station reads one.

### 2.5 The Morse identifier never follows a burst's frames

The identifier never follows a burst's frames, because that is where the answer goes. It goes at
the end of a communication, after the DISC or its answer as ADR-0022 waits it out, or, mid-session,
ahead of a burst's frames.

### What it retires

Most of the rules that exist only to guess an end:
* the family-dependent quiet (`_irs_reply_delay`);
* announcements moving the acknowledgement (`on_preamble` for an IRS, ADR-0061's `_arriving` hold);
* faint follow-on arrivals (ADR-0040, `FOLLOW_ON_*`);
* the pairs and "never short" bound (ADR-0046);
* `_burst_closed` (ADR-0045, now always);
* `_phantom` and `_answered_already` (a frame that cannot be read starts nothing);
* the sender's waits stretched for arrivals (ADR-0030);
* ADR-0034's and ADR-0059's family switches;
* most of the floor-quiet arithmetic in the poll and TURN waits (ADR-0028).

Rate control, HARQ, the families, the floor and the turn offer stay.

## 3. What it costs and risks

* **A protocol change.** Version 8 talks to nothing older. The author accepts the break now,
  with three or four regular testers.
* **A burst nothing of which was readable** now costs the sender one answer slot instead of an
  answer saying "nothing". At the floor that is about 4 s, about what the "nothing" answer took.
* **Four-frame bursts** cost a few percent of goodput (§2.1).
* **The tone detector changes** in one place: the middle block's best of four patterns. Its
  sensitivity and its false alarms are measured again (`tools/bench_floor.py`, a noise run), and
  a frame whose middle block is lost but which still decodes says no count: it is answered as if
  three followed — late, never early.
* **The trusted test becomes the gate for timing**, so its false-trust rate matters more. It is
  measured on noise and on the harness's interference first.
* **A late answer.** A burst that ends with a TURN answers a slot's difference late: the TURN is
  shorter than the slot it takes — 2.2 s on the floor, 0.6 s ordinary — once per change of
  direction.

## 4. How it is judged

Model first, behind a `LinkConfig` switch, so the same seeds run both ways:

1. `tools/bench_link.py` on the fading pipe, every class, −6…+12 dB, both airs: goodput within
   the four-frame burst's cost (§2.1), no more failures; `tools/bench_chat.py`: no more drops.
2. The scenario harness, every scenario, three seeds: no scenario slower beyond the baseline's
   range, collisions down, and the stress set's failures down.
3. The engine smaller: the lines and timers it removes, counted.
4. Then P9-1 against VARA on cables, and alternated on one path on the air.

If it fails 1 or 2, it is rejected with its numbers.

## 5. Build order

1. **The model** (`aether_model/link/`):
   * `LinkConfig.stated_answers`, off by default: the exact count in slots, four-frame bursts,
     the answer at the stated time, the sender's slot and the answer's family;
   * the simulators pass the count as they pass `follows` today;
   * the link bench and the chat bench A/B.
2. **The port and the PHY**:
   * the engine;
   * the tone frame's middle block as its count;
   * the OFDM count read exactly;
   * the control frame's field;
   * the vectors regenerated, a deliberate model change;
   * the daemon's identifier ahead of a burst's frames;
   * `LINK_PROTOCOL` 8 in `frames.rs` and the shell.
3. **The harness**: every scenario, three draws, against the baseline.
4. **Accepted or rejected** here with the numbers. Accepted, the switch goes, the retired rules go
   with it, and protocol 8 ships as a beta.

## 6. Field builds hold still

beta.105 stays on the air until then. ADR-0065, ADR-0066, ADR-0067 and ADR-0055's amendment
wait on `master` and ship with protocol 8.
