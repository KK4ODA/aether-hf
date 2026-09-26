# ADR-0027: In a chat, the receiving station asks for the turn — and the sender does not hand it over unasked

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`: `LinkConfig.chat`,
`LinkEngine.set_chat`, `LinkStats.turn_requests`), then the port (`aether-link`: the same, with
the model's tests mirrored and `TwoStationSim::send_at` for a line typed at a moment) and the
daemon (the host adapter's `CHAT ON` / `CHAT OFF` through `HostPresence.chat`; off when the
host program detaches; `counters.turn_requests`). The panel's own Send box does not turn it on
(the author's question, §8). No frame changes and no link protocol change: the request is an
acknowledgement, and a station without chat — an earlier version included — ignores it and
polls as before.

## 1. Context

VARA HF's published host command set includes `CHAT ON` / `CHAT OFF`, which its documentation
describes as optimising the modem's handover interchange for keyboard-to-keyboard use, as
against Winlink transfers. Aether's host adapter records the command — it decides whether a
KISS client's frames wait (ADR-0019) — and the link did nothing different. Nothing is known of
what VARA does with it and nothing of it is used here: this is designed from what a
keyboard-to-keyboard contact needs, and measured on Aether's own bench.

Today's turn-taking (ADR-0023): the sending station (ISS) sends bursts and the receiving
station (IRS) acknowledges each; an IRS with something to send says WANT_TX in its
acknowledgement, and the ISS hands over with a TURN when it has nothing more of its own, or
after `bursts_before_turn` bursts. An ISS with nothing to send polls every `keepalive_s`
(10 s), and that poll is the only moment the IRS may speak. So a line typed at the IRS waits
for the next poll — anywhere in a cycle of ten seconds plus the poll's own exchange — and then
for the poll, its acknowledgement and a TURN before its own burst goes. On the tone floor each
of those control frames is 3.2 s, and the idle cycle is about 16.7 s. On the bench (§2) a reply
at −6 dB on Moderate at 2 300 Hz took 23.6 s at the median, where a line from the station that
held the turn took 5.4 s — one frame.

In a chat that is nearly every line. The station that has just read a line is the one that
answers it, and the station that sent it still holds the turn: a reply is always typed at the
IRS.

## 2. The bench

`tools/bench_chat.py` runs the two engines of the link bench (`bench_link.py`'s fading pipe,
P9-6, with the tone floor's SNR reading capped at its class's ceiling, ADR-0016, and bursts held
to the daemon's key-time limit, ADR-0017) through a keyboard-to-keyboard contact. The script is
seeded by the trial alone, so every policy, class and SNR runs the same conversation: A calls B,
and once both are up the two exchange 10–20 lines of 20–120 bytes, each typed 5–30 s after the
line before it was delivered, by the other station or — one time in five — by the same station
again, before the reply. A line is typed only once the one before has arrived, so each line's
latency is its own: from `send()` to the delivery of its last byte at the other station. The
cost is keyed time — both transmitters' air time, every frame, from the first line typed to the
last delivered; a session the link gives up on before its last line is a drop.

The runs, all in `bench/baselines/chat_handover.csv`: every candidate at −12, −6, 0, +6 and
+12 dB on AWGN, Good, Moderate and Poor, at 2 300 and 500 Hz, 30 sessions a point (`first_trial`
0), with the adopted policy without its poll hold on the same grid; today's policy, the adopted
one and the four nearest candidates at −12, −6 and 0 dB on the fading classes, 100 fresh
sessions a point (`first_trial` 100), because at 30 a point the drop counts — one to nine —
could not tell the candidates apart; and a 3 kB file sent as the middle line (`file_bytes`
3000) on Moderate. The grid takes about ten minutes on four cores, the drop run twenty.

## 3. The candidates

1. **Today's** turn-taking.
2. **Hand over when done**: a sender whose line is acknowledged and whose queue is empty sends a
   TURN at once, unasked — the station likely to answer holds the turn when its operator starts
   typing. Also only while the sender's control frames are ordinary ones (on the tone floor a
   TURN and its confirmation are three frames of 3.2 s).
3. **A shorter idle poll** (5 s), so the IRS is asked sooner.
4. **The receiving station asks**: an IRS with something to send sends an acknowledgement nobody
   asked for, with WANT_TX, as soon as the channel is quiet; an idle ISS in a chat takes it as
   it takes the answer to a poll, and — with nothing of its own — hands over.
5. Combinations, and **asking with a longer idle poll** (20 s): once the IRS asks, a poll no
   longer carries its wish to send, only the news that the link is alive.

## 4. What was measured

**Latency and keyed time**, 30 sessions a point: the median and 90th-percentile latency of every
line, in seconds, and the keyed seconds per line (both stations); sessions dropped, where any,
in brackets.

2 300 Hz:

| Class | SNR dB | today | the station asks (adopted) | asks, and the sender hands over (rejected) |
|---|---|---|---|---|
| AWGN | −12 | 31.9 / 47.9 · 32.9 | 22.8 / 42.8 · 30.9 | 16.1 / 33.2 · 29.1 |
| AWGN | −6 | 21.3 / 28.4 · 21.3 | 12.3 / 18.9 · 19.3 | 8.8 / 17.1 · 20.7 |
| AWGN | 0 | 7.6 / 12.9 · 4.9 | 3.0 / 4.4 · 4.5 | 2.1 / 4.1 · 5.3 |
| AWGN | +6 | 6.8 / 12.0 · 3.8 | 2.0 / 3.4 · 3.5 | 1.1 / 3.0 · 3.9 |
| AWGN | +12 | 6.3 / 11.5 · 3.3 | 2.0 / 2.5 · 3.0 | 1.1 / 2.0 · 3.6 |
| Good | −12 | 40.9 / 65.0 · 43.9 (1) | 33.2 / 55.1 · 41.0 (1) | 28.3 / 52.7 · 40.2 (4) |
| Good | −6 | 24.5 / 38.7 · 25.5 (1) | 17.1 / 28.2 · 22.4 (2) | 11.6 / 24.6 · 24.3 |
| Good | 0 | 12.1 / 24.7 · 11.9 | 7.2 / 17.2 · 11.9 | 5.4 / 13.3 · 13.7 |
| Good | +6 | 7.2 / 12.8 · 4.3 | 3.0 / 4.5 · 3.9 | 2.1 / 4.1 · 4.5 |
| Good | +12 | 6.6 / 11.8 · 3.6 | 2.0 / 3.4 · 3.2 | 1.1 / 3.1 · 4.0 |
| Moderate | −12 | 34.3 / 53.1 · 36.5 | 27.9 / 42.9 · 34.2 | 22.5 / 40.8 · 35.1 |
| Moderate | −6 | 22.0 / 29.8 · 22.4 | 14.9 / 20.8 · 20.2 | 10.7 / 20.4 · 22.4 |
| Moderate | 0 | 11.2 / 22.4 · 10.7 | 6.9 / 17.1 · 10.6 | 5.4 / 11.8 · 12.4 |
| Moderate | +6 | 7.2 / 12.5 · 4.3 | 3.0 / 4.1 · 3.9 | 2.0 / 4.1 · 4.4 |
| Moderate | +12 | 6.5 / 11.7 · 3.5 | 2.0 / 3.1 · 3.1 | 1.1 / 3.1 · 3.8 |
| Poor | −12 | 32.0 / 49.3 · 34.1 | 25.7 / 42.8 · 32.0 | 21.5 / 36.4 · 33.2 |
| Poor | −6 | 22.2 / 31.1 · 22.9 | 14.9 / 22.5 · 21.0 | 10.7 / 20.2 · 22.4 |
| Poor | 0 | 11.0 / 21.0 · 9.3 | 6.1 / 14.9 · 9.0 | 5.3 / 10.7 · 10.7 |
| Poor | +6 | 7.2 / 12.4 · 4.1 | 3.0 / 4.1 · 3.8 | 1.7 / 3.2 · 4.2 |
| Poor | +12 | 6.4 / 11.6 · 3.5 | 2.0 / 3.1 · 3.2 | 1.1 / 3.1 · 4.0 |

500 Hz:

| Class | SNR dB | today | the station asks (adopted) | asks, and the sender hands over (rejected) |
|---|---|---|---|---|
| AWGN | −12 | 31.9 / 47.9 · 33.4 | 22.9 / 42.8 · 31.4 | 18.0 / 33.2 · 30.7 |
| AWGN | −6 | 26.8 / 37.2 · 26.6 | 17.6 / 27.9 · 24.6 | 11.6 / 22.8 · 24.4 |
| AWGN | 0 | 11.4 / 17.8 · 8.0 | 6.5 / 10.4 · 7.6 | 4.2 / 9.3 · 6.8 |
| AWGN | +6 | 7.8 / 13.1 · 4.7 | 3.0 / 5.1 · 4.4 | 2.1 / 4.1 · 4.6 |
| AWGN | +12 | 6.7 / 11.9 · 3.8 | 2.0 / 3.2 · 3.5 | 1.1 / 3.0 · 3.9 |
| Good | −12 | 40.6 / 64.7 · 43.8 (1) | 33.2 / 55.6 · 41.7 | 28.6 / 52.0 · 40.2 (6) |
| Good | −6 | 28.7 / 43.3 · 28.0 (3) | 22.5 / 33.9 · 26.9 (2) | 16.1 / 28.5 · 26.4 (4) |
| Good | 0 | 17.7 / 32.6 · 17.1 (1) | 14.7 / 27.4 · 17.7 (1) | 9.0 / 21.1 · 17.0 (1) |
| Good | +6 | 9.7 / 16.3 · 7.0 | 4.6 / 10.7 · 6.5 | 3.2 / 8.1 · 6.8 |
| Good | +12 | 7.3 / 12.8 · 4.4 | 3.0 / 5.1 · 4.1 | 2.1 / 4.2 · 4.6 |
| Moderate | −12 | 34.5 / 53.1 · 36.8 | 27.9 / 43.0 · 34.5 | 24.5 / 41.6 · 36.4 |
| Moderate | −6 | 26.9 / 37.7 · 27.5 | 18.9 / 27.9 · 25.2 | 14.9 / 25.8 · 26.6 |
| Moderate | 0 | 21.0 / 35.4 · 20.5 | 17.1 / 27.9 · 19.6 | 10.7 / 23.1 · 20.4 (1) |
| Moderate | +6 | 9.8 / 17.3 · 7.4 | 5.1 / 10.4 · 6.8 | 3.2 / 7.2 · 6.2 |
| Moderate | +12 | 7.3 / 12.6 · 4.4 | 3.0 / 5.1 · 4.0 | 2.1 / 4.2 · 4.5 |
| Poor | −12 | 32.3 / 49.3 · 34.8 | 26.8 / 42.8 · 32.9 | 22.5 / 36.4 · 34.6 |
| Poor | −6 | 26.8 / 37.6 · 27.1 | 18.6 / 27.9 · 25.1 | 14.9 / 25.7 · 26.2 |
| Poor | 0 | 17.3 / 32.9 · 16.7 (1) | 12.8 / 26.1 · 16.0 (1) | 9.8 / 22.5 · 17.2 (2) |
| Poor | +6 | 8.8 / 15.0 · 6.0 | 4.1 / 7.7 · 5.6 | 3.2 / 6.3 · 6.0 |
| Poor | +12 | 7.1 / 12.4 · 4.1 | 3.0 / 4.1 · 3.8 | 2.1 / 4.1 · 4.4 |

Over the fading classes, both airs and every SNR, as a ratio to today (the median of the 30
points, and their range):

| Candidate | median latency | 90th percentile | keyed per line | sessions dropped of 900 |
|---|---|---|---|---|
| today | 1 | 1 | 1 | 8 |
| the station asks (adopted) | 0.64 (0.30–0.83) | 0.72 (0.26–0.87) | 0.93 (0.88–1.03) | 7 |
| asks, with a 20 s idle poll | 0.66 (0.30–0.83) | 0.74 (0.26–0.89) | 0.86 (0.74–0.95) | 7 |
| hands over when done | 0.48 (0.17–0.78) | 0.75 (0.66–0.86) | 1.02 (0.92–1.18) | 19 |
| hands over, polls every 5 s | 0.50 (0.17–0.78) | 0.67 (0.48–0.88) | 1.13 (1.00–1.44) | 12 |
| asks, and hands over | 0.48 (0.17–0.71) | 0.59 (0.26–0.81) | 1.00 (0.84–1.17) | 18 |
| asks, and hands over on ordinary frames only | 0.55 (0.17–0.83) | 0.73 (0.26–0.87) | 1.02 (0.84–1.23) | 6 |
| asks, with a 20 s poll, and hands over | 0.47 (0.17–0.69) | 0.55 (0.26–0.83) | 0.94 (0.78–1.14) | 31 |
| polls every 5 s | 0.84 (0.68–0.97) | 0.89 (0.64–0.98) | 1.12 (1.05–1.31) | 8 |

**Sessions lost**, 100 fresh sessions a point at −12, −6 and 0 dB on Good, Moderate and Poor,
both airs (1 800 sessions a policy):

| Policy | sessions dropped of 1 800 | lines lost |
|---|---|---|
| today | 15 (0.8 %) | 126 of 27 612 (0.46 %) |
| the station asks (adopted) | 13 (0.7 %) | 104 (0.38 %) |
| asks, without the sender's hold | 16 (0.9 %) | 138 (0.50 %) |
| asks, with a 20 s idle poll | 11 (0.6 %) | 114 (0.41 %) |
| asks, and hands over | 66 (3.7 %) | 464 (1.68 %) |
| asks, and hands over on ordinary frames only | 25 (1.4 %) | 163 (0.59 %) |

Where they were lost (the points with any drop; sessions of 100):

| Air, class, SNR | today | asks (adopted) | no hold | 20 s poll | hands over | ordinary only |
|---|---|---|---|---|---|---|
| 2 300 Good −12 dB | 1 | 0 | 1 | 4 | 24 | 0 |
| 2 300 Good −6 dB | 1 | 0 | 0 | 0 | 1 | 0 |
| 2 300 Good 0 dB | 0 | 0 | 0 | 0 | 0 | 1 |
| 2 300 Moderate −12 dB | 0 | 0 | 0 | 0 | 1 | 0 |
| 2 300 Poor 0 dB | 0 | 0 | 0 | 0 | 0 | 1 |
| 500 Good −12 dB | 2 | 0 | 2 | 2 | 22 | 0 |
| 500 Good −6 dB | 3 | 4 | 3 | 3 | 2 | 1 |
| 500 Good 0 dB | 4 | 4 | 4 | 1 | 3 | 4 |
| 500 Moderate −12 dB | 0 | 0 | 0 | 0 | 1 | 0 |
| 500 Moderate 0 dB | 1 | 2 | 2 | 0 | 5 | 8 |
| 500 Poor 0 dB | 3 | 3 | 4 | 1 | 7 | 10 |

The adopted policy's drops are today's: the same slow Good fades, and the poll-wait fault of
§7 (1), which ended sessions under every policy — traced in today's (500 Hz, Good, −6 dB) and
in the adopted one's (500 Hz, Moderate, 0 dB, the spiral already running when the line was
typed).

The handover's losses are its own. Traced (`handover`, 2 300 Hz, Good, −12 dB): the sender hands
over on the tone floor, the other station takes the turn and polls to confirm it; the first
station misses the poll — undecodable, it answers the preamble late (§7, 1) — repeats its TURN,
and the new sender's re-polls and the old one's TURNs, 3.2 s frames every 7–9 s, collide time
after time; the old sender "carries on as ISS" after three tries while the other already holds
the turn (§7, 2), sends its next line as a 27 s burst over the other's polls, and the other gives
up — "no response". Every handover is three frames that must all arrive, and a chat hands over
after every line. Kept to ordinary control frames, the losses move to where those frames are
marginal: the 500 Hz air at 0 dB.

## 5. Decision

**In a chat the receiving station asks for the turn** (`LinkConfig.chat`, off by default;
`LinkEngine.set_chat(on)` for the host's `CHAT ON` / `CHAT OFF` in the middle of a session):

* **When.** An IRS with something to send — queued, or frames in flight — whose last
  acknowledgement did not already say WANT_TX asks once the channel is quiet: not while a burst
  is arriving or its acknowledgement is due (that acknowledgement asks), not while it
  transmits, and not until the other station's answer to its last transmission, or to the last
  frame it heard, could have announced itself (`_reaction_s`: a turnaround, the tone floor's
  announcement time, the detection latency and the reply margin — 1.34 s with preamble reports,
  the longest frame without). A line typed earlier waits for that moment on a timer
  (`request`); a frame heard arriving meanwhile takes its place.
* **What.** An acknowledgement nobody asked for, with WANT_TX: the station's receive window as
  it stands (so it is a true acknowledgement too), the SNR of the last frame it decoded
  (ADR-0021) and its recommendation. Counted in `LinkStats.turn_requests`.
* **Once.** A request is not repeated: if it is lost, the next poll's acknowledgement asks, as
  today.
* **The sender's side.** An idle ISS in a chat takes the request as it takes the answer to a poll
  and, with nothing of its own, hands over at once (the existing rule); and it does not poll
  over a frame it hears arriving (its keepalive moves past the frame). A poll keyed over a
  request loses both: without the hold (`request-without-hold`) a line on the tone floor, at −12
  and −6 dB, took up to 5 s longer at the median and 7 s at the 90th percentile, with 1.5 s
  more keyed time a line; above the floor it makes no difference.

On the fading classes from −12 to +12 dB, on both airs, it cuts a line's median time by 17–70 %
and its 90th percentile by 13–74 %, with 12 % less to 3 % more keyed time a line and no more
sessions lost. The replies gain it all — their median falls by 15–73 % — and a line from the
station that holds the turn goes as it did (its median unchanged at most points, within 3 s
either way). Polls a line halve (at −6 dB on Moderate, 2 300 Hz, 1.30 → 0.65), which is where
the keyed time goes: a reply's request and TURN replace the poll, its answer and the TURN, and
the conversation is shorter.

Transfers are untouched. `bench_link.py`'s sessions (2 kB, both airs, the four classes, −12 to
+12 dB, three a point on the fading pipe with the floor cap) are identical in every column with
chat on; a 3 kB file sent inside a chat while the receiving station types a line arrives exactly
when it does without chat (`test_chat_leaves_a_transfer_alone`) — a request is never sent into
a transfer, since the receiving station is always hearing a burst or about to acknowledge one,
and its acknowledgements ask as they always did; and on the bench a 3 kB file sent as a line of
the conversation (Moderate, −6 to +6 dB) took from 18 % less to 5 % more time than today, the
spread of the fades it happened to meet (at 2 300 Hz −6 dB, 197.8 s both ways).

## 6. Rejected

* **Handing the turn over unasked** (candidate 2, and with the request): the fastest — a reply
  typed at the station that now holds the turn goes at once, 1.1 s on a strong path against the
  request's 2.0 — but with the request it lost four times today's sessions (66 of 1 800 against
  15; 22–24 of 100 at −12 dB on Good), and kept to ordinary control frames still 25, most at
  500 Hz and 0 dB where those frames are marginal (8–10 of 100 on Moderate and Poor). A second
  line from the same station must then get the turn back, and without the request waits for
  the poll as replies do today. The bench keeps it (`tools/bench_chat.py`, `Candidate`): §7's
  timing faults are what made its handshakes fail, and with them mended it may be worth
  measuring again.
* **A shorter idle poll** (5 s): less than half the request's gain (the median ×0.84 against
  ×0.64) for an eighth more keyed time.
* **A longer idle poll with the request** (20 s): about the same latency and 7 % less keyed time
  than the request alone, and 11 sessions lost of 1 800 against 13 — but 4 of 100 at 2 300 Hz,
  Good, −12 dB, where the request alone lost none. The idle poll is also the link's heartbeat:
  a lost request waits longer for the poll that makes up for it, and a dead link is found later.
  A decision of its own, if field sessions show the idle keying matters.
* **Predicting the sender's poll**, so a request is never keyed into one about to start: with
  the sender's hold (§5) the two collide only when one starts within a preamble's detection of
  the other — about one request in fifteen on the floor, and each costs one poll's exchange.
  Not worth tying the receiving station to the other station's keepalive setting.

## 7. Found on the way (not changed: each changes the default behaviour)

1. **A poll's answer can outlast the poll's wait.** `_send_poll` waits for an answer that
   starts within a turnaround, the detection latency and the reply margin of the poll's end
   (`_response_wait(_reply_control_s())`: 0.8 s), but a receiving station that hears the
   poll's preamble and cannot decode it answers when its end-of-burst quiet runs out
   (`on_preamble`'s acknowledgement deadline, `_irs_reply_delay()`: the announcement time, the
   burst gap and a turnaround — 0.99 s after a floor frame). An ordinary poll the receiving
   station cannot decode, answered on the floor, is the case: the 3.2 s acknowledgement ends
   0.19 s after the sender has re-polled, and every repeat collides with the next answer until
   the sender gives up — "no response". It ended sessions under every policy, today's included
   (traced: 500 Hz, Good, −6 dB).
2. **A TURN whose confirmation is lost can leave both stations holding the turn.** The station
   that took the turn ignores the TURN repeated (a TURN is taken only by an IRS or a station
   waiting for one), the one that sent it "carries on as ISS" after `turn_retries`, and
   ADR-0023's yield needs the called station to hear the caller's poll, which it cannot while it
   sends a long burst.
3. **A sender repeats a poll or a burst over a frame it hears arriving.** Calls, probes, DISCs
   and TURNs wait for such a frame to end (ADR-0016, ADR-0022, ADR-0023); `_on_response_timeout`
   does not.
4. **Not a fault, a cost: every reply starts low.** A turn's first burst goes out at
   `first_mode` of the station's own reading of the other (`_take_iss`, P9-2) — two rungs under
   what it fits, and on the floor if all it has read is the call, whose tone-floor reading is a
   lower bound (ADR-0016). In a chat every reply is a new turn's first burst: traced at −6 dB
   on Good, a 39-byte reply went as two tone-24 frames (10.7 s) where the other station's
   recommendation, one acknowledgement later, was rung 7 (OFDM). The TURN has a
   recommended-mode field it does not use; carrying the TURN's sender's recommendation there
   would be a protocol change, and a question for the rate controller rather than for
   turn-taking.

## 8. Consequences

* The model: `LinkConfig.chat`, `LinkEngine.set_chat`, `LinkStats.turn_requests`, the `request`
  timer, and the idle sender's acceptance and hold — all inert with chat off. Tests: a line at
  the receiving station goes without waiting for a poll (10.4 s → 2.0 s on the test's path); a
  request waits out the reaction time and is not sent when a frame arrives in it, and is sent
  once; an idle sender hands over on one and holds its poll over a frame arriving (neither
  without chat); two lines typed at once both arrive; a chat station talks to one without chat
  either way round; `set_chat(False)` cancels a pending request; a transfer inside a chat is
  unchanged. `tools/bench_chat.py` and `bench/baselines/chat_handover.csv`.
* The port mirrors the model (`LinkConfig::chat`, `set_chat`, `turn_requests`, the timer, the
  acceptance and the hold) with the same tests; the request is an ACK in the existing format, so
  the link vectors do not change. The daemon calls `set_chat` from the host adapter's `CHAT ON` /
  `CHAT OFF` (`HostFlags.chat`), and turns it off when the host program detaches; the request
  goes through the regulatory gate and the busy hold like any frame of the session (a busy hold
  is one more guard against keying it over the sender's poll). Whether the panel's own Send box
  — a keyboard-to-keyboard chat too — should turn it on is an open question for the author.
* A chat station with a peer that is not in chat, or of an earlier version, sends one control
  frame per line that nobody takes; the line goes at the next poll, as it always did.
* A reply costs two control frames — the request and the TURN — where today it costs three —
  the poll, its answer and the TURN; with the poll's wait gone the conversation is shorter, and
  so is the idle polling in it.
* Owed: the air. VarAC at both ends with `CHAT ON` will say whether the bench's think times and
  the collision rate between a request and a poll hold there.
