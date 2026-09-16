# ADR-0007: The faster climb — a learned margin is given back at an accelerating rate

**Status:** accepted (2026-09-16) · **Code:** `model/aether_model/link/rate.py`
(`RateController.decay_growth`, `max_down_step_db`), `core/aether-link/src/rate.rs` ·
**Roadmap:** P9-2 · **Bench:** `tools/bench_link.py` (`--bandwidth`, `--rate`)

## Context

The rate controller's outer loop (ADR-0002's P2-2 design, `rate.py`) keeps a margin over
each mode's measured AWGN threshold. A failed burst is treated as a measurement of what the
channel costs beyond AWGN and widens the margin at once — by 1.5 dB, or toward the implied
figure, capped at 3 dB — and clean bursts narrow it again at 0.25 dB every three bursts.
Fast down, slow up: what stops a link parked on a mode boundary from oscillating.

The first VarAC file transfer on the bench (2026-09-16, `host-interfaces.md` §7) showed the
price of the slow half. A 16 kB transfer at 12 dB on the 500 Hz waveform ran its whole
length at mode 6 (615 bit/s) though mode 8 (934 bit/s) fits the margin and hysteresis at
that SNR, and the controller replayed in isolation reaches mode 8 in six bursts. Two lost
bursts — one a collision with the other station's acknowledgement, not a fade — had each
cost the margin what a fade costs, and at 0.25 dB per three bursts the margin needed
eighteen bursts, two and a half minutes at that rate, to give 1.5 dB back. The transfer was
over before it did.

## Decision

After the sticky bursts (`decay_every`, unchanged at three), every further clean burst gives
back more than the one before: the step starts at `down_step_db` (0.25 dB) and grows by
`decay_growth` (×2) per clean burst up to `max_down_step_db` (1 dB). A learned penalty is
still given up slowly at first — the stickiness that protects a measurement while it is fresh
is untouched, and `test_learned_margin_is_sticky_then_decays` still holds — and quickly once
clean burst follows clean burst, which is what a fade that has passed, or a collision that
never was a fade, looks like. 1.5 dB comes back in about seven bursts instead of eighteen; a
3 dB jump in about nine instead of thirty-six. A failure resets the step, so a channel that
keeps failing keeps the slow cadence.

Every failure stays a measurement, an isolated one included.

## Alternatives considered

- **Reading the first failure as an accident** (a collision, a missed preamble) and taking
  the targeted jump only on a repeat within a few bursts. It reproduced the VarAC case
  well (back at the top mode eight bursts after the collision) but cost 11 % on the
  Moderate channel at 16 dB on the link bench, where the climb it allowed ran straight into
  the fades. Rejected; the model keeps the P2-2b rule.
- **A gentler acceleration** (×1.5 per burst, capped at 0.5 dB). Half the gain and half the
  loss of the chosen setting at every contested point; the chosen one gains more where the
  old controller was stuck and loses no more than 3.4 % anywhere.
- **Tolerating a lone failed frame in a burst** as the ARQ's business rather than the rate
  controller's. Physically sound on AWGN, where the measured FER curves are cliffs
  (`phy_fer_500.csv`: mode 8 from 50 % at 8 dB to 0 % at 9 dB), but the link bench's lossy
  pipe is softer than the modem (a logistic of 1.2 dB⁻¹) and would have flattered it; left for
  a bench whose channel model is calibrated per channel for steepness too.
- **Longer bursts** (`burst_frames` 6 → 12) to halve the acknowledgement overhead. A
  separate lever, not taken here: it costs more per fade and needs its own curve.

## Evidence

`tools/bench_link.py`, sim backend, 16 kB transfers, old controller (`decay_growth=1.0`)
against this one; net goodput over the grid, and the worst single point:

| Bench | Net | Worst point | Notes |
|---|---|---|---|
| Wide table, AWGN/Good/Moderate/Poor × −4…20 dB, 3 trials | +1.8 % | −0.0 % | +22 % Good 8 dB, +22 % Moderate 8 dB, +18 % Poor 8 dB; AWGN unchanged |
| Narrow table (500 Hz), same channels × 4…16 dB, 3 trials | +3.2 % | −3.8 % | +13 % Poor 12 dB; the loss is at the table floor on Poor |
| Fade ramp ±8 dB / 60 s, four channels at 8 and 14 dB, 3 trials | +5.2 % | −0.1 % | +31 % Poor 8 dB |
| Contested points, 10 trials: narrow Moderate/Poor 4 & 8 dB | +3.3 % | −3.4 % | Poor 4 dB: the controller probes the next mode more often (85 vs 44 frames resent of ~700) |
| Contested points, 10 trials: wide Moderate/Poor 12 & 16 dB | +1.5 % | +0.0 % | +7 % Moderate 12 dB |
| Real modem (`--backend phy`), narrow table, 16 kB at 12 dB | AWGN: 693 → 693 bit/s | Poor: 213 bit/s (new) | with no frame lost the two controllers are the same controller; the modem's FER cliff loses none at mode 8 three decibels above threshold, so the difference only shows after a failure — the collision the bench session had |

The VarAC case itself, in the model (narrow table, 12 dB, 16 kB, one burst lost to a
collision at 120 s): 228 s → 212 s, back at the top mode eight bursts after the collision
instead of never.

## Consequences

- `rate_traces` in `core/aether-link/tests/data/link_vectors.json` regenerated (a
  deliberate model change); the port is bit-exact with the model on them.
- The link bench gained `--bandwidth 500` and `--rate key=value,…`, and `LinkConfig.rate`
  (model) carries controller overrides for benches that compare one controller with another.
- The lossy pipe's steepness is now a known gap between the fast bench and the modem; the
  `phy` backend is the arbiter for anything that turns on FER near threshold.
