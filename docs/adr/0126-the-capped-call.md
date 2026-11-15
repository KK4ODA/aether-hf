# ADR-0126: The capped call — a call in control frames, chosen by what was heard, over in 20 s

**Status:** accepted for building, 2026-11-15 — the revision of §7, the author's decision on its
model bench ("results accepted, go ahead"), its one narrow miss (§7.3) accepted. Link protocol 13.
Its first design (§2.1's call frame, the codes, the details at the head of the data: §2–§6) was
benched and failed R2 and R3 (§6); it is removed from the model and lives in the history at
`343be84`. The capped call is in the model behind `GrantConfig.capped_call`, off by default; next
are the port, the harness's A/B (§8) and the air, before it becomes the default and link protocol
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
