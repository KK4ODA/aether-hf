# ADR-0113: The daemon reaches its physical layer through one boundary

**Status:** accepted, 2026-11-13, the author's agreement to the boundary below; built, and step
10.1's gate passed, 2026-11-14 (§7, which also records what changed in the gate before it ran).
This is the Aether FM proposal's FM-2 (`docs/research/aether-fm/00-proposal.md` §B, §I, §L). The
author's decision 2 settled that it is done now, in HF first, and proven unchanged. No wire
change, no model change, no configuration key, no vector regenerated.

## 1. Context

Aether FM is a second air in this repository (ADR-0112). It needs a daemon that can run a
physical layer other than HF's. Today `aetherd` reaches into `aether-phy` directly: 129
references across 14 files, 43 of them in `station.rs`.

What the station holds and calls, read from the tree at `e0a13ef`:

| Part | Today | What the station calls |
|---|---|---|
| Audio to baseband | `AudioToBaseband` (`from_audio`) | `process` every captured block; `offset_hz`/`set_offset_hz` (ADR-0079) |
| Receiver | `StreamingReceiver` | `feed`, `take_preambles` |
| Decoder | `Rc<RefCell<Modem>>`, shared with every `PhyFrame` | `decode_received`, inside `SoftFrame::decode` |
| Transmitter | `Modem` | `rung_burst_following`, `control_burst_of` |
| Baseband to audio | `BasebandToAudio` (`to_audio`) | `process`, `flush` |
| Numbers | `WaveformParams` in `StationConfig` | `audio_rate`, `fs_baseband`, `bandwidth`, 53 times |
| The ladder | `modes::air_interface(params)` | rung sizes, frame lengths, names: the Test, the panel, `capabilities` |
| The engine's numbers | `phy_timing(params)` in `station.rs` | built once per air |
| Channel sense | `BusyDetector`, fed HF's baseband | about 30 calls: `skip`, `push`, `hear_passband`, `explain`, `mark_frame`, `busy`, `settled`, `retune`, readings |

Outside the station, the bandwidth move (`station/bandwidth.rs`) rebuilds all of these. The
offline tools (`replay.rs`, `busy_trace.rs`, `passband_trace.rs`) and `capabilities` build them
too. `config.rs` maps `[radio] bandwidth` to `WaveformParams`.

The link layer is already free of it. `aether-link` has no compile dependency on `aether-phy`
(only its tests use it). Its boundary is `PhyTiming` and `SoftFrame` (`aether-link/src/phy.rs`,
mirrored by `model/aether_model/link/phy.py`). HF's second family is already optional there:
every `floor_*` field is an `Option` or a count that can be 0, and `echo` and `follows` default
to `None`. So the proposal's "a generic `PhyTiming`/`SoftFrame`" needs no rename. What is open
is whether the engine runs well with no second family at all. That is FM-7's question, with
FM's numbers, and is not part of this step.

## 2. Decision

### 2.1 The boundary is at audio samples

The station hands the air the sound card's audio and gets audio back. Baseband stays inside the
air. HF's baseband is 8 kHz complex around 1 500 Hz. FM's band differs (the voice path
450–2 850 Hz, the flat port 300–5 100 Hz; proposal §D.4), and so does what it runs at. Audio is
the one thing every air shares.

### 2.2 One trait object per air

The station holds `Box<dyn Air>`, built from the configuration. HF's implementation is `HfAir`,
in `aetherd/src/air/hf.rs`. A sketch; names may change in the code, but the split may not without
amending this ADR:

```rust
pub trait Air {
    // what the air is
    fn info(&self) -> AirInfo;                 // its name, audio rate, signal band, bandwidths
    fn timing(&self) -> PhyTiming;             // the link engine's numbers (HF: today's phy_timing)
    fn ladder(&self) -> &[RungInfo];           // per rung: payload bytes, frame seconds, family, name

    // receive
    fn hear(&mut self, audio: &[f32], how: Listening, now: f64) -> Heard;

    // transmit
    fn render(&mut self, frames: &[TxFrame]) -> Result<Rendered, RenderError>;
    fn flush(&mut self) -> Vec<f32>;

    // channel sense: is someone else on the frequency, and the panel's reading of it
    fn sense(&self) -> &dyn ChannelSense;
    fn sense_mut(&mut self) -> &mut dyn ChannelSense;

    // abilities only some airs have
    fn receive_offset_hz(&self) -> Option<f64> { None }
    fn set_receive_offset_hz(&mut self, _hz: f64) -> bool { false }
}
```

- **`Listening`** says what the station knows and the air does not. It says whether this block
  is the station's own transmission, and if so how many of its audio samples came in after the
  transmission ended (today's `captured_after_transmission`). It also says whether channel sense
  is deaf (`deaf_until`). HF's air scales the count to baseband with today's formula and mutes in
  baseband as today: muted is not paused.
- **`Heard`** holds what the receiver found: arrivals (start, length, family, confidence) and
  frames, each a `PhyFrame` the engine decodes through the air that heard it. HARQ stays the
  engine's. Times are seconds on the station's clock, so the station no longer needs
  `fs_baseband`. When a block is heard in two pieces (the muted part, then the rest, with the end
  of the transmission between them), the air hands back what each piece found. The station then
  feeds the engine in today's order: the first piece's frames, the end of the transmission, the
  second piece's frames.
- **`Rendered`** is the burst's audio and the first frame's share of it, as `modulate` returns in
  baseband today. Each frame's countdown (ADR-0041, ADR-0068 §2.1) stays the air's business.
- **`ChannelSense`** is the busy detector's interface as the station uses it: busy, settled,
  reason, the reading, the level and floor, `explain` and `mark_frame` for heard frames, `retune`
  after a dial move, `state`/`restore` for recordings and traces, the threshold. HF's air owns
  its `BusyDetector` and feeds it the baseband it made, exactly as the station does now. FM-8
  decides FM's channel sense (squelch, activity) behind the same trait.

### 2.3 What stays in the station, and becomes air-neutral

These read audio and frame spans only, so they stay where they are and take their numbers from
`AirInfo`:

- the receive-filter reading (`passband.rs`, ADR-0105): the signal's band, the air's longest frame;
- the clipping reading (C1), the keyed-audio watch, the spectrum, the meter, recording and the
  preroll;
- the constellation, which takes a heard frame's equalised symbols when it has any.

### 2.4 What stays HF's

- **Bandwidth moves** (ADR-0026, 0035, 0058, 0101, 0102): a move builds a new `HfAir` for the
  other bandwidth and carries over what carries today (the busy detector's levels, the
  receiver's origin). An air that runs one bandwidth lists one in `AirInfo`, and the station
  refuses a move it cannot make.
- **Following the peer's offset** (ADR-0079): the two optional methods. On an air without them,
  `centre.rs` does nothing.
- **The busy detector** (ADR-0078, 0088): HF's channel sense, unchanged.

### 2.5 The tools and the rest of the daemon

`replay.rs`, `busy_trace.rs`, `passband_trace.rs`, `capabilities` and the configuration build an
air instead of its parts. `config.rs` still maps `[radio] bandwidth` to HF's parameters. FM's
`[radio] air` key comes with the FM air (ADR-0112 §2.4), not in this step.

## 3. What does not change

The wire, the link engine, the model, every vector file, the configuration's schema, the control
API, the host and KISS ports, the recordings' format, and what an operator sees. No test's
assertion changes. A test may change only in how it builds a station or a PHY part.

## 4. The gate (step 10.1)

Registered before the work. A failing check stops the step. It is explained before anything
changes, as ADR-0069's steps were.

1. `cargo test --release --workspace` and both clippy runs pass, with no assertion changed.
2. `tools/check_vectors.py` passes with nothing regenerated.
3. **Offline identity, the decisive check.** First a corpus is frozen: the recordings of one
   harness run of every scenario, made by `master`'s daemon, plus any field uploads in hand.
   Both builds then replay it with `aetherd --replay`, `--busy-trace` and `--passband-trace`. The
   outputs must be identical line for line; only the replay's timing columns may differ. This is
   the check that can see a changed sample. The harness cannot: one build repeats only a few of
   its runs to the second, because its steps are taken on the wall clock.
4. **The harness.** Every scenario at three draws, judged by `tools/scenario_trend.py` against
   `bench/baselines/scenarios.csv`: every scenario passes as on `master`, no radiated overlap, no
   step flagged.
5. **Cost.** The replay's cost per block stays within 5 % of `master`'s on the same machine. A
   call through a trait object per 20 ms block should cost nothing measurable, and this says so.

## 5. Alternatives considered

- **A type parameter, `Station<P, A: Air>`.** Static dispatch and nothing else gained. Every
  signature and test helper in 13 000 lines would carry it. Calls per block are too few for
  dispatch to matter.
- **The boundary at baseband.** It would fix FM to HF's 8 kHz complex baseband around 1 500 Hz,
  which the flat port's 300–5 100 Hz band does not fit.
- **The busy detector left in the station, fed by the air's baseband.** It would keep HF's
  baseband in the station's hands and give FM a detector it cannot use. Channel sense is the
  air's, as the proposal's architecture has it.
- **Renaming the link's `floor` to a generic family in this step.** Model and port churn, vectors
  rewritten, and nothing gained: the fields are already optional. FM-7 decides what one family
  needs of the engine.
- **Doing it when FM's physical layer exists.** The refactor would then land with FM code
  depending on it, and HF's testers would meet both at once. Decision 2 put it first, under HF's
  full suite.

## 6. Consequences

- What it adds: `aetherd/src/air/` (the traits, `HfAir`) and an `Air` per station.
- What it removes: the station's direct hold on HF's physical layer. The 43 references in
  `station.rs` and the rest of the 129 move into `HfAir`, or go through it. `phy_timing` moves
  into `HfAir`. `StationConfig.params` gives way to the air's `AirInfo`.
- After it, adding an air means implementing `Air` and building it from the configuration. No
  change to the station is needed beyond refusing what the new air cannot do.
- The model has no daemon, so this step is port only. Model first still holds for everything
  the step does not touch.

## 7. As built (step 10.1, 2026-11-13/14)

Built in three commits on the working branch: HF's parts gathered into `HfAir`; the block taken by
the air and the busy detector moved into it; the station on `Box<dyn Air>`. The split is §2.2's.
What differs from §2–§4, and why:

1. **The gate's decisive check was replaced before it ran.** §4.3 as registered replayed a corpus
   through `aetherd --replay`, `--busy-trace` and `--passband-trace`. Those tools drive HF's
   physical-layer parts directly and never run the `Station`, so they would have passed whatever
   the refactor did. This step changed none of them, nor anything they call (`replay.rs`,
   `busy_trace.rs`, `passband_trace.rs`, `busy.rs`, `passband.rs` and the three library crates
   are byte for byte as on `master`, and so are the two items of `station.rs` they use,
   `reported_cfo` and `CAPTURE_LAG_ALLOWANCE_S`), so their outputs are identical by construction. The check
   that does see the station is `core/aetherd/examples/air_identity.rs`: two real stations through
   a seeded channel in ten sessions (clean, weak and under the OFDM rungs on both airs; a 500 Hz
   call that moves a 2 300 Hz station; a 60 Hz offset the receiver follows; an abort; a probe and a
   beacon; a Test), printing every event, frame report, keying tag, delivery and received byte and
   a running hash of all audio played. It uses only the public interface both trees share.
   **Result:** `master` repeats itself byte for byte; `master` and this tree print identical
   output at 4 096-sample blocks (660 lines) and at the daemon's 960 (915 lines).
2. **§4.5's cost check measured the wrong thing for the same reason** (the replay never runs the
   station). It was taken on the identity driver instead, at the daemon's 960-sample blocks,
   the two builds run in turn three times on an idle machine: `master` 66.62, 66.27 and 65.95 s,
   this tree 66.40, 65.88 and 65.24 s (×0.993).
3. **The offline tools and `capabilities` stay HF's (§2.5).** They read HF recordings and report
   HF's ladder, and they are the referee the identity of everything below the station rests on.
   They get an air when there is an FM recording to read and an FM ladder to report (step 10.4).
4. **What is still HF's in the station:** `StationConfig.params` remains the configuration's choice
   of HF waveform (which air to build, and a bandwidth move's target), and the two confidence
   gates the station applies to a heard frame (`DETECT_CONFIDENCE_TRUSTED`,
   `MODE_RETRY_CONFIDENCE`) are HF's constants. The FM air states its own when FM-4 fixes its
   acquisition; they then move into `AirInfo`. Every other read of HF's numbers goes through the
   air (`info`, `timing`, `ladder`). Outside its tests `station.rs` now names `aether_phy` on
   these three lines; it named it on 14 before.
5. **One placement changed in a case the corpus never produced.** A frame is now placed and judged
   by the receiver that heard it. Before, when a narrower call moved the station to 500 Hz in the
   middle of a block, a later frame finished by the old receiver in that same block was placed
   by the new receiver's origin and judged against the new air's tables. That needs two frames
   to finish in one 20 ms block, the first a call that moves the station; the identity runs move
   a station and saw no difference.
6. **The harness (§4.4).** Every scenario at three draws on this tree's daemon, two jobs, as the
   nightly runs them: 192 runs, no radiated overlap. `scenario_trend.py` against
   `bench/baselines/scenarios.csv` flags no scenario slower, and 63 of 64 pass all three draws.
   The 64th is `80m-agc-carrier-outside-500`, a 1 000-byte message in 300 s at 500 Hz on ITU NVIS,
   which the notes keep at its budget's edge: it passed draw 31 and missed 1031 and 2031. Each
   missed draw was run twice more on each build. On draw 1031 both builds failed once and passed
   once, the passing runs identical (372.1 s of air, the message in 275 s). On draw 2031 the same,
   the passing runs identical (274.1 s, 177 s). The harness takes its steps on the wall clock, so
   one draw has two courses here, and `master` takes both as this tree does. The scenario
   behaves as on `master`.
