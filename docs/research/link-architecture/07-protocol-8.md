# Protocol 8: granted slots, as it will be built

`04-formal-spec.md` specified granted slots for a session's data exchange. This document is
the rest of the design: everything the daemon needs from a link engine today, decided before
the model engine is written. It covers:

* the frame formats, bit by bit;
* the timing constants and formulas both stations compute alike;
* connection, probes, ending, abort and the left session;
* the Morse identifier;
* the master's scheduling policy;
* the engine's interface against `LinkEngine`'s.

It is the brief for Step 1. When the model engine settles, the parts that are wire format go
into `docs/spec/air-interface.md` §7 (generated tables where they are numbers).

The rule from ADR-0069 §5 governs every choice here: **safety must not depend on anything a
lost or late frame can change**, and a rule that moves a deadline on undecoded evidence is not
allowed.

## 1. Names

* **Master** is the station whose call was accepted (the caller). **Slave** is the called
  station. The roles are fixed for the session.
* **Transmission** is everything one station radiates between key-up and key-down.
* **E** is the end on the air of a transmission's last frame, as the receiver places it: the
  end of a decoded frame, plus the count it states times the frame length of the stated
  family (S1 in `04-formal-spec.md`).
* **Slot** is the interval in which the slave may radiate (§3).
* **Family** is ordinary (OFDM) or floor (tone).

## 2. Constants

These are protocol constants, the same at both ends. They are not configuration. A station
slower than they assume asks for a gap level (§3.3).

| Name | Value | What it covers |
|---|---|---|
| `D_ORD`, `D_FLOOR` | 0.30 s, 0.30 s | from a frame's end on the air to its decode reaching the engine; measured on the harness (§11), checked again in hardware before freezing |
| `KEY` | 0.35 s | from the engine's decision to radiation: the daemon's lead (0.1 s) and the card (0.25 s); the daemon renders a slot's data frames ahead (§10) |
| `GUARD` | 0.05 s | propagation (≤ 20 ms one way on HF), clock error over a transmission, rounding |
| `SLACK` | 0.35 s | how late a slot may start on the air and still be used; the keying gate drops past it |
| `LEVEL` | 0, 0.5, 1.0, 2.0 s | extra gap by level (§3.3) |
| `MAX_DATA` | 6 | data frames in one transmission (also capped by the key limit) |
| `WINDOW` | 16 | selective-repeat window (unchanged) |
| `OPEN_WINDOW` | 4 s ordinary, 10 s floor | an idle link's open window, doubled once while idle |

## 3. Timing

### 3.1 The slave's slot

After a master transmission ending at `E`, whose last frame is in family `f`, at gap level
`L` (from the current header):

```
start(E) = E + max(D_f + KEY, T_master) + GUARD + LEVEL[L]
end_max  = start + SLACK + grant
grant    = ans(fam) + n · frame(slave rung)    or    window(index)
```

`T_master` is the master's declared turnaround (§4.3): its radio's VOX hold and receiver
recovery. It is in the max because the slave needs `D_f + KEY` to decode and key, and the
master needs `T_master` to hear again. Both must have passed.

The slave arms its keying for `start − KEY`. Past `start − KEY + SLACK` it forfeits. Its
keying gate refuses radiation that would begin after `start + SLACK`. This is the safety rule
(S2), and it holds whatever held the transmission back.

### 3.2 The master's next key

After a decoded slave frame of the current slot stated its end `S`:

```
key ≥ S + max(D_f, T_slave − KEY) + GUARD
```

The master must have decoded the answer, and its radiation, `KEY` later, must not land in the
slave's turnaround. If the master decoded nothing of the slot, it keys at
`end_max + D_slowest + GUARD`, where `D_slowest` is the decode budget of the slowest family the
answer may use. The master's own keying latency only delays its own transmission, because the
slave anchors on `E` as it hears it. It never affects safety.

### 3.3 Gap levels

* **The slave asks for a level in every ANS.** It asks for the level its measured lateness
  needs: decode lateness past `D_f`, plus its keying latency past `KEY`. It eases one level
  after six on-time slots.
* **The master grants `max(asked, silence level)`.** Two silent slots in a row raise the
  silence level by one; four slots answered in time lower it by one.
* **Calls step the level.** The acceptance slot's level rises every second try: capability
  bits 3–4.

### 3.4 Where the master's end is measured

The master's engine places `E` at the end of the last *frame* on the air, not at key-down. Today
the daemon calls `on_tx_done` at key-down, a tail later. Protocol 8 adds
`on_tx_done(frames_end)`, with the end the daemon computes from the card's clock less the tail
(§10). A later `E` would be safe (the slave's slot would start earlier than the master
assumes), but it would waste the tail every exchange.

## 4. Frames

### 4.1 The control frame (CONTROL container, 7 bytes, 56 bits)

| Bits | Field | HDR (master) | ANS (slave) | DISC / DISC_ACK |
|---|---|---|---|---|
| 2 | kind | 0 | 1 | 2 / 3 |
| 7 | session | the session number's low 7 bits | same | same |
| 3 | count | data frames after this header, exact (0–6) | data frames after this answer, exact | 0 |
| 1 | data family | those frames are floor frames | same | — |
| 2 | level | the gap level of this slot | the level this slave needs | the level of the DISC_ACK slot |
| 1 | fam | the family the answer must use | the family the slave needs headers in | — |
| 1 | flag | the slot is an open window | the slave asks to leave | — |
| 1 | id | the slot includes the slave's identifier time | the slave's identifier is due | — |
| 8 + 16 | base, bitmap | selective acknowledgement | selective acknowledgement | — |
| 6 | snr | how this station hears the other: −31 … +31 dB, `0x20` unknown | same | same (ADR-0021) |
| 5 | rung | rung recommended for the other's data | same | — |
| 3 | n | frames granted, or the window index | frames queued, capped at 7 | — |

Total 56 bits. The **SNR goes from 8 bits to 6**: a 3 kHz reading above +31 dB adds nothing a
rate controller can use, and below −31 dB nothing decodes. That frees the identifier bit.
**The session field is 7 bits.** Data frames keep their 8-bit session byte, so a foreign
control frame is taken for this session's at most one time in 128. Even then it grants a slot
the foreign master's own timing reserves, not this one's.

### 4.2 Data frames

The DATA container's header is unchanged: kind, flags, sequence, session, length. Each data
frame states how many frames of its transmission follow it, 0–3 (ADR-0068 §2.1): the OFDM
chips' quarter turn, or the tone frame's middle sync block. Three means *three or more*: a
transmission of six data frames has three frames that state 3. Its end is known from the header
(`count`) or from any of its last three frames (0, 1 or 2 follow). A header is left out only
when at most four data frames follow and one of them is an RV 0 copy (R7, ADR-0073), and then
too only the last three state the end exactly; a receiver that reads only the first holds no
anchor and does not answer. A count is believed only from a copy that decoded alone (R2), and
RV 1 and 2 copies never do: without the RV 0 condition a transmission of them granted nothing
by construction.

### 4.3 The connect body (CONNECT_REQ, CONNECT_ACK): 19 bytes

| Bytes | Field |
|---|---|
| 0–6, 7–13 | source and destination callsigns (unchanged) |
| 14 | capabilities: bit 0 compression; bits 1–2 bandwidth; **bits 3–4 the level of the acceptance slot** (a request) |
| 15 | version: **8** |
| 16 | SNR the request arrived at (an acceptance; unchanged) |
| 17 | **timing**: bits 0–3 turnaround `T` in 0.1 s (0–1.5 s); bits 4–7 reserved, zero |
| 18 | **identifier**: the sender's Morse identifier length in 0.5 s units, 0 for none |

Nineteen bytes fit the robust mode of both families on both airs, with an explicit length:
24-byte rung 0 on the floor, 25–26 bytes ordinary. A body shorter than 19 bytes is
version 7 or older, and is refused with an `ignored` event naming the version, as today.

### 4.4 The probe body (PROBE, PROBE_ACK): unchanged, 16 bytes

The probe's capability byte gains bits 3–4, the level of the answer's slot, as a call's does.
The answer goes in that slot or not at all.

## 5. A session

### 5.1 Connection

* The caller sends CONNECT_REQ:
  * tries alternate floor and ordinary, the floor first (ADR-0016);
  * the level rises every second try;
  * a try is repeated at `end_max(acceptance slot) + D_FLOOR + GUARD + jitter`, never earlier;
  * the jitter is `uniform(0, (1 + tries) · floor frame)`.
* The called station answers in the request's slot, or forfeits. **No acceptance goes late.**
* The caller becomes master on reading the acceptance.
* The master's first transmission:
  * its first header goes in the family the acceptance's SNR asks for;
  * the master's rung is `first_mode(snr)`;
  * the first grant is two frames, so a gateway can greet.
* **Simultaneous calls** (each calling the other): the station whose callsign sorts lower
  yields and answers. This is today's rule.
* **Called again** (ADR-0056): a call from the current peer under a new session number ends
  the session, at either end, and is answered as from idle.
* Version, bandwidth or callsign mismatches are `ignored` events, as today. The daemon's
  mismatch banner reads them unchanged.

### 5.2 The exchange

This is `04-formal-spec.md` R1–R12. The master's policy for each transmission:

1. **Own data:** up to `MAX_DATA` frames within the key limit (`set_max_burst_s`), at its rung.
   This means retransmissions first, at the next RV, then new frames.
2. **The grant:**
   * if the slave said it has frames queued, `min(queued, MAX_DATA, fit)`, where `fit` is
     what fits the key limit at the slave's recommended rung;
   * otherwise an answer-only slot;
   * an open window when neither side has anything (R12).

   When the operator asks for a **break** (`request_break` at the slave), the slave's ANS sets
   `n` to its queue and the master grants it in full before sending more of its own.
3. **Header elision** (R7).

The slave's transmission:

* **What it sends:** an ANS, then up to `n` data frames at its own rung (the master's
  recommendation, capped by the slave's ceiling and pinned rung).
* **Content without a header:** if the master's header was not read, the slave takes level 0
  and an answer-only slot, in the sticky family (R6). Its anchor `E` comes from the last four
  frames' exact counts. With no anchor, it does not transmit.

### 5.3 Ending

* **A master's disconnect:**
  * it waits until its own queue is acknowledged, or for patience:
    `max(20 s, 2 exchanges)` without progress (ADR-0039), and never longer than
    `disc_drain_s` (60 s) from the request (ADR-0072), saying what it leaves;
  * then it sends DISC, which grants a DISC_ACK slot;
  * it ends on the DISC_ACK, or after three DISCs ("closed (no DISC_ACK)").
* **A slave's disconnect:** it sets `flag` in its next ANS. The master then sends its DISC
  once what the slave still had is delivered; its own queue is left undelivered, since the
  slave's operator has stopped receiving (ADR-0023's receiving station). Patience and the
  drain's minute bound it as above, the minute counted from the ANS the master read the `flag`
  in. (Step 2 found the first wording, "as above, with the slave's queue counted too",
  keeping a weak session going for minutes of the master's queue after both operators had
  pressed Disconnect: `09-step-2.md` §2.2.)
* **Abort at the master:** it sends DISC at its next key opportunity (after the open slot's
  `end_max`), on the floor, up to three times without waiting for an answer, then ends.
* **Abort at the slave:** it sends DISC in its next granted slot, if one comes within the
  daemon's wind-down, and ends either way. A slave never keys outside a slot, even to leave.
  A master that hears nothing more ends by `max_retries` silent slots.
* **The left session** (ADR-0052):
  * a slave that left answers a header of its old session with DISC, in that header's slot;
  * a master that left has nothing to answer, because the slave keys only in slots.

  The left session is remembered for a link timeout, as today.
* **Link timeout:** four exchanges of the size in use, not the longest possible. That is two
  transmissions of the current grant sizes at the current rungs, plus control frames and gaps,
  with a minimum of 45 s. It is re-reckoned at each header the slave reads. A slave that hears
  nothing for it ends "link timeout".

### 5.4 The Morse identifier

**Since ADR-0075 (the author's decision, after v0.2.0-beta.108):** a station identifies once
per session, at its end, and the identifier is the last thing each station sends for it. Nothing
inside a session carries one, and no frame is sent after one. The identifier length each station
declares (connect body byte 18) is the one its own callsign needs, 0 for none.

* **A closing exchange** (either station's disconnect; R10):
  * the master's DISC grants the DISC_ACK's slot with the slave's identifier time added, as it
    always has;
  * the slave sends its DISC_ACK and its identifier right behind it, in that slot;
  * the master keys its own identifier alone once that slot is over: at the DISC_ACK's end plus
    the slave's declared identifier and R4's wait, or at `end_max` if no DISC_ACK was read.
* **A DISC heard again** is answered again, identifier and all, so the identifier stays last.
* **The slave starts nothing of its own** (a call, a probe, an answer to another station) until
  the master's identifier can be over: the slot's latest end, the master's wait at the top level
  with a slow key, and the master's declared identifier.
* **A slave's disconnect** (`flag`) is answered by the master's DISC, so it closes the same way.
* **An abort.** An aborting master identifies after its last leave DISC's slot, or after the
  DISC_ACK and the identifier behind it. An aborting slave's DISC is answered by the master's
  closing DISC, so both identify.
* **No closing exchange** (a link timeout, the master gone, a DISC_ACK never sent): each station
  sends its identifier on its own, listening first (`Transmit.listen_first`: the daemon waits for
  a clear channel up to 15 s, then sends). The master's goes after its last slot, and listens
  first when it read nothing of the slave at the end ("no response", a link timeout, every DISC
  unanswered). A slave that timed out sends it at once. A slave that
  left a session still heard waits one exchange first, since the master's next words may close
  it properly. A DISC_ACK that carried the identifier and did not go out (the gate dropped it,
  the header was cut) owes it again, the same way.
* **Called again** (ADR-0056): the session's identifier is dropped, since the communication goes
  on in the new session; a call to another station sends the identifier first.
* **Outside a session:** calls, acceptances, probes, probe answers, beacons and datagrams carry
  no identifier. A contact that went no further than its setup is the daemon's to identify, on
  its own, 30 s after (ADR-0055 §4).

Every rule here keys only where I1 already holds: in a slot the other station granted and
reserves, after a slot the master granted is over, or on a channel the station hears clear. No
deadline moves on undecoded evidence.

**Wire compatibility:** none of this changes a frame. A protocol 8 station of v0.2.0-beta.106 to
beta.108 sends its identifier ahead of its DISC_ACK, inside the same granted time, and grants the
same slot for one. The master's wait (the DISC_ACK's end plus the declared identifier) covers an
identifier on either side of it.

The engine places the identifier; the daemon decides whether the station has one and how long it
is (`set_identifiers`, per callsign) and renders it after the frames (`Transmit.identifier_s`).
This replaces ADR-0055's placement rules for sessions and the mid-session identifier
(`set_identifier_due`, the `id` bit in an ANS, which a station on this build still honours from
an older one).

## 6. Outside a session

* **Probes** (ADR-0006, ADR-0016, ADR-0035):
  * a PROBE grants a PROBE_ACK slot, at the level in its capability bits;
  * the answer goes in the slot or not at all;
  * the prober waits to `end_max + D_FLOOR + GUARD`, then reports "no answer";
  * probes across bandwidths are answered as today.
* **Beacons and datagrams** are the daemon's, unchanged. The engine ignores `OUTSIDE_SESSIONS`
  kinds.
* **Carrier sense:** the daemon's `wait_for_clear` still holds calls, probes, beacons and
  datagrams. Nothing inside a session waits for the channel: a slot is used or forfeited.

## 7. Rate and family

* **The receiver recommends the rung in every control frame, and the sender follows** it,
  capped by `set_ceiling` (regulatory), `max_mode` and a pinned rung (the Test ladder).
* **The family of control frames** comes from the peer's report:
  * the floor unless the report clears the ordinary control threshold by
    `answer_floor_margin_db` (3 dB; swept 0–3 dB, `08-steps-0-and-1.md` §2.5);
  * the floor after two silent slots;
  * back to the report on the next slot heard.
* **Silence steps the rate down.** A data slot heard silent steps the sender's rung down by
  `silence_step` (2). Two silent slots step the recommended rung for the slave too.
* **Unchanged from the model:**
  * the re-seed from the floor's lower bound (ADR-0016);
  * the trusted-failure rules (ADR-0020);
  * the hopeless re-encode (ADR-0038);
  * the RV sequence (ADR-0043).
* **The Test ladder:**
  * `pin_mode(rung, body_bytes)` pins this station's own data, at either end;
  * `take_ladder` reports rungs as today;
  * `top_acked_mode` is kept.

## 8. The engine's interface

The model engine is `aether_model.link.granted.GrantedEngine`, beside `LinkEngine`. The port
mirrors it in Step 2, and the daemon switches to it there. The daemon's present uses of
`LinkEngine`, with their fate:

| Kept, same meaning | Changed | Dropped (no counterpart) |
|---|---|---|
| `connect_as`, `connect_tries`, `connect_due`, `move_call` (ADR-0058), `probe`, `probing`, `last_probe`, `send`, `disconnect`, `disconnect_requested`, `abort`, `tick`, `next_deadline`, `on_frame`, `drain`, `state`, `session`, `connected`, `callsigns`/`set_callsigns`, `peer_capabilities`, `peer_snr_db`, `ended_peer_snr_db`, `rate_readings`, `current_mode`, `robust_mode`, `timing`, `config`, `set_air`, `set_max_mode`, `set_ceiling`/`ceiling`, `set_max_burst_s`, `burst_capacity`, `pin_mode`, `take_ladder`, `top_acked_mode`, `tx_pending_bytes`, `tx_undelivered_bytes`, `all_acknowledged`, `reset_stats`, `stats`, `on_tx_delayed` | `on_tx_done(frames_end)` (§3.4); `role()` returns `Master`/`Slave`/`None`; `Transmit` gains `not_after` (the gate's bound, §3.1), `identifier_s` and `listen_first`; `request_break` (§5.2); `set_identifiers` and `identifier_owed` (new, §5.4); `on_preamble` changes nothing in a session (calls and probes keep it as carrier information for their own next try) | `set_chat` (open windows), `acknowledgement_at` (the follow-on heed, ADR-0040), the turn's state, `transmitting` as a turn notion |

**Events kept:**

* `connected` and `disconnected`, with the same details;
* `probe`, `probed` and `ignored`, with the same texts, which the daemon parses.

`role` events become `master`/`slave` once, at connection. `accepted` and `acks` have no
counterpart.

**Stats kept:** frames sent, resent, received, failed; HARQ rescues; bytes acked and
delivered; probes. **New:** `silent_slots`, `forfeits` (decoded too late), `dropped` (the
gate), `level` (the gap level in use). The turn's counters go.

## 9. Step 1's tests

These are model tests, strict, in `model/tests/test_granted.py`.

* **I1, on the air.** `TwoStationSim` gains the review simulator's station model: keying lead
  and jitter, decode latency with a tail, phantoms (some trusted), the gate's `not_after`, and
  a record of radiated intervals. Every scenario of `research/link_arch/scenarios.py` and a
  randomised sweep assert that no two radiated intervals overlap.
* **I2–I4.** Exact once-only delivery under loss, late decodes and duplicates. An old ANS,
  replayed, acknowledges nothing new.
* **L1–L5:**
  * every session either delivers or ends;
  * a slow slave reaches level 2 and completes;
  * a call reaches a slave that decodes 1.35 s late.
* **The interface:** every method in §8's kept column, exercised. The Test ladder runs.
* **Benches** (`bench_link.py`, `bench_chat.py`): against protocol 7 on the calibrated pipe,
  every class, −6…+12 dB, both airs.
  * The pipe charges no keying, so it measures the header and slot cost only.
  * The criterion is field ×1.10 ± 0.05 on the review's simulator, which does charge keying,
    and no completion lost on the pipe.

## 10. What the daemon must do (Step 2)

Listed here so the model's interface already fits. Built in Step 2
([09-step-2.md](09-step-2.md) §2–§3), but for the capture path (§4 there):

* **The gate.** Before keying, the daemon drops a `Transmit` whose `not_after` has passed,
  after every hold (busy, answer gap, regulatory, CAT). It logs the drop and tells the engine
  (`on_tx_dropped`).
* **`on_tx_done(frames_end)`.** The end of the last frame on the air, from the card's clock
  less the tail.
* **The identifier, as §5.4:** the engine places it, and the daemon renders it after the
  frames (ADR-0075; first, before it).
* **Rendering ahead.** The engine decides a slot's answer as soon as what it says is final —
  every data frame the header stated has been reported, decoded or not; an answer to a call, a
  probe or a DISC at once — and hands it over with `not_before`, the slot's start. The daemon
  renders it at once and keys it to radiate there. Decided about 0.2 s before the last moment
  (median, the model's simulator), which is the render time four tone frames need (§11). A
  station whose decode, render and keying still exceed the constants asks for a level.
* **The capture path.** Received frames are placed on the air's clock: the card's reported input
  latency is taken off their times.
* **The answer gap becomes the declared turnaround.** `[radio] turnaround_ms` is the operator's
  radio, declared in the call and the acceptance. A configuration schema step (12).
* **The answer gap stays, for a peer slower than it declares** (a VOX interface whose hold the
  operator left at 0). `[radio] answer_gap_ms`, or the gap learned for that station (ADR-0054)
  when longer, is handed to the engine (`set_answer_gap`): as the master it keys no sooner than
  the gap after the slave's frames; as the slave it asks for the level that starts its slots the
  gap after the master's. An acceptance cannot honour it: its slot is the one the call granted.
  A caller's own declared turnaround covers that slot from the first try, and an unanswered
  caller steps its call's level every second try.
* **No session transmission waits for the channel.** `channel_clear` already passes them. The
  ADR-0022 holds on DISC and DISC_ACK go: a DISC_ACK goes in its slot.
* **Removed from the daemon:**
  * the follow-on heed (ADR-0040);
  * `answers_burst`;
  * `fragment_of_own_tx`'s role in the engine's input. The frames still go to the recording.
* **`LINK_PROTOCOL` 8** in the shell.

## 11. Measurements before the constants freeze

Decode latency per family is what `D_ORD` and `D_FLOOR` stand for. It is measured on the
instrumented harness, not assumed: the daemon's recording gives each frame's end and the moment
it was reported. The latency is taken under load (`--jobs 2`), with a stall-free machine and
with `80m-slow-machine-500`'s. The constants are set at the 99th percentile of a normal machine.
Slower ones use levels.

The same runs give the keying latency's distribution, from the tag's `waited_s` and the air
start. `KEY` and `SLACK` are set from it, with CAT keying measured on hardware in Step 4.

**Measured (2026-10-13, `08-steps-0-and-1.md` §1.3).** Over 11 895 frames of the Step 0 runs, both
families on both airs, a frame was reported at most 0.157 s after its end, the 99th percentile
0.156 s; a slow machine's no later. The floor's figure is no larger than the ordinary one's: the
tone floor's frames are reported as soon as they end, and the 0.65 s was the OFDM floor family's
settling (ADR-0009 §8), a family the tone floor replaced. Both are 0.30 s. What the harness cannot
show:

* **Processing.** The harness runs in lockstep with the audio, so a slow pass costs no air time.
  The receiver runs at 0.07× real time on the replay of a session, its slowest pass 35 ms.
* **Rendering.** Measured outside the harness: a control frame 4–20 ms, six OFDM frames
  0.06–0.11 s, four tone frames 0.10–0.18 s, about twice that under load. `KEY`'s 0.35 s is the
  lead and the card, so the daemon renders a slot's data frames before the header that grants the
  slot is decoded, and only the answer after it (§10).
* **The capture path.** The harness delays only what a station plays. A real card delivers what
  it hears late too, so a station places received frames late by its input latency `c`, and a
  slave's slot, reckoned from the `E` it places, falls `c` late on the air: its start inside
  `SLACK`, its end inside the master's wait past `end_max` (`D + GUARD`, 0.35 s). Cards are
  10–100 ms, and the daemon takes the latency the card reports off received frames' times (§10).

## 12. Decided while building the model engine

`aether_model.link.granted.GrantedEngine` (Step 1) settled what the sections above left open,
each found by a test or a scenario and each kept inside the rules of ADR-0069 §2:

* **Three or more** (§4.2). A data frame's count saturates at 3, so only counts 0–2 anchor a
  transmission whose header was not read.
* **Idle keepalive.** An open window nobody needs is silent, and a master that hears only
  silence cannot tell an idle link from a dead one: idle sessions timed out. Once the master has
  heard nothing for `keepalive_s` (10 s, protocol 7's poll interval) its idle header grants an
  answer slot instead of an open window.
* **Where a slave's data can go.** A header's `rung` both recommends the slave's rung and sizes
  its grant, so a slave held to the tone floor by its ceiling or a pinned rung was granted
  ordinary frames its own never fitted, and its data never went. An answer with no data after
  it uses `data_floor` to say its data can go only on the floor; the master then recommends,
  and sizes grants by, at most the floor's top rung. With data after it, the bit keeps its
  meaning.
* **The acceptance states the slave's level.** Its capability bits 3–4, zero in 07's first
  draft, carry the level the called station's measurement of the call asks for, so the first
  header's slot is already wide enough for a slow called station.
* **A frame where none can be.** A false detection that does not decode is part of a
  transmission only where the transmission's opening control frame says its data frames are:
  the family, the slots, the count (or, without it, in line with a frame of it that decoded).
  Counted in, phantoms stepped the rate down the floor in the review's phantom storm — 735 s
  on one trial where the prototype took 413; the scenario's 16-run median is now 229 s, against
  the prototype's 278 (`08-steps-0-and-1.md` §2.3).
* **Only decoded content starts a transmission at the slave.** A frame that does not decode,
  heard after a transmission whose answer is still owed, is nobody's: the master keys only after
  the slot. Taken as the start of the next transmission, it cancelled the owed answer.
* **The master's measured margin.** The master measures how far past the slack the slave's
  decoded answers start and adds it to the wait for a slot nothing of which decoded. It only
  ever delays the master. It is for a station whose keying is slower than declared, which the
  daemon's gate cannot see after keying (the review's `ptt-jitter`).
* **A station never answers into a slot it granted.** A call, a probe or a left session's frame
  whose slot would fall before the last slot this station granted can close is not answered:
  the caller tries again.
* **The control family goes by several readings.** A station asks for ordinary control frames
  only when the last `family_readings` (3) frames it read of the other all cleared the margin
  (§7). One reading was biased: on a fading path an ordinary frame decodes only at a peak.
  Swept 1–4 on the review's campaign, 3 is the fewest that never completes less than protocol 7.
* **A header never costs data frames.** It is left out only when at most four data frames
  follow anyway. Capping the data at four to leave it out cost the 500 Hz air's 15-byte rung 136
  transmissions where protocol 7 took 82.
* **The send queue survives the start of a session** — what an idle station is handed goes in
  the next one — and is cleared at its end (ADR-0052).
* **A probe's level covers the prober's turnaround**, since a probe carries none.
* **`on_tx_delayed`** moves every wait for this station's own transmission (the master's next
  key, a call's retry, a probe's wait, a leaving DISC), never the link timeout or a slot
  answer: those are absolute, and a late answer is the gate's to drop.
* **The closing identifier.** A DISC grants its DISC_ACK's slot with the slave's identifier time
  whenever the slave declared one; the slave puts it ahead of the DISC_ACK. The master's goes
  ahead of its first DISC. (Superseded by ADR-0075, §5.4: behind the DISC_ACK, and the master's
  alone after the slot.)
* **The link timeout is checked when it falls due** against whole exchanges of the family the
  link runs in then. Armed when a frame is heard, it had counted exchanges of the family that
  frame was heard in, and an answer that moved the link to the floor left it a third as long
  as the rule says (`08-steps-0-and-1.md` §2.6).
* **The engine's draws are SplitMix64**, so the port draws the same numbers from a seed.
* **An answer is decided when it is final, and keyed at its slot** (§10, `not_before`). Deciding
  at the last moment left the daemon no time to render. On the review's campaign: no overlap in
  5 040 sessions, the field profile ×1.084 of protocol 7 where it was ×1.093, the hostile one
  1.5–3.5 % faster (the daemon's holds fall before the slot), one cell 1 % slower (hostile-c
  exchange).
