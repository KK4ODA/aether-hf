# The start of a burst: what two videos of the FTDX10's scope showed (2026-09-19)

Two phone videos of the FTDX10's screen while it transmitted four *Set drive* bursts
(`drive.set {"bursts": 4}` at 09:06:57Z and 09:08:03Z, the daemon's log), the first with the
rig's AGC on AUTO and the second on FAST. The author's report: at the start of every burst
the signal on the scope wobbles before it settles, more so on FAST, and VARA HF on the same
rig does not do that.

## 1. What the videos actually show

The FTDX10 was the **transmitter** (its log keyed it over CAT; the S-meter sits at S1–S3
under a hump that fills the scope, which no received signal does). Its scope during
transmit is drawn through its own receiver — the noise floor vanishes the instant the key
goes down, and the picture changes with the receiver's AGC setting — so the videos show the
rig monitoring itself.

Frame by frame (24 fps, `tools`-less: ffmpeg crops of the hump and of the meter, in the
session's scratch), every burst start reads the same:

| time from keying | AUTO | FAST | what it is |
|---|---|---|---|
| 0 → ~250 ms | flat, no floor, no hump | same | the receiver muted at key-down; the audio has not started (100 ms keying lead + the sound card's latency) |
| ~250 ms → ~450 ms | hump | hump | the burst |
| **~450 ms, for 170–375 ms** | **blank** | **blank, 85–170 ms** | *a hole in the transmitted audio* — see §2 |
| then | hump, steady | hump, steady | the burst |

The wobble the author saw is the second row's hump appearing, vanishing and reappearing.
It is shorter on FAST because the rig's AGC recovers faster from the hole, not because the
signal differs. The rig's ALC meter climbs slowly over the first second in both videos:
the drive is set where the ALC is just active, which is where *Set drive* puts it.

## 2. The hole is real and it is the modem's

**The transmitted audio had a gap of 70–170 ms about a quarter of a second into the burst.**
Three independent measurements agree:

1. **The truck heard it.** `tools/tx_envelope.py` over the truck's OTA-2 recording of the
   base (`20260918-001658_KK4ODA-2_KK4ODA-1.wav`): the base's bursts at 4.1 s and 12.9 s
   each have a **70 ms hole 250–290 ms in**, one has another at 1.31 s, and a fifth burst
   drops 14 dB at 120 ms. The base's recording of the truck shows the same disease on the
   laptop: 50 ms at 1.19 s, 90 ms at 13.0 s. This is the "choppy" audio of OTA-2.
2. **The waveform has none.** The same tool over the model's exact drive burst (six mode-12
   frames, `model_drive.wav`): no holes, the preamble's RMS within 0.2 dB of the data's,
   the onset at full level within 20 ms (a 1 ms raised-cosine ramp and the band filter's
   9 ms delay), crest 10.2 dB at the sound card.
3. **The loop can stall for longer than the sound card was kept ahead.** `aetherd --replay`
   now times the receiver per 20 ms block. On the quiet-band recording (`busy-probe`,
   71 s, 500 Hz): **6.27× real time, slowest block 372 ms, 210 blocks over 250 ms**; on the
   OTA-2 base session: 5.04×, slowest 353 ms, 27 over 250 ms. The same quiet recording fed
   in **100 ms blocks** (`--block-ms 100`) takes 81 s instead of 444 s — 1.15× real time —
   and finds the same 26 candidates: the cost is **per call, ~115 ms**, not per sample, and
   a decode still makes a 303 ms block.

### The mechanism

The daemon runs one thread: answer commands → capture and run the receiver → top the
sound card up to **250 ms** ahead → repeat. The playback callback plays **silence when its
queue is empty, uncounted and unlogged**. The receiver's search costs on the order of 100 ms
*per call* whatever the block size (the replay's 6× real time on 20 ms blocks is that
constant cost fifty times a second; live, the loop simply grows its blocks until each pass
is one search long, ~100–125 ms, which is the 75 % of a core the daemon shows idle). A
quarter second of queue is therefore two passes of slack. A burst starts with the queue
filled to 250 ms; the next pass runs the search and, often, the decode of the candidate the
detector picked up just before keying — the other station's acknowledgement that the burst
answers, or a phantom — and takes 300–400 ms; the card runs dry for the difference. The
hole lands **~250 ms after the audio starts**, which is where the videos and the truck's
recording both put it.

The same mechanism explains the bench run where a station's audio clock fell seven seconds
behind, and it makes the ARQ's collision avoidance worse than the timing model assumes.

### What was ruled out

* **The waveform's envelope.** Preamble and data have the same power; the data's peaks
  sit 1–2.5 dB above the preamble's on 64-QAM (the clip target is 7 dB against the
  preamble's own 4.5 dB crest), 1 dB on QPSK. A receiver's AGC will not blank a display
  over that. Not changed; noted for P9-1.
* **The abrupt onset.** 1 ms of ramp is abrupt to an ALC, and the ALC meter's slow climb is
  the rig's ALC settling on a drive set at its threshold. It is a level question, not a
  modem defect, and it is what the drive-setting bursts exist to show.
* **CAT.** Yaesu keying sends `TX1;` and reads nothing back; the dial is polled at most every
  ten seconds and never while transmitting. A `record.start` during a burst would block the
  loop for up to 300 ms on the dial read — noted, not the cause here.
* **Control clients.** `publish` sends on unbounded channels; a slow panel cannot stall the
  loop.
* **Rendering.** A six-second burst renders in well under 100 ms and before the first sample
  is queued.

## 3. Mercury, as an engineering reference

Mercury (Rhizomatica, GPL; `modem/modem.c`, `audioio/audioio.c`, read in the session's scratch)
does not have this failure by construction:

| Mercury | where | prevents | Aether before | Aether now |
|---|---|---|---|---|
| renders the **whole burst** — 100 ms head silence, preamble, frames, postamble, 200 ms tail — into one buffer *before* keying | `send_modulated_data` | any dependence of the audio on what the modem does next | rendered whole, but handed over 250 ms at a time | handed over whole (ADR-0010) |
| PTT on → `tx_delay_ms` (10 ms default, up to 2 s) → **one write of the entire buffer** to the playback ring | same | an under-run mid-burst | queue topped up per pass | same as Mercury |
| holds PTT for the burst's length by an **absolute deadline** plus 100 ms, never `usleep(step)` in a loop | `tx_pacing.h` | Windows's 15.6 ms tick stretching the key by a second | released when the modem's queue drained, the card still holding a quarter second | released against the **card's own clock** |
| playback thread plays zeros when the ring is empty | `radio_playback_thread` | nothing — same as Aether; the whole-burst ring is what keeps it from mattering | same, uncounted | same, **counted and logged** |
| a tune tone is fed in 100 ms chunks with the ring capped at 600 ms so `TUNE OFF` acts within a chunk | `tune_thread` | a stuck carrier | a tone is rendered whole and cut by clearing the modem's queue | the card's queue is flushed too |
| a post-gain, pre-saturation **peak meter** | `tx_sample_with_gain` | drive set by the average | `tx_peak_dbfs` (beta.3x) | same |
| the modem's own **Hilbert clipper** and TX band-pass (codec2's OFDM) | `ofdm_hilbert_clipper` | PAPR | ADR-0004 clip-and-filter | same |
| a **postamble** | codec2 raw data modes | the receiver guessing the end of a burst | the receiver infers the end from silence | unchanged |

Mercury keeps no DSP state across bursts to speak of — each burst is rendered from a fresh
modulator call — and adds no amplitude ramp; its onset is the codec2 preamble at full level
after 100 ms of silence, the same shape as Aether's. Nothing there suggests a ramp is what
VARA has and Aether lacks; what VARA visibly lacks on this rig is the hole.

## 4. What changed (ADR-0010)

* **A whole burst is queued at the sound card the moment it is rendered.** The playback
  queue is bounded by the longest transmission (`max_key_s` + 2 s), not by a fraction of a
  second, and the loop hands everything the station has rendered over in one pass.
* **The key is released against the card's playback clock** (`AudioIo::played`), when the
  card has consumed as many samples since keying as the station handed it since keying —
  not when the station's own queue drained, which now happens at once. The keying tail
  still covers the card's own latency (`DEVICE_LATENCY_S`, 250 ms), and the engine's
  `tx_latency_s` is unchanged.
* A cut tune tone or drive burst flushes the card's queue as well as the station's.

## 5. Instrumentation kept

* `audio: the sound card ran dry for N ms inside a transmission` — a `warn` line and
  `diagnostics.audio.starved_samples`, from the callback's own count.
* `loop: a pass took N ms (commands, capture, playback)` — logged at most every ten seconds
  when a pass exceeds 250 ms, with `diagnostics.loop.{slowest_ms, slowest_phase, stalls}`.
* `[record] tx_audio = true` — every transmission's exact audio as handed to the card, as a
  32-bit float WAV under `tx/` in the recordings folder, with a sidecar carrying level,
  crest, clipping, the onset envelope at 10 ms and any holes, and one `tx:` log line.
* `aetherd --replay` prints the receiver's cost per block.
* `tools/tx_envelope.py` — the same envelope numbers for any WAV: a `tx/` capture, a session
  recording, a receiver's recording of another modem.

## 6. What to test on the radio

1. Update, run four *Set drive* bursts as before with `[record] tx_audio = true`, and film
   the scope the same way. The second blank must be gone: hump from ~250 ms after keying to
   the end, on AUTO and on FAST alike. The first flat quarter second (mute + latency) stays.
2. The log must show no `audio: the sound card ran dry` line for the session. `loop:` lines
   may still appear — they say the receiver is slow, which is true and is the next thing to
   fix (the search cost per call); they no longer mean a hole.
3. `python tools/tx_envelope.py recordings/tx/*.wav` — no holes, onset within 20 ms.
4. The decisive test is at the other end: the truck's recording of a session, through the
   same tool — no holes inside the base's bursts, and no "choppy" audio by ear. Then the
   OTA-2 numbers (mode ladder, throughput) are worth taking again.
5. The rig's ALC meter will still climb over the first second at a drive set where the ALC
   is just active; back the drive off until it barely moves, as the drive-setting notes say.

## 7. Open

* The receiver's per-call cost (~100 ms) is the underlying weakness: it should search only
  what is new. With the whole burst queued it no longer puts holes in transmissions, but it
  sets the loop's latency and it is why an idle daemon uses most of a core.
* The data's peaks exceed the preamble's by up to 2.5 dB on 64-QAM; a preamble drawn to the
  same crest as the data would present a peak-following AGC with one step instead of two.
  Cheap to try, needs a curve.
