# ADR-0015: The narrow middle kinds — four tones at 100 baud inside the floor's 400 Hz, the 500 Hz ladder's middle rungs (P9-10); contradicted sync symbols

**Status:** accepted, 2026-09-24. Model first (`aether_model/frame/modes.py`,
`aether_model/phy/tone.py`), then the port. Builds on ADR-0013 and ADR-0014; the link protocol
goes to version 4 and the configuration's schema to 4.

## 1. Context

ADR-0014 closed the 2 300 Hz ladder's gap between the tone floor and the OFDM modes with fast
tones, whose data spans 800 and 1 600 Hz — too wide for 500. The 500 Hz ladder kept its gap,
and a wider one: tone-36 (54 bit/s) reaches −17.3 dB on AWGN, QPSK ⅓ (114 bit/s), its first
OFDM rung, needs −6.0 dB — and +1.0 and +2.0 dB on ITU Good and Moderate. P2P contacts and
VarAC's calling frequencies are 500 Hz wide (P7-0), so the author asked for the narrow middle
modes ahead of time diversity (P9-5). The gate is ADR-0014's: through the detector, 3 dB or
more over the OFDM rung at the same rate on Good and Moderate, and session throughput on the
fading pipe up from −10 to 0 dB with no class worse elsewhere.

## 2. The design: four tones at 100 Bd

The floor's frame again — its three sync blocks of eight 25 Bd symbols at the same places,
its 134 slots (5.36 s), its two code rates — with four data symbols a slot on **four tones
100 Hz apart**, at ±50 and ±150 Hz: the data spans the floor's own 400 Hz. Two coded bits a
symbol, 440 data symbols, 880 coded bits a frame — as many as the 50 Bd fast kinds carry:

| Kind | Data | Payload | Rate | Net bit/s | Patterns (RV 0–3) |
|---|---|---|---|---|---|
| `tone4x100-51` | 440 symbols at 100 Bd on 4 tones, 400 Hz | 51 B | 0.49 | 76 | 25–28 |
| `tone4x100-75` | 440 at 100 Bd on 4 tones | 75 B | 0.71 | 112 | 29–32 |

The other ways to put more bits in 400 Hz were measured first with genie timing (40 frames a
point, 10 % FER, dB on AWGN / Good / Moderate / Poor):

| Candidate | bit/s | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|---|
| 16 tones at 25 Bd, rate 0.85 | 64 | −16.2 | −5.2 | −6.8 | −7.3 |
| 8 tones at 50 Bd, rate 0.71 | 82 | −15.1 | −5.0 | −7.0 | −8.6 |
| 8 tones at 50 Bd, rate 0.85 | 100 | −14.1 | −3.0 | −3.5 | −4.4 |
| **4 tones at 100 Bd, rate 0.49** | 75 | −15.0 | −7.0 | −8.8 | −10.3 |
| **4 tones at 100 Bd, rate 0.71** | 112 | −13.0 | −6.2 | −5.6 | −6.0 |
| 4 tones at 100 Bd, rate 0.85 | 134 | −12.0 | −0.3 | −2.0 | −0.4 |

The four-tone symbol is the shortest, so a frame has the most of them to spread over a fade,
and at the floor's two rates it reaches furthest on the fading classes at every rate; rate
0.85 falls off a cliff on all three and is not built.

**The glide** between two data tones is the floor's own 32 samples, two fifths of a 100 Bd
symbol. Against 8, 16 and 24 samples (8 is the fast kinds' proportion, a tenth), the power
outside ±250 Hz after the 500 Hz transmit filter is −25.5, −26.9, −28.6 and −30.5 dB — the
floor's frames put −31.5, the 500 Hz OFDM frames −15 — and the sensitivity is the same within
100-frame noise on AWGN and all three ITU classes. So each frame is as clean as the floor's.

**The patterns** are eight more from the seeded search carried on (ADR-0014 did the same for
its sixteen): the 32nd lies 2.63 million draws in, the 33rd 4.45 million. The search is now
batched — `permuted` shuffles each row of a batch exactly as successive `permutation` calls
would, from the same generator — and vectorised: a block is Costas when no two of its
displacements the same distance apart are equal, and two Costas blocks share three symbols
(one more than `PATTERN_CROSS`) exactly when a triangle of one is a translate of a triangle of
the other, so chosen blocks' triangles go into a lookup table. It finds all 33 in 23 s — the
first 25, which CI checks, in half a second; the whole set is in the slow tests — where the
pair-by-pair search took a minute for 25.

## 3. The gate, first half (`bench/baselines/tone_floor.csv`, 100 frames a point)

10 % FER through the 500 Hz air's detector, SNR in 3 kHz at equal peak power; the OFDM rows
from `phy_fer_500.csv` and the fading pipe's calibration targets:

| | bit/s | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|---|
| tone-36 (the floor's fastest) | 54 | −17.3 | −8.8 | −10.3 | −10.9 |
| **tone4x100-51** | 76 | **−14.3** | **−7.5** | **−10.0** | **−10.3** |
| **tone4x100-75** | 112 | **−13.0** | **−3.2** | **−4.9** | **−5.7** |
| QPSK ⅓ (OFDM mode 2, the first OFDM rung) | 114 | −6.0 | +1.0 | +2.0 | −1.3 |
| QPSK ½ (OFDM mode 3, the control rung) | 190 | −5.2 | +4.0 | +3.5 | 0.0 |

At the rate of QPSK ⅓, tone4x100-75 reaches 4.2 dB lower on Good and 6.9 dB lower on
Moderate (7.0 on AWGN, 4.4 on Poor): the gate asks for 3. Acquisition's 10 % point is −20 dB on
AWGN and −15 to −18 on the fading classes, far below either kind's decode threshold. At the
same rates the 2 300 Hz fast kinds (tone50-51, tone50-75) reach 1.1–3.3 dB further on the
fading classes: 800 Hz of frequency diversity and sixteen tones against 400 Hz and four.

## 4. The gate, second half: sessions (`bench/baselines/link_narrow_middle.csv`)

The link bench on the fading pipe at 500 Hz (`bench_link.py --fading --bandwidth 500`, 30
sessions of 1 kB a point; completed, then median bit/s). "beta.53" is that release's code
from a worktree against the same calibration:

| 500 Hz | −14 dB | −12 | −10 | −8 | −6 | −4 | −2 | 0 | +2 | +6 |
|---|---|---|---|---|---|---|---|---|---|---|
| AWGN, beta.53 | 30, 28 | 30, 35 | 30, 35 | 30, 35 | 30, 35 | 30, 38 | 30, 64 | 30, 147 | 30, 182 | 30, 284 |
| AWGN, middle kinds | 30, 28 | 30, 35 | 30, 44 | 30, 59 | 30, 59 | 30, 68 | 30, 74 | 30, 147 | 30, 182 | 30, 284 |
| Good, beta.53 | 21, 17 | 27, 21 | 29, 24 | 29, 32 | 29, 35 | 30, 37 | 30, 38 | 29, 42 | 30, 67 | 30, 204 |
| Good, middle kinds | 21, 17 | 27, 21 | 29, 25 | 29, 43 | 29, 51 | 30, 55 | 30, 64 | 30, 69 | 30, 83 | 30, 204 |
| Moderate, beta.53 | 30, 20 | 30, 24 | 30, 29 | 30, 35 | 30, 35 | 30, 35 | 30, 36 | 30, 40 | 30, 47 | 30, 164 |
| Moderate, middle kinds | 30, 20 | 30, 24 | 30, 31 | 30, 47 | 30, 50 | 30, 56 | 30, 60 | 30, 67 | 30, 76 | 30, 164 |
| Poor, beta.53 | 30, 24 | 30, 26 | 30, 33 | 30, 35 | 30, 35 | 30, 35 | 30, 36 | 29, 47 | 30, 74 | 30, 199 |
| Poor, middle kinds | 30, 24 | 30, 26 | 30, 36 | 30, 47 | 30, 53 | 30, 56 | 30, 60 | 29, 72 | 30, 93 | 30, 199 |

From −10 to +2 dB every point is as fast or faster — 1.3 to 1.7 times on the fading classes
from −8 to 0 dB, up to 1.8 on AWGN — and none completes fewer sessions; below −12 and at +6 dB
nothing changes.

**No cap at the floor boundary.** ADR-0013 §4 holds the 2 300 Hz air's first OFDM rung to a
1 dB margin against the floor. At 500 Hz the same cap was tried at the new boundary (the
CSV's third variant): it helps AWGN at −2 dB (74 → 100 bit/s) and costs every fading class
from −4 to 0 dB (Moderate at −2 dB: 60 → 43, three sessions lost), where QPSK ⅓, with
a fifth of the wide rung's frequency diversity, loses frames the learned margin knows about.
The 500 Hz air stays without one.

## 5. Contradicted sync symbols (the detector, both airs)

With these kinds each air's detector looks for kinds the other air's does not, and a strong
frame of one air's middle kinds, heard by a station on the other air, was **taken for a frame
of another kind**: from 0 dB up, two to four of sixteen strong 500 Hz middle-kind frames
through the 2 300 Hz detector (for example tone100-153 at redundancy version 2, read 1.8
symbols late and 0.4 of a bin off, inside tone4x100-51 at version 2). The search bounds
shared symbols at whole-symbol offsets and whole-tone shifts; at part-symbol and part-bin
offsets every window holds two tones and a pattern can match either, so two patterns can
share up to twice the bound in a block — here twelve and fourteen of 24 sync symbols, which is
`MIN_HITS`. Within one air the real frame's own hypothesis is stronger and rules the reading
out; across airs nothing did. The frame then failed its CRC: a wasted decode, and a frame
report of a kind nobody sent.

A sync symbol now **contradicts** its frame when its strongest tone is another one, at least
12 times the noise per bin (noise alone reaches that once in ten thousand symbols) and at
least four times that tone's own median over the frame's sync symbols (so a steady carrier or
spur, strong in every symbol, contradicts nothing); a confirmed frame has at most two.
Measured before choosing: every such ghost had 8–12; real frames of every kind had at most one
at 8 times the noise and none at 12, from their decode thresholds to 30 dB on AWGN and on ITU
Poor (264 frames); a weak frame with a steady carrier at 0.8 of a sync tone's energy on one of
the floor's tones was found 8 of 8 times with the carrier as without it, on every kind.

The rule also refuses, on its own, the early reading of ADR-0014 §5 — its end block lies over
the frame's data, whose strong tones stand where its pattern wants its own; the stream's rule
for it stays as the second defence. P9-9's stress, rerun on both airs with the rule: 240 of
240 frames of each air's own mixed strong bursts found, no phantom frame, and the one false
arrival P9-9's run had (seed 20) and no other; four minutes of noise per air, no frame and no
arrival; weak frames announced as before (tone-24 34 of 48 at −19 dB, every other kind 48 of
48 at its threshold, the new kinds included). The other air's bursts give no phantom frames
now, but their first blocks are still **announced** — about one arrival a frame — because the
stream announces on a first block's clipped ratios, which carry no energies to contradict: a
station holds its acknowledgement for another station's frame as it would for a busy channel,
which is what the channel is.

## 6. The ladder

The 500 Hz ladder has fifteen rungs: tone-24, tone-36, tone4x100-51, tone4x100-75, then the
eleven OFDM modes from QPSK ⅓ at rungs 4–14. QPSK ⅓ stays on it and usable: 114 bit/s against
tone4x100-75's 112, and a 1 s frame against 5.36 s. The 2 300 Hz ladder is unchanged.
Consequences:

1. **Link protocol version 4.** A 500 Hz mode number from 2 up means another frame than in
   version 3; the 2 300 Hz numbers mean what they did, but one number says what both ladders
   are. A call or an acceptance of another version is ignored, and said so: a station on
   beta.53 and one on beta.54 do not connect.
2. **Configuration schema 4.** Its third migration moves a 500 Hz station's stored
   `max_mode` from rung 2 up by two (the floor's two keep theirs; 2 300 Hz is untouched), with
   a fixture of a beta.53 500 Hz file. Profiles go through it.
3. **Sidecars are `aether-hf-session/4`**; `field_ingest.sidecar_rung` reads `/3`, `/2` and
   `/1` sidecars onto both ladders.
4. **The fading pipe** samples a middle kind at its four data tones, with β calibrated per kind
   like the others; the other rows of the calibration came out identical.

## 7. The port

`aether-phy`: `ToneKind::data_tones` and `data_bits` — the codec, the data's energies and
noise, the metrics and the soft bits all take the kind's own tone count — `narrow_kinds` and
`all_kinds`, `MiddleTones` in place of the air's fast-tones flag, and the contradiction rule
(`ToneDetector::contradictions`, in `confirmed`); `build.rs` compiles the narrow list, each
kind's data tones and the rule's two constants. Against the model's vectors the narrow kinds'
tones are exact and their frames are received by the 500 Hz detector as the model receives
them; every earlier case is byte-identical. `aether-link` has protocol 4 and the fifteen-rung
tables; `aetherd` schema 4 and its migration, sidecar format 4 and the panel's fifteen rungs.

## 8. Costs and limits

* **Less diversity than the fast kinds.** Four tones in 400 Hz reach 1.1–3.3 dB less far on
  the fading classes than the 2 300 Hz air's kinds at the same rates.
* **Granularity.** A frame is 5.36 s, like the floor's.
* **Cross-air arrivals** remain (§5); cross-air frames do not.
* **Compatibility.** Link protocol 4: every station of a test needs the same beta.
* Not measured on the air.

## 9. Rejected

Sixteen tones at 25 Bd with a lighter code, and eight tones at 50 Bd (§2); a third kind at
rate 0.85 (a cliff on every fading class); shorter glides (§2); a floor-boundary cap at 500 Hz
(§4); searching every air's kinds with both detectors — it would have suppressed the ghosts by
finding the real frame, at twice the 500 Hz detector's work, to decode nothing a station on the
other air can use; a contradiction measured against the frame's own sync energy rather than the
noise, which misfired on weak real frames (up to nine of 24 at −19 dB).
