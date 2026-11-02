# Gate 0: today's baseline

*In progress (2026-11-02): §4 and §5 are written; §1–3 are filled in as the runs finish.*

The brief (`00-brief.md` §4, Gate 0) asks for the evidence on today's build before any design:
the oracle again, today's controller pair on every scenario, the NVIS trace, the field sessions
since the first review, and the count K7 will be judged against. Nothing in the controller
changed for it.

**The build.** The daemon is `887a9e7` (beta.125 and ADR-0098). The deletion of C2 and the
cautious start (`4c425e3`, the author's answer 3) came after it and changes nothing the default
runs: the granted vectors and C1's vector sessions regenerate identical.

## 1. The oracle

*(pending: draw 0's coarse rungs are running.)*

## 2. Today's pair on every scenario

*(pending.)*

## 3. The NVIS trace

*(pending: three draws of each scenario are running.)*

## 4. The field record since the first review

No session has run C1 on the air (`00-brief.md` §1.3): it became the default in beta.124, and
every upload since the first review's reading came from beta.122. Of the 2026-10-25 uploads, three
pairs have both sides; KQ4SAX's four sessions with WG4PTP are calls that never connected (one
side, two control frames heard). Read with `tools/rung_reading.py --tree` at `v0.2.0-beta.122`,
`--replay`:

| session | air | data | what the old controller did |
|---|---|---|---|
| 03:30Z Winlink, KK4ODA → KK4ODA-1 | 2300 Hz | none | advice rung 5 both ways, nothing sent |
| 03:42Z Winlink, KK4ODA → KK4ODA-1 | 2300 Hz | 1 + 3 frames at rung 5 (tone100-153), all alone | — |
| 03:49Z Test, KK4ODA-2 → KK4ODA-1 | 500 Hz | message 108, file 101 bit/s at rung 4 | below |

**The Test** is a new instance of the first review's *start decides* and *the dip*, on the old
controller as the advice. The slave read the master's floor frames at −3 to −1 dB and
recommended rung 4 (tone4x100-93) for every exchange of the transfers, 198 s delivering 109
bit/s. Each burst with one frame of four lost widened its margin by the share lost (ADR-0084),
6.0 → 8.2 dB over the session, so it never asked for more. Its own ladder then measured rung 5
(QPSK ½) 4 of 4, rung 9 (16-QAM ½) 3 of 4 at +1.4 dB: the best neighbour would have delivered
+19 %, the best measured rung (9) 221 bit/s, twice what the session ran at. The ladder's OFDM
frames read +0.3 to +1.8 dB where the floor's read −1.3 to −2.4 on the same path, a family
offset of about 2.5 dB, which is what C1's family offset (`family_gain_db`) learns and the old
controller does not. The controller replayed reproduces all 17 of the slave's recommendations
and its state at every step, as the first review's nine sessions did.

## 5. The count for K7

What decides a rung today, in the model (`rate.py`, `rate_review.py`, `granted.py`) and the
port beside it.

**The old controller**, `RateController`, which is still the advice: seventeen numeric tunables
(`margin_db` 3, `min_margin_db` 1.5, `max_margin_db` 12, `up_step_db` 1.5, `max_jump_db` 3,
`down_step_db` 0.25, `decay_growth` 2, `max_down_step_db` 1, `decay_every` 3,
`up_hysteresis_db` 1.5, `up_dwell` 1, `max_up_step` 2, `first_mode_back` 2, `partial_share`
0.5, `echo_weight` 0.4, `echo_min_snr_db` −6, `reseed_margin_db` 3), one constant in code (the
SNR smoothing, 0.3 a burst), and three tables (the AWGN thresholds, the echo limits, the floor's
size).

**C1**, `DeliveryController`, which chooses the sender's own rung: seven tunables of its own
(`half_life` 12, `prior_frames` 2, `probe_every` 6, `probe_wait_max` 48, `slope_db` 0.5,
`prior_margin_db` 3, `family_gain_db` 0.5). It is a subclass of the old controller and inherits
the SNR smoothing, the echo reading (`echo_weight`, `echo_min_snr_db`, the echo limits) and the
thresholds, so it runs on ten. The model keeps a bench-only prior, the four fading channels'
mean (`prior_margin_db = None`, with the class penalty tables), rejected in the first review and
never ported.

**The engine's rules around them** (`GrantConfig` and `granted.py`): the silence step (R9,
`silence_step` 2), the HARQ limit (`max_combines` 4), the early re-encode (`reencode_early_after`
2, `reencode_hopeless_db` 1), `hold_unknown`, `floor_answer_memory` 4, the header kept on data
above the advice (`header_above_recommendation`), ADR-0020's counting (`SELF_DECODABLE_RVS`, the
rungs asked for), and the stranded frames' targets (`_reencode_target`, `_split_rung`, bounded by
the peer's advice and the sender's own choice). Seven tunables and five rules.

**In all**, 17 + 7 + 7: thirty-one tunables (C1's inherited three counted once, with the old
controller's), against K7-firm's eight.

**Code**, lines that are not blank, comments or docstrings:

| | model | port (before its tests) |
|---|---|---|
| the old controller | `rate.py` 266 | `rate.rs` 418 |
| C1 | `rate_review.py` 291 | `rate_review.rs` 476 |

and in the engine, twelve methods that exist for the controllers or the advice (`_price`,
`_heard`, `_burst_mode`, `_recommend`, `_back_off`, `_hopeless`, `_reencode_target`,
`_split_rung`, `_observe`, `current_mode`, `echo`, `_move`; 136 lines in the model), and parts
of eight more (the session's reset, the call and acceptance, the slot's close, the header, the
data frames, the acknowledgement).
