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
This version specifies the **WIDE_2300** bandwidth. Narrower and wider variants reuse
everything here with a different carrier count.

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
| Mode/RV chips | 168 | 4 x 14 sequences |
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

The transmitter clips and re-filters each finished burst to a target peak-to-average ratio
(§ constants), iterating four times with the same band-limiting filter the receiver uses.
This is transmitter-side only: a receiver needs no knowledge of it, and a transmitter may
omit it at a cost of roughly 1 dB of delivered power. Constant-modulus modes take the more
aggressive target; the QAM modes carry information in amplitude and take the gentler one.
See `../adr/0004-papr-reduction.md`.

---

## 3. Frame structure

A frame is a two-symbol preamble followed by data symbols. Every 8th data symbol, starting
with the first, is a **full pilot symbol** in which all carriers are known to the receiver.

<!-- BEGIN:layouts -->
| Layout | Symbols | Duration | Samples (8 kHz) | Full pilot symbols | QAM slots |
|---|---|---|---|---|---|
| LONG | 2 + 32 = 34 | 1054 ms | 8432 | 0, 8, 16, 24 | 1176 |
| SHORT | 2 + 12 = 14 | 434 ms | 3472 | 0, 8 | 420 |
<!-- END:layouts -->

LONG carries user data and the connection handshake. SHORT carries acknowledgements and
other control frames, always at the most robust mode.

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

## 4. Modes

<!-- BEGIN:modes -->
| Mode | Name | bits/sym | Rate | Base graph | Z | K' | E | Payload B | Net bps | AWGN dB |
|---|---|---|---|---|---|---|---|---|---|---|
| 0 | BPSK-1/5 | 1 | 1/5 | BG2 | 30 | 232 | 1176 | 26 | 197 | -5.1 |
| 1 | BPSK-1/3 | 1 | 1/3 | BG2 | 52 | 392 | 1176 | 46 | 349 | -3.2 |
| 2 | BPSK-1/2 | 1 | 1/2 | BG2 | 72 | 584 | 1176 | 70 | 531 | -1.8 |
| 3 | QPSK-1/3 | 2 | 1/3 | BG2 | 80 | 784 | 2352 | 95 | 721 | -0.4 |
| 4 | QPSK-1/2 | 2 | 1/2 | BG2 | 120 | 1176 | 2352 | 144 | 1093 | +1.4 |
| 5 | QPSK-2/3 | 2 | 2/3 | BG2 | 160 | 1568 | 2352 | 193 | 1465 | +2.9 |
| 6 | PSK8-1/2 | 3 | 1/2 | BG2 | 176 | 1760 | 3528 | 217 | 1647 | +4.7 |
| 7 | PSK8-2/3 | 3 | 2/3 | BG2 | 240 | 2352 | 3528 | 291 | 2209 | +6.9 |
| 8 | QAM16-1/2 | 4 | 1/2 | BG2 | 240 | 2352 | 4704 | 291 | 2209 | +6.0 |
| 9 | QAM16-2/3 | 4 | 2/3 | BG2 | 320 | 3136 | 4704 | 389 | 2953 | +8.9 |
| 10 | QAM16-3/4 | 4 | 3/4 | BG1 | 176 | 3528 | 4704 | 438 | 3324 | +9.9 |
| 11 | QAM64-2/3 | 6 | 2/3 | BG1 | 224 | 4704 | 7056 | 585 | 4440 | +13.9 |
| 12 | QAM64-3/4 | 6 | 3/4 | BG1 | 256 | 5288 | 7056 | 658 | 4994 | +15.6 |
| 13 | QAM64-5/6 | 6 | 5/6 | BG1 | 288 | 5880 | 7056 | 732 | 5556 | +16.9 |
<!-- END:modes -->

`K'` is the information block including CRC, `E` the coded bits after rate matching, and the
AWGN column the measured SNR (3 kHz) for 10 % frame error rate — every entry measured, not
interpolated (`bench/baselines/phy_fer_awgn14.csv`).

Modes are ordered from most robust to fastest. A mode that another mode beats on *both*
payload and threshold is never selected by the rate controller; mode 7 is in that position.

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

Kinds: `DATA`, `CONNECT_REQ`, `CONNECT_ACK` (callsigns do not fit in a control frame).

CONTROL container:

| Offset | Field |
|---|---|
| 0 | kind (4 bits) \| flags (4 bits) |
| 1 | session id |
| 2 | base sequence number — the next one the receiver needs |
| 3–4 | bitmap: bit *i* set ⇔ `base + i` received |
| 5 | measured SNR, signed dB, 3 kHz reference; 0x7F = unknown |
| 6 | recommended mode (4 bits) \| counter (4 bits) |

Kinds: `ACK`, `POLL`, `TURN`, `DISC`, `DISC_ACK`.

The SNR byte is the measurement rounded to the nearest integer decibel and clamped to
−40 … +40, with **ties rounded to even** (12.5 dB encodes as 12, 13.5 as 14). The tie rule
is stated because it is the kind of detail two implementations silently disagree on —
Python rounds halves to even and Rust rounds them away from zero — and a wire format that
two correct implementations encode differently is not a wire format.

### 7.2 Session

One station is the information sending station (ISS), the other the information receiving
station (IRS). The ISS sends a burst of data frames; the IRS answers each burst with one ACK
carrying the selective-repeat bitmap, the SNR it measured, and the mode it recommends. The
ISS retransmits what the bitmap reports missing — same codeword, next redundancy version —
and fills the remainder of the burst with new frames at the recommended mode.

`TURN` hands the sending role to the peer, and is sent when the peer has set `WANT_TX` or
`BREAK` in an ACK. `POLL` keeps an idle link alive. `DISC`/`DISC_ACK` close it. Connection is
a two-way handshake in DATA-container frames carrying both callsigns, with randomised
backoff so two stations calling each other simultaneously desynchronise instead of colliding
on every retry.

### 7.3 Capability negotiation

The connect request and its acceptance each carry a one-byte capability field. A capability is
used only if **both** stations offered it: a station that has not said it can do something
cannot be assumed to, and the only safe reading of a missing bit is that the feature is
unavailable. Unknown bits are ignored, so a later version can add one without breaking an
earlier one.

| Bit | Meaning |
|---|---|
| 0 | Stream compression: deflate, RFC 1951 |
| 1–7 | Reserved, must be zero |

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
| Most robust mode | −5.1 dB | +2.0 | +1.2 | −0.3 |
| QPSK 1/2 | +1.4 | +8.7 | +8.5 | +6.0 |
| Goodput at +12 dB | 1696 bps | 859 | 819 | 1034 |
| Goodput at +20 dB | 2106 bps | 1696 | 1565 | 1745 |

These are simulator figures. No on-air measurements exist yet, and none should be inferred.

---

## 11. Open items for v1.0

* A spreading or repetition mode below the current −5 dB floor.
* Narrow (500 Hz) and wide (2.75 kHz) bandwidth variants.
* Compression negotiation, CW identification, beacon and ping datagrams.
* Formal test vectors published alongside this document; the reference vectors in `vectors/`
  serve that purpose today but are not yet a normative part of the specification.
* On-air validation of everything in §10.
