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

## 7. Amendment (2026-09-26): the key comes up where the card drains

§2.2 released the key when the card's clock had advanced, since keying, by as many samples as
the station had handed over since keying, and took the clock at keying from the run loop's
last reading of it — the one just before `fill_card`. But the burst is rendered (modulation,
the conversion to audio, the Morse identifier, the record of what was sent, the start of the
transmit capture) and the radio keyed inside the fill's first `playback`, after that reading
and before the first sample reaches the card, and a real card plays silence meanwhile and
counts it (`fill_playback`, and the contract of `AudioIo::played`). Every sample of the burst
left that much later than the station reckoned, the last one too: the key came up, and the
deafness at the end of the transmission (§5) ended, early by the time it took to render the
burst and key the radio.

The keying tail — `key_tail_s` 0.05 s and the playback lead, `DEVICE_LATENCY_S` 0.25 s — was
paying for that time as well as for the card's own latency, which is all it is sized for. A
`[sim]` bench daemon on a loaded four-core machine (2026-09-25/26) logged passes of 255–366 ms
in the playback phase while transmitting; on a sound card the 366 ms one would have released
the key 66 ms before the burst's last sample had even left the queue, with the card's own
latency still to come after that — the end of the last frame cut on the air. The installed
station has logged no playback pass over the 250 ms threshold, so there it may be latent;
shorter renders still ate into the tail, and CI-V keying, which waits for the radio's answer,
and `rigctld`, a round trip, spend their time in the same place. Nothing saw it: the simulated
channel's clock counts no silence and, since it was fixed the same day to start when samples
are queued, the old reckoning was exact for it; and the station tests read the clock before
the fill and handed the burst over at that reading.

Two changes:

1. **The run loop says where the card drains.** `AudioIo::drains_at` is `played() + queued()`,
   where the clock will stand once everything queued now has left, by the trait's own
   contract. The default reads the queue first, so a callback between the two readings makes
   it late, never early; the sound card reads both under one lock, and the simulated channel
   answers with what it was handed in all. `fill_card` reports it once it has handed the
   burst over (`Station::device_drains_at`), and the station releases the key, and measures
   how much of a captured block came in after the transmission, against it. The first report
   after a handover counts — a later one reads the same while the card still holds the burst,
   and more once it has run dry — and the next handover forgets it, so between a handover and
   its report the end is not known and has not come. A harness that reports no clock still
   releases on drain, and a transmission cut short still flushes the card and releases at
   once; one that reports the clock must report this too, or its key stays down until the
   watchdog trips. The station tests' harnesses (`Air`, `run_alone`, the tests of §2 and §5)
   now report it as the run loop does.
2. **A card that runs dry after the last of a burst is not starving.** `set_playing` now says
   whether the station still has samples of the burst to hand over (`Station::handing_over`),
   not whether the radio is keyed. The early release had hidden this: once the key waits for
   the card to drain, the silence the card plays between draining and the next pass counted as
   starvation, and `audio: the sound card ran dry …` would have followed every burst. A burst
   is handed over whole, so what the count holds now is holes only.

`the_key_waits_for_the_last_sample_though_the_card_played_on_while_the_burst_was_rendered`
models a card that plays 0.3 s of silence between the loop's reading and the first block; on
the old code the key came up 290 ms before the last sample had left.
`the_run_loop_keeps_the_key_down_until_the_card_has_played_the_burst` runs `fill_card` itself
against such a card: a report that leaves the queue out releases 5.5 s early, and
`set_playing` following the key counts 474 samples of the card's silence after the burst as a
hole.

Not taken: reading the clock again between the render and the first block. It fixes the render
time, but the loop would have to know which call keyed, and it misses whatever is still queued
ahead of the burst; `played() + queued()` after the handover is the trait's own contract and
needs neither. Nor a fallback to the old reckoning for a harness that reports only the clock:
it is wrong for any card whose clock runs on, and the station tests would have gone on testing
a rule the daemon no longer uses.

The key now stays down past the drain point by up to one pass (≈20 ms), where it used to come
up the render time before it. The engine's timers do not move: they are armed from
`tx_latency_s` when the engine asks for the burst, and `on_tx_done` only ever brings the end of
a transmission earlier. The silent card a daemon runs on when its sound card would not open
discards what it is handed — nothing is queued — so it drains at once: a station with no card
keys for a pass rather than for a burst's length of silence.
