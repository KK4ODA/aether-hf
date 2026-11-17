# ADR-0130: A short transmission goes at the most robust rung that carries it

**Status:** accepted, 2026-11-17, the author's decision after report 18 §4
(`docs/research/link-architecture/18-radiopost-40m-beta162.md`). Model first
(`GrantedEngine._carrying_rung` in `model/aether_model/link/granted.py`), then the port
(`carrying_rung` in `aether-link/src/granted.rs`), held together by the regenerated
`granted_vectors.json`. **No wire change, no configuration key**: every frame carries its rung in
its chips, as before, so beta.162 and this build interoperate.

## 1. Context

CLAUDE.md's rule (*The rate controller waits for the air*) leaves the controller alone until on-air
sessions show the same common-case loss in several sessions. Report 18 shows one in seven of seven.

RadioPost Gateway's replies are small: about 87 bytes a session, in two blocks (its greeting and
its receipt). C1 (ADR-0099) put each block in one frame at its choice, and its choice for a rung it
has not measured is a guess, the AWGN curve 1 dB under the reading on the 2 300 Hz air (ADR-0111).
From the mobile's ~15 dB reading the guess named rungs 15–19. At rungs 17–19 none of nine first
copies decoded alone; every block was delivered by combining its third copy with the first two,
about 10 s a block and half of each 30–40 s session.

It bought nothing. Every OFDM data frame on the 2 300 Hz air is 1.054 s whatever its rung (every
tone frame 5.36 s, and likewise at 500 Hz). A 40-byte block in a BPSK ⅓ frame (43 data bytes) takes
exactly the air it took in 64-QAM, and needs 17–20 dB less SNR on AWGN. The choice is a guess for
a transmission filled to its rung; a transmission that does not fill its frames does not need it.

ADR-0085 already acts on this for a stranded frame: it is re-encoded at the most robust usable rung
of its family that its body fits, because a slower rung costs that frame no air.

## 2. Decision

1. **What a transmission's new blocks carry goes at the most robust usable rung, of the family of
   the station's choice and no faster than it, at which it takes no more frames than at the
   choice.** The engine cuts the queue at the choice as before (as many frames as the
   transmission's room, the window and the queue allow), counts the frames and the bytes, and
   takes the first rung of `RateController.modes` in that family that cuts the same bytes into no
   more frames (`cut`, `block_size`: the cut's own rule, a body one byte short of a full frame
   going as two). A transmission whose new blocks fill it is unchanged: nothing slower carries as
   much in as many frames.
2. **Never across a family's edge.** A short body under an OFDM choice never drops into the tone
   floor's frames, five times as long, and one under a floor choice goes down the floor's rungs
   (tone-24 at the bottom). Echo limits (ADR-0087) fall only on the top rungs of the floor's family,
   and the choice is never one the echo rules out, so no rung under it in its family is either.
3. **A try keeps its rung.** C1's try (one new frame of a transmission at the usable rung above its
   choice) is a measurement: its frame goes first, at its rung, and only what the transmission
   carries after it goes robustly. A try across the floor's edge, which takes the whole
   transmission, and a pinned rung (a bench's, a test's, `rate.pin`) are kept whole.
4. **First copies only.** A block keeps the rung it was cut at, so its copies combine; the rule
   chooses where a block is cut, never where a copy goes.
5. **C1 learns from where the frames went.** A block's outcomes are charged to the rung it was cut
   at (the engine's `origin`, as every copy has been since ADR-0099), so a short reply sent at
   BPSK ⅓ is BPSK ⅓'s measurement, worth a full frame of that rung when it arrives, as every
   partly filled frame has always been counted. The choice, never sent at, keeps its guess; the
   frames under it age its account as any air does.

**What it adds**: `_carrying_rung` / `carrying_rung` and the two cutting helpers, whose rule the
new-block loop now shares (it had its own copy of it). **What it removes**: nothing, and nothing
else changes: the headers (R7: a frame below the choice was never a reason to keep one), the counts,
the pieces, the stuck-frame targets (ADR-0085, ADR-0086: bounded by the choice, as before), the
slot sizing (the same number of frames of the same family), and the slave's stated queue.

## 3. Measured

**The unit tests**, both suites: 40 bytes at a 15 dB reading go at BPSK ⅓ (2 300 Hz) and QPSK ⅔
(500 Hz) where C1 chose rungs 17 and 14; what fills its frames goes at the choice; a try's frame
keeps its rung and the rest goes at the most robust rung (800 bytes: 655 at rung 18, 145 at rung
11); a 10-byte body under the first OFDM rung stays there; on the floor 19 bytes go at tone-24 and
20 at tone-36; a pinned rung is kept; and in a session a 40-byte reply goes at BPSK ⅓ and is
counted there, the choice's account left empty. Two tests built on transfers the rule now cuts
differently were moved to transfers that still exercise them: ADR-0128's variant J needs a second
transmission that fills its frames (12 000 bytes, not 6 000), and ADR-0073's header test a block
the floor's slowest rungs carry whole (40 bytes, not 100; a longer one ends in pieces, always at RV
0, on every seed).

**The link bench** (`tools/bench_link.py`'s lossy pipe, the build before against this one, the same
seeds): both airs, AWGN, Good, Moderate and Poor at −4…+20 dB, transfers of 300, 2 000 and 8 000
bytes, two trials, 336 points. Every one of the 217 points where no transmission went below the
choice came out identical. Of the 119 where one did, none failed; 51 were faster, 10 slower and 58
the same, the session time pooled ×0.978 (×0.942 and ×0.859 for 300-byte transfers at 2 300 and
500 Hz). The slower points are the pipe's draws: the block fails at the robust rung as at the
choice (both above the pipe's Good threshold at 12 dB) and a control frame's draw differs after it.

**The scenario harness**: §4.

## 4. The harness

`tools/session_matrix.py`, the build before (`87bc2e4`) against this one, fourteen scenarios: the
quick set, the Winlink- and RadioPost-shaped exchanges, the gateway scenarios (Trimode, three
clients, a client that vanishes, a scanning gateway, the FQ hang-up), VarAC's chat and the steady
mail, three draws each (`--seeds 3`), paired by draw.

| scenario | passed, before/after | air s, before: median (range) | after | median ratio | radiated overlaps |
|---|---|---|---|---|---|
| `40m-gateway-busy-when-called-2300` | 3/3 | 130 (117–134) | 126 (110–128) | ×0.95 | 0/0 |
| `40m-gateway-client-vanishes-2300` | 3/3 | 332 (326–337) | 330 (327–331) | ×1.00 | 0/0 |
| `40m-gateway-three-clients-2300` | 3/3 | 216 (214–236) | 197 (164–203) | ×0.86 | 0/0 |
| `40m-scanning-gateway-2300` | 3/3 | 71 (68–90) | 90 (90–90) | ×1.27 | 0/0 |
| `40m-trimode-gateway-2300` | 3/3 | 64 (63–96) | 66 (60–78) | ×1.06 | 0/0 |
| `40m-winlink-exchange-2300` | 3/3 | 93 (83–97) | 82 (79–83) | ×0.85 | 0/0 |
| `80m-agc-carrier-outside-500` | 2/3 | 343 (299–400) | 299 (270–354) | ×1.00 | 0/0 |
| `80m-gateway-fq-hangup-2300` | 3/3 | 94 (74–121) | 83 (73–88) | ×0.94 | 0/0 |
| `80m-steady-mail-2300` | 3/3 | 186 (186–195) | 186 (186–195) | ×1.00 | 0/0 |
| `80m-steady-mail-500` | 3/3 | 472 (463–472) | 472 (463–472) | ×1.00 | 0/0 |
| `80m-trimode-gateway-client-vanishes-500` | 2/2 | 653 (216–720) | 653 (216–800) | ×1.00 | 0/0 |
| `80m-varac-chat-and-ping-500` | 3/3 | 120 (100–135) | 87 (77–113) | ×0.77 | 0/0 |
| `80m-winlink-exchange-500` | 3/3 | 219 (208–229) | 196 (189–211) | ×0.92 | 0/0 |
| `quick-awgn-2300` | 3/3 | 44 (44–66) | 44 (44–66) | ×1.00 | 0/0 |

42 runs a build: 40 passed before, 41 after; no radiated overlap in either; pooled air ×0.960.
The short-reply transfers moved most: the three gateway clients' host sends 30 → 14 s and replies
13 → 5 s (medians), VarAC's chat lines 8 → 4 s, the FQ hang-up scenario's reply 21 → 15 s. The bulk
mail is unchanged: its transmissions fill their frames, and only each transfer's last one eased.

Where a scenario came out slower, the two builds' recordings were read frame by frame for the first
place they part:

* **`40m-scanning-gateway-2300`**, draws s1001 and s2001: the client's sixth message frame went
  alike in both builds, and its second copy decoded in one and not in the other, before either
  station had eased anything. That is the harness's draw (its steps are taken in wall time, so a
  run lands a block apart; CLAUDE.md, ADR-0099's review). The third draw is identical.
* **`80m-trimode-gateway-client-vanishes-500`**, the scenario's own draw: the call was answered seconds
  apart, before any data. Draw s1046 fails in both builds alike (the call is lost).
* **`80m-agc-carrier-outside-500`**, s1031: the call's OFDM part was read in one build and not the
  other, so one session started on OFDM (and missed its budget, before) and the other on the floor.
* **`40m-trimode-gateway-2300`**, s2045: the one place where the rule's own effect cost time. The
  client's first message block was short, and went at QPSK ⅓ where it had gone at C1's guess, rung
  15. Before, that block went three copies at rung 15, and C1 stepped down before the rest of the
  message, which then went at rung 14 and decoded. Now the block decoded at once, C1 learned nothing
  of rung 15, and the next transmission, four frames at rung 15, failed whole and went again: a
  round of the exchange where the short block used to pay its copies. The gateway's greeting in the
  same draw was eased too, and decoded at once where it had taken copies. §5 says what follows.

**A second pass** of those three scenarios, the same three draws on each build again:

| scenario, draw | before: pass 1, pass 2 | after: pass 1, pass 2 |
|---|---|---|
| `40m-scanning-gateway-2300` | 90.0, 90.0 | 89.9, 89.9 |
| s1001 | 71.1, 89.5 | 90.0, 90.2 |
| s2001 | 68.0, 89.9 | 90.0, 89.9 |
| `40m-trimode-gateway-2300` | 96.1, 59.3 | 59.6, 59.4 |
| s1045 | 62.7, 62.9 | 66.2, 63.5 |
| s2045 | 64.5, 87.2 | 77.9, 78.2 |
| `80m-trimode-gateway-client-vanishes-500` | 719.8, 748.2 | 799.8, 726.4 |
| s1046 (fails in both: the call) | 216.4, 216.5 | 216.5, 216.4 |
| s2046 | 653.0, 653.1 | 653.0, 653.1 |

The scanning gateway's first-pass difference was the build before's: its two fast draws came back
at 89.5 and 89.9 s, where this build ran 90 s both times. The Trimode scenario runs 59–96 s on the
build before alone; over both passes this build is ×0.94 of it. Nothing in the set is meaningfully
slower, and no run of either build overlapped another station's transmission.

## 5. What is left open

* **A short transmission no longer measures the guess.** Before, a short block at an unmeasured
  rung was C1's measurement of it, paid for in copies; now the first transmission that fills its
  frames is, paid for in a round when the guess was wrong (§4, the Trimode draw). Where replies are
  short and nothing fills a transmission (report 18's gateway), nothing pays at all; where a short
  block opens a long transfer, the first full transmission pays what the short block paid. The
  rule does not take sides between them: which rung a full transmission should use is the rate
  controller's review (CLAUDE.md, *The rate controller waits for the air*), and so is whether a
  guess should be tried by one frame of a full transmission, as C1's tries are.
* **The panel's rung reads lower while short replies go** (the header's rung is the data's): the
  manual says so.
* **`tools/c1_replay.py`** counts a transmission under the choice in its family apart from a
  disagreement (`below`), since the replay knows a transmission's frames and not their bytes.
