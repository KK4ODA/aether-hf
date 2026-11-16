# ADR-0126: The capped call — a call in control frames, chosen by what was heard, over in 20 s

**Status:** accepted for building, 2026-11-15 — the revision of §7, the author's decision on its
model bench ("results accepted, go ahead"), its one narrow miss (§7.3) accepted. Link protocol 13.
Its first design (§2.1's call frame, the codes, the details at the head of the data: §2–§6) was
benched and failed R2 and R3 (§6); it is removed from the model and lives in the history at
`343be84`. The capped call is in the model and the port behind `GrantConfig.capped_call`, off by
default, with the number of tries a setting (§7.4). The harness's A/B (§8) ran on 2026-11-16 and
failed H2–H5 with a narrow miss on H1 (§8.4): three causes, each with a change for the author to
choose, before the A/B is run again; then the air, before it becomes the default and link protocol
13 ships.

## 1. Context

The calling bench (`docs/research/calling/00-plan.md`, `01-results.md`) began with the author's
question whether Aether's call was slow. It is, in two ways:

* **A good path connects in 11.8 s.** The first try and its acceptance are both floor data frames
  of 5.36 s (ADR-0016). An ordinary try listens 6.8 s for an acceptance that may come on the floor
  (ADR-0056).
* **A call nobody answers holds the channel for 205 s** on average: eight tries, about 87 s of
  tries and listening and 118 s of random wait. Other stations that hear it hold their own calls.
  The author: the operator, or the program, can always call again; a call that nobody answers
  should give the channel back within about 20 s.

What the harness showed (`01-results.md` §3): the random wait matters only when two stations that
cannot hear each other start calling one station within about one try's length of each other, and
a wait shorter than a floor try does not separate floor tries anyway. The ordinary tries are what
reach a station tuned 150–300 Hz off (the tone detector searches ±150 Hz) and survive a carrier on
the floor's 400 Hz.

The control frames reach as far as the frames the call uses, and are shorter. Measured, 2 300 Hz:

| frame | length | 90 % decoded, AWGN | holds |
|---|---|---|---|
| OFDM control | 0.43 s | −5 dB (threshold −5.1; 500 Hz −4.5) | 7 bytes |
| OFDM data, robust (today's ordinary call) | 1.05 s | about −5 dB | 19 bytes and more |
| floor control (tone-control) | 3.2 s | −19 dB (threshold −19.5) | 7 bytes |
| floor data (tone-24, today's floor call) | 5.36 s | −19 dB (threshold −19.0) | 24 bytes |

On the fading channels the floor control frame decodes 1–2 dB deeper than tone-24
(`bench/baselines/tone_floor.csv`). Its cost is room: seven bytes, against the call's nineteen.

## 2. Decision

### 2.1 The call is a control frame

A **call** is a control frame whose session field is 0 and whose kind is HEADER; an
**acceptance** is one whose session field is 0 and whose kind is ANSWER. No session's control frame
has a session field of 0 (`_new_session_number`), so the two are told apart from every other
control frame by that field alone.

```
CALL        0: kind HEADER (2) | count (3) | data_floor (1) | level (2)
            1: 0 (7) | fam (1)
            2: bandwidth (2) | turnaround (4) | called code bits 18–17 (2)
            3–4: called code bits 16–1
            5: called code bit 0 (1) | caller code (7)
            6: session (8)

ACCEPT      0: kind ANSWER (2) | 0 (3) | 0 (1) | level (2)
            1: 0 (7) | 0 (1)
            2: 0 (1) | 0 (1) | SNR (6)
            3: bandwidth (2) | version (6)
            4: caller code (7) | 0 (1)
            5: session (8)
            6: turnaround (4) | identifier (4)
```

* **count**, **data_floor**: frames that follow this one in its transmission, and their family
  (a both-ways try's OFDM part says 1, floor). As in every control frame, with them a receiver
  that reads this frame and nothing after it still knows where the transmission ends.
* **level**: in a call, the gap level of the acceptance's slot (it rises every second try, as
  today); in an acceptance, the level the called station needs.
* **fam**: the caller listens for an acceptance on the floor after this try. Clear: OFDM only.
* **bandwidth**: the sender's air (the capability byte's bits 1–2, ADR-0026).
* **turnaround**: the sender's declared turnaround, 0–1.5 s in 0.1 s steps (`[radio]
  turnaround_ms`).
* **called code**, **caller code**: the low 19 bits, and bits 19–25, of the CRC-32 of the packed
  callsign (`pack_callsign`). A station answers a call whose called code is one of its callsigns'
  (it computes one for each it answers to). A station hears a call meant for another and takes it
  for its own about once in 524 288 calls it hears; §2.4 catches it.
* **session**: the whole session number, 1–255.
* **SNR**: how the called station read the call, as every control frame says it (ADR-0021).
* **version**: the called station's link protocol. A caller that reads another version gives the
  call up and says so.
* **identifier**: the called station's closing Morse identifier, 0–7.5 s in 0.5 s steps
  (ADR-0075).

A protocol-12 station reads a call as a control frame of no session of its own and ignores it; a
protocol-13 station that hears a protocol-12 call (a CONNECT_REQ data frame) ignores it and says
"calls with link protocol 12", as for any other version today.

### 2.2 Three kinds of try

| try | sent | length | the caller listens for | the called station answers |
|---|---|---|---|---|
| **quick** (C) | one OFDM control frame | 0.43 s | an OFDM acceptance | in OFDM |
| **floor** (S) | one floor control frame | 3.2 s | a floor acceptance | on the floor |
| **both** (H) | an OFDM control frame and a floor control frame, back to back | 3.6 s | either | in OFDM if it read the OFDM part at least `call_answer_margin_db` (6 dB) over the OFDM control threshold, on the floor otherwise |

A called station that reads both parts of a both-ways try answers once. It answers in the slot the
try grants, after the try's last frame, as every answer does. The 6 dB rule replaces ADR-0056's
12 dB, which was set for a 1.05 s data acceptance; at the threshold an OFDM acceptance is often lost
on the way back, and the floor one is not.

### 2.3 The plan: by what was heard, and no more than two tries that reach a weak path

The caller chooses the call's tries from the last time it heard the station it calls (the daemon's
heard list, `heard.rs`: the SNR of the last frame read from that callsign, if within 15 minutes):

| last heard | tries | a call nobody answers ends |
|---|---|---|
| not in the last 15 minutes | H H | 16.5 s |
| at least 6 dB over the OFDM control threshold | C C C S | 15.6 s |
| weaker | S S | 15.6 s |

There is **no random wait** between tries: the next try goes when the last one's slot can no longer
be in use. When two stations call each other at once, the higher callsign keeps calling, as today.
A call that goes unanswered ends after its last try's slot (`no answer`), and the operator or the
program calls again.

### 2.4 The call's details cross at the head of the session

A call carries codes, not callsigns. The caller's details — today's connect body
(`ConnectBody`: both callsigns, capabilities, version, turnaround, identifier length, 19 bytes) —
are the first 19 bytes of the caller's data stream in the session, sent, acknowledged and repeated
like any data. The called station takes them off the stream before it delivers anything:

* the called callsign is one of its own, and both codes match: the session goes on, and the host
  program is told `CONNECTED` now;
* otherwise the session ends at once ("a call for another station").

The details cost the session its first exchange before the callsigns are known: estimated at one
data frame, measured on the model bench at a median 1.9 s on a good path, 8.9 s on a weak AWGN
path and 18.5 s on a weak fading one, where the first data frame often goes at a rung the path
refuses and goes again (§6).

### 2.5 What stays

Probes, beacons and datagrams are as today. The slot rules (ADR-0069), the levels, a caller waiting
past a frame it hears arriving, "called again" (ADR-0056 §4: a call from the session's own peer
under a new session ends the session and is answered; under its own number, the acceptance is sent
again) and the identifier after a call that came to nothing (ADR-0055, ADR-0075) are unchanged.

## 3. Alternatives considered

* **Today's call with a shorter wait (W1).** Passed the bench's contention test; changes nothing
  on a good path (11.8 s) and leaves a call nobody answers at about 108 s (`01-results.md`).
* **Every try on the floor (F1).** Failed the contention test: 5.36 s tries with at most 5.36 s
  between them keep meeting.
* **A quick start without a protocol change (B, E).** Data-frame tries: good path 3.2 s, nobody
  answers 84–97 s (the fast-start candidates shown to the author on 2026-11-15). The control
  frames do the same in less air.
* **Every try both ways, eight of them (H).** Reaches every path on every try; 73 s unanswered.
  The cap keeps two of them.
* **Quick tries, then the floor (G).** The fastest good path (1.9 s) when nothing is known, but a
  weak path waits for the quick tries first. Kept as the opening when the station was heard strong.
* **A new tone frame of about 12 bytes for a call with full callsigns.** Would spare the details'
  frame, at about 4 s a floor try; a new tone kind, its sync patterns and its own false-detection
  run. Not taken: the details ride in the session for one frame.
* **Shorter codes, or no caller code.** The acceptance must name the call it answers among others on
  the channel: session number and caller code together (15 bits).

## 4. Consequences

* Link protocol 13. Protocol-12 stations cannot connect to it and do not hear its calls.
* A good path connects in about 2 s (heard strong) or 5 s (not heard); a weak path in about 8 s;
  a call nobody answers gives the channel back in 15.6–16.5 s, against 205 s.
* At the very edge of the floor (about −16 dB and below on a fading path) one call reaches less
  often than today's eight tries; a second call reaches more often than today's one (§5, R2).
* A called station knows who called only after the first data frame; a host program hears
  `CONNECTED` that much later. A `PENDING` it hears when the acceptance keys, as today (ADR-0103).
* Two stations that cannot hear each other and start calling one station within about one try's
  length (3.2–3.6 s) of each other on a weak path can both fail; each calls again later.
* **Removed:** the random wait (`call_wait_frames`, `_call_jitter`), the alternation
  (`_connect_floor`), ADR-0056's 12 dB floor acceptance (`floor_answer_margin_db`), the eight tries
  (`connect_retries`), and the connect request and acceptance as frames on the air (DataKind
  CONNECT_ACK is free; CONNECT_REQ's body is the details' format).
* **Added:** the call and acceptance layouts (§2.1), three kinds of try, the plan from the heard
  list, `call_answer_margin_db`, the details at the head of the stream.

## 5. How it is judged (registered before the runs)

On the model bench, `tools/bench_calls.py` (the fading pipe: AWGN, Good, Moderate, Poor; both airs;
−20 … +8 dB; 30 calls a cell), against today's call (`--wait growing`, alternating tries). A call
counts as connected once both stations are in the session and, for the capped call, the called
station has the details.

* **R1, nobody answers.** A capped call that nobody hears ends within 20 s of its first try, for
  each plan, on both airs.
* **R2, reach.** (a) From −14 dB up, in every cell, one capped call with nothing heard (H H)
  connects at least as often as today's call, less two calls of 30. (b) In every cell, two capped
  calls (the second after the first gives up) connect at least as often as today's one.
* **R3, speed.** In every cell where both connect at least 25 calls of 30, the capped call's median
  time to connect is lower than today's.
* **R4, the informed openings.** With the station heard at the path's SNR, C C C S (where that
  reading is strong) and S S (where weak) each connect at least as often as H H less two calls, and
  no slower at the median, in every cell from −14 dB up.
* **Reported, not judged:** a station heard strong (+10 dB) on a path that has gone weak (C C C S
  at −8 … −20 dB); each plan's keyed time.

A narrow miss goes to the author with its trace. Owed after the decision: the port (frames, engine,
vectors), the daemon's heard list feeding the plan and `CONNECTED` after the details, the host
adapter, the harness's contention and offset tests, and the air.

## 6. What the model bench found (2026-11-15)

`docs/research/calling/02-capped-call.md`, 2 880 calls an arm, both airs:

| criterion | result |
|---|---|
| R1 | passes: 16.5 s (H H), 15.6 s (S S), 15.7 s (C C C S); today 135–253 s |
| R2 (a) | fails: Good −14 dB, 24 of 30 against 30 |
| R2 (b) | fails: Good −16 … −12 dB, by 1–2 calls |
| R3 | fails in 50 cells: callsigns known at 16.5 s (AWGN) to 26.1 s (fading) under 0 dB, against today's 11.5 s |
| R4 | narrow miss: one cell 0.2 s slower |

The call alone is faster than today's in every cell (both stations in the session at a median
7.6 s, 4.8 s on a good path, 1.6 s when heard strong; today 11.5 s) and reaches more often at the
floor's edge on Moderate and Poor. What fails is the details step (§2.4) and, on Good's slow fades,
two tries in 16 s. The results note proposes, unbuilt: the callsigns kept in the call (a both-ways
try of today's two call frames, answered by §2.1's short acceptance: estimated 7.9 s good path,
10.7 s weak path, 22 s unanswered), the details at the most robust rung if they stay in the session,
and a silent pause between the two tries on slow fades.

## 7. Revision: the callsigns in the call, and a pause (registered 2026-11-15, before its runs)

The author's decision on §6: build and bench the revision that keeps the callsigns in the call, with
the pause between tries. It changes §2.1, §2.2, §2.3 and §2.4; the rest stands.

### 7.1 What changes

* **The call is today's connect request, in data frames.** Each try carries the full connect body
  (19 bytes, version 13) in the frames today's call uses: the robust OFDM data frame (1.05 s) and
  tone-24 (5.36 s). There are no codes and no details step: the called station knows who called
  from the call, and tells its host program `CONNECTED` at the acceptance, as today.
* **The acceptance stays §2.1's control frame** (ACCEPT: 0.43 s in OFDM, 3.2 s on the floor),
  matched by session number and caller code as in §2.1.
* **The three kinds of try**, now in data frames:

  | try | sent | length | the caller listens for | the called station answers |
  |---|---|---|---|---|
  | quick (C) | the OFDM data frame | 1.05 s | an OFDM acceptance | in OFDM |
  | floor (S) | tone-24 | 5.36 s | a floor acceptance | on the floor |
  | both (H) | the OFDM data frame, then tone-24, back to back | 6.4 s | either | in OFDM if it read the OFDM part 6 dB over the OFDM control threshold, on the floor otherwise |

  The OFDM part of a both-ways try states one frame to follow (its count, ADR-0041); a call frame
  that states one to follow is followed by tone-24, so a station that reads only the OFDM part
  knows where the try ends.
* **The plans are §2.3's** (H H; C C C S; S S), with **a silent pause of 15 s before the plan's
  last try** (`call_pause_s`). The caller neither keys nor holds a slot during it; it is there so
  that two tries do not fall in one slow fade.

Estimated from the engine's timing (no random wait, the pause left out of channel time):

| plan | good path | weak path | channel time, nobody answers | ends |
|---|---|---|---|---|
| H H | 7.9 s | 10.7 s | 22.0 s | 37.0 s |
| C C C S | 2.6 s | 9.6 s + 8.7 s of quick tries | 19.6 s | 34.6 s |
| S S | 9.6 s | 9.6 s | 19.9 s | 34.9 s |

H H's channel time is 2 s over the author's 20 s. A shorter both-ways try needs a shorter call
message, which is what §2 tried and §6 measured.

### 7.2 How it is judged

The same bench and arms as §5 (`tools/bench_calls.py`, both airs, AWGN/Good/Moderate/Poor,
−20 … +8 dB and a cell nobody hears, 30 calls a cell, a second call after a failed one), against the
same `today` runs. A call counts as connected once both stations are in the session: the callsigns
are in the call.

* **R1′, nobody answers.** In the cell nobody hears, every call's channel time — from its first
  try to the end of its last try's listening, the pause left out — is at most 22.5 s, and the call
  ends within 40 s of its start, for each plan, on both airs.
* **R2, R3, R4** exactly as §5.
* **Reported, not judged:** the pause at 0 s and at 30 s on the arm with nothing heard (R2 on Good
  is what the pause is for); the stale arm (heard strong at +10 dB on any path).

A narrow miss goes to the author with its trace.

### 7.3 What the model bench found (2026-11-15)

`docs/research/calling/02-capped-call.md` §5:

| criterion | result |
|---|---|
| R1′ | passes: channel time 22.1 s (H H), 20.0 s (S S), 19.7 s (C C C S); over at 34.7–37.1 s |
| R2 (a) | passes |
| R2 (b) | narrow miss: Good −16 dB, two calls 27 of 30 against today's 29 (both airs); three draws today connected only at 45–100 s |
| R3 | passes: 10.4 s (H H), 9.3 s (S S) on weak paths, 7.6 s and 2.2 s on good ones, against 11.5 s |
| R4 | passes |

The pause brings Good's reach near today's (−16 dB, one call: 25 against 20 without it); 30 s gains
nothing over 15 s. On Moderate and Poor at −18 dB the first design reached more often (its floor try
was the floor control frame, about 2 dB deeper on fading than tone-24). A station heard strong on a
path gone weak waits about 34 s (C C C S, then the pause, then its one floor try).

Owed before an acceptance: the narrow miss to the author, the harness's contention and 200 Hz
offset tests (the port first), and the air.

### 7.4 The number of tries is the operator's (the author's request, 2026-11-15)

The author asked for the number of tries to be a setting the operator picks in a profile in Setup:
**2 by default, or 6, 8, 10 or 15.** With N tries the plans are:

| what was heard | plan |
|---|---|
| not heard lately | H × N |
| heard strong (6 dB over the OFDM control threshold) | C C C, then S × (N − 1) |
| heard weak | S × N |

The pause comes before the last try only, whatever N. A call nobody answers, from the engine's own
timing (the same on both airs, the pause left out of channel time):

| tries | H × N: channel, ends | C C C + S × (N − 1) | S × N | keyed (H × N) |
|---|---|---|---|---|
| 2 | 22.1 s, 37.1 s | 19.7 s, 34.7 s | 20.0 s, 35.0 s | 12.8 s |
| 6 | 69.1 s, 84.1 s | 65.5 s, 80.5 s | 62.8 s, 77.8 s | 38.5 s |
| 8 | 95.2 s, 110.2 s | 89.4 s, 104.4 s | 86.7 s, 101.7 s | 51.3 s |
| 10 | 121.2 s, 136.2 s | 113.4 s, 128.4 s | 110.7 s, 125.7 s | 64.1 s |
| 15 | 186.3 s, 201.3 s | 173.2 s, 188.2 s | 170.5 s, 185.5 s | 96.2 s |

Even fifteen tries end before today's eight (about 205 s). R1′ judges the default; a larger number is
the operator's choice to hold the channel longer, and the manual says so beside the setting.

What is built now: `GrantConfig.call_tries` in the model and the port, and the granted vectors'
`capped-six` session, which holds the port to six tries. What waits for the release that makes the
capped call the default, because a new configuration key ships with a schema bump in a release
(CLAUDE.md): the key (`[radio] call_tries`, a portable setting, so in profiles; options 2, 6, 8, 10,
15), the configuration schema step, Setup step 4's *Call tries* list, and the manual's words, table
row and screenshot.

## 8. The harness's A/B (registered 2026-11-16, before its runs)

### 8.1 What the daemon does with it

Built before any run, and nothing else in the daemon changes:

* **What was heard.** The station keeps, for every callsign a decoded frame named as its sender,
  the station time and the SNR of the last one (`heard_lately`, `CALL_HEARD_S` = 15 minutes), and
  a call goes by it (`connect_heard`). It is the station's memory since it started, not the heard
  list's file, so a daemon restarted calls as to a station not heard; the harness starts every
  station fresh, so its Tests (which probe first) call as heard, and its other calls as not heard.
* **ADR-0058's move to 500 Hz.** A 2 300 Hz call to a station not known to run 2 300 Hz goes on
  at 500 Hz after four unanswered tries. A capped call of two tries never has a fifth, so the move
  comes before the plan's last try when the plan has fewer than five: after
  min(4, tries − 1) unanswered tries, in the silent pause. With two tries, a call whose first
  both-ways try went unanswered makes its second at 500 Hz; a 2 300 Hz station that hears it
  answers it by moving to 500 Hz (ADR-0026). That costs a session at 500 Hz where a fade took the
  first try, against today's four tries at 2 300 Hz; `40m-wide-calls-narrow-unprobed` is the case
  it keeps.

### 8.2 The arms and the runs

* **T**, today's call: the branch as committed with this section.
* **C**, the capped call: the same tree with `capped_call: true` in `GrantConfig::default`
  (`core/aether-link/src/granted.rs`, one line, never committed), so every daemon calls with two
  tries and answers only capped calls.

Both built `--release`; `tools/session_matrix.py --jobs 2 --drop-passing-audio`, each arm on the same
draws. One row a run goes to `docs/research/calling/data/capped-T.csv` and `capped-C.csv`.

### 8.3 Criteria

* **H1, nobody answers (judged on C).** New scenarios `40m-nobody-answers-2300` and
  `80m-nobody-answers-500` (a call into an outage: the called station hears nothing), 3 draws each:
  in every run the caller's tries (its transmissions while calling) are two, they radiate 13.5 s
  at most, and the last ends within 33 s of the first's start (§7.4: two both-ways tries of 6.4 s,
  one listening and the 15 s pause). T is reported.
* **H2, the calls that are retried today.** K3's eight scenarios (`00-plan.md` §4), 3 draws each
  (variants 0–2): (a) runs passed, C no fewer than T, and no invariant broken in C that T kept;
  (b) radiated overlaps pooled, C no more than T + 2; (c) the calls' time pooled (each call from
  its `CONNECT` to its session up, a call that did not connect counted at the scenario's
  `connect_within_s`), C ≤ T.
* **H3, two hidden callers that do not start together.** New scenarios
  `40m-gateway-hidden-callers-after-N-2300` for N = 3, 7, 12 and 18: K2's scenario with the second
  client calling N seconds of air after the first, 3 draws each: (a) runs with a session within
  240 s, C no fewer than T; (b) the median time from the first call to the first session, C ≤ T;
  (c) radiated overlaps pooled, C no more than T.
* **H4, 200 Hz off.** `40m-offset-200-2300` (K4), 3 draws: C connects in every draw T connects.
* **H5, no harm elsewhere.** Every scenario in `bench/scenarios/` (not `long/`), one draw
  (variant 0): (a) runs passed, C no fewer than T less one, and any scenario C fails that T passed
  is run on two more draws on both arms and reported; (b) no invariant broken in C that T kept;
  (c) radiated overlaps pooled, C no more than T + 2.
* **Reported, not judged:** K2 as registered (`40m-gateway-hidden-callers-2300`, 10 draws), where
  both clients dial at the same second. With no random wait both clients' tries meet at the
  gateway every time, so C is expected to connect neither; the author has judged two callers
  starting together not a case that happens on the air. Programs that connect on a clock (every
  hour at :00) are where it could, and the result goes to the author with that.

A narrow miss goes to the author with its trace, and does not pass by rule. The harness's whole-
second air clock and its scanning Trimode stand-in (`01-results.md` §3.1) apply to both arms.

### 8.4 What the harness found (2026-11-16)

`docs/research/calling/02-capped-call.md` §6 has every table; one row a run is in
`docs/research/calling/data/capped-T.csv` and `capped-C.csv`.

| criterion | result |
|---|---|
| H1 | narrow miss: two tries, 12.9 s radiated, both airs; the span 36.1 s at 2 300 Hz against 33 s (32.8 s at 500 Hz), the 3.2 s the busy detector held the try §8.1's move made; today 8 tries over 212–215 s |
| H2 | fails: (a) 22 runs against 24 and (c) 1 560 s against 1 522 s, both `80m-vox-caller-learned-gap-500` (3 calls lost); (b) 1 overlap against 0. The other seven: 641 s against 1 333 s |
| H3 | (a) 12 against 12 and (b) a median 10.5 s against 22 s pass; (c) fails, 58 overlaps against 35 (the hidden loser's two tries and its closing identifier over the winner's session) |
| H4 | fails: 11 s, not connected, 11 s against 78, 23, 23 s |
| H5 | fails: 62 of 64 against 64 (`80m-band-closes-2300`, the VOX scenario); no invariant, no overlap; air 0.987 of today's at the median, first calls 11 s against 15 s |
| K2 (reported) | 2 of 10 draws with a session against 10 of 10 |

The failures have three causes: §8.1's move to 500 Hz before the last try (H1, H4 and
`80m-band-closes-2300`: a 2 300 Hz station reads only the tone floor of a 500 Hz call); the 0.43 s
OFDM acceptance, lost whole in a VOX radio's hold and through an NVIS echo, with two tries too few
to teach the gap (H2, H5); and no random wait (K2, H3 (c)). The changes they point to — no move for
a call of fewer than five tries, a try heard again answered on the floor, a pause made a little
random — are the author's call; whichever are taken go into the model first, and the A/B is run
again on the same draws.

## 9. Second round: three changes (the author's decision, 2026-11-16; registered before its runs)

The author, on §8.4: "make all three fixes and rerun the test".

### 9.1 What changes

1. **No move to 500 Hz for a short call.** §8.1's move before the last try is withdrawn. A capped
   call moves as ADR-0058 has it — after four unanswered tries — only when it makes five or more
   (the tries setting at 6, 8, 10 or 15); a call of four tries or fewer stays in its bandwidth. A
   500 Hz station that hears a 2 300 Hz call's floor still tells its operator (ADR-0035).
2. **A both-ways try heard again is answered on the floor.** A called station in the session its
   acceptance opened that reads the same call's both-ways try again (the same session number) knows
   its acceptance was lost, and answers on the floor, which a both-ways try's caller listens for: the
   floor's acceptance (3.2 s) outlasts a VOX hold and comes through an echo. A quick try heard again
   is still answered in OFDM, the only family its caller listens for; a floor try's always was on the
   floor.
3. **The pause is random.** Before the plan's last try the caller waits 15 s and a uniform part of
   up to that try's length (6.4 s for a both-ways try, 5.4 s for a floor try, 1.05 s for a quick one),
   from the engine's draws, so that two callers who met on one try seldom meet the same way on the
   next. The tries before the pause keep no wait.

### 9.2 How it is judged

The criteria of §8.3, on the same draws, with one change: **H1's span** is the last try's end within
**39.5 s** of the first's start (§8.3's 33 s and up to one both-ways try of random pause); its two
tries and 13.5 s radiated stand. The arm is **C2**: the tree with these changes and `capped_call: true`
in `GrantConfig::default`. **T's runs of §8 are reused**: none of the three changes does anything with
`capped_call` off (the move's rule is read only for a capped call's plan). One row a run in
`docs/research/calling/data/capped-C2.csv`. H3 (c) — the hidden loser's overlaps — is aimed at by
none of the three and is expected to fail again; it is judged as registered.
