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
* The receiver's search cost per call is the open item: it belongs to the receiver, not
  to the transmit path, and it is measured now.
