# ADR-0010: A whole burst goes to the sound card at once, and the key follows the card's clock

**Status:** accepted 2026-09-19, port only (the model has no sound card) · **Roadmap:** P6-6 ·
**Builds on:** P3-3 (the run loop), the first on-air findings (the tail covers the lead) ·
**Evidence:** `field/TX-ONSET-FINDINGS.md`

## 1. Context

The daemon is one thread: it answers control requests, runs the receiver over what the
sound card captured, tops the card's playback queue up, and goes round again. Until now it
kept a quarter second of audio queued ahead of the card and topped it up every pass, and
the card's playback callback played silence — uncounted — whenever the queue was empty.

The receiver costs about 100 ms per call whatever the block size, and a pass that also
decodes a frame costs 300–400 ms (`aetherd --replay` now measures it). Any pass longer than
the quarter second put a hole in the burst on the air. Two videos of the FTDX10's scope
showed it, the truck's recordings of the base's bursts have 70–90 ms holes a quarter
second in, and OTA-2's "choppy" audio was it.

## 2. Decision

1. **The whole rendered burst is handed to the sound card in the pass that renders it.**
   The playback queue is bounded by the longest transmission the station may make
   (`max_key_s` + 2 s), not by a fraction of a second. Nothing the loop does afterwards —
   a decode, a slow disk, the scheduler — can starve the card short of stalling for the
   length of the burst.
2. **The key is released against the card's playback clock.** `AudioIo::played` counts the
   frames the device has consumed since it opened, silence included. The station records
   both clocks at keying, and releases when the card has consumed as many samples since
   keying as the station handed it since keying. The keying tail still covers the card's
   own latency (`DEVICE_LATENCY_S`, a quarter second, what the old queue depth was), and the
   engine's `tx_latency_s` is unchanged, so no timing the link layer relies on moves.
   A harness that reports no clock releases on drain, as before.
3. **Starvation is counted.** The callback counts every frame of silence it plays while a
   burst is in flight; the loop logs it (`audio: the sound card ran dry …`) and the
   diagnostic bundle carries it. A pass over a quarter second is logged with where the
   time went, rate-limited, and the slowest pass is kept in the bundle.
4. A transmission cut short — a tune tone, a drive burst, a watchdog trip — flushes the
   card's queue as well as the station's, and releases at once.

## 3. Alternatives considered

* **A bigger top-up backlog** (one second). Cheap, but the key release was tied to the
  station's queue draining and the tail covered the backlog, so every burst would have
  carried three quarters of a second more dead key — straight into the ARQ's turnaround.
  With the release on the card's clock the backlog can be the whole burst at no cost.
* **A second thread for the receiver.** It would keep the loop's passes short, but it
  moves the problem (a shared modem behind a lock) rather than removing it, and the
  receiver's per-call cost should be fixed on its own terms.
* **A transmit amplitude ramp or an AGC-settling tone**, as first suspected. The rendered
  waveform was measured: the preamble has the data's power, the onset is at level within
  20 ms, there are no holes. Mercury (Rhizomatica) adds no ramp either; it renders the
  whole burst, keys, writes the whole buffer once and holds the key by an absolute
  deadline — the shape adopted here.

## 4. Consequences

* A burst's audio is independent of the loop's timing from the moment it is rendered. The
  loop's latency (still ~100 ms a pass, the receiver's cost) now delays only what the loop
  does: commands, keying, the start of the next burst.
* `AudioIo` grew `played`, `set_playing`, `starved` and `clear`; every backend (cpal, the
  loopback, the silent card, the simulated channel) implements them.
* `[record] tx_audio` keeps each transmission's exact audio for holding the air against.
* The receiver's search cost per call was the open item: it belongs to the receiver, not
  to the transmit path, and it is measured now — and closed in §6.

## 5. Amendment (2026-09-20): the deafness ends with the transmission, not with the pass

Releasing the key on the card's clock moved the end of the station's *deafness* — the
receiver is fed silence for every captured block that arrives while `transmitting` — to the
true end of the tail, a quarter second later than the queue-drain release it replaced, and
a block is muted whole. On a slow pass the block that carries the end of the tail also
carries the start of the peer's reply, and that start was muted with the tail: the first
frame of the burst after every slow pass, on a loaded machine. CI showed it as the Test
session's mode ladder decoding one frame of two at 25 dB on the simulated channel, and a
run of two daemons pinned to one CPU reproduced it, the recording holding the whole burst
the receiver never saw the start of.

Two changes. The station now takes the card's clock *before* each block and mutes only as
far as the transmission's last sample (`captured_after_transmission`); the transmission is
finished there, inside the block, and the rest of the block is heard. And the simulated
channel delivers what a station plays a card's latency (`DEVICE_LATENCY_S`) after that
station's playback clock says it played — grouped into the runs it arrived in, so a burst
handed over whole is delivered whole — because the keying tail is sized for that delay and
without it the peer's reply reached a station inside its own tail on the wire and never on
the air. The simulated channel's timing now matches a sound card's rather than flattering
it; `CI` keeps a failed two-daemon test's daemon logs and sidecars as an artifact.

## 6. Amendment (2026-09-23): the receiver computes each bank row once

The open item of §4. The streaming receiver searched `[searched − lookback, seen)` on every
call, and `searched` trailed `seen` by a lookback, so every 20 ms block re-ran the
correlation bank over `block + 2·lookback` positions — about 1 650 on the wide air, 4 600
on the narrow — of which only the block's ~160 were new. Ninety percent of every pass was
repeated work, and the first ten-mile session (a mini PC at half its CPU) showed what that
costs: passes of 250–424 ms, 12–20× behind real time, acknowledgements late past the peer's
window and the station's own bursts holed.

A bank row at a position depends only on the samples of its own reference window and never
changes once they have arrived. The receiver now keeps one row per position from
`buffer_start` (`StreamingReceiver::rows`), computes rows only for positions whose window
has just completed (`FrameDetector::bank_row` over a persistent `BankState`, which holds the
floor family's ring and the scratch buffers so a position allocates nothing), and hands the
unchanged peak-picker (`detect_with`) the very same window it searched before, built from the
cached rows. The offline `bank()` is the same `bank_row` in a loop, so there is one
implementation of the per-position mathematics and the streaming-equals-offline test still
pins them together. The one term that was non-local — the normalisation floor taken from the
region's mean power — is taken from the buffer's mean instead; it only bites on a window
sixty decibels under the mean, where nothing is detectable either way.

Measured with `aetherd --replay --block-ms 20`, the same frames found before and after:

| recording | before | after |
|---|---|---|
| 10-mile session of 2026-09-23 (500 Hz, 1 597 acquisitions in 45 s) | 10.38× real time, slowest block 392 ms, 780 blocks over 250 ms | 0.79×, slowest 135 ms, none over 250 ms |
| idle 2.3 kHz listen (156 s) | 0.55×, slowest 124 ms | 0.18×, slowest 66 ms |

What remains of the cost is per candidate, not per position: the fine offset, the repetition
check and a decode attempt for every acquisition, and on the narrow air the floor detector
false-alarms often enough on a real band (OTA-2 finding 2) to keep that bill high. That is
the floor detector's threshold, a separate matter (ADR-0009).
