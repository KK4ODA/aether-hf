# ADR-0054: Each station's answer gap is learned from its asking again

**Status:** accepted, 2026-10-09. Daemon only (`station/gaps.rs`: `LearnedGaps`; `Station::answer_gap_s`,
`learn_gap`, `hand_over`; the stations-heard list's `answer_gap_ms`). No wire change, no configuration key.

## 1. Context

`[radio] answer_gap_ms` (ADR-0036) holds every transmission until a gap after the last frame heard, for
a station whose radio is keyed by VOX: its interface keeps the transmitter on, deaf, for its DLY time
after the audio stops, and an answer keyed into that hold is never heard. It is one number for every
station. KE4QCM's FT-100 (VOX, a USB codec, `ptt = none`) lost KK4ODA-1's probe answers and an
acceptance on 2026-10-09 with KK4ODA-1's gap at 500 ms, while the other testers need none: a number
long enough for his hold costs every exchange with everyone else.

The same morning his computer fell seconds behind its audio (189 slow passes, 27 s once, 21.6 s of
audio dropped; the receiver costs 0.08–0.10× real time here on noise and under every QRM model). A gap
cannot help that: the stalls come at random. The heartbeat (beta.95) now says which a slow pass is.

## 2. Decision

1. **A station's gap is learned from the one sign that the answer to it was lost: it asks again.** A
   caller whose call was accepted calls again — and each try the station answers with its acceptance
   again and is not heard says so again; a station whose probe was answered probes again within a
   minute (`PROBE_AGAIN_S`, `CALL_AGAIN_S`). Each raises that station's gap by `STEP_S` (250 ms), to
   `MOST_S` (2 s), and is logged: *answer gap for KE4QCM now 750 ms: it called again after its call was
   accepted*.
2. **A session whose first acceptance was heard eases it by `EASE_S` (25 ms)**, a tenth of a step, so a
   gap learned on one bad evening fades over ten clean sessions. A session that needed a second
   acceptance eases nothing.
3. **The gap used is the larger of the operator's and the learned one** of the station answered: the
   other end of the session, or — idle — the last station to call or probe this one.
4. **The learned gaps are kept in the stations-heard list** (`answer_gap_ms`, by base callsign: the gap
   belongs to the radio, whatever SSID it calls under) and seeded at start, as bandwidths are.

A repeated call is not only ever a VOX hold — an acceptance may simply fade — but a step costs a
quarter second on that one station's exchanges and nothing on anyone else's, and clean sessions take it
back.

## 3. Measured

The scenario harness, `80m-vox-caller-learned-gap-500` (the caller's radio holds 900 ms after its audio,
deaf, with 100 ms of recovery; the called station's gap 0; five sessions of a call, 200 bytes and the
caller's disconnect), four seeds:

| build | passing | air | collisions |
|---|---|---|---|
| beta.94 | 0/4 (most calls never connected in 240 s) | 2400 s (ran out) | 53+ |
| learned, easing 100 ms | 1/4 (the 4th session lost its message in three) | 386–730 s | 24–34 |
| **learned, easing 25 ms** | **4/4, 20/20 messages** | **313–345 s** | **15–17** |

In every seed the gap reaches 1000 ms within the first call (four lost acceptances) and eases to 900 ms
over the next four sessions. At 100 ms a session it fell to 700 ms by the fourth, under the hold.

## 4. Not done

* The VOX station saying its own hold in its calls: only it knows its DLY, and that is the complete
  answer, but it needs a protocol change and the operator to enter the number.
* A gap for a station that only ever receives from this one: nothing it does says an answer was lost
  until it asks again.

## 5. Amendment (2026-11-18): asking again counts from the answer on the air

Report 19 §7 (`docs/research/link-architecture/19-radiopost-p2p-40m-beta164.md`): the house learned
a longer gap for `WD5EMA` at each of the mobile's both-ways calls (ADR-0126), 75 → 325 → 575 ms over
two sessions. A both-ways try is accepted at its OFDM part, and the acceptance keys once its tone part
has ended. The tone part is decoded after the acceptance was decided and before it went out, and
`called` read it as the caller asking again: the decode time against the decision.

Now each answer is watched from the moment it is keyed. A call or probe says the answer was lost only
when it began on the air after that keying, within `CALL_AGAIN_S` or `PROBE_AGAIN_S` of it. An answer
decided and never keyed (the rules refused it, its slot passed) says nothing.

* `LearnedGaps::aired(Answer, at_s)`: the first keying after the decision is the answer's.
  `called` and `probed` take the frame's start on the air (`SoftFrame::t_start`), not the time it was
  decoded.
* The station notes what each rendered transmission answers (`answer_in`: an acceptance, a
  `CONNECT_ACK`, a `PROBE_ACK`; `Station::answer_rendered`) and tells the gaps when it keys it.
* Removed: counting a repeat from the decision.
* Tests: `a_call_begun_before_the_acceptance_went_out_did_not_miss_it` and
  `an_answer_never_keyed_was_never_missed` (`gaps.rs`), and
  `a_both_ways_try_heard_whole_teaches_the_called_station_no_gap` (`station.rs`, two stations over
  real audio; it fails on the rule before).

No wire change, no configuration key. A gap already learned this way is eased by clean sessions, as
any other.
