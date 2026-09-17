# OTA test 1 — findings and what to do next

The first radio-to-radio test (2026-09-17, KK4ODA home FTDX10/20 W ↔ KK4ODA-2 mobile
TS-480/50 W, 40 m 7101 kHz, 500 Hz) is analysed in full from the six saved sidecars under
`%APPDATA%\aether-hf\recordings\`. This is the action list it produced. Beta 26 fixed the
panel faults it exposed; the items here are the modem/protocol and operating work that a
second OTA test should chase.

## What the test established

The link works over real RF: probe, connect, ARQ, HARQ, rate control and disconnect all
ran between two physical radios. Six connections, all established. Session 1 completed
cleanly and delivered its text ("hello there"). The other five each ended in **link
timeout**, with roughly half the frames failing to decode, and a **compression-stream
desync** on the marginal path that turned delivered bytes into binary garbage.

Two root problems sit under that, and they are the point of the next test.

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

### 2. The mobile's frames arrived with a large frequency offset (modem + hardware)

The home receiver removed a stable −3 Hz; the mobile's inbound frames repeatedly showed
**+25 to +65 Hz** carrier offset, and those are the frames that failed. That, not raw SNR,
drove most of the losses.

- **Diagnose:** we need the **mobile's own sidecars** — the home's are not enough. If the
  mobile's RX offset on the home's frames is the same sign and size, it is a fixed dial/rig
  calibration difference; if only one direction is offset, it is that rig's TX. A parked
  truck rules out Doppler; a running engine/alternator does not rule out RFI.
- **Check the PHY:** confirm the 500 Hz acquisition CFO search window comfortably covers
  ±65 Hz, and that a frame at +65 Hz with good SNR still acquires on the bench. If it does
  not, the offset window is too narrow at 500 Hz and that is a model/PHY item.

### 3. Five of six ended in link timeout, not a clean disconnect (modem — medium)

On the marginal path the retransmit ladder (rv0→rv3) never cleared and the dead-man timer
fired. Confirm the link-timeout value is sensible for HF and whether a failing station
should attempt a graceful DISC before the timer, so the other end is not left waiting.

### 4. Sidecar counters are daemon-lifetime, not per-session (diagnostic quality — low)

The six sidecars' `counters` blocks are monotonic across the whole run; per-session truth
is only in the `frames` array. Record per-session counter deltas (or snapshot-and-zero at
session start) so one sidecar tells the whole story of one session. Beta 26's *Reset
counters* button is a manual stopgap.

### 5. CM108 keying is still untested on hardware (unrelated — low)

Carried over; not part of this path.

## Protocol for the next OTA test

1. Both stations on **Beta 26** (this release), both at **500 Hz**, same agreed
   frequency and time.
2. **Probe first** (Session tab). Record the both-way SNR. Proceed only if it is roughly
   ≥ 6 dB each way.
3. Run a **Test session** (Session tab) if the probe is good — it sends a probe, a message,
   a file and a burst at every mode, and leaves a `_test` sidecar the bench can replay.
   Then Contribute it (the button is on the Session tab now).
4. Do **two data passes**: one with **compression off**, one with it **on**, sending a
   known repeated string (e.g. "the quick brown fox jumps over the lazy dog") both
   directions so decode correctness is checkable by eye.
5. If it keeps timing out, **pin a slow mode**: set *Fastest mode* low (3–4) in Setup step
   4, to test whether the rate controller is over-climbing for the path.
6. Keep the mobile **stationary**; note whether the engine is running (alternator RFI).
7. **Both operators keep and send their `recordings/` sidecars + WAVs.** The mobile's are
   as important as the home's — item 2 cannot be settled without them.

## Owners

- Code/model/PHY (items 1–4): me, on request.
- Operating and hardware (the two data passes, the mobile sidecars, the rig-offset check):
  the operators.
