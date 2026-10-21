# Aether HF air interface — specification v0.1

Status: **draft**, tracking the reference model on `master` (link protocol version 11,
§7.1). Numbering and constants are stable enough to implement against; anything still open is
called out in §11.

This document is deliberately public. An amateur digital mode that cannot be decoded by a
third party is bad practice and, in the United States, arguably bad law: FCC §97.309(a)(4)
permits an unspecified digital code only when the technique is publicly documented. Every
table between BEGIN/END markers is generated from the reference implementation
(`python tools/make_spec.py`) and checked by the test suite; the numbers in the prose and in
§10 are written by hand from the model and the committed baselines.

Companion documents: `control-api.md` (how an application drives a modem),
`host-interfaces.md` (the VARA-compatible host interface — Winlink Express, Pat, VarAC — and
the KISS port), `../adr/` (why each choice was made),
`../../bench/README.md` (the measurements every performance claim here rests on).

---

## 1. Scope

Aether HF is an ARQ data mode for amateur HF: an OFDM physical layer in a 2.3 kHz or a
500 Hz SSB channel with a constant-envelope sixteen-tone FSK *tone floor* beneath it, a
3GPP-derived LDPC code, and a selective-repeat ARQ with hybrid retransmission; beacons, probes
and datagrams travel outside sessions. This version specifies two OFDM bandwidths on one
numerology, **WIDE_2300** (§2–§4) and **NARROW_500** (§2.3, §4.1), which reuses everything
here with twelve carriers instead of fifty-seven and its own mode table, and the tone floor
(§2.4), whose frames are the same on either air. A wider 2.75 kHz variant is reserved (§11).

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

### 2.2 Peak reduction

The transmitter clips and re-filters each OFDM frame to a target peak-to-average ratio
(§ constants), iterating four times with the same band-limiting filter the receiver uses.
This is transmitter-side only: a receiver needs no knowledge of it, and a transmitter may
omit it at a cost of roughly 1 dB of delivered power. Constant-modulus modes take the more
aggressive target; the QAM modes carry information in amplitude and take the gentler one. A
tone-floor frame (§2.4) has a constant envelope and is not clipped.
See `../adr/0004-papr-reduction.md`.

### 2.3 The 500 Hz waveform

The narrow waveform is the same numerology — the same sample rates, FFT, subcarrier
spacing, cyclic prefix, window and symbol period — with **twelve** active carriers centred
on the same passband centre, so it occupies 480 Hz. It exists because a 500 Hz signal is
what most HF peer-to-peer traffic is made with, and because it puts the transmitter's power
into a fifth of the band: at the same 3 kHz-referenced SNR each carrier has ≈ 6.8 dB more
signal-to-noise than a wide carrier, which is what lets its control mode be QPSK ½ and
still reach the SNR of the wide waveform's control mode, BPSK ⅕ (−5.2 against −5.1 dB on
AWGN, §10). The comb pilots follow the same rule (every 4th
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
is what that set holds. The acquisition threshold is higher (the table above) because
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
glide between tones caps it near +17.5 dB on a clean channel, and on a dispersive one the
echo's spill into the next symbol, counted as noise, caps it lower — about +15.5 dB on ITU
Good, +12 on Moderate and +5 on Poor (ADR-0016 §4, `bench/baselines/tone_snr_reading.csv`).
A rate controller seeded from a floor frame seeds again, once and upward only, from the first
clean burst it measures on an OFDM rung, when that reads more than 3 dB above the floor's
reading.

The floor's 40 ms symbols carry a path whose echo breaks the faster rungs, and the SNR such a
path reads cannot say so: on ITU-R F.1487's disturbed NVIS (two rays 7 ms apart) the floor reads
about −3 dB whatever the SNR, as a quiet path at −3 dB reads. So the reference receiver also
reads the echo on the floor's own frames — tone-24, tone-36 and the control frame, every call,
acceptance and floor control frame among them: the energy at the neighbouring symbols' tones
inside a symbol, both sides, where they stand two or more bins from its own, over its own tone,
each less the noise (ADR-0087). A two-ray path reads about 0.003 at 4 ms and 0.012 at 7; AWGN
and ITU's Good, Moderate and Poor paths read 0.0003 or less. Its rate controller keeps off the
rungs the smoothed reading rules out — on the 500 Hz air rungs 3 and 4 above 0.0038, on the
2 300 Hz air rung 5 above 0.0073 (`bench/baselines/echo.csv`) — and, since it climbs rung by
rung, does not reach the rungs above one it keeps off. Nothing of this is on the air: a
receiver that reads no echo recommends as before.

The tones are spaced at the symbol rate about the passband centre and span 400 Hz, inside
a 500 Hz channel on either air. Between symbols the frequency glides on a raised cosine and
the phase runs on, which keeps the spectrum inside the channel (99.9 % of the power within
±250 Hz, −56 dB beyond ±500 Hz) without touching the envelope. Each symbol carries four
coded bits under a Gray label, so neighbouring tones — the ones a carrier offset or a
Doppler smear confuses — differ in one bit: tone *k* (lowest first) carries the
binary-reflected Gray code of *k*, *k* XOR (*k* >> 1), most significant bit first.

A frame is three **sync blocks** of eight symbols — at its start, after 45 % of its data,
and at its end — with the data between them. Each block's tones are a *Costas sequence*:
every displacement (Δsymbol, Δtone) between two of its symbols occurs once (J. P. Costas,
*Proc. IEEE*, 1984), so a block shifted in time or frequency matches itself in at most one
symbol and a receiver finds timing and carrier offset together. The pattern names the
frame: one for the control frame and one per redundancy version of each data kind, no two
sharing more than two symbols under any offset within a block and any shift of up to eight
tones. The uneven split of the data makes the three distances between a frame's blocks
differ, so no shift of a frame lines up more than one of its blocks with another's.

The start and end blocks name the frame's kind and redundancy version. The middle block says
how many frames of its transmission follow it (link protocol 8, ADR-0068 §2.1): its pattern is
the kind's for redundancy version (RV + count) mod 4, the count 0–3, 3 meaning three or more.
A receiver reads a count only when the middle block's hits under one count reach five and beat
every other count's by three; a faded block states none, rather than a guess.

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

The 500 Hz ladder has three **middle kinds** of its own (ADR-0015, ADR-0081): the same frame
with four data symbols in each data slot on **four** tones 100 Hz apart, at ±50 and ±150 Hz —
the floor's own 400 Hz — two Gray-labelled bits a symbol and 880 coded bits a frame, at the
floor's two code rates (51 and 75 bytes) and at 0.87 (93 bytes, `tone4x100-93`). Their data
glides over the floor's own 32 samples, which keeps them inside the channel as the floor's
frames are (99.9 % of the power within ±250 Hz). The 2 300 Hz air does not look for them.
Four tones 100 Hz apart fade apart on a path with a millisecond or two of delay spread, and
they fade between sync blocks 2.4 s apart, so a receiver of these reads each tone's level from
the data itself (ADR-0080): every tone's probability in each symbol under the sync blocks'
level, the tone's energy weighted by it over the 25 symbols around, pulled toward the sync
blocks' level by how far the two disagree beyond the estimate's own noise; the metric is then
each tone's own, `−s + log I0(2·sqrt(E·s))`. Where the sync tones show the channel fading within
a block (flutter), that level is only a mean, and the tone's power is taken as Nakagami-
distributed about it, of an order the blocks measure (ADR-0080 §2.4). This is the receiver's
choice, not the signal's.

A candidate is a frame only if at least 12 of its 24 sync symbols have their own tone
strongest, at least 4 of them in a second block, and no more than two are **contradicted**:
a sync symbol whose strongest tone is another one, at least 12 times the noise per bin and
at least four times that tone's median over the frame's sync symbols (a steady carrier or
spur is not a contradiction). A pattern read at a part-symbol offset inside a strong frame
it does not name — another air's middle kind, or a frame read a block early — matches half its
symbols there and is contradicted in the rest. A frame whose two best blocks are whole and
uncontradicted stands whatever its third holds (ADR-0049). And a frame every block of which
holds it (5 of 8) counts only the contradictions that reach half its own sync level, the median
energy of its own tone where that tone is strongest (ADR-0083): on fast fading a sync tone fades
out inside its symbol and leaves the glide and its neighbours' spread tones strongest, at a
tenth of the level, while a ghost's contradictions are a real frame's tones at its level.

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
| Detector | hop 80 samples, bin 6.25 Hz | offset search +/-150 Hz |
| Acquisition threshold | 3.0 | mean sync-tone ratio, each clipped at 10; 12 of 24 sync tones strongest, 4 of them in a second block; a silent symbol is no evidence; at most 2 contradicted (another tone strongest, 12x the noise, not steady), unless the two best blocks each have 7 of 8 and none contradicted, or every block has 5 of 8 and at most 2 contradictions reach 0.5 of the frame's own sync level |
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
| tone4x100-93 | 93 | 440 at 100 Bd in 110 + 3 x 8 = 134 | 5.36 s | 0, 57, 126 | 0.87 | 138.8 | 33, 34, 35, 36 | -11.1 |

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
| 33 | 5 12 7 15 4 6 0 1 |
| 34 | 9 12 14 2 1 5 15 0 |
| 35 | 0 2 7 4 15 8 12 1 |
| 36 | 5 3 13 14 0 15 2 7 |
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
interleaver of §5 — ending in Gray-labelled bits a tone: four on sixteen tones, two on the
middle kinds' four. Redundancy versions 0 and 3 begin at or wrap round to the systematic bits
and can decode alone; 1 and 2 are mostly parity. Every version combines with the ones before
it (HARQ-IR) as an OFDM frame's does.

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

LONG carries every DATA-container frame sent at an OFDM rung: user data, a connection
request's ordinary tries and their answers, the answer to a probe that arrived in OFDM, and
datagrams sent at an OFDM rung. SHORT carries acknowledgements and the other control frames at
the air's **control mode** — BPSK ⅕ (OFDM mode 0) at 2 300 Hz, QPSK ½ (OFDM mode 3) at
500 Hz. A frame's *family* is the tone floor's or the ordinary (OFDM) one. While a link runs
any tone rung — the floor's own kinds, the fast kinds or the middle kinds — its data frames
are tone frames and its control frames the tone floor's control frame; a sending station's
control frames go on the floor too while the other station answers them unread (§7.2). A
connection request starts on the floor, and probes and beacons go out on it (§7).

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

**The count** (link protocol version 8, ADR-0068 §2.1; the chips' turn since version 5,
ADR-0041). A DATA frame's chips are multiplied by `j^f`, where `f` is how many frames of its
transmission follow it, exactly: 0 for the last, 3 for three or more. A receiver picks the
(mode, RV) sequence by the *magnitude* of its correlation, which the turn does not change, and
reads `f` as the nearest quarter turn of that correlation's phase against the comb pilots'
channel estimate; the pilot symbols are then known with the turned chips. The count is outside
the codeword, like the RV, so retransmissions still soft-combine whatever their place in a
later transmission. A tone-floor frame carries its count in its middle sync block (§2.4). A
receiver believes a count only from a copy that decoded alone: the chips of a copy too weak to
decode alone can read a quarter turn off, and one HARQ rescued at −1.5 dB on the scenario
harness carried three to follow read as none (§7.2, rule 2).

---

## 4. Modes — the ladder

What the link layer calls "mode N" is a **rung** of the air's ladder: the tone floor's data
kinds (§2.4) — its own two, then on the 2 300 Hz air its four fast kinds and on the 500 Hz
air its three four-tone middle kinds — then the air's OFDM modes, most robust first. An OFDM frame's chips carry its OFDM mode index (§3.2),
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
wide air, and rungs 2–4 its four-tone middle kinds (ADR-0015, ADR-0081). Above them the
narrow OFDM mode *m* is rung *m* + 2: OFDM modes 0 and 1 were the OFDM floor of ADR-0009,
which the tone floor replaced, and mode 2, QPSK ⅓, was rung 4 until `tone4x100-93` took its
place (it carried less after its header than rung 3 from 7 dB more); all three are on no rung.
Rung 5, QPSK ½, is the **control mode** — the slowest
whose SHORT frame carries a seven-byte control frame and whose LONG frame carries a
connection request; ordinary control frames go out at it, and so do a connection request's
ordinary tries and the answers to requests and probes that arrived in the ordinary family
(§7.2). Rung 4, the four-tone frame at rate 0.87, is the step between the middle kinds at the
floor's rates and the control mode. Rung 8 (8-PSK ⅔, OFDM mode 6) is never selected: rung 9 carries the same
53 bytes 0.6 dB lower.

<!-- BEGIN:modes500 -->
| Rung | Name | Frame | bits/sym | Rate | Base graph | Z | K' | E | Payload B | Net bps | AWGN dB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | tone-24 | TONE, 25 Bd data | 4 | 0.49 | BG2 | 28 | 216 | 440 | 24 | 36 | -19.0 |
| 1 | tone-36 | TONE, 25 Bd data | 4 | 0.71 | BG2 | 40 | 312 | 440 | 36 | 54 | -17.3 |
| 2 | tone4x100-51 | TONE, 100 Bd data, 4 tones | 2 | 0.49 | BG2 | 56 | 432 | 880 | 51 | 76 | -14.3 |
| 3 | tone4x100-75 | TONE, 100 Bd data, 4 tones | 2 | 0.71 | BG1 | 30 | 624 | 880 | 75 | 112 | -13.0 |
| 4 | tone4x100-93 | TONE, 100 Bd data, 4 tones | 2 | 0.87 | BG1 | 36 | 768 | 880 | 93 | 139 | -11.1 |
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

* **Acquisition** of an OFDM frame over a carrier offset of at least ±250 Hz at the most
  robust OFDM mode's threshold — the reference receiver's partial-matched-filter/FFT bank
  searches ±300 Hz over both preamble sequences and acquires 100 % of frames at −5 dB and
  87 % at −7 dB — and of a tone frame at the tone floor's own threshold over at least
  ±150 Hz, which is what the reference detector searches (§2.4; ADR-0064). The reference
  receiver also centres itself on the station it is in session with, moving only while it
  transmits and back on the dial between sessions, so a transmitter that drifts is followed
  past either search (ADR-0079).
* **Sample-rate offset** of at least ±50 ppm between the two stations' clocks.
* **Residual carrier offset** estimation from the comb pilots; 64-QAM needs the frame's
  residual well below 0.1 Hz.
* **Channel estimation** from the comb pilots, interpolated across frequency and smoothed
  across symbols — by less where the channel moves within the frame. The reference receiver
  averages three symbols a third each, and where the pilots' second difference in time stands
  clear of the noise their second difference across frequency shows on the full pilot
  symbols, it weighs the neighbours by `w = 2·noise / D2` instead, under a fifth (ADR-0082): on
  ITU flutter (10 Hz) the three-symbol average had averaged a channel that turns over in
  three symbols, and its own error was the frame's noise.
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
| 3–4 | a piece's place: offset (12 bits) \| last (1 bit) \| 0 (3 bits), only when the FRAGMENT flag is set |
| then 2 bytes | data length, only when the PARTIAL flag is set |

Flags: PARTIAL is bit 0 (0x01 of byte 0) and FRAGMENT bit 1 (0x02, link protocol 11); the other
three are zero. A body that fills the payload after the header bytes is full; any shorter body
is PARTIAL with its length in the two bytes after the header, and the rest of the payload is
zeros — so a body one byte short of full cannot be carried, and a sender leaves that byte for
the next frame.

A FRAGMENT frame carries a **piece** of the body of the `DATA` frame with its sequence number
(ADR-0086, rule 9 of §7.2): the offset is where in that body the piece starts, in bytes, and
`last` says the piece ends it. A receiver places pieces by offset — pieces of two splits at
different boundaries make the body as well — and acknowledges the sequence number only once
every byte of the body has arrived. A piece one byte short of a full frame is the one length
the container cannot carry, so such a last piece goes as two.

**Nothing in this header may differ between transmissions of the same codeword.** A
retransmission is the same codeword under another redundancy version, and the receiver
soft-combines them; a header that changed would make the combination meaningless. That is why
no transmission length or position appears here — the receiver derives a frame's place in
its transmission from its air time, and the transmission's end from the count every frame
carries outside its codeword (§3.2, §2.4) or the header before it (§7.2).

Kinds: `DATA` (0), `CONNECT_REQ` (1), `CONNECT_ACK` (2) (callsigns do not fit in a control
frame), `BEACON` (3), `PROBE` (4), `PROBE_ACK` (5), `DATAGRAM` (6) and `MEASURE` (7, link
protocol 9: §7.2, rule 14). The kind's three bits are all in use. `BEACON`, `PROBE`,
`PROBE_ACK` and `DATAGRAM` are sent outside sessions, and a station in a session never takes one
of them for session data, whatever its session id says.

A `CONNECT_REQ` and a `CONNECT_ACK` carry this body with sequence number 0 and the session
id the caller chose (at random, 1–255), at the slowest rung of their family that carries it:
tone-24 on the tone floor; BPSK ⅕ (rung 6) at 2 300 Hz or QPSK ½ (rung 5, the control mode)
at 500 Hz in OFDM (§7.2 says which family):

| Offset | Field |
|---|---|
| 0–6 | calling station, packed |
| 7–13 | called station, packed |
| 14 | capability byte (§7.3) |
| 15 | link protocol version: 11 |
| 16 | measured SNR, one signed byte (whole dB, 3 kHz reference, ties to even, −40 … +40; 0x7F = not measured): in an acceptance, the SNR the request it answers arrived at — a request that arrives again after the session is up (its acceptance was lost) is answered again with its own; in a request, 0x7F |
| 17 | the sender's **turnaround**: bits 0–3, how long its radio takes to hear again after it stops sending (VOX hold, receiver recovery), in 0.1 s steps rounded up, 0–1.5 s; bits 4–7 reserved, zero |
| 18 | the sender's **identifier**: its closing Morse identifier's length in 0.5 s steps rounded up, 0 for none |

Nineteen bytes, with a DATA header and an explicit length, fill the 24 bytes of tone-24, the
floor's slowest rung. A body shorter than nineteen bytes is of version 7 or earlier.

A station ignores a request or an acceptance whose version is not its own, and says so.
Version 2 made a mode number a rung of the ladder (ADR-0013), 3 widened the control frame's
recommended mode to five bits (ADR-0014), 4 added the 500 Hz middle kinds (ADR-0015), 5 the
burst countdown (ADR-0041), 6 counted it in pairs (ADR-0046), 7 the turn offered at the end of
a burst (ADR-0047), 8 granted slots (ADR-0069: the control frame below, the connect body's two
more bytes, and every rule of §7.2), 9 the measurement frames (ADR-0077), 10 the 500 Hz
rung 4, `tone4x100-93` in the place of QPSK ⅓ (ADR-0081), and 11 a stuck frame sent in pieces,
the FRAGMENT flag (ADR-0086). The same number
names different frames in different versions, so a session between two versions cannot run.

The SNR byte is the **faster start**: a session used to begin at the most robust mode
and climb from there, a transmission per step, proving what the connect frames had already
measured. The called station starts its rate controller from the request's SNR and
sends that SNR back; the caller starts its first data two usable rungs below the fastest
rung that SNR supports with the controller's margin and hysteresis — never falling from an
OFDM rung onto the tone floor for that caution; on the floor the two rungs are counted in
decibels, so a SNR the floor's top rung clears by as much as the two rungs below it would have
kept starts there (ADR-0066) — and starts its own controller from the SNR
the acceptance arrived at, which is what it will recommend once it receives. A body that stops
at the version byte reads as "not measured"; only a version-1 station sends one, and its call is
ignored.

A `BEACON` frame is **unproto**: sent outside any session, addressed to nobody, with a session
id of zero and this body:

| Offset | Field |
|---|---|
| 0–6 | the station's callsign, packed — or the name a host program gave the beacon when one of the station's callsigns is its base, the part before any `-` (VarAC's `KK4ODA-9` for `KK4ODA`) |
| 7 | capability byte (§7.3): the bandwidth its sender runs — absent from a beacon of before ADR-0024, which a receiver reads as not said |

A receiver of an earlier version reads the callsign and never looks further. It is how an
operator answers "can anybody
hear me?" without arranging a contact first, which on HF is most of what a new station needs
to know. A receiver reports the callsign (or name), the bandwidth and the SNR it measured and
does nothing else — a
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
| 14 | measured SNR, as the connect body's byte: signed dB, 3 kHz reference, ties to even, −40 … +40; 0x7F = not measured (a `PROBE` always says 0x7F) |
| 15 | capability byte (§7.3): the bandwidth the frame was sent in, and in a probe the gap level of the answer's slot |

A station that is addressed by a probe and is idle answers with one `PROBE_ACK`, on the floor —
the same body with the callsigns swapped and the SNR it measured on the probe in the SNR byte —
in the slot the probe grants (§7.2, rule 11), or not at all. A probe is answered across
bandwidths (ADR-0035): its frames are the floor's, the same on both airs, and the answer's
capability byte tells the prober what the other station runs. The prober
then has the two numbers that describe a path, one from each end, and reports them (a reading
from a floor frame is a lower bound on a strong path, §2.4); a probe that draws no answer by
the end of its answer's slot — moved past any frame the prober hears arriving, which may be the
answer — is reported as unanswered, and there are no retries — the
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

Every fragment but the last is a full frame; the last is partial, with the explicit length,
unless it fills the frame — and a remainder exactly one byte too long for a partial frame goes
as two fragments. All the
fragments of a datagram go at the rung the sending station is configured to send datagrams
at (tone-36 by default: the tone floor's frames are the same in both bandwidths, so a station
of either hears them), in as few bursts as the transmitter's key limit allows, one after
another, and only while the station is in no session. A receiver
joins the pieces it decodes, hands the frame and its type to its own KISS clients once every
piece has arrived, keeps at most eight incomplete datagrams, and drops one that has waited
two minutes for a piece that is not coming.

CONTROL container, seven bytes (link protocol 8, ADR-0069):

| Byte | Bits | Field |
|---|---|---|
| 0 | 7–6 | kind: `HEADER` (0, the master's), `ANSWER` (1, the slave's), `DISC` (2), `DISC_ACK` (3) |
| 0 | 5–3 | count: data frames after this one in its transmission, exactly, 0–6 |
| 0 | 2 | those data frames are tone-floor frames |
| 0 | 1–0 | level: in a header, the gap level of the slot it grants; in an answer, the level its sender needs; in a `DISC`, the level of the `DISC_ACK`'s slot |
| 1 | 7–1 | the session number's low seven bits (data frames carry all eight) |
| 1 | 0 | fam: in a header, the family the answer goes in (1 = the tone floor); in an answer, the family its sender needs headers in |
| 2 | 7 | flag: in a header, the slot is an open window; in an answer, its sender asks to leave |
| 2 | 6 | in a header or an answer, `measure` (version 9): the data frames after it are measurement frames, or with none after it, its sender's last were and it has not read the report; in a `DISC` or a `DISC_ACK`, the `DISC_ACK`'s slot includes the slave's closing identifier |
| 2 | 5–0 | SNR: how the sender hears the other station, signed whole decibels −31 … +31 (3 kHz reference, ties to even), 0x20 = not measured |
| 3 | | base: the next sequence number the sender needs of the other's data |
| 4–5 | | bitmap: bit *i* set ⇔ `base + i` received — or, with bit 0 set, a **report** of the other station's last measurement: bit *i* + 1 set ⇔ its measurement frame *i* arrived, and nothing at or above `base` is acknowledged |
| 6 | 7–3 | rung recommended for the other station's data |
| 6 | 2–0 | n: in a header, the data frames granted, or the open window's size index; in an answer, the frames its sender has queued, at most 7 |

Bit 0 of a plain acknowledgement's bitmap is always clear, since `base` is the first sequence
number not received, so a report is never read as an acknowledgement.

A **measurement frame** (`MEASURE`, version 9) has the frame's index among its transmission's
measurement frames, 0–5, in the sequence number's byte and the session number in the session
byte, and is filled to the rung's full size with SplitMix64 drawn from the session and the
index: bytes that look like data, not a block of zeros. It takes no sequence number and is
never delivered or sent again.

A callsign is one to nine characters of `A–Z 0–9 - /`, coded 1–26 for A–Z, 27–36 for 0–9, 37
for `-` and 38 for `/`, padded with 0 to nine; the nine six-bit codes, first character most
significant, fill 54 bits, and two zero bits make seven bytes.

The SNR byte of the connect and probe bodies is the measurement rounded to the nearest
integer decibel and clamped to −40 … +40, with **ties rounded to even** (12.5 dB encodes as 12, 13.5 as 14). The tie rule
is stated because it is the kind of detail two implementations silently disagree on —
Python rounds halves to even and Rust rounds them away from zero — and a wire format that
two correct implementations encode differently is not a wire format.

### 7.2 Session

A session identifier is never zero: with kind DATA and sequence zero an all-zero body
would make an all-zero frame, and a receiver **discards any decoded block that is all
zeros** — the all-zero word is a codeword of every linear code and its CRC is zero, so a
decoder fed a false detection converges to it. Every other frame carries a non-zero kind.

**Granted slots** (link protocol 8, ADR-0069). The station whose call was accepted is the
**master** for the whole session; the called station is the **slave**. The master is the only
station that keys on its own initiative. Each master transmission is a `HEADER` and up to six
data frames, and it grants the slave one **slot**: an answer, an answer and *n* data frames, or,
when neither station has anything to send, an open window. The slave keys only in a slot granted
by a frame it decoded in time to start the slot on time. There is no turn, nothing to hand over
and no request to send: data flows both ways in every exchange, sized by the grants. Only
decoded content changes a station's state; a detection, an announcement or a silence moves no
deadline. The rules, numbered as in the formal specification
(`../research/link-architecture/04-formal-spec.md`, where they are proved):

1. **One initiator.** Only the master starts a transmission on its own. The slave transmits only
   in a slot (rule 2), with its acceptance (rule 11), with its `DISC_ACK` (rule 10) and, after
   the session, its closing identifier (rule 13).
2. **The slot.** After a master transmission whose last frame ends on the air at `E` and is of
   family `f`, at gap level `L`:

   ```
   start   = E + max(D_f + KEY, T_master) + GUARD + LEVEL[L]
   end_max = start + SLACK + grant
   grant   = answer + n × the slave's data frame,   or an open window
   ```

   `D_f`, `KEY`, `GUARD`, `SLACK` and `LEVEL` are the constants of §9; `T_master` is the
   turnaround the master declared in its connect body. A station places `E` from the header's
   count, or from any data frame's count (§3.2), whichever ends latest — believing a count only
   from a copy that decoded alone. The master reads the slave's frames the same way.
3. **The forfeit.** The slave keys for `start` and radiates no later than `start + SLACK`, or not
   at all: whatever held its answer — a late decode, a busy computer, a radio slow to key — it
   drops it rather than send it late.
4. **The master's next key.** After a decoded slave frame of the slot stated its end `S`, the
   master keys no sooner than `S + max(D_f, T_slave − KEY) + GUARD`; when it decoded nothing of
   the slot, no sooner than `end_max` plus the slowest answer family's decode budget and
   `GUARD`. Nothing else moves this deadline.
5. **Attribution by air time.** A slave frame belongs to the slot when it began after the
   master's last transmission ended. An older frame's content is kept (sequence numbers make
   that safe); its timing is ignored.
6. **What the slave may assume.** The slave takes the level, the grant and the answer's family
   only from the header of the transmission it answers. Without that header it answers alone,
   at level 0, in the family it last used, anchored on the data frames' exact counts, and with
   no anchor it does not transmit. The master's slot reserves for the slowest family the slave
   may hold.
7. **Header elision.** The master may leave its header out only when the transmission grants
   nothing beyond an answer, at level 0, after a slot it heard, with the answer's family the only
   one the slave may hold; when at most four data frames follow, one of them a copy at RV 0 (the
   only version that decodes alone on every rung, ADR-0073); when none of them goes above the
   rung the slave recommends, unless the header would be a floor frame (ADR-0076); and when no
   measurement is under way.
8. **Gap levels.** The slave asks in every answer for the level its own lateness needs, and for
   one that starts its slots no sooner than its answer gap (configured, or learned of the master)
   after the master's frames;
   it eases a level after six slots answered on time. The master grants the larger of what was
   asked and its silence level, which two silent slots in a row raise one step and four slots
   answered on time lower one. A call's or a probe's level is in its capability bits 3–4 and
   rises every second try.
9. **Rate.** Each station recommends the other's rung in every control frame, and the other
   follows it, capped by its own regulatory ceiling (below), its `max_mode` and a pinned rung. A
   data slot heard silent steps the master's own rung down two usable rungs; two silent slots
   step the slave's too and put headers and answers on the floor. Control frames go on the floor
   unless the other station's last three reports cleared the ordinary control frame's threshold
   by 3 dB, and while any of the last four answer slots went silent. A copy goes at the next RV
   in the order 0, 0, 2, 3, then round again (ADR-0043: the second copy is RV 0 so that it
   decodes on its own when the first was never detected; the receiver reads the RV from the
   chips and combines any order). A frame sent four times unacknowledged at a rung the
   recommendation has since left, or after two copies one the other station measured further
   under its rung than four copies can make up (ADR-0038), is re-encoded at the most robust
   usable rung below its own, in the recommendation's family and no faster than the
   recommendation, that carries its body (ADR-0085): a new codeword under the same sequence
   number, which the receiver does not combine with the old one. A body no such rung carries
   goes **in pieces** (ADR-0086) after one copy more — the receiver combines a fifth copy with the
   four it holds before it starts over — or after two when the report says combining cannot
   reach, and only when the rung the other station itself last recommended — not a step down
   for a silent slot — is below the frame's: at the usable rung of its family up to
   that recommendation that cuts the body into the fewest pieces, the most robust of those (every
   frame of a family is as long as the others, so the pieces are the air), each piece a full
   FRAGMENT frame (§7.1), in transmissions of pieces
   alone and always at RV 0, since the receiver gives a frame that does not decode a sequence
   number by its slot and a piece has none; a block sends each piece once before any again. A
   receiver that decodes a piece infers no sequence number for the transmission's other frames,
   nor for any while a block it holds pieces of is incomplete. A rung is offered only when no other carries as much data, after the
   DATA header, per second of air from a threshold no higher (ADR-0073).
10. **Ending.** The master leaves with a `DISC`, which grants the slave a slot for its
    `DISC_ACK`; it repeats it at most three times, then ends ("closed (no DISC_ACK)"). It sends
    its `DISC` once its queue is acknowledged; once the longer of two whole exchanges and 20 s
    has passed with nothing new acknowledged; or a minute after the operator asked — whichever
    comes first — and says what it left. The slave asks to leave with `flag` in an answer; the master then sends its `DISC` once
    what the slave had is delivered, without waiting for its own. A station that left a session
    without the other's word that it was over (an abort, "closed (no DISC_ACK)", "no response",
    a link timeout) remembers it for a link timeout and answers a header of it with a `DISC`, or
    a `DISC` of it with a `DISC_ACK`, in the frame's slot (ADR-0052); an aborting master says its
    `DISC` on the floor, granting the `DISC_ACK`'s slot, up to three times. A station's link
    timer is four exchanges of the size in use, 45 s at least; the master also ends once more than
    eight slots in a row have gone silent ("no response").
11. **Connection.** The `CONNECT_REQ` grants the acceptance's slot, at the level in its
    capability bits 3–4, and the called station accepts in that slot or not at all. The first try
    goes out on the tone floor and the tries alternate between the floor and the ordinary
    family's robust mode (ADR-0016); the level rises every second try, so that a slow called
    station is reached; and the next try waits until the acceptance's slot is over, plus a
    random part of a floor frame that grows with the tries, so that two stations calling each
    other fall apart. An ordinary call heard less than 12 dB above the ordinary control frame's
    threshold is accepted on the floor (ADR-0056). The caller is the master once it reads the
    acceptance: its first header goes in the family the acceptance's SNR asks for, its first
    data at the rung that SNR supports with two usable rungs in hand (§7.1), and its first grant
    is two frames, so that a gateway can greet. A call from the session's own peer under a new
    session number ends the session and is answered as from idle (ADR-0056). When two stations
    call each other at once, the one whose callsign sorts higher keeps calling and ignores the
    other's request, and the other answers it.
12. **Open windows.** When neither station has anything to send, the header grants an open
    window, 4 s after an ordinary header and 10 s after a floor one, doubled once while the link
    stays idle. The slave may start inside it when it has something to send and can finish
    inside it. An idle master that has heard nothing from the slave for 10 s grants an answer
    instead: a link that is only ever silent cannot tell idle from dead.
13. **The closing identifier** (below).
14. **Measurement** (link protocol 9, ADR-0077). A station measures a rung — the Test's ladder
    — with measurement frames (§7.1), as many as one transmission holds, in a transmission of
    their own, once none of its data is unacknowledged; no new data goes while one is asked for.
    Its control frames set `measure` until it reads the report, carry no data, and are never
    left out. The other station, learning of the measurement from `measure` or a measurement
    frame it decodes, drops every soft buffer it holds and keeps none for a measurement frame
    that did not decode, and reports in every acknowledgement it sends until data comes again.
    A measurement's slot heard silent steps no rung down, and one the keying gate dropped or a
    cut left unread is asked again.

A station whose rules admit only tone-floor rungs where it is (the regulatory ceiling,
ADR-0018) sends every frame on the floor: every try of a call, its answers to calls and
probes, and its control frames. The ceiling is the station's own: the rung it recommends to
the other station is not capped by it, and each station caps the recommendation it follows by
its own.

Data frames carry no callsign. A station that identifies in Morse (the reference daemon: a
1 500 Hz tone, at most 20 wpm under US rules; on for a US callsign whatever its setting) does
it once a session, at the end, and its identifier is the last thing it sends for the session
(ADR-0075). Nothing inside a session carries one, and neither do calls, acceptances, probes,
probe answers, beacons or datagrams. The connect request and its acceptance each state the
sender's identifier length, and every `DISC` grants the other station its `DISC_ACK` plus that
length:

* the station that answers a `DISC` sends its identifier right behind its `DISC_ACK`, inside that
  slot, and starts nothing of its own until the other's identifier can be over;
* the station that sent the `DISC` keys its own on its own once the slot is over;
* a session that ends without that exchange (a link timeout, a `DISC` never answered) is
  identified by each station on its own, after listening for a clear channel for up to 15 s;
* a session ended by the same station calling again is not identified: the communication goes
  on in the new one.

A contact that went no further than its setup (a call or a probe) is identified on its own,
30 s after (ADR-0055 §4).

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
| 3–4 | In a request (a call, a probe): the gap level of the slot its answer goes in (§7.2, rule 8). An answer carries 0 |
| 5–7 | Reserved, must be zero |

The bandwidth and level bits are not negotiated: the first state the air the sender runs, the
second the slot it grants. An OFDM frame's
air is a physical fact the receiver knows from having decoded it, but a tone-floor frame is the
same on either air, and a call, an answer or a probe on the floor says which air its sender
runs only here. A station ignores a request or an acceptance whose stated bandwidth is not its
own, and says so; a station that listens in more than one bandwidth answers in the one it was
called in. A probe is answered across bandwidths (§7.1). Both stations of a session use one
bandwidth for its whole life.

**Compression is applied to the payload byte stream, above the ARQ, not to individual
frames.** A frame carries 24 bytes on the slowest rung (tone-24), and a compressor with no
history makes a
block that size larger rather than smaller. The link layer already delivers bytes in order and
exactly once, which is precisely what a stream decompressor needs; selective repeat, HARQ and
retransmission all happen underneath and are invisible to it.

The sender flushes the coder (a deflate *sync flush*) at the end of each application write, so
nothing is left sitting in the compressor waiting for input that may never arrive. A receiver
therefore never has to wait for more data to decode what it already has.

### 7.4 Ordering rule

A station composes every transmission's data as **unacknowledged frames in ascending sequence
order, then new frames**, all of one family. This is required, not advisory: it is what allows
the receiver to infer the sequence number of a frame whose payload did not decode, and therefore
to soft-combine it with the copy that follows. A transmission's frames must be one length for
the receiver to place them by air time, so when the oldest unacknowledged frame is of the other
family than the rung new frames would go at, the transmission carries that family's
retransmissions alone and new frames wait for the next. A transmission carries at most six data
frames, and no more than fit in the transmitter's key time, one at least (ADR-0017).

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
| Link DATA header | 3 bytes (5 with an explicit length, 2 more for a piece) |
| Link CONTROL frame | 7 bytes |
| Callsign encoding | 6 bits/character, 9 characters in 7 bytes |
| Selective-repeat window | 16 frames |
<!-- END:constants -->

---

## 9. Timing

A session's timing is the slot of §7.2, and its constants are protocol constants, the same at
both ends:

| Constant | Value | What it covers |
|---|---|---|
| decode budget, ordinary and floor | 0.30 s, 0.30 s | a frame's end on the air to its decode reaching the link layer: the reference receiver reports within 0.157 s on the scenario harness and 0.161 s on the air |
| `KEY` | 0.35 s | a station's decision to its radiation: keying lead, sound card |
| `GUARD` | 0.05 s | propagation (under 20 ms one way on HF), clock error, rounding |
| `SLACK` | 0.35 s | how late a slot may start on the air and still be used |
| gap levels | 0, 0.5, 1.0, 2.0 s | extra gap a slow station asks for, or a master grants after silence |
| open window | 4 s ordinary, 10 s floor | doubled once while the link stays idle |

A station whose computer decodes or keys later than these allow does not change them; it asks
for a gap level. A receiver may announce a frame as arriving before it is decoded: an OFDM
preamble two symbols after it starts, a tone frame's first sync block 0.54 s after. Inside a
session an announcement changes nothing (rule 5); a caller or a prober waits past a frame
announced, which may be the answer.

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
| Goodput at +12 dB (16 kB sessions, lossy-pipe link bench, before the tone floor) | 1879 bps | 910 | 910 | 1140 |
| Goodput at +20 dB (the same) | 3399 bps | 2222 | 2144 | 2547 |

At 500 Hz (`bench/baselines/phy_fer_500.csv`), the same reference — the same transmitter
power into the same noise:

| | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| Tone floor, 36 bit/s (rung 0) | −19.0 dB | −11.7 | −13.2 | −14.5 |
| Tone floor, 54 bit/s (rung 1) | −17.3 | −8.8 | −10.3 | −10.9 |
| Middle kind, 76 bit/s (rung 2) | −14.3 | −7.8 | −10.2 | −10.8 |
| Middle kind, 112 bit/s (rung 3) | −13.0 | −3.6 | −6.2 | −7.2 |
| Middle kind, 139 bit/s (rung 4) | −11.1 | −0.7 | −2.2 | −3.8 |
| Control mode (QPSK ½) | −5.2 | +4.0 | +3.5 | +0.0 |
| 16-QAM ½ | −0.1 | +9.0 | +9.5 | +8.0 |
| Fastest narrow mode (64-QAM ⅚) | +10.4 | +21.0 | > +22 | > +23 |
| Best single-rung throughput at −10 dB | 139 bps | 67 | 70 | 76 |
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

These are simulator figures; the goodput rows come from `bench/baselines/link_throughput.csv`
(2026-09-16), measured before the tone floor and the calibrated fading pipe. The on-air
sessions logged so far (`field/LOG.md`) do not measure them, and none should be inferred from
them.

---

## 11. Open items for v1.0

* A call on the tone floor is heard by a station of either bandwidth; negotiating across
  the two is not specified.
* The wide (2.75 kHz) bandwidth variant.
* Formal test vectors published alongside this document; the audio vectors in `vectors/` (the
  2 300 Hz OFDM frames) and the cross-validation vectors under `core/*/tests/data/` (the
  500 Hz and tone-floor frames and the link formats as well) serve that purpose today but are
  not yet a normative part of the specification.
* On-air validation of everything in §10.
