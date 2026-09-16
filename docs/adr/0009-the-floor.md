# ADR-0009: The floor — a frame family for the SNR region below the mode table

**Status:** accepted 2026-09-16 (model), curves in `bench/` · **Roadmap:** P9-4 ·
**Builds on:** ADR-0002 (waveform), P7-0 (the 500 Hz air), ADR-0007/0008 (rate control)

## 1. Context

Both air interfaces stopped at ≈ 190 bit/s: the narrow table's slowest mode, QPSK ½ on the
LONG frame, decodes at −5.5 dB (3 kHz reference) on AWGN and the control frame (QPSK ½ on
SHORT, 7 bytes) at the same −5.5 dB — measured for this ADR, twenty frames a point, through
the real detector and again with genie timing (`tools/bench_floor.py`). A great deal of real
HF traffic lives below that. VARA HF's published 2017 level table (52 carriers at 37.5 baud)
starts at **35 bit/s net with 20-byte packets** and climbs through five more BPSK levels
before its first 4PSK one at 1 929 bit/s; VarAC's material says links are held and beacons
decoded at −22 dB and speed-level-2 beacons at −18/−19 dB, with no reference bandwidth
stated. None of that is a measurement of ours and no per-level S/N table is public; the
figures are recorded here as the claims they are. The roadmap's own target for P9-4 was
≈ −10 dB on AWGN and 0 dB or better on ITU Good for a mode below 200 bit/s; the author's
brief is *comparable or better than VARA HF where it is commonly run*. The earlier design's
DSSS/FSK "emergency" concept was dropped by the roadmap for an honest floor; this is it.

What physics allows is set by energy per bit, not bandwidth: at 30 bit/s,
E_b/N_0 = SNR(3 kHz) + 20 dB, so a coded QPSK/BPSK mode that needs 3–5 dB of E_b/N_0
after pilots, prefix, synchronisation and estimation would decode near −15 dB in either
bandwidth. Every layer of the receiver has to hold at that SNR, not only the code.

## 2. Where the floor broke (measured before this ADR)

500 Hz, AWGN, frames of 20 (acquired within half a symbol / decoded through the detector /
decoded with genie timing):

| frame | −12 | −11 | −10 | −9 | −8 | −7 | −6 | −5 | −4 |
|---|---|---|---|---|---|---|---|---|---|
| QPSK ½ LONG (25 B) | 4/0/0 | 8/0/0 | 10/0/0 | 16/0/0 | 19/0/0 | 20/1/1 | 20/15/14 | 20/19/19 | 20/20/20 |
| QPSK ⅓ LONG (15 B) | 1/0/0 | 4/0/0 | 8/0/0 | 13/1/4 | 18/12/14 | 20/20/20 | 20/20/20 | 20/20/20 | 20/20/20 |
| QPSK ¼ LONG (11 B) | 1/0/0 | 5/0/0 | 7/1/7 | 15/9/14 | 18/18/20 | 20/20/20 | 20/20/20 | 20/20/20 | 20/20/20 |
| QPSK ⅕ LONG (8 B) | 0/0/0 | 5/1/3 | 11/7/10 | — | — | — | — | — | — |
| control, QPSK ½ SHORT (7 B) | 2/0/0 | 3/0/0 | 11/0/0 | 18/0/0 | 18/0/0 | 19/3/1 | 20/14/13 | 20/17/16 | 20/20/20 |

Three walls, in the order a lower mode hits them:

1. **The code and modulation**: QPSK ½ at −5.5 dB, ⅓ at −7.5, ¼ at −9, ⅕ at −9.5 —
   capacity's 2 dB per halving of the rate, less a short-block penalty that grows as the
   block shrinks (88 information bits at ⅕). Genie and detected agree wherever the frame
   is acquired, so channel estimation is not the limit down to −9 dB.
2. **The detector**: the two-symbol Schmidl–Cox preamble is acquired 19/20 at −8 dB,
   16/20 at −9, 10/20 at −10. A mode that decodes below −9 dB is lost before it is found.
3. **The fading channels**: on ITU Good the narrow waveform has a fifth of the band's
   frequency diversity and a frame shorter than a fade, so acquisition is 7–8/20 from −10
   to −6 dB and the code only follows. Time diversity across frames — ARQ with soft
   combining, which the link already does, and P9-5's interleaving — is what buys anything
   there; a lower rate alone does not.

The control frame was exactly as robust as the data floor (−5.5 dB), so a data mode below
it would have helped only the weak direction of an asymmetric link.

## 3. What a floor needs

A frame at −12 dB (3 kHz) sees −4 dB per carrier at 500 Hz. At that SNR:

* the **preamble** must integrate longer — every doubling of its length is 3 dB of
  detection; two symbols reach −8.5 dB (90 %), eight reach ≈ −13 (measured, §6);
* the **code** must run at a tenth rate, which the TS 38.212 rate matcher already gives by
  repeating the circular buffer below the BG2 mother rate of ⅕; a tenth-rate frame still
  has to carry something useful — a 7-byte control frame, a 20-byte connect body, a data
  frame whose 3-byte header is not most of it — so **the frame must be longer**, roughly
  in proportion to 1/rate;
* the **channel estimate** must average more pilots — the comb pilots are 4 of 12
  carriers on every symbol and the estimate was smoothed over ±1 symbol; ±3 is free on
  Good and Moderate, whose fades last seconds, and about the limit on Poor (1 Hz Doppler,
  ≈ 0.3 s coherence);
* the **control and connect frames** must live at the same SNR as the data.

Payload bytes per frame of candidate rates on the 500 Hz waveform (`Mode.payload_bytes`;
"net" subtracts the 3-byte DATA header; K = information bits with CRC):

| layout | rate | bytes | net bit/s | K | E_s/N_0 at capacity + gap → est. SNR (3 kHz) |
|---|---|---|---|---|---|
| LONG, 32 symbols, 1.05 s | QPSK ½ | 25 | 167 | 224 | 0 + 1.5 → **−5.5 (measured)** |
| | QPSK ⅓ | 15 | 91 | 144 | −2.5 + 2 → **−7.5 (measured)** |
| | QPSK ¼ | 11 | 61 | 112 | −3.9 + 2 → **−9 (measured)** |
| | QPSK ⅕ | 8 | 38 | 88 | −5.1 + 2.3 → **−9.5 (measured)** |
| ×2, 64 symbols, 2.05 s | QPSK ⅕ | 19 | 63 | 176 | ≈ −10 |
| | QPSK 1/10 | 8 | 20 | 88 | −8.4 + 2.5 → ≈ −12.5 |
| ×4, 128 symbols, 4.03 s | QPSK ⅕ | 41 | 75 | 352 | ≈ −10 |
| | QPSK 1/10 | 19 | 32 | 176 | ≈ −12.5 |
| | QPSK 1/16 | 11 | 16 | 112 | −10.5 + 2.5 → ≈ −15 |

(The estimate is the BICM capacity of the effective rate plus the short-block gap that the
measured rows calibrate; the −7 dB is the narrow waveform's per-carrier SNR minus its
prefix.)

## 4. Decision

**A floor frame family for the narrow air interface.**

* **Two floor layouts**, mirroring LONG/SHORT: `FLOOR_LONG` — eight preamble symbols and
  128 data symbols (136 symbols, 4.2 s) for data and connect frames; `FLOOR_SHORT` — eight
  preamble symbols and 64 data symbols (72 symbols, 2.2 s) for control frames. Sixteen full
  pilot symbols carry a floor DATA frame's chips (128 chips, their own set at 0.2); the
  receiver smooths its comb-pilot estimate over ±3 symbols on a floor layout.
* **The floor preamble is its own**: eight identical symbols of a second pair of PN
  sequences, one per frame type (`FLOOR_SC_SEEDS`), drawn on **every** active carrier. Eight
  symbols give the detector four times the preamble energy: its *floor statistic* averages
  the two-symbol matched-filter output over the seven symbol-spaced windows the preamble
  fills, non-coherently (within half a decibel of coherent combining in the prototype and
  indifferent to phase drift over a quarter second on a fading channel), with its own
  threshold set the way ADR-0002's were, just above the maximum over 60 s of noise (0.314
  → 0.32). Its start is then refined within a symbol and a half by combining four windows
  coherently on a fine frequency sub-grid, and its CFO comes from all seven symbol lags.
* **Why every carrier, and why two checks.** Twelve-carrier OFDM symbols correlate with any
  six-carrier PN reference at 0.4–0.75 at high SNR once the receiver has searched over
  carrier offset — half the even carriers are comb pilots, which repeat on every symbol —
  so a strong frame's *body*, of either family, scores on the other family's references
  above their thresholds, and no test on carrier energies survives the noise of six
  carriers at −13 dB. What a body cannot fake is the preamble's own structure. The ordinary
  pass runs first and each candidate must show the even-carriers-only symbol's two
  identical halves (0.55 at −9 dB; a twelve-carrier floor symbol shows about none, its odd
  carriers cancelling its even ones between the halves; a data symbol shows the pilots'
  0.17); the floor pass then takes what is left, outside the ordinary frames' spans, and
  each candidate must show its eight symbols repeating over all seven lags. Putting the
  floor sequences on every carrier halves the cross-talk of data onto them and is what
  makes the half-symbol test sharp. The ordinary two-symbol path is untouched for the wide
  air and for narrow frames of the ordinary family (its vectors hold).
* **Three modes below the former table**, so the narrow ladder is
  `0 QPSK 1/10·FLOOR (19 B) · 1 QPSK ⅕·FLOOR (41 B) · 2 QPSK ⅓·LONG (15 B) · 3 QPSK ½·LONG
  (25 B, the control/connect mode) · 4–12 as before` — thirteen modes, which is what the
  32-chip sequence set holds at |ρ| ≤ 0.25 (52 sequences; 56 do not exist). Modes are
  compared for the rate controller's Pareto front by bytes *per second*, not per frame
  (`usable_modes(…, frame_s)`), since a floor frame is four times as long.
* **Control frames at the floor** go on `FLOOR_SHORT` at QPSK 1/10 (8 bytes ≥ 7). The
  IRS answers in the family of the frames it last decoded; the ISS sends its POLL, TURN
  and DISC in the family of the bursts it sends and waits for a reply of that family's
  length. **One family per burst**: the receiver infers a frame's burst slot from its air
  time, which needs every frame of a burst to be the same length, and a frame keeps its
  codeword — and so its mode — across retransmissions; when the oldest unacknowledged frame
  is of the other family the burst carries that family's retransmissions alone and new
  frames wait. The HARQ buffer a receiver keeps per sequence number remembers the mode it
  was sent at and is discarded when another mode arrives (another codeword).
* **Connecting at the floor**: the request stays on LONG at mode 3 for its first two
  tries, then alternates with `FLOOR_LONG` at mode 1 (41 bytes carries the 20-byte body;
  mode 0's 19 do not); the answer goes back on the layout the request arrived on, and both
  stations seed their rate controllers from the connect SNR exactly as ADR-0008 has them
  do. Beacons and probes stay on LONG at mode 3.
* **The all-zero block is refused** by the codec, and the link layer never assigns session
  0: the all-zero word is a codeword of every linear code and its CRC is zero, so a decoder
  fed a false detection converges to it and passes — seen twice in the first debug run.
* **Renumbering** is a wire change: the narrow modes' chip sequences move (mode 0 is now
  the floor). Narrow golden vectors regenerate under this ADR; wide ones do not move.

Rejected: lower rates on the existing frames alone — the rate can go to ⅕ on LONG (8
bytes, −9.5 dB) but the detector stops at −9 and the control frame at −5.5, so the session
floor would not move; the preamble's *length* alone as the family's signal — a receiver
then cannot know a frame is ordinary until eight symbols have passed, and the two-symbol
statistic's skirt of partial matches up to six symbols early defeats every timing rule;
carrier-energy tests of a floor candidate (odd/even ratios over the eight symbols, their
halves and quarters) — sharp at high SNR, useless at −13 dB with six carriers a side, and
fooled by an ordinary preamble anywhere inside the window; coherent combining of the seven
windows in the search — a fine sub-grid per bin for half a decibel; a 2 300 Hz floor
first — the roadmap asks for the narrow floor and a measured comparison after it (§7).

## 5. Costs and limits

* The bank runs four references instead of two on the narrow air, and the floor statistic
  evaluates an extra six symbols per chunk: about twice the detector's CPU in the model.
* An older station hears a floor frame as nothing (its references do not match); a floor
  station calling an older one falls back to LONG every other try.
* A burst is one family, and a frame keeps its codeword: a large frame left unacknowledged
  when the link drops into the floor keeps being retransmitted at its own mode until its
  redundancy versions get it through or the link times out — the same limit the ordinary
  table has had between its own modes. Re-encoding a stranded frame is a P9 item.
* Four-second frames on Poor: a frame spans several fades, which the interleaver turns
  into diversity; on Good a frame is still inside one fade, so the floor's Good numbers are
  set by ARQ soft combining across bursts — the session bench, not the frame bench, is the
  number that counts there.
* At 30 bit/s a 2.2 s acknowledgement per six 4.2 s frames is 8 % of the air time.

## 6. Acceptance (each with the numbers in `bench/`)

1. `bench_floor.py`: the floor modes', the floor control frame's and the two-symbol
   detector's acquisition/decode curves (`bench/baselines/floor_500.csv`).
2. `bench_phy.py --bandwidth 500` curves for modes 0–2 on AWGN, Good, Moderate, Poor;
   modes 3–12 unchanged from `phy_fer_500.csv` within noise; thresholds into the table
   by `update_rate_table.py --bandwidth 500`.
3. `bench_link.py --backend phy --bandwidth 500` sessions at −8, −10, −12 dB AWGN and at
   −2, 0, +2 dB Good: bytes per second through the real modem, and nothing above the
   floor slower than before.
4. Rust cross-validation: narrow `phy_vectors.json` regenerated (this ADR), floor frames
   bit-exact, `two_daemons.rs` at 500 Hz still completes.

## 7. After this ADR

A wide floor family by the same mechanism (eight-symbol all-carrier preamble, ×4 layouts,
BPSK/QPSK at a tenth rate over 42 carriers) measured against the narrow one on
Good/Moderate/Poor — the roadmap's "2 300 Hz with repetition" question; P9-5 time
diversity, which is where the Good numbers of §2 move; re-encoding stranded frames; a 1/16
mode on `FLOOR_LONG` if the eight-symbol detector holds at −15 dB. A JS8-class emergency
mode — 2–5 bit/s at −20 dB and below, thirty-second frames, non-coherent MFSK — is a
different waveform, not a mode of this table; it stays a roadmap question, not a promise.
