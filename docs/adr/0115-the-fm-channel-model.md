# ADR-0115: The FM channel model, a radio at each end, calibrated against recordings

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-3
(`docs/research/aether-fm/00-proposal.md` §D.1, §J, §K, §L). Built uncalibrated in step 10.2a
(§7). It is accepted at Gate 2, when the model reproduces the recordings of step 10.2b within
§5's limits. No wire change, no configuration key, nothing in the daemon.

## 1. Context

Every number in the FM proposal rests on a model of the radio. A waveform chosen on a model the
air disagrees with is HF's lesson again: the link bench's fading pipe detected every frame and
added 3 dB a copy, and the rate controller tuned on it chose rungs the air did not carry
(ADR-0093, ADR-0099). So the model is written properly before the waveform is chosen (FM-4), and
it is checked against real radios before it is trusted (risks R1–R4, R14).

The proposal's scratch model (`docs/research/aether-fm/scratch/fmchain.py`, frozen) had the core
of the chain. It left out what the proposal itself names as the likely gaps:

- microphone ALCs and compressors (the TM-V71A has one);
- sound-card noise, clipping and clocks;
- emphasis mismatch between radios;
- keying and squelch timing;
- mobile fading;
- an RF frequency error between the radios.

It was not block-exact, so the harness could not carry it. And it referred CNR to its own IF
filter's noise bandwidth, so a 12.5 kHz and a 25 kHz receiver at the same CNR were not seeing
the same signal.

## 2. Decision

### 2.1 Where it lives

`model/aether_model/fm/`:

- `channel.py`, the stages and the link they make;
- `measure.py`, how a recording or the model's output is measured;
- radio profiles in `model/aether_model/fm/radios/`, one JSON file per radio model and port,
  once step 10.2b has recorded it.

The harness gets FM radios from it later (`tools/channel_server.py`, step 10.4; proposal J.2). The
daemon's `[sim]` keeps HF's simple channel until FM needs more there. No Rust port is planned: the
daemon never runs the channel model.

### 2.2 Conventions

These are the ones every FM number will be quoted in, as `channel.py`'s are for HF.

- **Audio is real, at 48 kHz**, the sound card's rate. The RF path is complex baseband at
  48 kHz around the carrier.
- **CNR is referenced to a 16 kHz noise bandwidth**, whatever the IF filter. 16 kHz is
  Carson's bandwidth of a 25 kHz channel: 2 × (5 kHz deviation + 3 kHz audio). So two
  receivers at the same CNR see the same carrier and the same noise density, and a narrower IF
  admits less of it. The scratch model's CNR was in its 25 kHz IF's own noise bandwidth,
  15.2 kHz: 0.2 dB from this one, so its numbers keep their meaning.
- `cnr_db(p_dbm, noise_figure_db)` relates the CNR to the power at the antenna:
  N = −174 dBm/Hz + 10·log10(16 000) + NF.
- **Levels:**
  - the modem's samples are in full-scale units, dBFS;
  - the transmitter's modulation is in units of its rated peak deviation, so 1.0 is where its
    limiter sits;
  - the receiver's audio is in units of the same rated deviation until its output gain, then
    in dBFS at the ADC.
- **Every stage is a streaming process.** Feeding a signal in blocks gives the same output, bit
  for bit, as feeding it whole. Random draws use `complex_normal()` and plain generators in a
  fixed order per sample, so blocking never changes what is drawn. A test holds every stage and
  the whole link to this, as `test_channel.py` does for HF.
- **Fading has unit mean power.** Nothing is normalised per block.
- **Seeds are explicit.** One seed per link, split into one generator per random stage, so
  adding a stage never changes another's draws.

### 2.3 The chain

Sending side, in the order a sample passes:

| Stage | What it is | Parameters (defaults) |
|---|---|---|
| Sound card out | The OS volume, clipping at full scale, 16-bit quantisation, the DAC's clock | `volume_db` (0), `bits` (16), `clock_ppm` (0), `noise_dbfs` (none) |
| Radio input | Where full scale sits against the limiter | `tx_gain_db` (0: full scale is the limiter) |
| Voice path | High-pass 300 Hz (3rd order); a microphone ALC or compressor if the radio has one; pre-emphasis 750 µs (zero at 212 Hz, pole at 6 kHz, unity at 1 kHz) or none; the deviation limiter (hard, or with a soft knee); the splatter filter (3 kHz, 4th order) | `emphasis` (750 µs), `compressor` (none), `limiter_knee` (hard) |
| Flat port | A DC block (10 Hz); no emphasis, no limiter. Over-deviation shows as the far IF filter's distortion | `port_lowpass_hz` (none) |
| Tone squelch | A CTCSS tone added under the audio, if the radio sends one | `ctcss_hz` (none), `ctcss_deviation` (0.15) |
| Modulator | FM at the rated deviation; the radio's frequency error; the carrier following the key after the attack time | `deviation_hz` (5 000), `rf_offset_hz` (0), `key_delay_s` (0.05), `attack_s` (0.01) |

The path:

| Stage | What it is | Parameters |
|---|---|---|
| Carrier and noise | Complex white noise at the CNR in 16 kHz | `cnr_db` |
| Mobile fading | None, or Rician with a Clarke (Jakes) Doppler spectrum. The Doppler is v·f/c: 13.5 Hz at 146 MHz and 100 km/h | `fading` (none), `doppler_hz`, `rician_k_db` |

Receiving side:

| Stage | What it is | Parameters (defaults) |
|---|---|---|
| IF filter | 6th-order low-pass on the complex baseband, so its group delay is in the model too | `if_half_hz` (7 500 for 25 kHz, 4 250 for 12.5 kHz) |
| Discriminator | The phase difference of successive IF samples. The FM threshold and its clicks arise by themselves; none is added | — |
| Squelch | Opens when the IF power stands `threshold_db` over the noise for `open_s`; closes `tail_s` after it falls, passing the discriminator's noise until then (the squelch tail) | `threshold_db` (6), `open_s` (0.08), `tail_s` (0.15) |
| Voice path | De-emphasis or none; high-pass 300 Hz (6th order, a tone-squelch filter); low-pass 3 kHz (4th order) | `emphasis` (750 µs) |
| Flat port | A DC block and a 6 kHz low-pass | `port_lowpass_hz` (6 000) |
| The radio's floor | Hum (60 Hz and its harmonics) and broadband noise, so many decibels under a full-deviation 1 kHz tone | `floor_db` (−42) |
| Radio output | The audio level at the sound card | `rx_gain_db` |
| Sound card in | Clipping, 16-bit quantisation, the ADC's clock, the OS input gain | `volume_db` (0), `bits` (16), `clock_ppm` (0) |

Each side's emphasis is its own, so a mismatch is two settings.

**The repeater** is the receiving side to audio, then a controller, then a sending side, then a
second path:

- the controller's key-up delay, its delay line, its hang time and its courtesy tone (frequency,
  level, delay, length);
- its own limiter and emphasis;
- a CNR of its own on each hop.

It is built after the simplex chain is calibrated. Its numbers come from FM-9's recordings of real
repeaters (risk R9).

### 2.4 What it leaves out, and why

- **Multipath delay spread.** At VHF it is under a microsecond on simplex paths, flat over the
  channel. Mobile fading is modelled flat.
- **Adjacent-channel interference and intermodulation.** They are the band's, not the radio's.
  Interference inside the channel can be added as a second transmitter on the same path when a
  scenario needs it (FM-8).
- **The operating system's audio processing** (enhancements, AGC, noise suppression). Calibration
  must detect it, and that is tested on the real operating systems (risk R6). The model has no
  stand-in that could make a detector look better than it is.

### 2.5 One measure for recordings and the model

`measure.py` is used on both, so Gate 2 compares like with like:

- **Alignment:** the start and the clock. A known marker at each end of the test signal gives
  the delay and the clock offset, and the recording is resampled before anything else. 100 ppm
  over a 6 s segment moves a 2.8 kHz component by 1.7 cycles, which would ruin a coherence
  estimate.
- **SNDR against frequency:** the magnitude-squared coherence C(f) between what was sent and
  what was heard, with SNDR(f) = C/(1 − C). Distortion counts as noise.
- **The effective SNDR over a band:** the flat SNDR that gives the same information rate,
  log2(1 + SNDR(f)) integrated over the band. This is the scratch model's measure.
- **The band's response:** the linear response, S_xy/S_xx, normalised at 1 kHz.
- **The audio floor:** the level heard while the transmitter sends a carrier with no audio,
  against the level heard at the curve's peak. A bench has no deviation meter, so the floor is
  read against something both the recording and the model have.
- **Timing:** from the key to the audio (the squelch opening, or with the squelch open the noise
  quieting), from the release to the carrier gone (the discriminator's noise rising), and from the
  release to silence (the squelch closing). Each is read from the recorded envelope against the
  keying times.

## 3. Calibration, the gate

Step 10.2b records 5–10 radio models, each port it has, through a dummy load and an attenuator
(`docs/research/aether-fm/02-recording-plan.md`). For each, the model's parameters are fitted:

- the radio input's gain;
- the emphasis at each end;
- the compressor, if one shows;
- the limiter's knee;
- the filters' corners;
- the audio floor;
- the timings;
- the clocks.

The fit is kept as that radio's profile.

**Gate 2, registered now, before any recording.** For every recorded radio and port:

1. SNDR against drive within 2 dB of the recording, at every drive from 15 dB under the curve's
   peak to 6 dB over it, at each recorded CNR.
2. The drive at the peak within 2 dB.
3. The band's response within 2 dB, wherever the recording is within 20 dB of its own peak.
4. The audio floor within 2 dB.
5. Timings and clocks are the profile's measured values, so they are not judged. The model must
   take them, and the analysis must report them.

A radio the model cannot fit is a finding: a stage missing, or a stage wrong. It is named and
added, never covered by forcing a parameter. Once fitted, a radio's profile is not refitted to
make a later result look better.

## 4. What this removes

- The scratch model is no longer the model. It stays frozen as the proposal's evidence and is not
  maintained.
- The roadmap's P10-1 row is replaced by this ADR (it was kept for its history).
- The proposal's CNR, quoted in each IF's own noise bandwidth, is restated in 16 kHz.

It adds no configuration key, nothing in the daemon and nothing on the air.

## 5. Consequences

- FM-4 (the waveform) and FM-5 (calibration) are decided on this model, after Gate 2.
- The harness's FM radios (step 10.4) are this model. Each scenario key in the proposal's J.2 is
  one of §2.3's parameters.
- Every FM performance claim rests on it, so a curve is quoted with the radio profile it ran on.
- The profiles are measurements of real radios. Before Gate 2 the model runs on its defaults, and
  any number taken then is labelled uncalibrated.

## 6. Alternatives

- **Keep the scratch model.** It is not block-exact, has no keying, no sound cards, no fading
  and no squelch, and its CNR changes meaning with the IF filter.
- **Model only the audio channel**, as a linear filter, a limiter and noise, with no RF. The FM
  threshold, its clicks, over-deviation in the far IF filter and the CNR at which a mobile path
  fails all come from the RF stages. Without them the floor rung (risk R11) cannot be judged.
- **Fit an empirical audio-to-audio model to each recording** (a measured response, a measured
  noise floor, a measured compression curve). It reproduces the recorded points and nothing
  between or beyond them: not another CNR, not fading, not a repeater. The stage model fits
  the same parameters and keeps their physics.

## 7. Built (step 10.2a, 2026-11-14)

**What was built.**
- `model/aether_model/fm/channel.py`: every stage of §2.3 except the repeater, with three presets:
  `wide_voice`, `narrow_voice` and `flat_port`.
- `measure.py`: §2.5's measure.
- `signals.py`: the known signals, a single carrier of QAM symbols and chirp markers.
- `tools/fm_bench.py`: the bench for step 10.2b, with five commands:
  - `make`, the test sequence and its manifest;
  - `play`, play one sound card and record another, keying by VOX, RTS or DTR;
  - `analyse`, the recording through `measure.py`;
  - `model`, the same sequence through the model, analysed the same way;
  - `baseline`.
- `docs/research/aether-fm/02-recording-plan.md`: the procedure for 10.2b.

**How it was checked.** `model/tests/test_fm_channel.py` holds the model to its conventions and its
physics:
- every stage and the whole link block-exact;
- the CNR in 16 kHz;
- a tone at its deviation on both ports;
- emphasis 6 dB an octave, and undone by de-emphasis;
- the limiter holding the voice path, and the flat port having none;
- the FM threshold arising by itself: 14.7 dB of audio SNR for the 15 dB from CNR 30 to 15, about
  3 dB per dB below CNR 8, clicks from CNR 8 down and none at 12 or above;
- squelch times;
- unit-power Rician and Clarke fading;
- the clocks;
- the measure itself.

It reproduces the proposal's scratch point (24.6 dB at CNR 18 in its IF) at 24.9 dB.
`model/tests/test_fm_bench.py` checks the bench end to end on the model. The model was run with
its sound cards 60 ppm fast and 90 ppm slow and a recording 0.37 s early. The analysis found
−150 ppm and 0.37 s, and read back the curve, the band, the squelch and its times.

**Found on the way.** The coherence measure is fragile at a key-up. One Welch segment with no
carrier, among 140, caps an estimate near 18 dB however good the rest is. So every measurement
leaves the key-up out: the bench starts each transmission's audio 0.75 s after the key, and skips
the first 0.5 s of each drive step.

**What the uncalibrated model gives** (`bench/baselines/fm_model_uncalibrated.csv`, from
`fm_bench.py baseline`). Peak SNDR over the 24 drive steps, against the proposal's §D.5 range for
CNR 9 to 30:

| Preset | CNR 9 | CNR 12 | CNR 18 | CNR 30 | Proposal §D.5 |
|---|---|---|---|---|---|
| 25 kHz voice path, ±5 kHz | 17.8 dB | 20.6 dB | 25.3 dB | 29.6 dB | 17.6–29.5 dB |
| 12.5 kHz voice path, ±2.5 kHz | 13.5 dB | 15.9 dB | 20.6 dB | 28.1 dB | 10.7–27.1 dB |
| 25 kHz flat port, ±3 kHz | 11.3 dB | 13.7 dB | 18.5 dB | 27.9 dB | 13.1–30.6 dB |

- **The voice path agrees.**
- **The narrow path reads 1–3 dB higher** because CNR is now in 16 kHz: at the same CNR its IF
  admits 2.7 dB less noise than the scratch model's did.
- **The flat port reads 2–3 dB lower** because the model's receiver keeps the radio's −42 dB floor
  on the data port, which the scratch model left out.

All of these are uncalibrated (§5). The recordings settle them.

**Not built yet.**
- The repeater, after the simplex chain is calibrated.
- The radio profiles, one per recorded radio.
- The harness's FM radios (step 10.4).
