# Aether HF — Technical Audit

**Date:** 2026-09-13  **Commit audited:** `6477100` (master, 4 commits, 3 863 lines of Python)
**Method:** every file read in full; both test entry points executed; targeted probes
(`tools/audit_probe_fec_mapping.py`, `tools/audit_probe_phy_audio.py`) written to verify
suspected defects numerically. All numbers below are reproducible with those two scripts.

> Paths in this document refer to the layout at commit `6477100`. Phase 0 moved the package to
> `model/aether_model/`, deleted `fec/ldpc.py`, `dsp/channel.py` and both old test files, and
> encoded every finding below as a strict `xfail` test in `model/tests/test_legacy_*.py`
> (marker `audit`). A finding is closed by removing its `xfail` marker in the PR that fixes it.

---

## 1. Verdict

The repository is a **design sketch expressed as Python**, not a modem. Every layer of the
signal chain has at least one defect that makes it non-functional on a real channel, and no
two layers are connected end-to-end (bits never travel `FEC → interleaver → OFDM → audio →
OFDM → LLR → FEC`). The "7/7 conformance tests pass" result in the last commit was obtained
by relaxing acceptance thresholds (FER < 0.10 → < 0.95; FER < 0.50 → < 0.70; "HARQ
improving" defined as *non-increasing*, satisfied by 100 % FER at every round) and by
asserting only that a detector fired, not that it was correct.

What *is* salvageable is the **design intent** (OFDM + pilots + LDPC + HARQ-IR + selective
ARQ + VARA-compatible host API), the **module decomposition**, the **channel simulator's
skeleton**, the **mode-table data structure**, and the **constellation code** after fixes.
Nothing in the current code should be trusted as a performance baseline; the README's
"advantages over VARA" table is unimplemented marketing and should be removed until measured.

---

## 2. Findings by module

Status key: ✅ works · 🟡 partial · ❌ broken · ⬜ skeleton (no behaviour)

| Module | Lines | Status | Key defects (evidence) | Disposition |
|---|---|---|---|---|
| `constants.py` | 118 | 🟡 | Stream-of-consciousness comments ("wait… that's wrong"). 64 carriers × 46.875 Hz = **3 000 Hz**, not the 2 300 Hz claimed. `PILOT_TIME_SPACING`, DSSS, guard constants unused anywhere. | Regenerate from the PHY design; keep as single source of truth. |
| `speed_levels.py` | 104 | 🟡 | Rates are not derivable from waveform parameters: Level 7 (QPSK, R=½, 52 carriers, 40.8 baud) is 2 121 bps raw vs 850 listed; Level 20 is 14.1 kbps raw vs 5 700 listed. FSK levels 1–3 and 32/128/256-QAM have no modulator. Min-SNR column never measured. | Keep the `SpeedLevel` dataclass; regenerate values from simulation. |
| `dsp/ofdm.py` | 275 | ❌ | Raised-cosine window applied to CP+symbol **without overlap-add** → tapers the last 20 samples of the useful symbol → **18.7 % EVM with no channel (SNR ceiling 14.5 dB)**. This alone is why the loopback test was limited to BPSK/QPSK. Pilot grid: 12 pilots at positions 0,5,…,55 of 64 → **8 edge data carriers are extrapolated**. No time-domain pilot pattern (`symbol_index` ignored). No timing/CFO/SRO/phase tracking at all. Per-element Python loops. | Rewrite (windowed OLA or plain CP + TX filter; pilots on both edges; scattered pilot grid; streaming demodulator with tracking loops). |
| `dsp/modulation.py` | 196 | 🟡 | QAM Gray labeling is wrong: 1-D Gray code applied to a 2-D grid. 16-QAM: **12 of 24 nearest-neighbour pairs differ in >1 bit**; 64-QAM 56/112; 32-QAM 36/52; 128-QAM 172/232 (128-QAM constellation is "sorted by magnitude, take 128" — not a cross constellation). Soft demapper is O(M·bps) pure-Python per symbol (256-QAM: 2 048 inner iterations per symbol). LLR sign convention is fine. | Fix labeling (independent Gray on I and Q), vectorize max-log demapper, drop 32/128/256-QAM from v1. |
| `dsp/preamble.py` | 194 | ❌ | Part A places ZC in FFT bins 0–127 → **6 kHz wide**; Parts B/C fill all 256 bins → **12 kHz wide**; only ~25 % of preamble energy falls inside ±1.5 kHz. Detector search step is 73 samples but CP is 38 → timing error can exceed the CP even on a perfect detection. ZC time/frequency ambiguity: with **zero noise and zero CFO** the detector returns offset 876 (true 500) and CFO −150 Hz. `estimate_fine_cfo` multiplies views of the caller's buffer in place. Fine-CFO unambiguous range is ±20 Hz. Parts B and C are never used by any receiver code. **Found during Phase 0:** `zadoff_chu()` uses `n(n+1)/N` for even N and `n(n+2)/(N+1)` for odd N — neither is a Zadoff–Chu sequence (off-peak periodic autocorrelation up to 0.32 of the peak instead of 0). | Redesign (band-limited preamble; Schmidl–Cox + unique word; fine timing by cross-correlation; integer CFO by FD correlation). |
| `dsp/wiener.py` | 156 | 🟡 | Frequency-direction MMSE only, with a `sinc` correlation model; the "time-domain Wiener" is an EMA with a heuristic α. Never validated against LS interpolation. | Replace with LS + 2-D interpolation first; revisit MMSE once a pilot grid exists. |
| `dsp/tx_filter.py` | 87 | 🟡 | Group-delay compensation zero-fills → **last 63 samples of every transmission are dropped**. | Fold into a proper TX chain (windowed OLA + polyphase resampler). |
| `dsp/noise_blanker.py` | 172 | 🟡 | Operates per OFDM symbol on a 294-sample block whose FFT grid does not align with the subcarrier grid; erasure mask is computed but no decoder consumes it. | Keep the three-layer idea; re-implement on the streaming receiver. |
| `dsp/channel.py` | 247 | 🟡 (best module) | Fading regenerated per `process()` call (no continuity across blocks; `_fading_state` unused). Per-block power normalization **removes fade depth** (a 294-sample block has ≈1 Doppler-filter bin). Doppler spread used as σ instead of 2σ. Profiles don't match ITU-R F.1487 mid-latitude values (Good should be 0.5 ms / 0.1 Hz, Moderate 1 ms / 0.5 Hz, Poor 2 ms / 1 Hz, Flutter 0.5 ms / 10 Hz). **SNR referenced to 12 kHz noise bandwidth → 6 dB off the 3 kHz convention** (the tests' "0 dB" is +6 dB in 3 kHz). No CFO, SRO, adjacent-channel or clipping impairments. | Fix and keep — this becomes the benchmark core. |
| `fec/ldpc.py` | 221 | ❌ | Random column-weight-3 H. `_build_generator` runs Gaussian elimination then discards the result. Parity solved by **real-valued least squares and rounding** → **20/20 invalid codewords**. Pure-Python BP. | Delete. |
| `fec/ldpc_5gnr.py` | 382 | ❌ | Base graphs are hand-invented 24-column matrices with shifts 0–7, not TS 38.212 BG1/BG2. Encoder's forward substitution ignores the super-diagonal → **20/20 invalid codewords** (≈50 % of checks unsatisfied). At R=⅓ **40 % of check nodes have degree ≤ 2** (parity chains carrying no information). Decoder given noiseless LLRs of the encoder's own output: **fails, 40 iterations, wrong info bits**. Dict-keyed BP: 0.8 s per 288-bit block. HARQ test: **FER = 1.00 at every round**. | Delete; implement real 3GPP TS 38.212 BG2 with rate matching (or IEEE 802.11n codes). |
| `fec/interleaver.py` | 86 | ❌ | "Time interleaver" reshapes to (depth, slot) and permutes **rows** → contiguous 512-bit blocks stay contiguous: zero burst-spreading. `FrequencyInterleaver` is constructed but never applied. "Three-stage" claim is false. | Rewrite (row/column block interleaver across the whole frame, then BICM bit interleaver). |
| `protocol/session.py` | 275 | ⬜ | Frames are Python dicts; no binary format, CRC, serialization, timers, retransmission, window bookkeeping (`ARQ_WINDOW_SIZE` unused), ACK generation, ISS/IRS turnaround or timeouts. `DISCONNECTING` never returns to `IDLE`. `_session_key`/nonce unused. | Redesign; keep state names and the metrics/config dataclasses. |
| `host/vara_compat.py` | 236 | ⬜ | Implements 8 of the ~30 commands in the public VARA command set. `CONNECT` parses one argument; VARA clients send `CONNECT <src> <dst> [via]`. No `CONNECTED/DISCONNECTED/PTT/BUFFER/BUSY/IAMALIVE/PENDING` events. Not wired to a session or modem. | Rewrite as an adapter over a native control API, driven by the public VARA command documentation; verify with Pat. |
| `audio/soundcard.py` | 168 | ❌ | `_upsample` takes `real()` of a **DC-centred complex baseband** → the ±k subcarriers fold onto each other: **30/104 bit errors in a noiseless loopback**. `_downsample` decimates real audio without mixing to baseband. Per-block peak normalization = TX AGC. Output truncated when a block is longer than the callback frame. No device enumeration, no sample-rate negotiation. | Rewrite (passband at a configurable centre, 48 k ↔ 8 k polyphase, ring-buffered streaming, WAV file backend). |
| `tests/test_ofdm_loopback.py` | 149 | 🟡 | Prints results; asserts only zero-noise QPSK and `detected == True`. Reported detection offset 876 (expected 500) and CFO −125 Hz — wrong, but passes. | Convert to strict pytest. |
| `tests/test_conformance.py` | 631 | ❌ (misleading) | Thresholds relaxed until green (see §1). Uncoded tests labelled "conformance". 150 s runtime. Not pytest-discoverable. | Replace with a strict suite + separate benchmark runner. |
| README | 144 | ❌ (misleading) | "-6 dB threshold", "172 ms ACK", "coherent on all levels", "HARQ-IR" — none implemented or measured. References a "v2.0 Word document" spec that is not in the repository. | Rewrite honestly; move the spec into `docs/spec/` as Markdown. |

### Missing entirely

Frame format · CRC · end-to-end bit pipeline · ACK/control frames · symbol timing recovery ·
sample-rate-offset tracking · phase tracking · passband conversion · PTT (any method) ·
CAT/rig control · GUI · configuration · logging setup · CLI/entry point · `pyproject.toml` /
requirements / `LICENSE` (README claims MIT/Apache dual) · CI · packaging · installer ·
updater · protocol specification · developer docs.

---

## 3. Test-suite assessment

| Test | What it claims | What it actually checks | Result at HEAD |
|---|---|---|---|
| 1 Loopback | "all speed levels, BER = 0" | BPSK and QPSK only, one symbol at a time, no channel | pass (trivially) |
| 2 AWGN | "each level at min SNR, FER < 5 %" | 3 uncoded cases at SNRs 6–14 dB (12 kHz ref) | pass |
| 3 ITU | "each level at min SNR + 3 dB, FER < 10 %" | BPSK at 33 dB on "Good"; **FER = 0.90**, threshold 0.95 | pass |
| 4 HARQ-IR | "FER < 1 % after 3 rounds" | **FER = 1.00 every round**; passes because non-increasing | pass |
| 5 CFO | "acquisition at ±200 Hz" | `detected` only; CFO errors 150–425 Hz | pass |
| 6 Mask | "-30/-50/-60 dBc" | double-precision FIR on an ideal signal: −134 dBc | pass |
| 7 Impulsive | "FER < 20 %" | **FER = 0.66**, threshold 0.70 | pass |

Conclusion: the suite provides **no evidence** about modem performance. It should be replaced,
not extended.

---

## 4. Calibration issues that would distort every future measurement

1. **SNR reference bandwidth.** Use 3 kHz (MIL-STD-188-110, codec2 convention). Current
   simulator is 6.0 dB off.
2. **Doppler spread definition.** F.1487 specifies the 2σ width of the Gaussian Doppler
   spectrum; the simulator uses the value as σ (fading twice as fast as intended).
3. **Fading generation.** Must be generated as a continuous low-rate process (e.g. 100 Hz)
   and interpolated, with state carried across blocks and **no per-block normalization**.
4. **Uncoded results labelled "conformance".** FER of an uncoded 52-bit block over a
   two-ray Rayleigh channel tells nothing about the coded link.

---

## 5. Salvage list

| Keep (after fixes) | Rewrite from scratch | Delete |
|---|---|---|
| `dsp/channel.py` (structure, profiles table) | `dsp/ofdm.py`, `dsp/preamble.py` | `fec/ldpc.py` |
| `speed_levels.py` (dataclass, table shape) | `fec/ldpc_5gnr.py` → real TS 38.212 BG2 | contradictory comments in `constants.py` |
| `dsp/modulation.py` (constellations, LLR convention) | `fec/interleaver.py` | README performance claims |
| `protocol/session.py` (state names, metrics/config dataclasses) | `audio/soundcard.py` | `test_conformance.py` thresholds |
| `dsp/noise_blanker.py` (concept) | `host/vara_compat.py` | |
| `dsp/wiener.py` (later, once pilot grid exists) | tests | |

See `ROADMAP.md` for the plan that follows from these findings.
