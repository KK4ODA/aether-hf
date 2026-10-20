# ADR-0083: A tone frame is not refused for its faded sync tones

**Status:** accepted, 2026-10-20. Model first (`ToneDetector.confirmed` and
`_block_contradictions` in `model/aether_model/phy/tone.py`), then the port
(`ToneDetector::confirmed`, `block_contradictions_in` in `aether-phy/src/tone.rs`), the two
constants through `preamble_tables.json`. **Receiver only: no wire change, no configuration
key.**

## 1. Context

ADR-0080 §3 left one thing open: on ITU flutter (10 Hz) the 500 Hz detector lost four-tone
frames at high SNR that decode with genie timing, whichever receiver read them. Of 60 frames:

| Detected and decoded | +3 dB | +6 dB | +9 dB |
|---|---|---|---|
| tone4x100-75 | 58 | 56 | 52 |
| tone4x100-93 | 58 | 56 | 53 |

Every frame lost was refused by `confirmed` for its contradictions. Its sync blocks held it
well: 6–8 of 8 hits each, at least 5 in every block. But 3–5 of its 24 sync symbols were
*contradicted*: another tone was strongest there, over 12 times the noise and four times that
tone's median, where ADR-0015 allows two. On flutter a sync tone fades out within its 40 ms now
and then, and what is left strongest is the glide from the previous tone and the neighbouring
symbols' tones, spread by the fading. At high SNR that leakage is far above the noise.

The contradiction rule exists for ghosts: a strong frame of the other air's kinds read by a
pattern of this one at a part-symbol offset (ADR-0015), and a hypothesis read a block early
over the frame's data (ADR-0014). What tells the two apart is how strong a contradiction is
against the frame's own sync level, the median energy of its own tone where that tone is
strongest:

| Contradicting tone over the frame's own level | median | 90th percentile | largest |
|---|---|---|---|
| Real frames on flutter, 0–20 dB (four kinds, both airs) | 0.09–0.16 | 0.18–0.30 | 0.64 |
| Ghosts: a strong four-tone frame read by the 2 300 Hz detector, 0–20 dB | 0.8–1.0 | — | 1.2 (smallest 0.28) |
| ADR-0014's early reading over a fast kind's data | 0.22 | — | 0.23 |

Real frames on AWGN, Moderate and Poor had no contradictions at all at any SNR. A ghost's
contradictions are a real frame's tones at its own level: 7 to 9 of a ghost's 8–10 reach half
of it. The early reading's contradictions are as weak as a fade's, because a fast kind's data
symbol fills a quarter of the sync numerology's window. It is told apart by its hits: its first
block lies in silence and has none.

## 2. Decision

A candidate that the rules before refuse (more than two contradictions, and not two whole
uncontradicted blocks, ADR-0049) still stands when **every one of its three sync blocks holds
it** (at least `FADED_BLOCK_HITS` = 5 of 8 hits) and **no more than two of its contradictions
reach half its own sync level** (`CONTRADICTION_SHARE` = 0.5). Contradictions themselves are
counted as before, so ADR-0049's clean-block test and ADR-0070's stream rule are unchanged.

## 3. Measured

The same 60 frames of §1 through the detector: 60 of 60 found and decoded at +3, +6 and +9 dB,
for both kinds. On a wider set of 16 tone4x100-75 frames at +9 dB on flutter (the model's
test): 16 against 11.

False frames, the rule before against this one, on the same input: strong frames of the other
air's kinds through each detector, strong OFDM traffic through both, and each air's own kinds
(any frame found that was not the one sent). AWGN, flutter, Moderate and Poor, 0–30 dB, 1 152
transmissions: one false frame under each rule, the same one, a fast kind read by the 500 Hz
detector on AWGN at 0 dB. The minute of noise and the OFDM traffic tests do not move: the rule
acts only after the statistic and the hits.

## 4. Alternatives

* **The share alone, for every frame**: it confirmed ADR-0014's early reading, whose
  contradictions are a fast kind's data at a fifth of its level. The stream's own rule and the
  offline detector's strongest-first order still found only the real frame, but the
  confirmation itself lost a defence, and requiring the three blocks keeps it.
* **More contradictions allowed** (three or four): flutter frames had up to five, and ghosts
  begin at seven or eight, so it would have cut into a margin the share leaves wide.

## 5. Consequences

* Tone frames on a fast fade, the floor's control frames included, are no longer lost at high
  SNR. The frames this fixes are the ones a link on flutter uses for its calls, acceptances and
  floor answers.
* The spec's acquisition rule (§2.4) says so.
