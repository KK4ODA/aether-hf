# OTA test 1 — findings and what to do next

The first radio-to-radio test (2026-09-17, KK4ODA home FTDX10 / 20 W into a dipole ↔
KK4ODA-2 mobile TS-480 / 35 W into a screwdriver antenna, 40 m 7101 kHz, 500 Hz) is
analysed in full from **both stations'** saved sidecars (the home's six and the mobile's
five). Beta 26 fixed the panel faults it exposed; the items here are the modem/protocol and
operating work that a second OTA test should chase.

## What the test established

The link works over real RF: probe, connect, ARQ, HARQ, rate control and disconnect all
ran between two physical radios. Six connections, all established. Session 1 completed
cleanly and delivered its text ("hello there"). The other five each ended in **link
timeout**, with roughly half the frames failing to decode, and a **compression-stream
desync** on the marginal path that turned delivered bytes into binary garbage.

With the mobile's sidecars now in, the cause is plain: **a marginal, weak-signal path.**
Decoded frames on both ends sat at +3 to +9 dB SNR and the link only ever carried the
robust floor modes (0 and 3); failures were frames that arrived at −5 to −14 dB. The dials
agree to within ~3 Hz — the large frequency offsets seen from the home side were
failed-acquisition artifacts, not a rig problem (item 2). So the two things worth engineering
on are the **compression desync** (item 1), which is a real modem robustness gap, and the
**diagnostics** around failed frames; the rest is antenna and power.

## Action items, most important first

### 1. Compression desync corrupts the whole stream (modem — blocks trusting compression)

On a lossy path a single frame that the ARQ accepts but that carries the wrong bytes, or a
gap the reassembly does not catch, desyncs the deflate stream, and **every byte after it
decompresses to garbage with no recovery**. Session 1 (one short frame) was fine; the long
marginal sessions were not.

- **Diagnose:** replay the OTA-1 recordings — `aetherd --replay <wav> --expect <json>` and
  `python tools/bench_link.py --replay <sidecar>` — to find the exact frame where the
  stream broke, and decide whether a CRC-passing-but-wrong frame was accepted (a CRC-16
  collision at ~50 % frame loss is plausible over a long run) or a reassembly/ordering
  defect let a gap through.
- **Fix options (needs an ADR):** a lightweight per-burst stream checksum so a desync is
  *detected* and the session can resync or fall back to uncompressed; or flush the deflate
  stream at burst boundaries so a loss costs one burst, not the rest; or SNR-gate
  compression off below the level where frame corruption becomes likely. Per-frame
  compression stays rejected (a small frame with no history grows — P3-6).
- **Operate around it now:** for the next test, run at least one pass with **compression
  off** (`[radio] compress = false`, or clear the compress box in Setup step 4). The
  payload is then delivered verbatim even on a lossy path, which both isolates the desync
  and gives a clean control run.

### 2. Frequency offset — RESOLVED as a non-issue by the mobile's sidecars

The first read (from the home side alone) suspected a mobile TX offset, because many of the
home's inbound frames showed **+25 to +65 Hz** CFO. The mobile's own sidecars (analysed
2026-09-17) settle it: **the dials agree.** On *decoded* frames the offset is symmetric and
small — the mobile read the home at −0.2 to +3.5 Hz (median +2.2), the home read the mobile
near −3 Hz. The large CFO values on both ends were attached **only to frames that failed to
decode**: they are the acquisition correlator locking onto noise and reporting a spurious
offset, not a real carrier error. There is no rig-calibration or Doppler problem.

- **Done (beta.27):** the sidecar and the live displays no longer report a CFO for a
  probable noise trigger (a non-decoding frame below the modem's confidence threshold); a
  real near-miss keeps its offset, and every sidecar frame now records `confidence` so a
  near-miss can be told from noise. This is what would have avoided the wrong first read.
- **The real cause of the losses was the link budget, not offset — see item 2b.**

### 2b. The path was marginal and asymmetric — only the floor modes carried (operating)

The mobile ran 35 W into a screwdriver antenna on the truck; the home 20 W into a dipole.
Decoded frames sat at +3 to +9 dB SNR; failures were at −5 to −14 dB. **The link only ever
carried the robust floor modes (0 and 3) — it never successfully climbed.** So the sessions
died whenever the SNR dipped below the floor-mode threshold, which on this path was often.
This is a plain weak-signal path, not a modem defect. For a cleaner next test: a better
mobile antenna or more power, or accept it as a floor-mode path and test at modes 0–3 with
compression off (item 1) so at least the payload is intact when frames do get through.

- **Test sessions abort early here.** All three of the mobile's Test sessions aborted "the
  session dropped during the transfer" before the probe or message finished, so the air gave
  no clean ladder. That is the Test session behaving correctly on a dying link, but it means
  a marginal path yields little. Worth considering whether the Test session should lead with
  a longer, floor-mode-only probe so *something* is captured before it gives up.

### 3. Five of six ended in link timeout, not a clean disconnect (modem — medium)

On the marginal path the retransmit ladder (rv0→rv3) never cleared and the dead-man timer
fired. Confirm the link-timeout value is sensible for HF and whether a failing station
should attempt a graceful DISC before the timer, so the other end is not left waiting.

### 3b. The busy detector versus a crowded band (operating, not a defect)

On the 40 m calling frequency, "anything in the passband" tripped busy with the radios'
RX filters wide open (common digital-mode practice). The daemon already band-limits its own
busy measurement to about ±400 Hz around the signal, so far-off QRM is ignored for the
direct power reading — the mechanism that trips it with a wide filter is the radio's **AGC**:
a strong signal anywhere in the SSB passband pumps the gain inside that ±400 Hz slot and
disturbs the noise floor the detector learns.

- **Operate:** set the radio's RX/DSP filter to ~500 Hz centred on 1500 Hz audio (keep it
  ≥ 500 Hz so it passes Aether's full 1260–1740 Hz signal), and use AGC FAST/AUTO or OFF.
  That keeps adjacent QRM out of the AGC. QRM within ±400 Hz of the tone genuinely occupies
  the channel and still trips busy — move the dial to a clear spot or raise the busy
  threshold (Setup step 4).
- **Modem:** there is no clean modem substitute — the AGC is the radio's. Not an action item.

### 4. Sidecar counters are daemon-lifetime, not per-session (diagnostic quality — low)

The six sidecars' `counters` blocks are monotonic across the whole run; per-session truth
is only in the `frames` array. Record per-session counter deltas (or snapshot-and-zero at
session start) so one sidecar tells the whole story of one session. Beta 26's *Reset
counters* button is a manual stopgap.

### 5. CM108 keying is still untested on hardware (unrelated — low)

Carried over; not part of this path.

## Protocol for the next OTA test

1. Both stations on **Beta 27** (this release), both at **500 Hz**, same agreed
   frequency and time.
2. **Set the radio RX filter to ~500 Hz centred on 1500 Hz** and AGC FAST/AUTO or OFF
   (item 3b), so a crowded band does not keep the busy detector tripped.
3. **Probe first** (Session tab). Record the both-way SNR. Proceed only if it is roughly
   ≥ 6 dB each way.
4. Run a **Test session** (Session tab) if the probe is good — it sends a probe, a message,
   a file and a burst at every mode, and leaves a `_test` sidecar the bench can replay.
   Then Contribute it (the button is on the Session tab now).
5. Do **two data passes**: one with **compression off**, one with it **on**, sending a
   known repeated string (e.g. "the quick brown fox jumps over the lazy dog") both
   directions so decode correctness is checkable by eye.
6. If it keeps timing out, **pin a slow mode**: set *Fastest mode* low (3–4) in Setup step
   4, to test whether the rate controller is over-climbing for the path.
7. Keep the mobile **stationary**; note whether the engine is running (alternator RFI).
8. **Both operators keep and send their `recordings/` sidecars + WAVs.** The mobile's are
   as important as the home's.

## Owners

- Code/model/PHY (items 1–4): me, on request.
- Operating and hardware (the two data passes, the mobile sidecars, the rig-offset check):
  the operators.
