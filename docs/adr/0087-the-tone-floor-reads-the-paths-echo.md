# ADR-0087: The tone floor reads the path's echo, and the rungs it breaks stay off the ladder

**Status:** accepted, 2026-10-21. Model first (`echo_share` in
`model/aether_model/phy/tone.py`; `SoftFrame.echo`, `PhyTiming.echo_limit`,
`RateController.observe_echo` with `ECHO_LIMIT` and `NARROW_ECHO_LIMIT` in
`aether_model/link/`; the engine feeds it in `GrantedEngine._heard`), then the port
(`tone::echo_share`, `Received::echo`, `SoftFrame::echo`, `PhyTiming::echo_limit`,
`RateController::observe_echo`, `GrantedEngine::heard`, the daemon's `PhyFrame`). Held together
by `phy_vectors.json` (`tone_receive` carries the echo, `tone_echo` two-ray cases),
`link_vectors.json` (the tables, an `echo_traces` controller trace) and `granted_vectors.json`
(every frame's echo, the engine's echo after every input, an `echo-path` session). The limits
come from `tools/bench_echo.py` (`bench/baselines/echo.csv`), and a test holds them to it. **No
wire change, no configuration key.** The `frame` event and the sidecars' frame records gain
`echo`.

## 1. Context

The author asked for the rate controller on the NVIS path to be tackled. ADR-0085 and ADR-0086
came from `80m-agc-carrier-outside-500`: ITU-R F.1487's disturbed NVIS, two Rayleigh rays 7 ms
apart with a 1 Hz Doppler spread each, at 10 dB on the 500 Hz air. On it the receiving station's
controller recommended rungs 2–4, then OFDM, then rungs 2–4 again. Those rungs fail on that
path whatever the SNR.

**What the path does to each rung** (`tools/bench_echo.py`, the air's own receiver, 16 frames a
point, 10 and 20 dB):

* the tone floor's own kinds, tone-24 and tone-36 (40 ms symbols), decode every frame on every
  delay up to 8 ms;
* the four-tone kinds' 10 ms symbols do not. On a 7 ms path tone4x100-51 (rung 2) decodes about
  half its frames, tone4x100-75 (rung 3) a tenth and tone4x100-93 (rung 4) none;
* the 500 Hz OFDM rungs lose most frames once the echo outruns their 5 ms prefix.

The four-tone receiver's timing matters: with its windows a quarter of the echo after the first
ray, rung 2 decodes 19 frames in 20 on 7 ms, against 12 at the detector's own timing. But the
detector locks onto one ray or the other, and a search for the best timing by the data's own
energy did no better than the detector (§4).

**What the path reads.** The floor's SNR reading saturates near −3 dB on that path whatever the
SNR: −2 to −9 dB from 0 to +20 dB. The echo counts as noise. A quiet path at −3 dB reads the
same, and there rung 4 is the right start: it sits eight decibels above its threshold. The
learned margin cannot represent "rungs 0–2 carry, rungs 3 and up do not, whatever the SNR". It
is one number for the whole ladder, capped at 12 dB, and it decays on the clean bursts the
floor's rungs give.

**What it cost**, on the link bench driving both engines through the real modem and channel
(`bench_link.run_point` with the `phy` backend, one fade for the session, six seeds a point):

| 500 Hz, ITU disturbed NVIS | 5 dB | 10 dB | 15 dB |
|---|---|---|---|
| 1 kB, beta.118's controller | 256 s | 254 s | 253 s |
| 1 kB, the link held at rung 1 | 225 s | 224 s | 223 s |
| 1 kB, the link held at rung 2 | 206 s | 197 s | 205 s |

At 4 kB the controller already matched the link held at rung 2 (836 against 847 s, 790 against
795 s). Once the session has measured the path, the margin and HARQ settle it on rungs 1–2. A
rung-4 frame decodes on its third copy combined, so even rung 4 delivers about 30 bytes per
frame of air there. The loss is at the start. The caller's first data goes at the rung the
call's floor reading supports, rung 4, and on this path two bursts of six frames carry a whole
kilobyte. Those frames then take three copies each, and the next burst's new frames go at
rung 3, which needs three copies too. On the 2 300 Hz air the controller lost little: within
1–5 % of the link held at rung 3 for 4 kB. Its 50-baud rungs 2–3 carry 7 ms, and its OFDM
BPSK ⅕ carries 60 % of its frames there.

So the controller lacks a piece of information, not a rule. One floor reading at −3 dB cannot
tell a quiet path from a dispersive one, and at the start nothing else has been heard.

## 2. Decision

**The floor's frames read the echo.** An echo puts the previous symbol's tone at the start of a
symbol window and, the receiver's timing being on the later ray, the next symbol's tone at its
end. Noise puts nothing there. `echo_share` sums, over triples of consecutive symbols whose
tones are known — the sync symbols', and the data symbols' strongest — the energy at the
neighbours' tones inside the middle symbol, less the noise, over that symbol's own tone energy
less the noise:

* **both sides**, so the reading does not depend on which ray the detector took;
* **only neighbours two or more bins from the symbol's own tone** (`ECHO_SPACING`): a Doppler
  spread or a residual offset spreads a tone's energy into the bins beside it, while an echo puts
  the neighbour's tone wherever it is;
* **only on the floor's own kinds**, tone-24, tone-36 and the control frame: the whole frame at
  40 ms symbols. A fast kind's sync blocks alone hold too few triples and read lower, so a fast
  kind reports none.

Measured (median of a frame's reading, 10 and 20 dB, detector timing): 0.0003 or less on AWGN and
ITU Good, Moderate and Poor; 0.0013 for two rays 3 ms apart at 1 Hz; 0.0028 for 4 ms; 0.004–0.006
for 5; 0.009 for 6; 0.012 for 7; 0.017–0.019 for 8. Flutter reads 0.0010–0.0016 and ITU
high-latitude moderate (3 ms, 10 Hz) 0.0019. Every call, acceptance, floor header and floor
answer is a frame of the floor's own kinds, so a session never lacks readings, and the call and
the acceptance give one before any data goes.

**The controller keeps off the rungs the echo breaks.** `RateController.observe_echo` smooths the
readings (each moves it by `echo_weight`, 0.4) and ignores a frame read under `echo_min_snr_db`
(−6 dB). Below that the noise in the bins it weighs is as large as a 4 ms echo's spill, and the
rungs an echo breaks are not on offer there. A rung whose limit the smoothed echo exceeds does
not fit (`_fits`), so `first_mode` and the climb stop under it, and a recommendation it rules out
moves down at once to the fastest rung below it that the SNR carries. The controller climbs rung
by rung, so the rungs above an excluded one are not reached past it either. The engine feeds
every decoded frame of the session to the controller, the call or the acceptance first
(`_heard`).

**The limits are measured** (`tools/bench_echo.py --limits`):

* on a path and at an SNR, a rung is *beaten* when a slower rung measured there delivers more
  (decode rate times what a frame carries a second);
* a path *refuses* the rung when it is beaten at every SNR measured, and *carries* it otherwise;
* a rung's limit is the lowest median echo of a path that refuses it, less a tenth, provided it
  stands a quarter above the highest of any path that carries it;
* within a family (the floor's rungs, the OFDM ones), a rung is held to the lowest limit of a
  slower rung of it;
* a path that does not carry rung 0 is left out, and rung 0 has no limit.

The limits are drawn on ITU-R F.1487's conditions and the delay sweep at its 1 Hz Doppler (§3).

## 3. Measured

**The limits** (`bench/baselines/echo.csv`: 704 points, both airs, 22 paths, 10 and 20 dB, 16
frames each). On the 500 Hz air rungs 3 and 4 are kept off above 0.0038. Their lowest refusing
path is two rays 5 ms apart: 4 ms reads 0.0028 and carries both, 5 ms reads 0.0042 and refuses
both. On the 2 300 Hz air only rung 5, tone100-153, is kept off, above 0.0073. Its 16 tones
spread over 1 600 Hz carry more echo than the four-tone kinds' 400 Hz: tone100-105 still
carries a 6 ms path at a reading of 0.011, close to what a 7 ms path that refuses it reads, so
it gets no limit. No OFDM rung gets one on either air: ITU flutter and high-latitude moderate
carry them at readings a little under the lowest refusing path's. On the paths above the 500 Hz
limit, the first OFDM rung keeps at most 6 of 16 frames, and the faster ones at most 2.

**The probes, and what they would change.** Two rays at 0.1 and 10 Hz show that one reading
cannot always tell the rungs apart. Frozen 7 ms multipath (0.1 Hz) reads 0.006 and carries rung 4
three times in four. Two rays 4 ms apart at 10 Hz read 0.0034–0.0039 and carry rung 4 while the
OFDM rungs fail. Drawn on as well, the probes set the four-tone limit at 0.0083, which half the
NVIS sessions' readings fall under. On the link bench below, that build took 222 s at 5 dB
against this one's 213 s. Neither is an ITU-R F.1487 condition. A 7 ms spread between modes
comes with a disturbed ionosphere, not a still one, so the limits are drawn on the
recommendation's conditions and its delays, and the probes stay in the file. The cost falls on a
path like the frozen one: rungs 3 and 4 kept off where they would carry.

**The reading's noise.** At SNRs from −6 to −3 dB, 95 % of single frames read under 0.0023 on
AWGN, 0.0028 on Poor and 0.0025 on Moderate, and under 0.002 on flutter. ITU Good is the
exception: 5 % of frames read up to 0.0057 at −3 to −5 dB, when a slow fade takes a whole frame
down. If such a frame is the call or the acceptance, the session's first burst goes at rung 2
rather than rung 3 or 4, and the next reading brings the smoothed echo back under the limit.

**The link bench through the real modem** (`bench_link.run_point`, `phy` backend, one fade for
the session; the base arm with the limits off):

| ITU disturbed NVIS | 5 dB | 10 dB | 15 dB |
|---|---|---|---|
| 500 Hz, 1 kB, beta.118 (6 seeds) | 256 s | 254 s | 253 s |
| 500 Hz, 1 kB, this ADR | 213 s (−17 %) | 211 s (−17 %) | 210 s (−17 %) |
| 500 Hz, 1 kB, the link held at rung 2 | 206 s | 197 s | 205 s |
| 500 Hz, 4 kB, beta.118 (4 seeds) | 840 s | 830 s | |
| 500 Hz, 4 kB, this ADR | 819 s (−2.6 %) | 797 s (−4.0 %) | |
| 2 300 Hz, 4 kB, beta.118 (4 seeds) | 401 s | 392 s | |
| 2 300 Hz, 4 kB, this ADR | 367 s (−8.4 %) | 352 s (−10.0 %) | |

Every session delivered. The slowest single 1 kB session against its own seed was +2 %: seed 135
at 5 and 10 dB, whose readings stayed near the limit and which reached rung 3. Every 2 300 Hz
session was faster.

**Paths without a long echo.** The fading pipe's frames carry no echo, so the link bench there
is beta.118's by construction. Both trees on the same points, 500 Hz Good and Poor and 2 300 Hz
Moderate and Poor at −4 and +4 dB: identical, session for session. Through the real modem, 72
sessions of 1 kB on ITU Good, Moderate, Poor, flutter, low-latitude moderate and high-latitude
moderate, both airs, at −3, +5 and +15 dB, two seeds each: every one delivered, and the echo
ruled out no rung in any of them. The highest smoothed echo any controller held was 0.0020,
against limits of 0.0038 and 0.0073, so those sessions are beta.118's.

**The scenario harness** (the eight scenarios on ITU NVIS, four draws each, against beta.118's
daemon; then all 56 × 2) is reported in its own commit once it has run.

## 4. Alternatives

* **A rung the path refuses, learned from its failures**: a rung lost in the main twice at a
  reading that should carry it, with no clean burst between, is refused for a while. On a
  1 kB session the refusal comes after the frames it should have spared were committed. The
  same seed ran exactly as before (250 s).
* **What each rung delivers, measured** (a Minstrel-like filter on the recommendation:
  delivered bytes per copy received, HARQ included). At 4 kB, where it had evidence, it changed
  nothing, because the controller already ended on rungs 1–2. At the start it has no evidence.
* **The data's own timing**: try offsets around the detector's start and keep the one whose data
  symbols concentrate their energy most. On 7 ms it decoded 10 frames in 20 against the
  detector's 13. A receiver that undoes the echo is the real answer for rung 2 and up there, and
  it is a receiver project of its own.
* **Starting every session lower**: ADR-0066 measured what starting a session the call had
  measured with ten decibels to spare at half the rate costs on a clean path.
* **A mirror-bin estimate** (each neighbour's bin less the bin as far on the other side of the
  symbol's tone). It cancels a Doppler spread in expectation and halves the echo's reading. It
  doubles every reading's spread, so flutter and a 4 ms echo overlapped more, not less.

## 5. Consequences

* On a path whose echo the floor's frames read, a session starts below the rungs the echo breaks
  and does not climb into them. On ITU's disturbed NVIS that is 17 % off a kilobyte at 500 Hz and
  8–10 % off 4 kB at 2 300 Hz.
* A path without a long echo reads 0.0003 or less, and the controller behaves exactly as before.
* On a frozen two-ray path 7 ms apart (0.1 Hz, not an ITU condition), rungs 3 and 4 at 500 Hz are
  kept off where they would carry three frames in four (§3).
* The `frame` event and the sidecars' frame records carry each floor frame's `echo`, so field
  recordings will show what real 80 m paths read. Nothing the operator sees on the panel changes.
* The limits are measurements. A receiver change that moves a rung's tolerance re-runs
  `tools/bench_echo.py`, and the test holding the tables to `bench/baselines/echo.csv` says when
  they are stale.
* Open: a receiver that undoes the echo (rung 2 decodes 19 frames in 20 on 7 ms with its windows
  placed right, against 12 at the detector's timing), and the Doppler dependence the probes
  show. A reading of the delay itself, not its spill, would separate the frozen path from the
  disturbed one.
