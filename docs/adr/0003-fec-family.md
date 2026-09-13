# ADR-0003: FEC = 3GPP TS 38.212 LDPC base graph 2 with circular-buffer rate matching

**Status:** accepted (2026-09-13) · **Roadmap:** P1-1 · **Replaces:** `fec/ldpc.py`
(deleted), `fec/ldpc_5gnr.py` (to be deleted when P1-1 lands)

## Context

The legacy "5G NR-inspired" code used hand-invented 24-column base graphs with shifts 0–7,
an encoder that produced invalid codewords 100 % of the time, degree-2 parity chains at low
rates, and a dict-keyed decoder at ~0.8 s per 288-bit block (`docs/AUDIT.md` §2). The
project needs one FEC family that covers rates from ~1/5 (low-SNR modes) to ~5/6 (64-QAM),
block lengths of a few hundred to a few thousand bits (frames of 1–10 s at HF rates), and
**incremental-redundancy HARQ**, which the ARQ design relies on.

## Decision

Implement the **3GPP TS 38.212 §5.3.2 LDPC code, base graph 2 (BG2)**, with the standard's
lifting sizes (Z ∈ {2 … 384}, 8 sets), **§5.4.2 circular-buffer rate matching** with
redundancy versions RV0–RV3, and the standard's **CRC-24** transport-block attachment for
decode verification. Encoding uses the double-diagonal structure (back-substitution) exactly
as in the standard; decoding is layered normalised min-sum (numpy in the model, Rust in the
core).

Why BG2 specifically: it targets K ≤ 3 840 and rates down to 1/5 — precisely the HF regime —
and its rate-compatible structure gives HARQ-IR for free: a NAK'd frame is retransmitted as
RV1/RV2 bits that were punctured in RV0 rather than as a copy.

Validation: bit-exact comparison of encoder output and rate-matched bit sequences against
an independent open implementation (NVIDIA Sionna's `LDPC5GEncoder`, Apache-2.0) for a grid
of (K, rate, RV); BLER-vs-E_s/N_0 curves committed as baselines.

## Alternatives considered

| Option | Why not |
|---|---|
| IEEE 802.11n/ac LDPC (n = 648/1296/1944, R = ½…⅚) | Excellent, simple, public — but no rates below ½ and no rate-compatible puncturing scheme; would need a second code family for low-SNR modes and HARQ. Kept as the fallback if BG2 proves too heavy. |
| codec2/FreeDV LDPC matrices | Proven on air, but LGPL-2.1 data/code in an MIT/Apache project is avoidable friction, and the set is not rate-compatible. |
| Turbo codes (VARA's choice) | Higher error floor, worse at short blocks, no advantage over LDPC in 2026 tooling. |
| Polar codes for short control frames | Attractive for ACKs; deferred — BG2 at small K with a strong outer CRC is adequate for v1, revisit in Phase 2 if ACK robustness is the bottleneck. |
| PEG-designed custom low-rate codes | Would need our own design/verification machinery; the 3GPP tables are already optimised and public. |

## Consequences

- The shift tables of TS 38.212 Table 5.3.2-3 (BG2, 42 × 52, 8 lifting sets) must be
  transcribed exactly; the Sionna cross-check is the guard against transcription errors.
- Frame sizes in the mode table are chosen as valid (K, Z) combinations of BG2.
- The soft demapper must produce properly scaled LLRs (noise-variance aware) for min-sum to
  perform; that is part of P1-2.
- Legacy `fec/ldpc_5gnr.py` and its `xfail` tests are deleted when P1-1 merges.
