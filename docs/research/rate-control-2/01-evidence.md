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

The two nightly scenarios and `80m-nvis-disturbed-500`, three draws each (the nightly's three
variants: seeds *s*, *s* + 1000, *s* + 2000), today's daemon, read from both sidecars with
`tools/rung_reading.py`. All three are ITU NVIS at 500 Hz: two rays 7 ms apart, where the floor's
tone rungs carry and OFDM's 5 ms prefix does not.

All nine passed this time; the nightly's failures are the slow tail of the same distribution.

| run | transfer (budget) | OFDM: exchanges' air | sends | first copies | decoded alone | by combining | floor's exchanges' air |
|---|---|---|---|---|---|---|---|
| agc-carrier *s* | 219 s (300) | 51 s | 23 | 6 | 0 | 3 | 173 s |
| agc-carrier *s* + 1000 | 201 s | 46 s | 23 | 6 | 0 | 1 | 160 s |
| agc-carrier *s* + 2000 | **277 s** | **108 s** | 39 | 12 | 2 | 7 | 173 s |
| nvis-disturbed *s* | 210 s (—) | 46 s | 13 | 6 | 4 | 2 | 160 s |
| nvis-disturbed *s* + 1000 | 180 s | 0 | — | — | — | — | 184 s |
| nvis-disturbed *s* + 2000 | 206 s | 63 s | 22 | 6 | 0 | 4 | 147 s |
| trimode, the send *s* | 216 s (400) | 0 | — | — | — | — | 212 s |
| trimode, the send *s* + 1000 | 202 s | 0 | — | — | — | — | 199 s |
| trimode, the send *s* + 2000 | **273 s** | **75 s** | 29 | 9 | 1 | 3 | 194 s |

In the Trimode runs the gateway's own replies (120 B each) went on OFDM as well: its first data
of the client's first session at rungs 5–6 in all three draws (26–39 s of exchanges, 1–4 frames
of 11–17 decoded alone), and its reply in the second session of draw *s* + 2000 (69 s, none
alone).

**What happened.**

1. **The first data of a session goes on OFDM.** In seven of the nine runs, and in the gateway's
   replies in all three Trimode runs, the sender's first data transmission went at rung 5 or 6,
   a whole transmission of four to six frames. It was C1's first choice from the call's reading
   (`DeliveryController.first_mode`): its prior is the AWGN curve 3 dB under the reading. The
   floor's reading on this path is −2 to +1 dB (it saturates near there on a 7 ms echo,
   ADR-0087) though the path is at 10 dB. From such a reading the prior rates the first OFDM
   rungs best: rung 5 at −2 dB, rung 6 at −1 and 0 dB, rung 7 at +1 dB (computed from the
   500 Hz engine's own `_delivery_controller()`; at 0 dB rung 6 is valued 21.8 against the
   floor's top 13.6 and rung 2's 7.3). The echo limits (`NARROW_ECHO_LIMIT`) refuse only rungs
   3–4, so nothing keeps OFDM out (`02-survey.md` §2).
2. **Those blocks then take their copies there.** The first copies were followed by two to
   three more transmissions of the same blocks at the same rung (RV 0, 2, 3), each a 13–14 s
   exchange, before the blocks still missing were re-encoded onto the floor (ADR-0085). Combining
   rescued some (1–7 a run); frames decoded alone were 0–4. No block was split into pieces in
   these nine runs.
3. **The slow draws went back.** The two slowest runs (277 s and 273 s) each made a second visit
   to OFDM later in the session (128–210 s and 104–228 s): C1's try across the floor's edge. The
   second visit alone cost 50–60 s of exchanges.
4. **The advice was right.** The receiving station's recommendation, from the old controller,
   stayed at rungs 0–2 throughout every run: it climbs one rung at a time, and with rungs 3–4
   refused by the echo it never reaches OFDM. On this path the controller that ships as the
   sender's choice is the one that goes wrong.

**What it cost.** The OFDM air delivered 31–243 B a run. At the floor's own rate in the same run
(40–45 bit/s of exchange air), those bytes need 6–43 s, so the OFDM visits cost about 30–65 s a
run in which they occurred. Without them the slowest message (277 s) would have taken about
210 s, inside K8's 240 s.

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
