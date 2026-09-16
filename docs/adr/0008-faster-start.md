# ADR-0008: The faster start — a session begins where the connect frames measured it

**Status:** accepted (2026-09-16) · **Code:** `model/aether_model/link/frames.py`
(`ConnectBody.snr_db`), `rate.py` (`RateController.first_mode`, `seed`, `first_mode_back`),
`engine.py` (`_handle_connect_req`, `_handle_connect_ack`); `core/aether-link` likewise ·
**Roadmap:** P9-2 · **Spec:** `docs/spec/air-interface.md` §7.1

## Context

Every session began at the most robust mode and climbed from there, at most two modes per
clean burst: 0 → 2 → 4 → 6 → 8 → 9 on a clean 12 dB channel, six bursts and about twenty
seconds of a short message spent proving what the connect frames had already measured. The
called station decodes the request and knows how the caller is heard; the caller decodes the
acceptance and knows how the called station is heard. Neither number reached the rate
controller, which started blind.

## Decision

The `CONNECT_ACK` body gains one byte: the SNR the request arrived at, in the CONTROL
frame's convention (signed whole decibels, 3 kHz reference, 0x7F for "not measured"; a
request sends 0x7F). The body is seventeen bytes; a receiver reads a sixteen-byte body from
an earlier version as "not measured" and starts as before.

Each station seeds its rate controller with the SNR of the connect frame it decoded — the
request at the called station, the acceptance at the caller — so the first recommendation
either makes is not mode 0. The caller's first burst goes out at
`RateController.first_mode(snr)`: the fastest mode whose threshold fits under the reported
SNR with the controller's starting margin and up-hysteresis, **less two steps**. The
configured `initial_mode` remains a floor under it (a bench that pins a mode sets it with
`max_mode`).

Two steps, not one, because the measurement is of a mode-0 frame — the most robust there
is — and on a fading channel it flatters what a burst at a fast mode will meet.

## Alternatives considered

- **One step in hand.** The same session times as two on every clean channel of the link
  bench, and +17 % on Poor at 8 dB over 500 Hz (ten trials), where the first burst went out
  a mode the channel does not sustain and the controller then had to learn the fading
  penalty from failures at the top instead of on the way up. Two steps: −5 % there, nothing
  lost elsewhere. It also decided a real-audio loopback test (`aetherd`,
  `every_frame_is_reported…`, ~50 dB): with one step the first burst went out at 64-QAM ¾
  and nothing decoded; with two it does. Why a first burst at that mode fails on a wire the
  same mode is reached on by climbing is an open question for the A/B bench (P9-1).
- **A short first burst** (two frames) to limit what a wrong placement costs. It did not
  help the Poor 8 dB case at all — the loss there is the called station's seeded
  recommendation, not the size of the caller's first burst — and added a turnaround
  everywhere else. Rejected.
- **Carrying the recommended mode instead of the SNR.** The receiver recommends, in this
  protocol; but the measurement is the fact and the mode is a policy, and the caller's
  controller may be configured differently. The SNR is carried; the mode is derived.
- **Seeding from the probe** (ADR-0006) as well. Reasonable, and not done: a probe is a
  separate exchange and a session that follows it starts from its own connect frames anyway.

## Evidence

`tools/bench_link.py`, 2 kB sessions (connect, transfer, orderly disconnect), median of three,
before → after:

| | Wide table, four channels × 4…20 dB | Narrow table, four channels × 4…16 dB |
|---|---|---|
| Total session time | 887 s → 673 s (−24 %) | 1033 s → 940 s (−9 %) |
| Worst point | +0.0 % | +3.7 % (Poor 4 dB) |
| AWGN 12 / 16 / 20 dB | 29.3 → 13.5 / 10.3 / 9.3 s | 39.8 → 28.2 s (12 dB), 38.7 → 24.0 s (16 dB) |
| Poor 8 / 12 dB | 35.5 → 25.0 s, 29.3 → 19.8 s | 87.0 → 79.7 s (ten trials), 55.5 → 53.4 s |

On the real modem (`--backend phy`, 2 kB at 12 dB on AWGN): wide 29.3 → 14.5 s (547 → 1100
bit/s), narrow 402 → 528 bit/s; first bursts at mode 6 and 5 instead of 0.

## Consequences

- `connect_bodies` in `core/aether-link/tests/data/link_vectors.json` regenerated (a
  deliberate model change); the port is bit-exact on them and tolerates the shorter body.
- The committed link baselines are regenerated; long transfers gain the few seconds the
  climb used to cost, short ones the half.
- A station of beta.18 or earlier talking to this one starts as before in both directions:
  it neither sends the byte nor reads it.
