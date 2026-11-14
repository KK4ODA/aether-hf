# ADR-0116: The FM waveform, chosen on the calibrated model by criteria set now

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-4
(`docs/research/aether-fm/00-proposal.md` §D, §J, §K, §L). It registers the candidates and the
criteria before any run. Step 10.2c runs them once the FM channel model passes Gate 2 (ADR-0115
§3), and Gate 3 accepts this ADR on them. Nothing is built for it yet.

## 1. Context

On FM the modem sees an audio channel, not an RF one:
- fixed filters and emphasis;
- a deviation limiter on the voice path;
- the FM threshold with its clicks;
- static paths at fixed stations, picket-fence fading when mobile;
- sound-card clocks, and no carrier offset worth tracking (proposal §D.1).

HF's machinery for Doppler, a CFO search and a −14 dB floor has nothing to do there.

The proposal recommends single carrier with frequency-domain equalisation (SC-FDE). Its reasons,
from the scratch model (§D.2, §D.3):
- it keeps 1.5–2.5 dB more SNDR through the limiter than CP-OFDM on the voice path;
- the two are within half a decibel on a flat port;
- one frame family;
- headroom in the sound cards;
- OFDM's frequency-domain tools kept.

But the scratch model is not the model of record, and the choice should not rest on it.
ADR-0068 §1 and the rate controller's reviews (ADR-0094, ADR-0099) are the lesson. A waveform or
a controller tuned on a model the air disagrees with costs months.

So the candidates and the criteria are fixed here, before the calibrated model exists. The runs
then decide.

## 2. What is fixed whatever the runs say

- **The boundary is audio** (ADR-0113): the FM air renders and hears 48 kHz audio.
- **The code is NR LDPC** from `aether-fec`, the code HF uses and the port already has.
- **The receiver reads the noise per bin** (ADR-0104): the radio's filters and narrowband
  interferers (CTCSS residue, a courtesy tone, hum) are weighted, not equalised up.
- **Two profiles, decision 3:** the voice path (microphone and speaker, or the emphasized 1200
  pin) and the flat port (the 9600 pin). Both are simulated; the voice path goes on the air first.
- **One frame family** carries calls, control, data and calibration, unless §3.3's floor earns a
  second one by its criterion. ADR-0068 §1 measured what three families cost HF.
- **The count of frames to follow** is stated exactly in every frame (ADR-0068 §2.1). In one
  family it is a header field, not a chip phase.

## 3. The candidates

### 3.1 A: SC-FDE (the proposal's recommendation)

One carrier, root-raised-cosine shaped, in blocks of N symbols separated by a known guard word.
The guard word is both the cyclic guard and the pilot that tracks the clock and phase, as in
IEEE 802.11ad's single-carrier PHY.

Equalisation is one FFT a block, with per-bin MMSE weights from the guard words and ADR-0104's
noise reading. The constellations run from π/2-BPSK to 256-QAM. Clicks below threshold are found
as short wideband spikes and marked as erasures.

Starting numerology (proposal §D.4); §4.2 decides it:

| Profile | Symbol rate | Band | Block | Guard |
|---|---|---|---|---|
| Voice | 2 000 Bd, roll-off 0.2 | 450–2 850 Hz | 512 symbols | 32 symbols, 16 ms |
| Voice, narrow fallback | 1 600 Bd | 600–2 600 Hz | 512 symbols | 32 symbols |
| Flat | 4 000 Bd | 300–5 100 Hz | 512 symbols | 32 symbols, 8 ms |

### 3.2 B: CP-OFDM

HF's family moved to FM's bands:
- carriers 40 Hz apart over the same bands;
- a cyclic prefix as long as A's guard;
- pilots as HF's;
- the same codes and constellations;
- ADR-0004's peak reduction.

It is the baseline A must beat, run with the same care: the same per-bin noise reading, the same
drive search.

### 3.3 C: a constant-envelope floor beside A or B

A 4-level continuous-phase FSK family for the rungs below the main family's lowest, decoded by
energy like HF's tone rungs. It survives any level error and the limiter, at about 2.4 kb/s at
most. It is taken only by §4.3's criterion: by the proposal's own rule, it must beat the main
family's lowest rung by 2 dB or more.

## 4. The runs and the criteria

All runs are on the FM channel model **after Gate 2**, with every recorded radio's profile, and
with seeds and draws fixed in the run's manifest. Each candidate gets its own best drive and the
same receiver effort. The results go to `docs/research/aether-fm/03-waveform.md` with their
curves under `bench/baselines/`.

### 4.1 The measures

- **Block error rate against CNR**, rung by rung, at the drive that maximises each rung's
  delivery. CNR goes from 3 to 33 dB in 1.5 dB steps.
- **The delivery envelope:** at each CNR, the most data per second any rung delivers. That is a
  rung's payload rate times (1 − its block error rate), the best a rate controller could do.
- **The drive window:** the range of drive, in dB, over which the envelope stays within 10 % of
  its value at the best drive.
- **The conditions:**
  - static paths;
  - Rayleigh and Rician (K = 6 dB) fading at 13.5 Hz and 40 Hz: 2 m and 70 cm at 100 km/h;
  - clocks at ±200 ppm;
  - CTCSS on the flat port;
  - each recorded radio's response.

### 4.2 Choosing A or B: registered criteria

A is chosen if all of these hold. Otherwise B is, unless B fails one of them too; then the author
decides with both results in hand.

- **W1 (delivery).** On each profile and each recorded radio, A's envelope is at least B's less
  3 % at every CNR from 9 to 30 dB, static paths.
- **W2 (the limiter).** On the voice path, A's peak SNDR over the drive is at least B's, on the
  median recorded radio.
- **W3 (level tolerance).** A's drive window is at least as wide as B's on each profile.
- **W4 (mobile).** On each fading condition, A's lowest rung reaches 10 % block error at a CNR at
  most 0.5 dB above B's lowest rung.
- **W5 (clicks, risk R11).** At the CNR where the model's clicks begin (8 dB in 16 kHz, uncalibrated
  ADR-0115 §7), A's lowest rung delivers at least B's.

### 4.3 The floor: a registered criterion

C is added only if, at and below the FM threshold, its 10 % block-error CNR beats the chosen
family's lowest rung by **2 dB or more**, on static paths and on 13.5 Hz Rayleigh fading both.
The comparison is at equal delivered rate, or C is credited with the rate it delivers. Otherwise
the floor is the main family's π/2-BPSK, and one family stays one family.

### 4.4 The numerology: registered rules

- **Symbol rate.** The highest of 2 000, 2 400 and 2 800 Bd (voice) or 4 000 and 4 800 Bd (flat)
  whose occupied band, the roll-off included, sits inside the −3 dB band of at least 80 % of the
  recorded radios, with 50 Hz to spare each side. The narrow fallback (1 600 Bd) covers the
  rest.
- **Guard length (risk R3).** The shortest of 16, 32 and 64 symbols whose residual
  inter-symbol interference, after equalisation, sits at least 10 dB under the top rung's
  required SNDR on every recorded response.
- **Block length.** 512 symbols, unless at ±200 ppm and on 40 Hz fading 512 loses more than
  0.5 dB against 256. Then 256.
- **Preamble.** Two repeated sync blocks (about 50 ms) unless a shorter one meets both of these:
  - detection at 99 % or better at the lowest rung's 10 % block-error CNR;
  - false alarms under one an hour on noise and on recorded voice and packet traffic.
- **The ladder.** Rungs from the code and constellation table such that:
  - neighbouring rungs' required SNDR is at least 1.5 dB apart;
  - each rung delivers at least 10 % more than the one below at the CNR where it becomes the
    best;
  - the top rung's required SNDR is under the 90th percentile of the recorded radios' peak SNDR.

## 5. What this removes

- On FM: HF's tone floor and the fast and four-tone kinds (ADR-0013, 0014, 0015, 0081), unless
  §4.3 brings back a floor of FM's own; and HF's CFO search and its acquisition across ±150 Hz.
- The roadmap's P10-2 row (a level-tolerant constant-envelope ladder), replaced by §3.3 and §4.3.
- The proposal's numerology table as a decision. It becomes the starting point §4.4 tests.

## 6. Consequences

- Step 10.4 ports the chosen waveform to `aether-phy-fm`, bit-exact with the model, with vectors
  as every ported layer has.
- FM-5 (calibration) runs on the chosen family. Its ladder only needs the data's statistics, so
  calibration work can start on A and finish on whichever is chosen (proposal §J.1).
- FM-7 (the link) takes the frame lengths, the decode budget and the count field from here.

## 7. Alternatives already turned down (proposal §D.3)

- OFDM with per-carrier bit loading: it recovers the equaliser's 0.7–1.2 dB on a flat port only,
  at the cost of loading tables in the handshake and OFDM's peaks.
- A time-domain equaliser with decision feedback: it is kept as the fallback if §4.4's guard
  cannot be made long enough.
- Baseband multi-level PAM in G3RUH's style: it needs a port flat to near DC, and many are not.
- GMSK or multi-level FSK for everything: about 1 bit/s/Hz, at most 2.4 kb/s in the voice band.
- 1 024-QAM: it needs 33 dB or more; the model's ceiling is 29–31 dB.
