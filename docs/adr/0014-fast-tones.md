# ADR-0014: Fast tones — the floor's frame at 50 and 100 baud on the 2 300 Hz air (P9-9)

**Status:** accepted, 2026-09-24. Model first (`aether_model/frame/modes.py`,
`aether_model/phy/tone.py`), then the port. Builds on ADR-0013; the link protocol goes to
version 3.

## 1. Context

The equal-peak-power curves of P9-6 and P9-8 left a gap on the 2 300 Hz ladder: the tone
floor's fastest kind carries 54 bit/s and reaches −17.3 dB on AWGN, the first OFDM rung
(BPSK ⅕) carries 197 bit/s and needs −5.1 dB. Nothing lived in the twelve decibels between
them, and on the fading bench a session from −10 to −2 dB ran at the floor's 39–55 bit/s.
The weak-signal plan put "middle modes" there (phase 4, P9-9) with a gate: through the
detector, 3 dB or more over the OFDM rung a new kind displaces at the same rate on ITU Good
and Moderate, and session throughput on the fading pipe up from −10 to 0 dB with no class
worse elsewhere.

The candidate the curves pointed at was the floor's own modulation made faster: sixteen
tones at 50 and 100 Bd, 50 and 100 Hz apart, 800 and 1 600 Hz wide — too wide for the
500 Hz air, which keeps its two kinds. The plan's wording, "modes on 3–10 carriers", was not
built: the tone floor's frame filled the gap with five decibels to spare at the gate and keeps
its constant envelope.

## 2. The design: faster data under the floor's sync

The roadmap planned the fast kinds as frames of their own — their own sync numerology, a
detector per numerology, frame lengths per rung in the link layer. They are not built that
way. A fast kind is **the floor's frame with faster data**: the same three sync blocks of
eight 25 Bd symbols at the same places, the same 134 slots (5.36 s), the floor's two code
rates, and in each of the 110 data slots two data symbols at 50 Bd or four at 100 Bd:

| Kind | Data | Payload | Rate | Net bit/s | Patterns (RV 0–3) |
|---|---|---|---|---|---|
| `tone50-51` | 220 symbols at 50 Bd, 800 Hz | 51 B | 0.49 | 76 | 9–12 |
| `tone50-75` | 220 at 50 Bd | 75 B | 0.71 | 112 | 13–16 |
| `tone100-105` | 440 at 100 Bd, 1 600 Hz | 105 B | 0.49 | 157 | 17–20 |
| `tone100-153` | 440 at 100 Bd | 153 B | 0.71 | 228 | 21–24 |

What that buys:

* **One detector.** The sync blocks are the floor's, so the detector the floor already runs
  finds every kind; the patterns name the kind as they name the floor's — sixteen more, the
  same seeded search carried on, so the first nine are unchanged. A detector per numerology
  would have cost the receiver two and four times the floor's hop rate on top of it.
* **Acquisition never limits.** The sync is 25 Bd's: a fast kind is found six decibels and
  more below where its data decodes, where 100 Bd sync blocks of its own would have had a
  quarter of the energy a symbol.
* **One frame length.** Every floor data frame is 5.36 s, so the link layer's timing, its
  one-family-per-burst rule and its slot inference are untouched; only the ladder grows.
* **Longer codewords at the same rate**, and a little more rate than frames of their own
  (72, 108, 143 and 215 bit/s measured that way with genie timing; the two designs' curves
  agree within the measurement's noise).

The data glides between tones like the floor's, over a tenth of its own symbol (16 and 8
samples), and at a boundary with a sync symbol over the shorter of the two glides; the phase
runs on across every boundary, so the envelope is constant and the frame goes out at the
floor's 5.5 dB over an OFDM frame's average. What spills past the 2 300 Hz air's band edge
(±1 150 Hz) is −39 dB of a 100 Bd frame's power, −68 dB of a 50 Bd one's.

The receiver measures the noise and the three blocks' symbol SNR from the sync slots at the
sync numerology — a fast kind's data slots, read at a sync symbol's length, hold its data's
energy smeared over the bins — and the data's tone energies at the data's own numerology,
each divided by its own noise; a data symbol's SNR in the metric is the level interpolated at
its middle, scaled by the ratio of the symbol lengths. The SNR it reports is the sync
blocks', in the OFDM frames' reference like the floor's.

## 3. The gate, first half (`bench/baselines/tone_floor.csv`, 100 frames a point)

10 % FER through the detector, SNR in 3 kHz, at equal peak power; the OFDM rows from
`phy_fer.csv` and the fading pipe's calibration targets:

| | bit/s | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|---|
| tone-36 (the floor's fastest) | 54 | −17.3 | −8.8 | −10.3 | −10.9 |
| **tone50-51** | 76 | **−16.0** | **−8.6** | **−11.2** | **−11.8** |
| **tone50-75** | 112 | **−14.2** | **−5.7** | **−8.2** | **−7.5** |
| **tone100-105** | 157 | **−13.1** | **−6.5** | **−8.2** | **−8.4** |
| **tone100-153** | 228 | **−11.2** | **−3.2** | **−4.3** | **−4.1** |
| BPSK ⅕ (OFDM mode 0, the rung displaced) | 197 | −5.1 | +2.0 | +1.2 | −0.3 |
| BPSK ⅓ (OFDM mode 1, the next rung up) | 349 | −3.2 | +4.2 | +4.2 | +1.8 |

At a higher rate than BPSK ⅕, tone100-153 reaches 5.2 dB lower on Good and 5.5 dB lower on
Moderate (6.1 on AWGN, 3.8 on Poor): the gate asks for 3. The detector costs at most 0.3 dB
against genie timing; its 10 % acquisition point is −15 to −20 dB for every fast kind. The
floor's own kinds measure exactly as before with twenty-five patterns in the search instead
of nine. On the fading classes the wider 100 Bd kind at 157 bit/s reaches lower than the
50 Bd kind at 112 (frequency diversity over 1 600 Hz): the rate controller orders the rungs
by AWGN and learns one margin, so on a fading path tone50-75 is a rung it may use where the
next is better — a small cost, visible in no session result below.

## 4. The gate, second half: sessions (`bench/baselines/link_fast_tones.csv`)

The link bench on the fading pipe (`bench_link.py --fading`, 30 sessions a point, 2 kB at
2 300 Hz and 1 kB at 500 Hz; completed, then median bit/s). "Before" is beta.52's code, run
from a worktree against the same calibration, so the frames both have are judged alike:

| 2 300 Hz | −14 dB | −12 | −10 | −8 | −6 | −4 | −2 | 0 | +2 | +6 |
|---|---|---|---|---|---|---|---|---|---|---|
| AWGN, beta.52 | 30, 35 | 30, 39 | 30, 39 | 30, 39 | 30, 39 | 30, 41 | 30, 144 | 30, 196 | 30, 312 | 30, 638 |
| AWGN, fast tones | 30, 35 | 30, 50 | 30, 67 | 30, 96 | 30, 123 | 30, 142 | 30, 142 | 30, 263 | 30, 325 | 30, 638 |
| Good, beta.52 | 18, 17 | 26, 21 | 28, 25 | 30, 30 | 30, 36 | 30, 39 | 30, 46 | 29, 90 | 30, 131 | 30, 364 |
| Good, fast tones | 18, 17 | 26, 22 | 28, 26 | 30, 42 | 30, 80 | 30, 105 | 30, 119 | 30, 142 | 30, 191 | 30, 383 |
| Moderate, beta.52 | 30, 21 | 30, 26 | 30, 30 | 30, 38 | 30, 39 | 30, 40 | 30, 52 | 30, 110 | 30, 145 | 30, 397 |
| Moderate, fast tones | 30, 21 | 30, 26 | 30, 45 | 30, 76 | 30, 100 | 30, 117 | 30, 137 | 30, 142 | 30, 203 | 30, 397 |
| Poor, beta.52 | 30, 24 | 30, 27 | 30, 34 | 30, 39 | 30, 39 | 30, 39 | 29, 55 | 30, 113 | 30, 167 | 30, 478 |
| Poor, fast tones | 30, 24 | 30, 28 | 30, 52 | 30, 74 | 30, 97 | 30, 108 | 30, 124 | 30, 143 | 30, 222 | 30, 478 |

From −10 to 0 dB every point is faster — two to three times on the fading classes from −6
to −2 dB — but one: −2 dB on AWGN, where BPSK ⅕ carried 144 bit/s and tone100-153 carries
142. Nothing is worse elsewhere: +2 dB gains 4–46 %, −18 to −14 dB are unchanged, and +6
dB is unchanged or up to 5 % faster. At 500 Hz every point is as it was: the narrow ladder
did not change.

**The floor boundary.** ADR-0013 §4 capped the first OFDM rung's margin against the floor at
1 dB on this air, because the floor was a quarter of its rate; the first *usable* OFDM rung
is now BPSK ⅓ (BPSK ⅕ is beaten on both counts) and the floor below it runs at two thirds of
its rate. The cap was measured again, on and off: off, 0 dB on AWGN falls from 263 to 142
bit/s and +2 dB on the fading classes from 191–222 to about 170 — the link stays on the tones
where BPSK ⅓ with HARQ carries more. It stays as it was, and so does the "once" rule.

## 5. The streaming detector with 25 patterns

Nine patterns became twenty-five, and strong bursts of mixed kinds through the streaming
detector (160 bursts of four frames at 10–30 dB, after the silence a station keeps while it
transmits) found what the offline detector does not:

* **The early reading, again.** The hypothesis read a block-spacing early — its first block
  in the silence, its middle block on a frame's first, its end block over that frame's
  data — is what ADR-0013 §5 met with the floor's data under the end block, and refused by
  asking for four hits in a second block. A fast kind's data puts chance hits there too, and
  once in 160 bursts it put four: [0, 8, 4] passed, the stream took it as it ended — before
  the real frame did — and the real frame, overlapping it, was lost. Only a burst's first
  frame is exposed: a later frame's early reading overlaps the frame before it, already
  taken. The real frame has been *announced* long before its early reading ends, so a
  candidate is not taken while an announced frame starts inside it (more than a symbol after
  its start) with a first block at least as strong as the candidate's whole — two frames of
  a half-duplex burst never overlap
  (`test_a_fast_frame_after_silence_is_not_taken_for_its_early_reading`: tone50-75, seed
  310).
* **A false arrival that hides the real one.** With 25 patterns a first block of noise, or
  of noise and a symbol or two of a strong frame's first block, now and then passes the
  announcement's threshold (4.8; the noise maximum went from 4.1–4.5 to 4.3–4.7 a minute).
  Announced, it covered the real frame's start, and the real frame's announcement was
  refused as "inside a frame already arriving" — the rule that keeps a frame's own middle and
  end blocks from being announced as frames. Now a first block inside an arrival, not at one
  of that arrival's block positions and stronger, replaces it
  (`test_a_false_arrival_gives_way_to_a_frame_announced_inside_it`). Widening the
  announcement's neighbourhood to a whole block (±8 symbols) was tried first: it removed the
  partial-overlap case but not the noise one, and cost 0.2 s of announcement delay.

After both: 640 of 640 frames found at the right start, kind and RV, none extra, every frame
announced; four arrivals in about eleven minutes of noise that were no frame (two replaced
within a fraction of a second by the real one), each of which holds a receiving station's
acknowledgement for at most a frame. At each kind's 10 % point on AWGN the weak frames are
announced as before: 48 of 48 for every fast kind, 34 of 48 for tone-24 at −19 dB.

## 6. The ladder

The 2 300 Hz ladder has twenty rungs: tone-24, tone-36, the four fast kinds, then the
fourteen OFDM modes at rungs 6–19. BPSK ⅕ (rung 6) is beaten by tone100-153 on rate and
threshold on every class and is never recommended; it stays on the ladder as the ordinary
family's most robust mode, which connection requests, probes and beacons go out at
(`control_rung`). The 500 Hz ladder is unchanged. Consequences:

1. **Link protocol version 3.** A wide mode number from 2 up means another frame than in
   version 2, and the control frame's recommended mode needs five bits: byte 6 is now the
   mode in five bits and the acknowledgement counter — a label for logs, which no receiver
   reads — in three. A call or an acceptance of another version is ignored, and said so; a
   station on beta.52 and one on beta.53 do not connect.
2. **`max_mode` defaults to 19**; the configuration's schema is 3, and its second migration
   moves a wide station's stored `max_mode` from rung 2 up by four (the floor's two keep
   theirs; 500 Hz is untouched). Profiles go through it.
3. **Sidecars are `aether-hf-session/3`**; `field_ingest.sidecar_rung` reads `/2` and `/1`
   sidecars onto the new ladder, and `compare_air.py`, which had refused anything but `/1`
   since the tone floor, reads them all.
4. **The fading pipe** samples a fast kind at its data's sixteen tones, 800 or 1 600 Hz
   across, with β calibrated per kind like the floor's. Found in the calibration: the pipe
   filled an unmeasured OFDM rung's fading threshold with the channel's mean penalty over
   every measured rung, the tone kinds' included — adding four moved the OFDM fills by up to
   half a decibel. Each family is now filled from its own rungs, and the narrow ladder's rung
   2 is calibrated against the −6.0 dB its table says (it had been fitted against −7.0).
   `calibrate_fading.py --jobs` fits on several processes.

## 7. The port

`aether-phy`: `tone.rs` has the general modulator (`cpfsk`, `frame_symbols`,
`modulate_frame`), the data's energies at its own numerology and `fast_metrics`, the sync
noise from the sync slots for a fast kind, `ToneDetector::for_kinds` (an air's kinds: the
narrow air never looks for the fast ones) and `ToneStream::with_detector`, and the two
stream rules above; `build.rs` compiles each kind's data numerology and the fast kinds in,
and each waveform's tone kinds by name, which `preamble.rs` checks against the ladder
(`AirInterface::fast_tones`, `tone_data`, `tone_kinds`). Against the model's vectors the fast
kinds' tones are exact and their waveforms, detection and soft bits agree to the vector file's
tolerances; the floor's cases are byte-identical to before. `aether-link` has protocol 3, the
five-bit field and the twenty-rung tables; `aetherd` the schema-3 migration, `max_mode` 19,
sidecar format 3 and the panel's twenty-rung list. The daemon's Test session ran all twenty
rungs through the real modem, every fast kind 3 of 3 at 15–17 dB, and two daemons over
`[sim]` run a Test session over the six tone rungs.

## 8. Costs and limits

* **The 2 300 Hz air only.** 800 and 1 600 Hz do not fit in 500; the narrow air keeps its
  gap from 54 bit/s to QPSK ⅓'s 114.
* **Granularity.** A fast frame is 5.36 s like the floor's, so a short message pays a whole
  frame and a burst of six runs half a minute; frames of their own would have been 1.3–2.7 s.
* **False arrivals** in noise are a little more frequent with twenty-five patterns — about one
  in five minutes — and hold an acknowledgement for at most a frame.
* **Compatibility.** Link protocol 3: every station of a test needs the same beta.
* Not measured on the air.

## 9. Rejected

Frames of their own at 50 and 100 Bd (the roadmap's plan: two more detectors, sync blocks
with a quarter of the energy, and per-rung frame lengths through the link layer, for the same
curves); OFDM on a few carriers (the plan's wording — the fast tones passed the gate with five
decibels to spare and keep the constant envelope); a whole-block announcement neighbourhood
(§5); raising the announcement threshold or the first block's hits (either costs the weakest
frames their announcement); dropping the cap at the floor boundary (§4).
