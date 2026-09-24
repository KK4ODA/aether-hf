# ADR-0013: The tone floor — a steady-envelope FSK family under both ladders (P9-8)

**Status:** accepted, 2026-09-24. Model first (`aether_model/phy/tone.py`, the ladder in
`frame/modes.py`), then the port. Supersedes ADR-0009's OFDM floor family, which leaves the
500 Hz air.

## 1. Context

The weak-signal plan (`docs/ROADMAP.md`, "The weak-signal plan from 2026-09-23") put the
tone floor third, with a gate: it ships only if it beats the best existing frame at the same
bit rate by 3 dB or more at **equal peak power** on ITU Good and Moderate.

Below its ordinary modes an OFDM frame spends its energy on things a weak signal cannot
afford. A transmitter is driven to a fixed peak, and Aether's frames as transmitted peak
5.6–5.9 dB (PSK) and 7.4–7.5 dB (QAM) above their average (`peak_to_average.csv`): that
much of the transmitter's power never reaches the air. Pilots, the cyclic prefix and an
eight-symbol preamble take another third of the 500 Hz floor frame's resource elements, and
its channel estimates fail before its code does. The 2 300 Hz ladder had nothing below BPSK
⅕ at −5 dB; the 500 Hz OFDM floor (ADR-0009) reached −12 dB on AWGN and −3 to −4.5 on the
fading channels, its detector cost three times the receiver's CPU, and it decoded live only
from beta.49.

## 2. The design

One tone at a time, at a constant envelope, detected by energy — the classic weak-signal
design, from public sources: non-coherent orthogonal M-FSK with soft decisions (Proakis &
Salehi, *Digital Communications*), Costas-array synchronisation (J. P. Costas, *Proc. IEEE*,
1984; the same idea synchronises MIL-STD-188-141's link establishment and the published
FT4/FT8 design), and Aether's own CRC-24, TS 38.212 LDPC with its redundancy versions and
golden-ratio interleaver. `docs/spec/air-interface.md` §2.4 has every number.

* **Sixteen tones, 40 ms symbols, 25 Hz apart, 400 Hz wide**, four Gray-labelled coded
  bits a symbol. The frequency glides between tones on a raised cosine over 32 samples and
  the phase runs on: 99.9 % of the power inside ±250 Hz, −56 dB beyond ±500 Hz, at a loss
  under 0.01 dB. The same frames on both airs, centred on the passband.
* **Sent at the OFDM frames' peak.** A tone frame goes out 5.5 dB above an OFDM frame's
  average at the same transmit level, at or under every OFDM frame's peak, and every SNR it
  reports is taken back by 5.5 dB: the link layer compares both families in one currency,
  the OFDM frames' average power.
* **Three kinds.** A control frame (7 bytes, 80 symbols, 3.2 s, rate 0.36) and two data
  kinds on one 134-symbol frame (5.36 s): 24 bytes at rate 0.49 (36 bit/s, which carries a
  connect request) and 36 at rate 0.71 (54 bit/s).
* **Three sync blocks** of eight symbols — start, after 45 % of the data, end — whose
  pattern names the kind and the redundancy version: one pattern for the control frame, four
  per data kind. Each is a Costas sequence over sixteen tones, and no two share more than two
  symbols under any offset within a block and any shift of up to eight tones.
* **The detector** keeps a spectrogram at a quarter-symbol hop and a quarter-tone bin
  and, for every kind, redundancy version, start and ±100 Hz of offset, takes the mean over
  the 24 sync symbols of the sync tone's energy over the other fifteen tones' mean, each
  ratio clipped at 10. Threshold 3.0 (noise maximum 2.73 a minute; narrow OFDM traffic at
  30 dB 2.75; a carrier 30 dB up 2.72). A candidate is refined — over two hops and two bins
  either side, then to the sample and a fraction of a hertz, on the sync tones' own energy —
  and kept only if at least 12 of its 24 sync tones are the strongest in their symbols, four
  of them in a second block (§5); a silent symbol is no evidence either way.
* **The demodulator** measures the noise from the bins that should hold none, the signal
  from each sync block, interpolates the symbol SNR between the blocks, and scores each tone
  `log I0(2·sqrt(E·s)/σ)`; a bit's LLR is the log-sum over the tones labelled 0 against 1.
* **Streaming**: the spectrogram's ratio rows are kept by absolute hop, each computed once;
  each frame length is evaluated as soon as its own rows are in; a frame is taken a symbol
  after it ends (the largest statistic within a symbol either side, confirmed). A frame is
  *announced* once its first block is in, above 4.8 with five of eight tones strongest and
  the largest within three symbols: 0.54 s after it starts (`announce_delay_s`), which the
  link layer's `PhyTiming.floor_preamble_detect_s` carries.

## 3. The gate (`bench/baselines/tone_floor.csv`, 100 frames a point)

10 % FER through the detector, SNR in 3 kHz. Both families on one axis — the OFDM frames'
average power at a given transmit level — with the tone frames 5.5 dB above it, which is
what equal peak power means; the OFDM floor from `floor_500.csv` (20 frames a point):

| | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| **tone-24**, 36 bit/s | **−19.0** | **−11.7** | **−13.2** | **−14.5** |
| OFDM floor, QPSK 1/10, 36 bit/s | −13.0 | −4.0 | −4.7 | −3.0 |
| **tone-36**, 54 bit/s | **−17.3** | **−8.8** | **−10.3** | **−10.9** |
| OFDM floor, QPSK ⅕, 78 bit/s | −10.1 | −4.4 | −4.0 | −6.4 |
| **tone-control** | **−19.5** | **−11.1** | **−14.5** | **−15.8** |
| OFDM floor control frame | −11.2 | −4.0 | −4.7 | −6.8 |

At the same 36 bit/s the tone floor is 6.0 dB better on AWGN — 5.5 of it the envelope — and
7.7, 8.5 and 11.5 dB better on Good, Moderate and Poor, where its frames span several fades
and it needs no channel estimate: the gate (3 dB on Good and Moderate) passes with nearly five
to spare. The 54 bit/s kind reaches 4–7 dB below the OFDM floor's 78 bit/s mode. The detector
costs 0.1–0.3 dB against genie timing. The tone floor **replaces** the OFDM floor at 500 Hz
— lower everywhere, and a far cheaper detector than ADR-0009's — and goes under the
2 300 Hz ladder, which reaches 14 dB lower than it did.

## 4. The ladder

What the link layer, the rate controller and the operator call "mode N" is a **rung** of the
air's ladder (`AirInterface.ladder`): the tone floor's two data kinds, then the air's OFDM
modes. The OFDM frames are unchanged bit for bit — their chips still carry the OFDM mode
index — so the 2 300 Hz OFDM mode *m* is rung *m* + 2 and the 500 Hz ladder skips OFDM modes 0
and 1, the retired floor. The rate tables, the fading pipe's shapes and keys, the
calibration (keyed by what a frame *is*: `mode N` for OFDM mode N, the kind's name for a
tone frame) and every tool read the ladder. Consequences found on the way:

1. **The link protocol is version 2.** A mode number means another frame on the wide air
   than it did, so a station ignores a call or an acceptance of another version and says so
   (`PROTOCOL_VERSION`); a session between versions would have run on numbers that mean
   different frames at either end. Sidecars go to `aether-hf-session/2`; the field tools map
   a `/1` sidecar's OFDM mode numbers onto the ladder (`field_ingest.sidecar_rung`).
2. **`max_mode` defaults to 15**, the top of the wide ladder; a daemon configuration
   migrates a wide station's stored value by two.
3. **A turn starts where the station's own measurements put it.** A station taking the
   channel started at `initial_mode`, which is now the tone floor; it now starts at
   `first_mode` of the SNR it has been measuring the peer at — HF is reciprocal — as a caller
   starts at the acceptance's.
4. **The ISS waits out the IRS's quiet for the burst it sent.** It sized its wait for the ACK
   with the IRS's own formula, applied to what it had last *heard* — an ordinary acceptance
   before a session's first floor burst — and under-waited by the difference: 0.4 s, the
   whole ACK margin, with preamble reports; a whole tone frame without, where a session
   pinned to the floor at 16 dB died of ACK timeouts. A latent bug of ADR-0009's floor that
   its shorter frames hid.
5. **The rate controller crosses the floor boundary by what the rungs are worth.** The
   margin and the hysteresis were tuned for neighbours a third apart in rate; the first OFDM
   rung is four times the tone floor's. On the fading bench the plain rules left 2 300 Hz
   sessions on the floor from −2 to +2 dB at half their old rate, because `first_mode`
   stepped back two rungs into the floor and a fading channel's learned margin kept the link
   there. Now `first_mode` keeps its steps in hand inside the family the measurement fits;
   on an air that says so (`PhyTiming.floor_margin_db`, 1 dB on the wide air) the first OFDM
   rung is held to its 10 % point plus at most that margin, however wide the learned one,
   and one failed burst there does not drop the link to the floor — two in a row do. The
   wide air's first rung spreads a frame over 2.3 kHz and, with HARQ, stayed productive a
   decibel above its 10 % point on every class before the floor existed; the narrow air's
   has a fifth of that diversity, and the learned margin decides there as between any two
   rungs. Rejected on the bench: a fixed "floor exit" discount of 1–3 dB (it pinned the
   narrow air to a rung on its steep AWGN waterfall: −6 dB fell from 44 to 10 bit/s), the
   cap with no exception (Good 0 dB 69 bit/s), and a cap that ignored any number of failures
   (Moderate −2 dB 26/30 sessions: the silence back-off and the capped recommendation took
   turns).

   Sessions on the fading pipe (`bench/baselines/link_tone_floor.csv`; 30 a point, 2 kB at
   2 300 Hz, 1 kB at 500 Hz; completed, then median bit/s):

   | 2 300 Hz | −14 dB | −10 | −6 | −4 | −2 | 0 | +2 | +6 |
   |---|---|---|---|---|---|---|---|---|
   | AWGN, beta.50 | 0 | 0 | 1, 50 | 30, 144 | 30, 144 | 30, 196 | 30, 312 | 30, 638 |
   | AWGN, tone floor | 30, 35 | 30, 39 | 30, 39 | 30, 41 | 30, 144 | 30, 196 | 30, 312 | 30, 638 |
   | Good, beta.50 | 0 | 0 | 4, 44 | 25, 64 | 29, 89 | 30, 110 | 30, 136 | 30, 362 |
   | Good, tone floor | 18, 17 | 29, 25 | 30, 36 | 30, 39 | 30, 46 | 29, 90 | 30, 131 | 30, 362 |
   | Moderate, beta.50 | 0 | 0 | 3, 37 | 22, 61 | 30, 88 | 30, 114 | 30, 147 | 30, 408 |
   | Moderate, tone floor | 30, 21 | 30, 30 | 30, 39 | 30, 40 | 30, 52 | 30, 110 | 30, 146 | 30, 408 |
   | Poor, beta.50 | 0 | 0 | 0 | 18, 56 | 30, 89 | 30, 118 | 30, 167 | 30, 478 |
   | Poor, tone floor | 30, 24 | 30, 34 | 30, 39 | 30, 39 | 29, 55 | 30, 113 | 30, 167 | 30, 478 |

   | 500 Hz | −14 dB | −10 | −6 | −4 | −2 | 0 | +2 | +6 |
   |---|---|---|---|---|---|---|---|---|
   | AWGN, beta.50 | 0 | 30, 25 | 30, 44 | 30, 54 | 30, 73 | 30, 112 | 30, 178 | 30, 276 |
   | AWGN, tone floor | 30, 28 | 30, 35 | 30, 35 | 30, 38 | 30, 64 | 30, 147 | 30, 182 | 30, 284 |
   | Good, beta.50 | 0 | 0 | 24, 21 | 30, 28 | 30, 37 | 30, 53 | 30, 71 | 30, 197 |
   | Good, tone floor | 21, 17 | 29, 25 | 29, 35 | 30, 37 | 30, 38 | 29, 42 | 30, 67 | 30, 204 |
   | Moderate, beta.50 | 0 | 9, 18 | 30, 33 | 30, 49 | 30, 48 | 30, 50 | 30, 60 | 30, 166 |
   | Moderate, tone floor | 30, 20 | 30, 29 | 30, 35 | 30, 35 | 30, 36 | 30, 38 | 30, 45 | 30, 164 |
   | Poor, beta.50 | 0 | 24, 20 | 30, 44 | 30, 49 | 30, 47 | 30, 54 | 30, 75 | 30, 189 |
   | Poor, tone floor | 30, 24 | 30, 33 | 30, 35 | 30, 35 | 30, 35 | 30, 41 | 30, 63 | 30, 199 |

   At 2 300 Hz everything from −6 dB down is new ground — sessions that failed complete — and
   from 0 dB up nothing moved; −4 dB is more reliable and slower on AWGN (the floor where BPSK
   ⅕ used to carry it), −2 dB on the fading classes about half as fast. At 500 Hz the gain
   is below −6 dB; from −4 to +2 dB on Moderate and Poor the link runs a quarter slower: the
   OFDM floor's 78 bit/s mode is gone and its place is the tone floor's 54. (Also found: the
   narrow table's rung 2 says −6.0 dB where `phy_fer_500.csv` crosses at −7.0 with 30
   frames; left as it was.)

## 5. Found on the way

* **A pattern bound at zero offset is not enough.** Every block of a frame repeats its
  pattern at the same places, so two patterns sharing three symbols three symbols apart put
  nine of 24 sync symbols on the other kind read three symbols early, and the data's
  coincidences supplied the rest; the patterns are now bounded at every offset within a
  block (at most two symbols; one at zero offset left only eight patterns).
* **An even split repeats the frame under a shift.** With the data halved, a frame read one
  block-spacing early had its middle and end blocks on the true first and middle — 16 of 24
  — and it finished first. Splitting at 45 % makes the three block distances differ.
* **Energy over the bin's median noise is fooled twice**: by a wideband burst (every bin up
  at once) and by strong narrowband QRM (an FT8-like signal 30 dB up scored 8.4). The
  per-symbol ratio to the other fifteen tones fixes the first; clipping and the hit test the
  second.
* **The clipped statistic plateaus on a strong frame** over two hops and two bins, so the
  refinement starts from a grid, and it finishes to the sample: a strong frame read a few
  samples off smears the neighbouring symbol over every bin and the SNR estimate with it.

* **A frame read a block-spacing early** — found by two daemons over `[sim]` after the port
  (a Test session: tone-36 decoded 0 of 2, and the rest of the rung went to pieces), and
  fixed model first. A station mutes its receiver while it transmits and the peer's burst
  follows at once, so the receiver holds exact silence and then a frame. The hypothesis whose
  first block lies in the silence and whose middle block sits on the frame's first had eight
  hits there, one from the silence — at zero every tone ties, and the argmax is tone 0, which
  that pattern holds — and three from the data's coincidences: twelve, the bar. A stream
  takes a frame as it ends, so the phantom was taken first and its span blocked the real
  frame; the real frame left the arrivals with it, a pseudo-arrival at its middle block took
  its place, and the receiving station's acknowledgement fired inside the next frame. Now a
  silent symbol is no evidence — neither a hit nor noise: a frame half under the station's
  own transmission had read its noise as zero and its SNR as 290 dB — and a frame needs
  `MIN_BLOCK_HITS` = 4 in a second block: evidence in two of three blocks, which is what
  three blocks are for. Without it a phantom ahead of any burst passed about 1.8 % of the
  time, with it about 0.16 %; it costs a few acquisitions a hundred at the lowest SNRs, nearly
  all of frames that would not have decoded. The vectors carry the case
  (`tone_after_silence`).

## 6. Costs and limits

* **Duty cycle.** A tone frame holds the transmitter at its peak power for 5.4 s, a burst of
  them for half a minute. Rigs rated for FT8 at full power take it; an operator of one that
  is not lowers the drive for both families alike.
* **Rate.** 36 and 54 bit/s; the gap to the first OFDM rung (197 bit/s at −5 dB wide, 114
  at −6 narrow) is P9-9's to measure. The OFDM floor's 78 bit/s rung at −10 dB (−4.4 at equal
  peak) is gone; the 54 bit/s tone kind reaches seven decibels lower.
* **The transition.** −4 to −2 dB at 2 300 Hz and −4 to +2 dB at 500 Hz on the fading
  classes run slower than before (§4.5): the price of a floor that holds, until rungs
  between the floor and the full-width modes (P9-9) are measured.
* **A call on the floor is heard by a station of either bandwidth** — the frames are the
  same — but the handshake still requires both to run one; negotiating across is not built.
* Not measured on the air.

## 7. The port

`aether-phy` has `tone.rs` — the codec, the modulator, the detector and `ToneStream`, the
numerology, sync patterns and kinds compiled in from the model's export — and the ladder in
`modes.rs` (`Rung`, `AirInterface::ladder`, `rung_of`, `control_rung`). Against the model's
vectors the tones are exact, and the waveform, the detector's start, offset and statistic,
the SNR and the soft bits agree to the vector file's tolerances (`tone_frames`,
`tone_receive`). A decoded frame is either family (`Received::Ofdm` / `Received::Tone`), the
streaming receiver runs the tone stream on the same band-limited buffer and keeps the tone
floor's longest frame whatever buffer it is asked for, and a tone frame on its way is
announced (`PendingFrame { tone }`) once its first sync block has won. `aether-link` has the
timing fields, the ladder's tables and the boundary rules; the daemon's `phy_timing` is the
harness's. The configuration file's schema is 2: its first migration moves a wide station's
`max_mode` two rungs up, and profiles go through it.

Found in the port: the daemon sent beacons at the control mode's *OFDM* index, which on the
wide ladder is rung 0 — a tone frame. Beacons go out at `control_rung()`.

## 8. Rejected

Eight tones at 32 ms (0.4 dB worse at every rate, measured); four tones at 16 ms (1.5 dB
worse); per-bin median normalisation; the energy statistic; patterns bounded at zero offset
only; an even split; keeping the OFDM floor beside the tone floor at 500 Hz (three detectors,
and nothing the tone floor does not do better at equal peak).
