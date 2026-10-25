# ADR-0095: A weak frame hides no strong one, and a span's last half symbol is outside it

**Status:** accepted, 2026-10-25, the author's choice of R3h with K2's miss accepted
(`docs/research/hidden-frames/01-results.md` §8). The receiver's stream only, model first, then
the port; **no wire change, no configuration key, no schema bump**. Stations on this build and on
beta.123 interoperate.

## 1. Context

The streaming receiver drops an OFDM candidate that starts inside a frame it already knows, pending
or handed out. That is right for what it was built for: a frame's own data symbols correlate with
the preamble reference, and the lookback finds the same preamble again. It also did two things it
should not have.

1. **A weak false detection hid the real frame behind it.** A noise trigger (acquisition 1.04–1.11)
   took a frame's span, and a real frame starting 0.26–0.47 s later, inside it, was never looked
   at. On the air: ND1J's header at KK4ODA-1 and three answers at ND1J and KK4ODA-1 (reports 16 §3
   and 17 §6), and two more cases the bench found in recordings already in hand. Offline decoding
   (`Modem.decode_buffer`) does not have the fault: it takes the strongest peak first.
2. **A burst's next frame was hidden whenever the last one's timing locked late.** A burst's frames
   go back to back, so the next frame starts exactly where the last one ends, and the span ran to
   that end with no tolerance. On a two-ray path whose later ray is the stronger — 2 ms, 16 samples,
   on ITU Poor — a frame's timing locks on that ray, its span reaches past the next frame's true
   start, and the next frame is hidden. Its copy a symbol late (the preamble's second symbol
   correlates too) is taken in its place and fails, and the copy's own span, a whole data frame,
   hides the frame after that. This one the bench found; nobody had seen it on the air, but three
   frames in the recordings in hand were lost to it (KK4ODA-1's sides of ND1J's Test of 10-24 and
   KQ4SAX's of 10-23), and on ITU Poor the receiver missed about one decodable frame in six or
   seven at either bandwidth.

The bench (`docs/research/hidden-frames/`, `tools/bench_hidden.py`) registered its criteria
before the runs, amended twice as the runs found the second fault, and measured seven rules.

## 2. Decision

A candidate is **inside** a known frame from a symbol before its start to half a symbol before its
end. Inside one, it is hidden — as before — unless all three hold:

* it reads at or over `TRUSTED_CONFIDENCE` (1.3);
* it is not a duplicate (more than a symbol from the known frame's start);
* the known frame reads under 1.3 and has not decoded.

A candidate taken beside a weak frame that then decodes is dropped undecoded at harvest: the weak
frame was real, so the candidate inside it was its own data.

The half symbol is bounded both ways: longer than the timing error a delay spread makes (16 samples
on Poor, 56 on ITU NVIS), shorter than the symbol by which the next frame's early sidelobe leads it.
A whole symbol let that sidelobe in where a header ends at its first data frame, and its span hid
the data frame (§4).

`TRUSTED_CONFIDENCE` is the daemon's `DETECT_CONFIDENCE_TRUSTED`, now defined as the stream's
(`aether_phy::stream::TRUSTED_CONFIDENCE`): one number for "acquisition alone is believed".

**What it removes.** The old test (any known frame hides any candidate up to its end) and the
bench's switches on the model's receiver (`weak_spans_hide`, `strongest_wins`, `end_guard`). The
bench keeps every rule it measured in a subclass of its own (`BenchReceiver`), so a later change to
this test can be measured against all of them.

## 3. What the bench measured

The rule is R3h. On the model's receiver, 20 ms blocks as the daemon feeds it:

* **The field** (15 recordings, both sides of seven Tests): every hidden frame recovered — the five
  behind false detections and the three lost to the span's end — and no frame lost; 1 104 → 1 112
  decoded on the twelve replayed under every rule.
* **Recovery** (1 200 trials: a cut preamble at 1.0–1.25, a real frame inside its span): 91.0 % at
  2300 Hz and 94.1 % at 500 Hz of the frames that decode alone, against the 95 % registered (K2);
  100 % and 99.8 % where the false candidate read under 1.3. Every miss is a false candidate the
  stream read at 1.30–1.49, which the rule trusts by definition, or a real frame read just under
  1.3. Noise does not make a candidate at 1.3: in half an hour per air the stream's noise candidates
  read at most 1.07. **The author accepted this miss** (2026-10-25).
* **Traffic** (the link's transmissions through AWGN and ITU Good, Moderate, Poor and flutter, −8
  to +20 dB, 2.7 hours of air): 133 more frames decoded of 7 467 (+2.6 % at 2300 Hz, +3.7 % at
  500 Hz; +16 % and +19 % on Poor, +6 % on 500 Hz Moderate, nothing on AWGN or Good), none lost, no
  false frame added; decode attempts +2.7 % and +1.0 %.
* **Noise** (an hour an air): candidates unchanged, 1 and 16 an hour, none decoded.

Rejected on the same runs: R1 without the guard (four losses, every one the span's end), R2 the
offline rule (three losses, 10.6 false frames per 1 000 at 500 Hz: a weak real frame's own data
symbols taken whenever they read stronger than its preamble), the guard alone (lost a frame on the
air: a second weak candidate got in through a phantom's last half symbol, and its span hid the next
real frame) and a one-symbol guard (one loss, §2).

## 4. In the port and the daemon

`core/aether-phy/src/stream.rs`: `known()` becomes `hidden()` with the rule; the frames handed out
keep whether they decoded and their confidence (`Known`); `inside_decoded()` at harvest;
`end_guard` is half a symbol. Tests in both suites build the two faults deterministically — a
control frame 0.3 s behind a cut preamble read 1.0–1.3, and two data frames back to back, the first
through two rays whose later one leads by 16 samples — and fail on the old rule.

Every recording in hand (16, both sides of eight Tests) through `aetherd --replay … --expect`,
muted where the station was keyed as on the day, on the daemon before this change (`6a82774`) and
after: **850 → 859 frames decoded, none lost**. The nine are the bench's: the five frames behind
false detections — KK4ODA-1's answers at ND1J on 10-23 and 10-24, ND1J's headers at KK4ODA-1 on
10-23, 10-24 15:55Z (with the data frame after it) and 10-24 16:00Z — and the three the span's end
had cost, two of ND1J's rung-11 frames at KK4ODA-1 on 10-24 and one of KQ4SAX's on 10-23. In
KK4ODA-1's side of WC4Y's 2300 Hz Test of 10-23 the receiver now detects six frames more, all of
them WC4Y's frames it never detected before, none decoding (§6). One recording, KK4ODA-1's side of
WC4Y's Test of 10-24, replays 30 decoded frames against 31 live under both daemons alike: older
than this change.

## 5. On the scenario harness

Two draws of all 60 scenarios (`--seeds 2`) on each daemon, before (`6a82774`) and after:

* **120 of 120 runs pass on both.** Air time 33 524 → 32 084 s (−4.3 %): the 500 Hz scenarios
  −5.4 %, the 2300 Hz ones −0.9 %. The median scenario is unchanged, and so are the Tests'
  rates (message ×1.00, file ×1.00 over 19 Test scenarios). Most of the 2300 Hz scenarios ran
  the same air to the tenth of a second on both daemons: nothing in them was hidden.
* Against the baseline's ranges (`scenario_trend.py`), five more scenarios are faster than any
  baseline run, all at 500 Hz: `20m-dx-500` (air 414 → 317 s), `80m-poor-500-pactor-signalink`
  (330 → 195 s), `80m-winlink-exchange-500` (301 → 205 s), and the Tests' rates of
  `80m-lopsided-acks-500` and `80m-slow-machine-500`. None is slower. Both daemons read
  `80m-thunderstorm-agc-2300`'s message rate as slower than the baseline at the same 311 bit/s,
  so that one predates this change. The baseline stays: being faster is not a fault, and a
  baseline takes three draws.
* **Radiated overlaps: none before, one after**, 0.32 s in `80m-flutter-500` (seed 1021). The
  stream rule did not cause it. The master's header did not decode at the slave; its first three
  data frames each said "3 or more follow" and the fourth, decoded on its own at a reported
  −5 dB, said 1 where 2 were sent. The slave took the transmission's end from that count, one
  frame early, and its answer went over the master's last frame. The frame was found at its true
  start, where the rule as built would also have found it, and the count comes from the PHY's
  chip phase, which this change does not touch. A count read wrong from a frame that decoded on
  its own is rare: none in 9 984 such frames on both daemons' runs, none in 1 238 on the model's
  flutter at +6 to +12 dB (OFDM mode 7 at 500 Hz, the scenario's rung). Ten reruns of the draw on
  each daemon did not repeat it. The frames before it had already said the transmission ran at
  least to its true end. A rule that never takes an end earlier than such a bound would have kept
  the slot here. That is a protocol rule (R2 in `docs/research/link-architecture/04-formal-spec.md`),
  so it is left to the author.

## 6. Consequences

* More frames decoded on delay-spread paths, and some frames now **detected and failed** that were
  never detected before: six of the 26 frames KK4ODA-1 "never detected" in WC4Y's 2300 Hz Test of
  10-23 (report 15) were hidden by the old rule, and fail under the new one (they were weak). Under
  granted slots a frame that does not decode counts only on its transmission's stated grid, so the
  rate controller sees them as the review's instruments do: as failures, not as nothing.
* The stream announces the recovered frames (`take_preambles`), as it announces every frame it
  takes. Inside a session an announcement changes nothing (ADR-0069 §2.5).
* A real frame behind another station's signal read at 1.3 or more stays hidden, as before.
