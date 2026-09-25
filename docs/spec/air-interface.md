# Aether HF air interface — specification v0.1

Status: **draft**, tracking the reference model on branch `phase-2`. Numbering and constants
are stable enough to implement against; anything still open is called out in §11.

This document is deliberately public. An amateur digital mode that cannot be decoded by a
third party is bad practice and, in the United States, arguably bad law: FCC §97.309(a)(4)
permits an unspecified digital code only when the technique is publicly documented. Every
number here is generated from the reference implementation
(`python tools/make_spec.py`), so the specification cannot quietly drift from the code.

Companion documents: `control-api.md` (how an application drives a modem),
`host-interfaces.md` (Winlink/Pat compatibility), `../adr/` (why each choice was made),
`../../bench/README.md` (the measurements every performance claim here rests on).

---

## 1. Scope

Aether HF is an ARQ data mode for amateur HF: an OFDM physical layer in a 2.3 kHz SSB
channel, a 3GPP-derived LDPC code, and a selective-repeat ARQ with hybrid retransmission.
This version specifies two bandwidths on one numerology: **WIDE_2300** (§2–§4) and
**NARROW_500** (§2.3, §4.1), which reuses everything here with twelve carriers instead of
fifty-seven and its own mode table. A wider 2.75 kHz variant is reserved (§11).

It is an independent design. It is built from public standards (3GPP TS 38.212, ITU-R
F.1487, IEEE literature) and its own measurements; it is not compatible with, and contains
nothing derived from, any proprietary mode.

Conventions: all SNR figures are referenced to a **3 kHz** noise bandwidth. Doppler spread
is the ITU-R F.1487 **2σ** value. Bit order is most-significant-first; multi-byte integers
are big-endian.

---

## 2. Waveform

<!-- BEGIN:waveform -->
| Parameter | Value | Notes |
|---|---|---|
| Baseband sample rate | 8000 Hz | complex |
| Audio sample rate | 48000 Hz | interpolation x6 |
| FFT size N | 200 | subcarriers in the transform |
| Subcarrier spacing | 40 Hz | fs / N |
| Useful symbol time | 25.0 ms | 1 / spacing |
| Cyclic prefix | 48 samples (6.0 ms) | before windowing |
| Window taper | 8 samples | raised cosine; effective CP 5.0 ms |
| Symbol period | 248 samples (31.00 ms) | CP + N |
| Symbol rate | 32.26 Bd |  |
| Active subcarriers | 57 | centred on the passband centre |
| Comb pilots | 15 | every 4th, both edges |
| Data subcarriers | 42 | per ordinary symbol |
| Occupied bandwidth | 2280 Hz | active carriers x spacing |
| Passband centre | 1500 Hz | audio |
| Full pilot symbols | every 8th data symbol | all carriers known |
| Preamble | 2 symbols | two identical Schmidl-Cox symbols |
| Mode/RV chips | 168 | 4 x 14 sequences, pairwise |correlation| <= 0.2 |
| Acquisition threshold | 0.36 | normalised matched-filter peak |
<!-- END:waveform -->

The subcarrier spacing is set by the worst Doppler the mode targets, and the cyclic prefix by
the worst delay spread: 40 Hz spacing keeps inter-carrier interference from a 1 Hz Doppler
spread negligible, and a 5 ms effective guard covers the 2 ms two-path profile of ITU Poor
with margin. See `../adr/0002-waveform-parameters.md`.

Each symbol is transmitted as a cyclic prefix, the useful part, and a cyclic *suffix* of
`taper` samples; a raised-cosine ramp is applied across the taper and consecutive symbols
overlap-add by that amount. The symbol period is unchanged by windowing — the taper is taken
out of the guard, so the FFT window never sees a tapered sample and windowing adds no
inter-carrier interference.

### 2.1 Subcarrier layout

Active subcarriers are centred on the passband centre frequency. Every 4th carrier, counting
from and including both edges, is a **comb pilot**; the rest carry data. Pilots take their
values from a fixed Zadoff–Chu sequence, chosen for its flat spectrum and low peak-to-average
ratio.

### 2.3 The 500 Hz waveform

The narrow waveform is the same numerology — the same sample rates, FFT, subcarrier
spacing, cyclic prefix, window and symbol period — with **twelve** active carriers centred
on the same passband centre, so it occupies 480 Hz. It exists because a 500 Hz signal is
what most HF peer-to-peer traffic is made with, and because it puts the transmitter's power
into a fifth of the band: at the same 3 kHz-referenced SNR each carrier has ≈ 6.8 dB more
signal-to-noise than a wide carrier, which is what lets its most robust mode be QPSK ½ and
still reach the wide waveform's floor. The comb pilots follow the same rule (every 4th
carrier and both edges: carriers 0, 4, 8 and 11), leaving eight data carriers.

<!-- BEGIN:waveform500 -->
| Parameter | Value | Notes |
|---|---|---|
| Baseband sample rate | 8000 Hz | complex |
| Audio sample rate | 48000 Hz | interpolation x6 |
| FFT size N | 200 | subcarriers in the transform |
| Subcarrier spacing | 40 Hz | fs / N |
| Useful symbol time | 25.0 ms | 1 / spacing |
| Cyclic prefix | 48 samples (6.0 ms) | before windowing |
| Window taper | 8 samples | raised cosine; effective CP 5.0 ms |
| Symbol period | 248 samples (31.00 ms) | CP + N |
| Symbol rate | 32.26 Bd |  |
| Active subcarriers | 12 | centred on the passband centre |
| Comb pilots | 4 | every 4th, both edges |
| Data subcarriers | 8 | per ordinary symbol |
| Occupied bandwidth | 480 Hz | active carriers x spacing |
| Passband centre | 1500 Hz | audio |
| Full pilot symbols | every 8th data symbol | all carriers known |
| Preamble | 2 symbols | two identical Schmidl-Cox symbols |
| Mode/RV chips | 32 | 4 x 13 sequences, pairwise |correlation| <= 0.25 |
| Acquisition threshold | 0.56 | normalised matched-filter peak |
<!-- END:waveform500 -->

The frame layouts keep their symbol counts, so a narrow frame lasts exactly as long as a
wide one and a link layer's timers do not know which waveform is under them:

<!-- BEGIN:layouts500 -->
| Layout | Symbols | Duration | Samples (8 kHz) | Full pilot symbols | QAM slots |
|---|---|---|---|---|---|
| LONG | 2 + 32 = 34 | 1054 ms | 8432 | 0, 8, 16, 24 | 224 |
| SHORT | 2 + 12 = 14 | 434 ms | 3472 | 0, 8 | 80 |
<!-- END:layouts500 -->

The ordinary preamble uses the same PN seeds drawn to the six even carriers (the two frame
types come out orthogonal at that length). The mode and redundancy version ride on the 32
data-carrier chips of the four full pilot symbols; with 52 (mode, RV) pairs to tell apart
the sequences are held to a pairwise correlation of 0.25 rather than 0.2 — thirteen modes
is what that set holds. The acquisition threshold is higher (§ constants) because
band-limited noise has a fifth of the degrees of freedom in a preamble's span, and so are
the signal peaks by about as much.

Below both OFDM tables is the tone floor (§2.4), the same frames on either air, with its
fast kinds between it and the OFDM modes on the 2 300 Hz air and its four-tone middle kinds
there on the 500 Hz air.

### 2.4 The tone floor

The slowest rungs of both ladders (§4) are not OFDM at all. A **tone-floor** frame sends
one of sixteen tones at a time, with a continuous phase and a constant envelope, and is
detected non-coherently, from the energy in each tone (ADR-0013). Its value is its
envelope: a transmitter driven to a fixed peak puts an OFDM frame's average power 5.6–7.5
dB below that peak (§2.2) and a steady tone's at it, so a tone-floor frame goes out **5.5 dB
above an OFDM frame's average** at the same transmit level — at or under every OFDM frame's
peak. Every SNR a receiver reports from a tone-floor frame is taken back to the OFDM
frames' reference by the same 5.5 dB, and every threshold in §4 is in that reference. The
estimate is by energy, exact where the floor is used and a lower bound on a strong path: the
glide between tones caps it near +17 dB on a clean channel, and on a dispersive one the echo's
spill into the next symbol caps it at a few decibels (ADR-0016 §4). A rate controller seeded
from a floor frame starts again from the first ordinary burst it measures.

The tones are spaced at the symbol rate about the passband centre and span 400 Hz, inside
a 500 Hz channel on either air. Between symbols the frequency glides on a raised cosine and
the phase runs on, which keeps the spectrum inside the channel (99.9 % of the power within
±250 Hz, −56 dB beyond ±500 Hz) without touching the envelope. Each symbol carries four
coded bits under a Gray label, so neighbouring tones — the ones a carrier offset or a
Doppler smear confuses — differ in one bit.

A frame is three **sync blocks** of eight symbols — at its start, after 45 % of its data,
and at its end — with the data between them. Each block's tones are a *Costas sequence*:
every displacement (Δsymbol, Δtone) between two of its symbols occurs once (J. P. Costas,
*Proc. IEEE*, 1984), so a block shifted in time or frequency matches itself in at most one
symbol and a receiver finds timing and carrier offset together. The pattern names the
frame: one for the control frame and one per redundancy version of each data kind, no two
sharing more than two symbols under any offset within a block and any shift of up to eight
tones. The uneven split of the data makes the three distances between a frame's blocks
differ, so no shift of a frame lines up more than one of its blocks with another's.

The 2 300 Hz ladder also has four **fast kinds** (ADR-0014) between the floor's own two and
the OFDM modes. A fast kind is the same frame — the same sync blocks at the same places, the
same 134 slots and 5.36 s, the same code rates — with two or four data symbols in each data
slot: 50 Bd symbols on tones 50 Hz apart (800 Hz) or 100 Bd symbols on tones 100 Hz apart
(1 600 Hz), sixteen tones and four Gray-labelled bits each. The phase runs on across every
symbol boundary, a glide as short as the shorter of the two symbols' own, so the envelope
stays constant and the frame goes out at the same 5.5 dB above an OFDM frame's average. Its
sync blocks name it like any other kind, so one detector finds every kind, and it finds a
fast kind 6 dB or more below where its data decodes. A receiver measures the noise and the
symbol SNR from the sync blocks at the sync numerology and the data's tones at their own,
each on its own bins; a data symbol's SNR is the level interpolated at its middle, a
quarter or a half of a slot's. The 500 Hz air has no room for them.

The 500 Hz ladder has two **middle kinds** of its own (ADR-0015): the same frame with four
data symbols in each data slot on **four** tones 100 Hz apart, at ±50 and ±150 Hz — the
floor's own 400 Hz — two Gray-labelled bits a symbol and 880 coded bits a frame. Their data
glides over the floor's own 32 samples, which keeps them inside the channel as the floor's
frames are (99.9 % of the power within ±250 Hz). The 2 300 Hz air does not look for them.

A candidate is a frame only if at least 12 of its 24 sync symbols have their own tone
strongest, at least 4 of them in a second block, and no more than two are **contradicted**:
a sync symbol whose strongest tone is another one, at least 12 times the noise per bin and
at least four times that tone's median over the frame's sync symbols (a steady carrier or
spur is not a contradiction). A pattern read at a part-symbol offset inside a strong frame
it does not name — another air's middle kind, or a frame read a block early — matches half its
symbols there and is contradicted in the rest.

<!-- BEGIN:tone -->
| Parameter | Value | Notes |
|---|---|---|
| Tones | 16 | 4 Gray-labelled coded bits a symbol |
| Symbol | 320 samples (40 ms) | 25 Bd |
| Tone spacing | 25 Hz | tones at (k - 7.5) x spacing about the passband centre |
| Span | 400 Hz | lowest tone to highest, plus a spacing |
| Tone change | 32 samples | raised-cosine frequency glide centred on the boundary; continuous phase |
| Frame edges | 16 samples | raised-cosine amplitude fade in and out |
| 16-tone data, 50 Bd, 2 300 Hz air | 160 samples (20 ms), 50 Hz apart, span 800 Hz | 2 data symbols a slot; tone change 16 samples, the shorter glide at a boundary with a sync symbol |
| 16-tone data, 100 Bd, 2 300 Hz air | 80 samples (10 ms), 100 Hz apart, span 1600 Hz | 4 data symbols a slot; tone change 8 samples, the shorter glide at a boundary with a sync symbol |
| 4-tone data, 100 Bd, 500 Hz air | 80 samples (10 ms), 100 Hz apart, span 400 Hz | 4 data symbols a slot; tone change 32 samples, the shorter glide at a boundary with a sync symbol |
| Level | +5.5 dB | over an OFDM frame's average power at the same transmit level |
| Sync blocks | 3 x 8 symbols | start, middle, end; 45 % of the data slots before the middle one; the same for every kind |
| Detector | hop 80 samples, bin 6.25 Hz | offset search +/-100 Hz |
| Acquisition threshold | 3.0 | mean sync-tone ratio, each clipped at 10; 12 of 24 sync tones strongest, 4 of them in a second block; a silent symbol is no evidence; at most 2 contradicted (another tone strongest, 12x the noise, not steady) |
| Arrival threshold | 4.8 | first block's mean ratio; 5 of 8 strongest |

| Kind | Payload B | Data + sync = slots | Duration | Sync blocks at | Rate | Net bps | Patterns (by RV) | AWGN dB |
|---|---|---|---|---|---|---|---|---|
| tone-control | 7 | 56 + 3 x 8 = 80 | 3.20 s | 0, 33, 72 | 0.36 | 17.5 | 0 | -19.5 |
| tone-24 | 24 | 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.49 | 35.8 | 1, 2, 3, 4 | -19.0 |
| tone-36 | 36 | 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.71 | 53.7 | 5, 6, 7, 8 | -17.3 |
| tone50-51 | 51 | 220 at 50 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.49 | 76.1 | 9, 10, 11, 12 | -16.0 |
| tone50-75 | 75 | 220 at 50 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.71 | 111.9 | 13, 14, 15, 16 | -14.2 |
| tone100-105 | 105 | 440 at 100 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.49 | 156.7 | 17, 18, 19, 20 | -13.1 |
| tone100-153 | 153 | 440 at 100 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.71 | 228.4 | 21, 22, 23, 24 | -11.2 |
| tone4x100-51 | 51 | 440 at 100 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.49 | 76.1 | 25, 26, 27, 28 | -14.3 |
| tone4x100-75 | 75 | 440 at 100 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.71 | 111.9 | 29, 30, 31, 32 | -13.0 |

| Pattern | Tones |
|---|---|
| 0 | 11 5 8 1 15 2 4 10 |
| 1 | 0 15 1 13 14 4 7 12 |
| 2 | 6 15 8 3 13 4 11 1 |
| 3 | 2 11 10 15 3 1 12 7 |
| 4 | 12 3 4 11 14 13 5 1 |
| 5 | 14 2 7 13 4 1 5 0 |
| 6 | 2 0 12 15 1 9 13 6 |
| 7 | 11 14 1 15 10 6 8 0 |
| 8 | 8 1 14 4 6 15 10 13 |
| 9 | 11 4 2 3 6 14 1 10 |
| 10 | 12 9 5 15 1 0 6 13 |
| 11 | 15 13 4 3 0 10 6 14 |
| 12 | 13 3 11 7 6 8 9 1 |
| 13 | 7 2 13 11 15 5 14 3 |
| 14 | 12 7 13 2 0 14 8 10 |
| 15 | 0 3 14 1 2 11 8 7 |
| 16 | 13 8 12 11 14 5 3 0 |
| 17 | 9 2 4 12 3 13 1 15 |
| 18 | 12 8 0 3 9 14 15 6 |
| 19 | 1 4 3 13 2 14 8 12 |
| 20 | 2 7 11 5 15 10 9 0 |
| 21 | 0 13 6 7 15 12 4 9 |
| 22 | 3 9 6 13 14 10 0 15 |
| 23 | 8 13 15 14 11 0 10 2 |
| 24 | 15 0 4 6 1 14 5 10 |
| 25 | 3 9 1 14 15 13 10 0 |
| 26 | 3 14 15 10 2 1 13 6 |
| 27 | 10 2 13 15 0 7 5 14 |
| 28 | 7 6 0 15 8 14 2 5 |
| 29 | 2 9 5 0 14 13 1 10 |
| 30 | 6 13 5 14 10 4 2 15 |
| 31 | 0 3 1 5 11 12 14 4 |
| 32 | 4 13 11 0 8 15 2 14 |
<!-- END:tone -->

A receiver searches a spectrogram at a quarter-symbol hop and a quarter-tone bin for every
kind, redundancy version, start and carrier offset: the statistic is the mean, over the 24
sync symbols, of the sync tone's energy over the mean of the other fifteen in the same
symbol (a wideband burst lifts every tone together and leaves it at one; a strong carrier
on one tone scores only where a pattern happens to use that tone), each ratio clipped. A
candidate above the threshold is refined to the sample and a fraction of a hertz and kept
only if at least half its sync tones are the strongest in their symbols — inside a strong
frame, data symbols line up with some pattern at some offset in a handful of positions,
never in half. A receiver may announce a frame as arriving once its first block is in
(§7.2). The codeword is the OFDM frames' — CRC, LDPC, rate matching and the golden-ratio
interleaver of §5 — ending in four bits a tone; a redundancy version other than 0 is not
decodable alone and combines with the ones before it (HARQ-IR) as an OFDM frame's does.

### 2.2 Peak reduction

The transmitter clips and re-filters each finished burst to a target peak-to-average ratio
(§ constants), iterating four times with the same band-limiting filter the receiver uses.
This is transmitter-side only: a receiver needs no knowledge of it, and a transmitter may
omit it at a cost of roughly 1 dB of delivered power. Constant-modulus modes take the more
aggressive target; the QAM modes carry information in amplitude and take the gentler one.
See `../adr/0004-papr-reduction.md`.

---

## 3. Frame structure

An OFDM frame is a two-symbol preamble followed by data symbols. Every 8th data symbol,
starting with the first, is a **full pilot symbol** in which all carriers are known to the
receiver. (The tone floor's frames are §2.4's.)

<!-- BEGIN:layouts -->
| Layout | Symbols | Duration | Samples (8 kHz) | Full pilot symbols | QAM slots |
|---|---|---|---|---|---|
| LONG | 2 + 32 = 34 | 1054 ms | 8432 | 0, 8, 16, 24 | 1176 |
| SHORT | 2 + 12 = 14 | 434 ms | 3472 | 0, 8 | 420 |
<!-- END:layouts -->

LONG carries user data and the connection handshake. SHORT carries acknowledgements and
other control frames, always at the control mode. While a link runs the tone floor its data
and control frames are the tone floor's; a connection request starts on the floor, and probes
and beacons go out on it (§7).

### 3.1 Preamble, and what it signals

Both preamble symbols are identical: a PN sequence on the **even** carriers only, odd
carriers zero, so the useful part of each symbol consists of two identical halves and the two
symbols repeat one another. That double structure is what a receiver uses to find the frame
and estimate carrier offset before it knows anything else (Schmidl & Cox, *IEEE Trans.
Commun.*, 1997).

**The frame type is carried by the choice of PN sequence** — DATA and CONTROL frames use
different seeds — so the type decision carries the full processing gain of the preamble
rather than depending on a separate header symbol that would be unreadable at the SNRs where
the robust modes operate.

Zadoff–Chu is deliberately *not* used anywhere in the preamble: a ZC chirp shifted in
frequency is, up to phase, the same chirp shifted in time, so a matched filter could not
distinguish a carrier offset from a timing error. A PN sequence decorrelates under either.

### 3.2 Mode and redundancy version

A DATA frame's mode and HARQ redundancy version are carried as PN chips on the **data
carriers of its full pilot symbols** — one sequence per (RV, mode) pair. The comb pilots of
those symbols remain known, so a receiver estimates the channel from them, correlates the
chips coherently against every sequence, and only then treats the pilot symbols as fully
known.

Carrying the redundancy version outside the codeword — as a cellular downlink carries it in
control information — is what allows a receiver to soft-combine a retransmission with a
first transmission whose payload, and therefore sequence number, it never decoded.

CONTROL frames always use the control mode and RV 0; their pilot symbols carry the plain
pilot sequence.

---

## 4. Modes — the ladder

What the link layer calls "mode N" is a **rung** of the air's ladder: the tone floor's data
kinds (§2.4) — its own two, then on the 2 300 Hz air its four fast kinds and on the 500 Hz
air its two four-tone middle kinds — then the air's OFDM modes, most robust first. An OFDM frame's chips carry its OFDM mode index (§3.2),
which is not its rung: on the 2 300 Hz ladder OFDM mode *m* is rung *m* + 6.

<!-- BEGIN:modes -->
| Rung | Name | Frame | bits/sym | Rate | Base graph | Z | K' | E | Payload B | Net bps | AWGN dB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | tone-24 | TONE, 25 Bd data | 4 | 0.49 | BG2 | 28 | 216 | 440 | 24 | 36 | -19.0 |
| 1 | tone-36 | TONE, 25 Bd data | 4 | 0.71 | BG2 | 40 | 312 | 440 | 36 | 54 | -17.3 |
| 2 | tone50-51 | TONE, 50 Bd data | 4 | 0.49 | BG2 | 56 | 432 | 880 | 51 | 76 | -16.0 |
| 3 | tone50-75 | TONE, 50 Bd data | 4 | 0.71 | BG1 | 30 | 624 | 880 | 75 | 112 | -14.2 |
| 4 | tone100-105 | TONE, 100 Bd data | 4 | 0.49 | BG2 | 88 | 864 | 1760 | 105 | 157 | -13.1 |
| 5 | tone100-153 | TONE, 100 Bd data | 4 | 0.71 | BG1 | 60 | 1248 | 1760 | 153 | 228 | -11.2 |
| 6 | BPSK-1/5 (OFDM mode 0) | LONG | 1 | 1/5 | BG2 | 30 | 232 | 1176 | 26 | 197 | -5.1 |
| 7 | BPSK-1/3 (OFDM mode 1) | LONG | 1 | 1/3 | BG2 | 52 | 392 | 1176 | 46 | 349 | -3.2 |
| 8 | BPSK-1/2 (OFDM mode 2) | LONG | 1 | 1/2 | BG2 | 72 | 584 | 1176 | 70 | 531 | -1.8 |
| 9 | QPSK-1/3 (OFDM mode 3) | LONG | 2 | 1/3 | BG2 | 80 | 784 | 2352 | 95 | 721 | -0.4 |
| 10 | QPSK-1/2 (OFDM mode 4) | LONG | 2 | 1/2 | BG2 | 120 | 1176 | 2352 | 144 | 1093 | +1.4 |
| 11 | QPSK-2/3 (OFDM mode 5) | LONG | 2 | 2/3 | BG2 | 160 | 1568 | 2352 | 193 | 1465 | +2.9 |
| 12 | PSK8-1/2 (OFDM mode 6) | LONG | 3 | 1/2 | BG2 | 176 | 1760 | 3528 | 217 | 1647 | +4.7 |
| 13 | PSK8-2/3 (OFDM mode 7) | LONG | 3 | 2/3 | BG2 | 240 | 2352 | 3528 | 291 | 2209 | +6.9 |
| 14 | QAM16-1/2 (OFDM mode 8) | LONG | 4 | 1/2 | BG2 | 240 | 2352 | 4704 | 291 | 2209 | +6.0 |
| 15 | QAM16-2/3 (OFDM mode 9) | LONG | 4 | 2/3 | BG2 | 320 | 3136 | 4704 | 389 | 2953 | +8.9 |
| 16 | QAM16-3/4 (OFDM mode 10) | LONG | 4 | 3/4 | BG1 | 176 | 3528 | 4704 | 438 | 3324 | +9.9 |
| 17 | QAM64-2/3 (OFDM mode 11) | LONG | 6 | 2/3 | BG1 | 224 | 4704 | 7056 | 585 | 4440 | +13.9 |
| 18 | QAM64-3/4 (OFDM mode 12) | LONG | 6 | 3/4 | BG1 | 256 | 5288 | 7056 | 658 | 4994 | +15.6 |
| 19 | QAM64-5/6 (OFDM mode 13) | LONG | 6 | 5/6 | BG1 | 288 | 5880 | 7056 | 732 | 5556 | +16.9 |
<!-- END:modes -->

`K'` is the information block including CRC, `E` the coded bits after rate matching, and the
AWGN column the measured SNR (3 kHz) for 10 % frame error rate — every entry measured, not
interpolated (`bench/baselines/phy_fer_awgn14.csv`).

Rungs are ordered from most robust to fastest. A rung that another rung beats on *both*
payload per second and threshold is never selected by the rate controller; rung 13 (OFDM
mode 7) is in that position, and so is rung 6 (BPSK ⅕, OFDM mode 0), which the fastest tone
kind beats by six decibels at a higher rate — it stays on the ladder as the ordinary family's
most robust mode, which a connection request's ordinary tries and the answers to requests and
probes that arrived in the ordinary family go out at (§7.2). The tone floor's
thresholds come from `bench/baselines/tone_floor.csv`, at equal peak power.

---

### 4.1 Modes at 500 Hz

The narrow ladder has fifteen rungs and its OFDM modes are its own: rung 9 at 500 Hz is
16-QAM ½, not the wide ladder's QPSK ⅓. A station knows which ladder applies from the
waveform the frame arrived in. Rungs 0 and 1 are the tone floor, the same frames as on the
wide air, and rungs 2 and 3 its four-tone middle kinds (ADR-0015). Above them the narrow OFDM
mode *m* is rung *m* + 2: OFDM modes 0 and 1 were the OFDM floor of ADR-0009, which the tone
floor replaced, and are on no rung. Rung 5, QPSK ½, is the **control mode** — the slowest
whose SHORT frame carries a seven-byte control frame and whose LONG frame carries a
connection request; ordinary control frames go out at it, and so do a connection request's
ordinary tries and the answers to requests and probes that arrived in the ordinary family
(§7.2). Rung 4, QPSK ⅓ on the ordinary frame, is the step between the tone floor and the control
mode.

<!-- BEGIN:modes500 -->
| Rung | Name | Frame | bits/sym | Rate | Base graph | Z | K' | E | Payload B | Net bps | AWGN dB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | tone-24 | TONE, 25 Bd data | 4 | 0.49 | BG2 | 28 | 216 | 440 | 24 | 36 | -19.0 |
| 1 | tone-36 | TONE, 25 Bd data | 4 | 0.71 | BG2 | 40 | 312 | 440 | 36 | 54 | -17.3 |
| 2 | tone4x100-51 | TONE, 100 Bd data, 4 tones | 2 | 0.49 | BG2 | 56 | 432 | 880 | 51 | 76 | -14.3 |
| 3 | tone4x100-75 | TONE, 100 Bd data, 4 tones | 2 | 0.71 | BG1 | 30 | 624 | 880 | 75 | 112 | -13.0 |
| 4 | QPSK-1/3 (OFDM mode 2) | LONG | 2 | 1/3 | BG2 | 24 | 144 | 448 | 15 | 114 | -6.0 |
| 5 | QPSK-1/2 (OFDM mode 3) | LONG | 2 | 1/2 | BG2 | 28 | 224 | 448 | 25 | 190 | -5.2 |
| 6 | QPSK-2/3 (OFDM mode 4) | LONG | 2 | 2/3 | BG2 | 40 | 296 | 448 | 34 | 258 | -3.6 |
| 7 | PSK8-1/2 (OFDM mode 5) | LONG | 3 | 1/2 | BG2 | 44 | 336 | 672 | 39 | 296 | -2.1 |
| 8 | PSK8-2/3 (OFDM mode 6) | LONG | 3 | 2/3 | BG2 | 56 | 448 | 672 | 53 | 402 | +0.5 |
| 9 | QAM16-1/2 (OFDM mode 7) | LONG | 4 | 1/2 | BG2 | 56 | 448 | 896 | 53 | 402 | -0.1 |
| 10 | QAM16-2/3 (OFDM mode 8) | LONG | 4 | 2/3 | BG2 | 72 | 592 | 896 | 71 | 539 | +2.0 |
| 11 | QAM16-3/4 (OFDM mode 9) | LONG | 4 | 3/4 | BG1 | 32 | 672 | 896 | 81 | 615 | +3.5 |
| 12 | QAM64-2/3 (OFDM mode 10) | LONG | 6 | 2/3 | BG2 | 96 | 896 | 1344 | 109 | 827 | +6.9 |
| 13 | QAM64-3/4 (OFDM mode 11) | LONG | 6 | 3/4 | BG1 | 48 | 1008 | 1344 | 123 | 934 | +8.8 |
| 14 | QAM64-5/6 (OFDM mode 12) | LONG | 6 | 5/6 | BG1 | 52 | 1120 | 1344 | 137 | 1040 | +10.4 |
<!-- END:modes500 -->

---

## 5. Forward error correction

Per frame, in order:

1. payload bytes → bits, most significant first;
2. **CRC** appended (§ constants);
3. zero fillers to the lifted block length;
4. **LDPC encoding**, 3GPP TS 38.212 base graphs 1 and 2, selected per §7.2.2 of that
   specification (BG2 for small blocks or low rates, BG1 otherwise), one code block per frame;
5. **rate matching** to `E` bits at the frame's redundancy version, TS 38.212 §5.4.2;
6. **interleaving** by `π(k) = k·p mod E`, where `p` is the integer nearest `E/φ` that is
   coprime with `E` (φ the golden ratio). Consecutive coded bits land about 0.618·E apart in
   the time-major symbol grid, so a fade in time or a notch in frequency is scattered across
   the whole codeword;
7. Gray-labelled mapping onto the constellation.

The receiver inverts this with soft information throughout: per-symbol log-likelihood ratios
weighted by an estimated noise variance, de-interleaved, rate-recovered into a full-codeword
LLR buffer, and decoded. **Keeping that buffer and adding the next redundancy version to it
is hybrid ARQ with incremental redundancy**, and it is how the link survives below a mode's
nominal threshold.

---

## 6. Receiver requirements

A conforming receiver is not required to use any particular algorithm, but must handle:

* **Acquisition** over a carrier offset of at least ±250 Hz, at the most robust mode's
  threshold. The reference receiver uses a partial-matched-filter/FFT bank over both preamble
  sequences and acquires 100 % of frames at −5 dB and 87 % at −7 dB.
* **Sample-rate offset** of at least ±50 ppm between the two stations' clocks.
* **Residual carrier offset** estimation from the comb pilots; 64-QAM needs the frame's
  residual well below 0.1 Hz.
* **Channel estimation** from the comb pilots, interpolated across frequency and smoothed
  across symbols.
* **Per-symbol noise variance**, so a symbol damaged by impulsive noise is discounted rather
  than trusted. This matters more on HF than the interpolation does: the FFT spreads a single
  hot sample across every carrier of its symbol.

An impulse blanker ahead of the receive filter is strongly recommended and is essentially
free; see `bench/README.md`.

---

## 7. Link layer

### 7.1 Containers

The link layer places its own header at the front of each PHY payload.

DATA container:

| Offset | Field |
|---|---|
| 0 | kind (3 bits) \| flags (5 bits) |
| 1 | sequence number |
| 2 | session id |
| 3–4 | data length, only when the PARTIAL flag is set |

**Nothing in this header may differ between transmissions of the same sequence number.** A
retransmission is the same codeword under another redundancy version, and the receiver
soft-combines them; a header that changed would make the combination meaningless. That is why
no burst length or position appears here — the receiver derives a frame's position in its
burst from its air time, and the end of a burst from the silence that follows.

Kinds: `DATA` (0), `CONNECT_REQ` (1), `CONNECT_ACK` (2) (callsigns do not fit in a control
frame), `BEACON` (3), `PROBE` (4), `PROBE_ACK` (5) and `DATAGRAM` (6). A receiver ignores a
kind it does not know, which is what lets a kind be added. `BEACON`, `PROBE`, `PROBE_ACK` and
`DATAGRAM` are sent outside sessions, and a station in a session never takes one of them for
session data, whatever its session id says.

A `CONNECT_REQ` and a `CONNECT_ACK` carry this body, at the most robust mode:

| Offset | Field |
|---|---|
| 0–6 | calling station, packed |
| 7–13 | called station, packed |
| 14 | capability byte (§7.3) |
| 15 | protocol version, 1 |
| 16 | measured SNR, as the CONTROL frame's byte (signed dB, 3 kHz reference, ties to even, −40 … +40; 0x7F = not measured): in an acceptance, the SNR the request arrived at; in a request, 0x7F |

The SNR byte is the **faster start**: a session used to begin at the most robust mode
and climb from there, a burst per step, proving what the connect frames had already
measured. The called station starts its rate controller from the request's SNR and
sends that SNR back; the caller starts its first burst one step below the fastest mode
that SNR supports with the controller's margin and hysteresis, and starts its own
controller from the SNR the acceptance arrived at, which is what it will recommend once
it receives. A station of an earlier version sends a sixteen-byte body, and a receiver
reads a body that stops at the version byte as "not measured" and starts as before.

A `BEACON` frame is **unproto**: sent outside any session, addressed to nobody, with a session
id of zero and a body that is one packed callsign. It is how an operator answers "can anybody
hear me?" without arranging a contact first, which on HF is most of what a new station needs
to know. A receiver reports the callsign and the SNR it measured and does nothing else — a
beacon is never answered on the air, because a channel where every beacon drew a reply would
be unusable. It is sent on the tone floor (tone-24), the most robust frame there is, because the
whole point is to be heard by somebody who cannot yet hear anything else; the floor's frames are
the same in both bandwidths, so a station of either hears it.

A `PROBE` frame is a beacon with a destination: "can *you* hear me, and how well?" It is
sent outside any session (session id zero, sequence zero, on the tone floor) with this
body:

| Offset | Field |
|---|---|
| 0–6 | source callsign, packed |
| 7–13 | destination callsign, packed |
| 14 | measured SNR, as the CONTROL frame's byte: signed dB, 3 kHz reference, ties to even, −40 … +40; 0x7F = not measured (a `PROBE` always says 0x7F) |
| 15 | capability byte (§7.3): the bandwidth the frame was sent in |

A station that is addressed by a probe, is idle, and finds the probe's stated bandwidth to
be its own answers with one `PROBE_ACK`, in the family the probe arrived in — the same body
with the callsigns swapped and the SNR it measured on the probe in the SNR byte. The prober
then has the two numbers that describe a path, one from each end, and reports them (a reading
from a floor frame is a lower bound on a strong path, §2.4); a probe that draws no answer
within a floor frame's turnaround is reported as unanswered, and there are no retries — the
operator asks again, so a probe can never fill a channel by itself. A station in a
session ignores probes (the session's frames matter more), and a station never answers a
probe addressed to somebody else. Answering is a *response* in the sense of
§97.221(c), so a station restricted to answering may answer a probe; sending one is a
call, and it may not.

A `DATAGRAM` frame carries a piece of **another program's frame** — an AX.25 frame a KISS
client handed the modem (ADR-0019) — outside any session and with no acknowledgement: the
client repeats whatever its own protocol needs repeated. The two header bytes a session uses
number the pieces instead:

| Header field | In a `DATAGRAM` |
|---|---|
| sequence number | the fragment's index (high nibble) \| the last fragment's index (low nibble): at most sixteen fragments |
| session id | the datagram's number, 1–255, advanced by the sender for every datagram so that pieces of two datagrams are never joined |

The pieces, joined in index order, are this body:

| Offset | Field |
|---|---|
| 0–6 | sending station, packed — every datagram identifies its station in the emission itself, whatever the client's frame holds |
| 7 | frame type, as a VARA-style KISS client names it: 0 an AX.25 frame, 1 an AX.25 frame with eight-byte address fields, 2 unformatted data |
| 8… | the frame, byte for byte |

Every fragment but the last is a full frame; the last is partial, with the explicit length —
and a remainder exactly one byte too long for a partial frame goes as two fragments. All the
fragments of a datagram go at the rung the sending station is configured to send datagrams
at (tone-36 by default: the tone floor's frames are the same in both bandwidths, so a station
of either hears them), in as few bursts as the transmitter's key limit allows, one after
another, and only while the station is in no session. A receiver
joins the pieces it decodes, hands the frame and its type to its own KISS clients once every
piece has arrived, keeps at most eight incomplete datagrams, and drops one that has waited
two minutes for a piece that is not coming.

CONTROL container:

| Offset | Field |
|---|---|
| 0 | kind (4 bits) \| flags (4 bits) |
| 1 | session id |
| 2 | base sequence number — the next one the receiver needs |
| 3–4 | bitmap: bit *i* set ⇔ `base + i` received |
| 5 | measured SNR, signed dB, 3 kHz reference; 0x7F = unknown — in an acknowledgement, the mean over the burst's frames that decoded or that the receiver acquired with confidence, unknown when there was none (ADR-0020); in any other control frame, the SNR of the last frame of the session its sender decoded from the other station (ADR-0021) |
| 6 | recommended mode (5 bits) \| counter (3 bits) |

Kinds: `ACK`, `POLL`, `TURN`, `DISC`, `DISC_ACK`. The recommended mode is a rung of the
air's ladder; the counter numbers a station's acknowledgements modulo 8, for logs. Protocol
version 2 split the byte four and four; the fast kinds (ADR-0014) took the 2 300 Hz ladder to
twenty rungs.

The SNR byte is the measurement rounded to the nearest integer decibel and clamped to
−40 … +40, with **ties rounded to even** (12.5 dB encodes as 12, 13.5 as 14). The tie rule
is stated because it is the kind of detail two implementations silently disagree on —
Python rounds halves to even and Rust rounds them away from zero — and a wire format that
two correct implementations encode differently is not a wire format.

### 7.2 Session

A session identifier is never zero: with kind DATA and sequence zero an all-zero body
would make an all-zero frame, and a receiver **discards any decoded block that is all
zeros** — the all-zero word is a codeword of every linear code and its CRC is zero, so a
decoder fed a false detection converges to it. Every other frame carries a non-zero kind.


One station is the information sending station (ISS), the other the information receiving
station (IRS). The ISS sends a burst of data frames; the IRS answers each burst with one ACK
carrying the selective-repeat bitmap, the SNR it measured, and the mode it recommends. The
ISS retransmits what the bitmap reports missing — same codeword, next redundancy version —
and fills the remainder of the burst with new frames at the recommended mode.

`TURN` hands the sending role to the peer, and is sent when the peer has set `WANT_TX` or
`BREAK` in an ACK. `POLL` keeps an idle link alive. `DISC`/`DISC_ACK` close it. Connection is
a two-way handshake in DATA-container frames carrying both callsigns, with randomised
backoff so two stations calling each other simultaneously desynchronise instead of colliding
on every retry. The first request goes out on the tone floor and the tries alternate between
the floor and the ordinary family's robust mode (ADR-0016); the acceptance goes back in the
family the request arrived in, and a caller does not send a try over a frame it hears
arriving. An ISS waits for an acknowledgement in the longer of two families: its burst's, and
the one the IRS last heard it in — the IRS answers in the latter when it decoded none of the
burst.

### 7.3 Capability negotiation

The connect request and its acceptance each carry a one-byte capability field (§7.1). A capability is
used only if **both** stations offered it: a station that has not said it can do something
cannot be assumed to, and the only safe reading of a missing bit is that the feature is
unavailable. Unknown bits are ignored, so a later version can add one without breaking an
earlier one.

| Bit | Meaning |
|---|---|
| 0 | Stream compression: deflate, RFC 1951 |
| 1–2 | Bandwidth of the waveform this frame was sent in: 0 = 2 300 Hz, 1 = 500 Hz, 2 = 2 750 Hz (reserved), 3 = reserved |
| 3–7 | Reserved, must be zero |

The bandwidth bits are not negotiated: a frame's waveform is a physical fact the receiver
already knows from having decoded it, and the bits state it so a station can refuse a
request whose stated bandwidth is not the one it arrived in, and so a station that listens
in more than one bandwidth answers in the one it was called in. Both stations of a session
use one bandwidth for its whole life.

**Compression is applied to the payload byte stream, above the ARQ, not to individual
frames.** A frame is 26 bytes on the slowest mode, and a compressor with no history makes a
block that size larger rather than smaller. The link layer already delivers bytes in order and
exactly once, which is precisely what a stream decompressor needs; selective repeat, HARQ and
retransmission all happen underneath and are invisible to it.

The sender flushes the coder (a deflate *sync flush*) at the end of each application write, so
nothing is left sitting in the compressor waiting for input that may never arrive. A receiver
therefore never has to wait for more data to decode what it already has.

### 7.4 Ordering rule

The ISS composes every burst as **unacknowledged frames in ascending sequence order, then new
frames**. This is required, not advisory: it is what allows the IRS to infer the sequence
number of a frame whose payload did not decode, and therefore to soft-combine it with the
retransmission that follows.

---

## 8. Constants

<!-- BEGIN:constants -->
| Constant | Value |
|---|---|
| Payload CRC | CRC24A, polynomial 0x864CFB, 24 bits |
| Schmidl-Cox PN seed, DATA | 4649 |
| Schmidl-Cox PN seed, CONTROL | 7919 |
| Mode/RV chip seed | 20260913 |
| Redundancy versions | 4 |
| Peak reduction target, PSK modes | 5.0 dB |
| Peak reduction target, QAM modes | 7.0 dB |
| Link DATA header | 3 bytes (5 with an explicit length) |
| Link CONTROL frame | 7 bytes |
| Callsign encoding | 6 bits/character, 9 characters in 7 bytes |
| Selective-repeat window | 16 frames |
<!-- END:constants -->

---

## 9. Timing

A receiver establishes the end of a burst from silence. Two cases:

* if the physical layer can report a *detected preamble* before the frame is decoded, the
  receiver need only wait that long plus a small guard;
* otherwise it must wait a full data-frame time, because a contiguous next frame would not
  otherwise have announced itself.

The second costs roughly a quarter of the air time, so reporting preambles early is worth
implementing. Turnaround guard, detection latency and acknowledgement air time are
implementation parameters; the reference values are in `model/aether_model/link/harness.py`.

---

## 10. Performance

Measured on the reference implementation against the calibrated channel simulator; see
`bench/README.md` for method and the raw data. Minimum usable SNR (3 kHz, 10 % frame error
rate), and end-to-end link goodput with adaptive rate control:

| | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| Tone floor, 36 bit/s (rung 0) | −19.0 dB | −11.7 | −13.2 | −14.5 |
| Tone floor, 54 bit/s (rung 1) | −17.3 | −8.8 | −10.3 | −10.9 |
| Fast tones, 76 bit/s (rung 2) | −16.0 | −8.6 | −11.2 | −11.8 |
| Fast tones, 112 bit/s (rung 3) | −14.2 | −5.7 | −8.2 | −7.5 |
| Fast tones, 157 bit/s (rung 4) | −13.1 | −6.5 | −8.2 | −8.4 |
| Fast tones, 228 bit/s (rung 5) | −11.2 | −3.2 | −4.3 | −4.1 |
| Tone floor, control frame | −19.5 | −11.1 | −14.5 | −15.8 |
| Most robust OFDM mode (rung 6, BPSK ⅕, 197 bit/s) | −5.1 dB | +2.0 | +1.2 | −0.3 |
| QPSK 1/2 | +1.4 | +8.7 | +8.5 | +6.0 |
| Goodput at +12 dB | 1696 bps | 859 | 819 | 1034 |
| Goodput at +20 dB | 2106 bps | 1696 | 1565 | 1745 |

At 500 Hz (`bench/baselines/phy_fer_500.csv`), the same reference — the same transmitter
power into the same noise:

| | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| Tone floor, 36 bit/s (rung 0) | −19.0 dB | −11.7 | −13.2 | −14.5 |
| Tone floor, 54 bit/s (rung 1) | −17.3 | −8.8 | −10.3 | −10.9 |
| Middle kind, 76 bit/s (rung 2) | −14.3 | −7.5 | −10.0 | −10.3 |
| Middle kind, 112 bit/s (rung 3) | −13.0 | −3.2 | −4.9 | −5.7 |
| Control mode (QPSK ½) | −5.2 | +4.0 | +3.5 | +0.0 |
| 16-QAM ½ | −0.1 | +9.0 | +9.5 | +8.0 |
| Fastest narrow mode (64-QAM ⅚) | +10.4 | +21.0 | > +22 | > +23 |
| Best single-rung throughput at −10 dB | 112 bps | 69 | 69 | 73 |
| Best single-rung throughput at +12 dB | 1040 bps | 533 | 469 | 389 |

The tone floor's rows are the same on either air (`bench/baselines/tone_floor.csv`, 100
frames a point, through the detector; the fast kinds are the 2 300 Hz air's alone, the
four-tone middle kinds the 500 Hz air's) and are at
equal peak power: its frames go out 5.5 dB
above an OFDM frame's average at the same transmit level (§2.4), and the SNR is the OFDM
frames' reference, so every row of both tables reads against the same transmitter. The
OFDM floor the tone floor replaced at 500 Hz (ADR-0009, `floor_500.csv`) needed −13.0,
−4.0, −4.7 and −3.0 dB for its 36 bit/s mode: the tone floor is 6 dB better on AWGN and 8–12
dB better on the fading channels, where its frames span several fades and it needs no
channel estimate. The narrow control mode equals the wide table's most robust OFDM mode on
AWGN, because twelve carriers carry ≈ 6.8 dB more per carrier than fifty-seven; on the
fading channels a fifth of the frequency diversity costs it about 2 dB on ITU Good. The
fading-channel crossings of the slowest rungs sit on shallow curves and move a decibel or
two between runs of a hundred frames.

These are simulator figures. No on-air measurements exist yet, and none should be inferred.

---

## 11. Open items for v1.0

* A call on the tone floor is heard by a station of either bandwidth; negotiating across
  the two is not specified.
* The wide (2.75 kHz) bandwidth variant.
* Compression negotiation, CW identification, beacon and ping datagrams.
* Formal test vectors published alongside this document; the reference vectors in `vectors/`
  serve that purpose today but are not yet a normative part of the specification.
* On-air validation of everything in §10.
