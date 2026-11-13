# ADR-0113: The daemon reaches its physical layer through one boundary

**Status:** proposed, 2026-11-13. This is the Aether FM proposal's FM-2
(`docs/research/aether-fm/00-proposal.md` §B, §I, §L). The author's decision 2 settled that it is
done now, in HF first, and proven unchanged. This ADR is the design. It is accepted when the
author agrees the boundary below, and done when step 10.1's gate (§4) passes. No wire change, no
model change, no configuration key, no vector regenerated.

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
