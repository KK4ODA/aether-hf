# ADR-0016: Calls, probes and beacons start on the tone floor; a reading from the floor is a lower bound

**Status:** accepted, 2026-09-24. Model first (`aether_model/link/engine.py`, `rate.py`,
`sim.py`), then the port (`aether-link`, and the daemon's beacon). The frames and the link
protocol are unchanged: a beta.54 station and a beta.55 station connect, probe each other and
hear each other's beacons.

## 1. Context

The author asked how the rung is chosen for a probe, a beacon and a call, suspecting the modem
still started optimistic. It did, in the one place that matters most — before anything is known
of the path:

* **A call** went out at the ordinary family's robust mode — the slowest OFDM mode whose frame
  carries a connect body: wide rung 6, BPSK ⅕, −5.1 dB on AWGN; narrow rung 5, QPSK ½,
  −5.2 dB — on its first two tries, and alternated with the tone floor (tone-24, −19.0 dB) only
  from the third. That rule is ADR-0009's, from when the floor was an OFDM frame family a few
  decibels below the ordinary one. The tone floor (ADR-0013) reaches 14 dB lower, and a weak
  path spent two tries — twenty seconds — on frames it could not carry before it was called
  on one it could.
* **A probe and a beacon** always went out at the ordinary robust mode. Below about −5 dB no
  probe was ever answered — 0 of 30 at −8 dB and below on every channel class, both airs —
  and no beacon heard, on paths the floor carries whole sessions over.

The first burst of a session is chosen from the connect frames' SNR (ADR-0008) — optimistic by
design, and bounded: two rungs in hand, and the rate controller steps down on the first
failure. That is not this ADR's subject, except where §4 makes it one.

## 2. Decision

1. **A call starts on the tone floor** and alternates: tries 1, 3, 5, … go out on the floor,
   tries 2, 4, … in the ordinary family. The ordinary tries stay because a path the floor does
   not carry exists — a carrier sitting on the floor's 400 Hz denies it what 2 300 Hz of OFDM
   with a code across its carriers still gets through. The answer goes back in the family the
   request arrived in, as before (ADR-0009).
2. **A probe goes out on the tone floor**, and is answered in the family it arrived in. A
   beta.54 station probes in the ordinary family and hears its answer there; it answers a floor
   probe in the ordinary family, which the prober — waiting for a floor-length answer — still
   hears if the path carries it. There are still no retries (ADR-0006).
3. **A beacon goes out on the tone floor** (tone-24 on both airs). The floor's frames are the
   same on both airs and a beacon carries no bandwidth, so a 2 300 Hz station hears a 500 Hz
   station's beacon and the other way round.
4. **The ISS waits for an answer in the family the IRS last heard.** The IRS answers a burst
   in the burst's family when it decodes any of it and otherwise in the family it last heard
   (ADR-0009). After a call on the floor the session's first burst is usually OFDM; when the
   IRS decoded none of it, its acknowledgement came on the floor (3.2 s) while the ISS waited
   for an OFDM one (0.43 s), gave up a second into it, and lost the recommendation it carried.
   The ISS now waits for the longer of the two.
5. **A caller does not call over a frame it hears arriving, and a prober does not give up on
   one.** At −14 dB on ITU Moderate a called station whose acceptance was lost is connected,
   and answers the preamble of the caller's next try — an ordinary one it cannot decode — with
   an acknowledgement on the floor (ADR-0012: the acknowledgement waits for the frame its
   preamble announced); the caller's try after that, on the floor, ran into it, every time,
   until the caller gave up. A preamble heard while calling or probing now moves the next try
   (or the probe's deadline) past the announced frame's end and a turnaround. The link
   simulators now announce control frames' preambles too, as the daemon does; they announced
   data frames only, which is why the bench had not seen this.
6. **A reading from the floor is a lower bound** (§4): a rate controller seeded from a floor
   frame seeds again from the first clean burst it measures on an ordinary frame — upward
   only, when that reads more than `reseed_margin_db` (3 dB) above the seed, and once.

## 3. Calls and probes

`tools/bench_calls.py` on the fading pipe: per air, class and SNR, 30 calls (connected within
400 s, and when) and 30 probes (answered within 120 s); beta.54 from a worktree against the same
calibration (`bench/baselines/calls.csv`). Both airs give the same numbers after the change —
their floors are the same frames — and the 2 300 Hz ones are shown:

| Class | SNR | Calls connected, median s: beta.54 → now | Probes answered: beta.54 → now |
|---|---|---|---|
| AWGN | −16 … −8 | 30/30, 20 s → 30/30, 11 s | 0 → 30 |
| AWGN | −4 … +8 | 30/30, 3 s → 30/30, 11 s | 30 → 30 |
| Good | −16 | 19/30, 39 s → 29/30, 33 s | 0 → 13 |
| Good | −12 | 28/30, 20 s → 30/30, 11 s | 0 → 26 |
| Good | −8 / −4 / 0 | 30/30, 20 / 7 / 3 s → 30/30, 11 s | 1 / 9 / 27 → 30 |
| Moderate | −16 | 15/30, 20 s → 26/30, 32 s | 0 → 11 |
| Moderate | −12 / −8 / −4 | 30/30, 20 / 20 / 19 s → 30/30, 11 s | 0 / 0 / 9 → 27 / 30 / 30 |
| Moderate | 0 / +4 | 30/30, 3 s → 30/30, 11 s | 24 / 27 → 30 |
| Poor | −16 | 24/30, 22 s → 23/30, 28 s | 0 → 11 |
| Poor | −12 … −4 | 30/30, 20 / 20 / 19 s → 30/30, 11 s | 0 / 0 / 7 → 30 |
| Poor | 0 | 30/30, 3 s → 30/30, 11 s | 24 → 30 |

A probe is now answered wherever a session can be carried, and a weak path connects in half the
time. A strong path's handshake costs 11 s instead of 3: two floor frames of 5.4 s where two
OFDM frames of 1.05 s did.

## 4. What the floor's SNR reading can show

The tone floor measures SNR by energy (ADR-0013): the sync symbols' tone energy over the median
of the bins that hold no tone, taken back to the OFDM reference. That is exact where the floor
is used and reads low on a strong path. `tools/bench_tone_snr.py`, genie timing, 20 frames a
point, median reading (`bench/baselines/tone_snr_reading.csv`):

| True SNR | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| 0 dB | −0.1 | +0.8 | −0.8 | −1.8 |
| +10 | +9.2 | +9.0 | +6.7 | +3.1 |
| +20 | +15.6 | +12.6 | +11.2 | +4.8 |
| +30 … +40 | +17.5 | +15.5 | +12 | +5 |

Two things cap it. The glide between two tones spills a little of every symbol into every bin,
some 43 dB down, and a tone symbol's own Es/N0 is 26 dB above the OFDM-reference SNR, so the
reading stops near +17 dB however clean the path. On a dispersive path the echo's spill into
the next symbol counts as noise too: 2 ms of a 40 ms symbol, and ITU Poor reads +5 dB at any
SNR. OFDM, with the echo inside its cyclic prefix, does not see it. (The OFDM frames' own
reading has a ceiling as well: 16–27 dB on the daemon's loopback, by mode.)

Tried and not adopted: estimating the noise from the bins outside the neighbouring tones' span
(6 dB better: the spill is broadband), and subtracting the known clean spill of the sync blocks'
interior symbols — which tracks the truth to +30 dB on AWGN, part of the way on Good and not at
all on Poor, where the spill is the channel's. A time-domain residual after decoding would count
Doppler as noise. The estimator stays; what reads it has to know it is a lower bound.

What reads it, now that the first frames of every contact are floor frames:

* **The first burst.** Both rate controllers are seeded from the connect frames (ADR-0008), so
  after a floor call a strong path's session starts from the floor's ceiling — on a dispersive
  path from +5 dB, eight rungs low — and the controller climbs at most two rungs a burst
  (`max_up_step`), its smoothed SNR three-tenths of the way a burst. Decision 6 is the fix: the
  first clean ordinary burst is a measurement the path can show, and the session starts again
  from it, as it would have from an ordinary connect frame. Measured in §5.
* **Probe results, beacons, the stations-heard list and the panel's frame readings** show the
  floor's reading as measured: above about +10 dB (+3 dB on a dispersive path) it is a lower
  bound. The documentation says so; the numbers are not dressed up.
* **The Test session** sizes its first message from the probe's reading (half a kilobyte below
  0 dB, a kilobyte below +6): on a strong dispersive path it now sends a kilobyte where it sent
  the plan's ceiling. The file is sized from the rate the message measured, as before.

## 5. Sessions

`bench_link.py --fading`, 30 sessions a point, 2 kB at 2 300 Hz and 1 kB at 500 Hz, beta.54's
code from a worktree against the same calibration. Two runs (`bench/baselines/
link_floor_calls.csv`):

**Readings as the tone floor gives them** (`--floor-cap`: each class's ceiling from §4), median
seconds per session, beta.54 → calls on the floor with the re-seed (without it, in brackets,
where it differs); every point completes 30 of 30 unless marked:

| Air, class | −12 dB | −6 | 0 | +6 | +12 | +18 | +24 |
|---|---|---|---|---|---|---|---|
| 2 300 AWGN | 319 → 303 | 130 → 121 | 61 → 69 | 25 → 34 | 14 → 23 | 12 → 21 | 8 → 21 |
| 2 300 Good | 739 (26/30) → 768 (28/30) | 201 → 185 | 112 → 121 | 42 → 49 (52) | 19 → 26 | 12 → 21 | 8 → 21 |
| 2 300 Moderate | 616 → 606 | 160 → 146 | 113 → 115 | 40 → 48 | 17 → 25 | 12 → 23 | 8 → 22 (23) |
| 2 300 Poor | 568 → 555 | 165 → 152 | 112 → 118 | 33 → 44 | 16 → 33 (36) | 12 → 28 (36) | 8 → 23 (36) |
| 500 AWGN | 228 → 219 | 135 → 126 | 54 → 63 | 28 → 37 | 18 → 26 | 14 → 23 | 14 → 24 (23) |
| 500 Good | 384 (27/30) → 418 (29/30) | 156 (29/30) → 157 | 116 → 123 (120) | 39 → 48 (50) | 24 → 33 (34) | 18 → 26 | 14 → 24 (23) |
| 500 Moderate | 329 → 323 | 161 → 146 | 120 → 118 (116) | 49 → 58 | 24 → 34 | 17 → 26 | 14 → 26 |
| 500 Poor | 309 → 300 | 150 → 146 | 112 (29/30) → 110 | 40 → 50 | 22 → 36 | 14 → 30 (36) | 14 → 30 (36) |

**Readings as the channel gives them**, from −18 dB, the first run, before the re-seed (which
the second run shows makes no difference below 0 dB, and 3 s either way at 0 dB): more
sessions complete at the edge — 2 300 Hz Good −14 dB 18 → 21 of 30, −10 dB 28 → 29; 500 Hz
Good −18 dB 4 → 6, −14 dB 21 → 26, −10 dB 29 → 30; 500 Hz Moderate −18 dB 0 → 1 — and of the
52 points from −18 to −4 dB, 39 are shorter (by up to 15 %: a call heard on its first try).
The longer ones: AWGN at −4 dB, where the ordinary call already got through, by the handshake
(+8 %); 2 300 Hz Moderate −10 dB 358 → 398 s; 500 Hz Good −12 dB 391 → 418 s and −8 dB 184 →
225 s; the rest within 4 % (and 2 300 Hz Good −18 dB, one session in thirty either way).

From 0 dB up every session is longer, by the handshake and, on a strong path, a first burst
started from the floor's ceiling. The re-seed takes back most of what the ceiling costs where
the ceiling is lowest — ITU Poor at 2 300 Hz, +24 dB, 36 → 23 s — and changes nothing where the
session is over before its second burst.

## 6. Cost, and the option not taken

The cost is the strong path's handshake: 11 s instead of 3, which a short session feels — 2 kB
at 2 300 Hz took 8 s on a +24 dB path and takes 21–23 s, 1 kB at 500 Hz 14 s and 24–30 s; at
+6 dB the difference is 8–11 s — and a 16 kB transfer barely does. It buys the weak path's
contact: a call heard on its first try, a probe answered, a beacon heard, down to the floor.

Not taken: **answering a floor call in the ordinary family when the request arrived well above
the ordinary robust mode.** It would halve the strong path's cost (a floor request and an OFDM
answer, about 7 s), but on an asymmetric path — more noise at the caller's end — the acceptance
would fail where a floor one gets through, and the caller's retries would be answered the same
way; the rule would need an exception for a repeated request, and a request's reading, the
lower bound of §4, is a poor judge of "well above" on a dispersive path. The robust start first;
the faster answer when the air says it is safe.

## 7. Consequences

* `LinkEngine`: `_connect_floor` (the first try on the floor), `probe` and `_send_probe`'s
  family, `_handle_probe`'s answer in the probe's family, `_send_burst`'s wait for the longer
  family, `on_preamble` while calling or probing; both ports. `RateController.seed(snr,
  lower_bound)`, `reseed_margin_db`; the engine passes the family of the connect frame it
  decoded. `LinkEngine::robust_mode` is public in the port, and the daemon's beacon goes out at
  `robust_mode(true)`.
* The link simulators announce every frame's preamble, control frames included, and take a
  floor-reading cap (`floor_reading_cap_db` / `with_floor_reading_cap`); `bench_link.py
  --floor-cap` applies each class's ceiling from `tone_snr_reading.csv`.
* New tools and baselines: `tools/bench_calls.py` (`calls.csv`), `tools/bench_tone_snr.py`
  (`tone_snr_reading.csv`); `link_floor_calls.csv`.
* The daemon's tests that measured a beacon's, a probe's or a call's first frame as an OFDM
  frame measure it as the tone frame it now is — its RMS at the OFDM frames' peak
  (`tone::gain_db()` above their average), its SNR reading within the floor's ceiling, no
  constellation — and the OFDM checks moved to OFDM frames: the call's second try, the
  session's acknowledgements.
* Spec: §3 (a connection request starts on the floor), §4 (the control rung is no longer what
  requests, probes and beacons go at), §7 (the probe and the beacon), ADR-0006 and ADR-0009 are
  amended by this one.
